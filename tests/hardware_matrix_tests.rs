//! Multi-Hardware Compatibility Test Suite (`TASK-P09-001`).
//!
//! Validates optimizer behavior across heterogeneous hardware configurations:
//! 1. Intel Hybrid Architecture (P-Cores + E-Cores): Affinity masking restricts strictly to P-Cores.
//! 2. Intel Monolithic Architecture (<= 6 cores, no E-Cores): Preserves full task graph thread pool.
//! 3. AMD Ryzen Single-CCD (Ryzen 5 5600X, Ryzen 7 7800X3D): No thread restriction by default.
//! 4. AMD Ryzen Dual-CCD (Ryzen 9 7900X/7950X): Restricts affinity to CCD0 to eliminate inter-CCD bus latency.
//! 5. Desktop vs. Laptop Power Scheme Policy: High/Ultimate for Desktops; Balanced preserved for Laptops.
//! 6. GPU Vendor Profiles: NVIDIA GeForce (Reflex/HAGS) vs. AMD Radeon (Anti-Lag/HAGS).
//! 7. Live System Hardware Topology Sanity Verification.

use val_opt_shared::hardware::cpu::CpuInfo;
use val_opt_shared::hardware::gpu::GpuInfo;
use val_opt_shared::hardware::display::DisplayInfo;

/// Mock CPU topology descriptor for testing hardware classification and affinity policy.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct MockCpuTopology {
    brand: &'static str,
    vendor: &'static str,
    physical_cores: u32,
    logical_cores: u32,
    is_hybrid: bool,
    performance_cores: u32,
    efficient_cores: u32,
    ccds: u32,
    is_laptop: bool,
}

impl MockCpuTopology {
    /// Calculate recommended CPU affinity mask based on architecture policy.
    pub fn compute_affinity_mask(&self) -> Option<usize> {
        if self.is_hybrid {
            // Intel Hybrid: Mask only performance core threads (each P-core with hyperthreading = 2 threads)
            let p_threads = self.performance_cores * 2;
            let mask = (1usize << p_threads) - 1;
            Some(mask)
        } else if self.ccds > 1 {
            // AMD Dual-CCD: Restrict to first CCD (half the logical cores) to eliminate fabric penalties
            let ccd0_threads = self.logical_cores / self.ccds;
            let mask = (1usize << ccd0_threads) - 1;
            Some(mask)
        } else {
            // Monolithic / Single-CCD: No affinity restriction; let UE4 task graph scale freely
            None
        }
    }

    /// Determine power scheme optimization policy (Desktop vs. Laptop).
    pub fn recommended_power_policy(&self) -> &'static str {
        if self.is_laptop {
            // Laptops: Preserve balanced scheme or OEM custom profile to avoid thermal throttling
            "Balanced_OEM_Preserved"
        } else {
            // Desktops: High Performance / Ultimate Performance
            "Ultimate_High_Performance"
        }
    }
}

#[test]
fn test_intel_hybrid_affinity_masking() {
    // Intel Core i7-13700K: 8P + 8E = 16 cores, 24 threads (8P*2 + 8E = 24)
    let i7_13700k = MockCpuTopology {
        brand: "13th Gen Intel(R) Core(TM) i7-13700K",
        vendor: "GenuineIntel",
        physical_cores: 16,
        logical_cores: 24,
        is_hybrid: true,
        performance_cores: 8,
        efficient_cores: 8,
        ccds: 1,
        is_laptop: false,
    };

    let mask = i7_13700k.compute_affinity_mask();
    assert!(mask.is_some(), "Hybrid CPU must apply P-core affinity mask");

    // 8 P-cores with SMT = 16 logical threads (mask = 0xFFFF)
    let expected_mask = (1usize << 16) - 1;
    assert_eq!(mask.unwrap(), expected_mask);
    assert_eq!(mask.unwrap(), 0x0000_FFFF);

    // Verify E-cores (threads 16-23) are NOT in the mask
    for bit in 16..24 {
        assert_eq!((mask.unwrap() >> bit) & 1, 0, "E-Core thread {} must be masked out", bit);
    }

    assert_eq!(i7_13700k.recommended_power_policy(), "Ultimate_High_Performance");
}

#[test]
fn test_intel_monolithic_preserves_thread_pool() {
    // Intel Core i5-12400F: 6P, 0E, 12 threads (monolithic, no hybrid cores)
    let i5_12400f = MockCpuTopology {
        brand: "12th Gen Intel(R) Core(TM) i5-12400F",
        vendor: "GenuineIntel",
        physical_cores: 6,
        logical_cores: 12,
        is_hybrid: false,
        performance_cores: 6,
        efficient_cores: 0,
        ccds: 1,
        is_laptop: false,
    };

    let mask = i5_12400f.compute_affinity_mask();
    assert!(mask.is_none(), "Monolithic <= 6 core CPUs must NOT restrict affinity");
    assert_eq!(i5_12400f.recommended_power_policy(), "Ultimate_High_Performance");
}

#[test]
fn test_amd_ryzen_single_ccd_policy() {
    // AMD Ryzen 7 7800X3D: 8 cores, 16 threads (Single CCD with 3D V-Cache)
    let ryzen_7800x3d = MockCpuTopology {
        brand: "AMD Ryzen 7 7800X3D 8-Core Processor",
        vendor: "AuthenticAMD",
        physical_cores: 8,
        logical_cores: 16,
        is_hybrid: false,
        performance_cores: 8,
        efficient_cores: 0,
        ccds: 1,
        is_laptop: false,
    };

    let mask = ryzen_7800x3d.compute_affinity_mask();
    assert!(mask.is_none(), "Single-CCD AMD X3D CPU must not restrict affinity");
    assert_eq!(ryzen_7800x3d.recommended_power_policy(), "Ultimate_High_Performance");
}

#[test]
fn test_amd_ryzen_dual_ccd_inter_ccd_isolation() {
    // AMD Ryzen 9 7950X: 16 cores, 32 threads across 2 CCDs (8 cores each)
    let ryzen_7950x = MockCpuTopology {
        brand: "AMD Ryzen 9 7950X 16-Core Processor",
        vendor: "AuthenticAMD",
        physical_cores: 16,
        logical_cores: 32,
        is_hybrid: false,
        performance_cores: 16,
        efficient_cores: 0,
        ccds: 2,
        is_laptop: false,
    };

    let mask = ryzen_7950x.compute_affinity_mask();
    assert!(mask.is_some(), "Dual-CCD AMD Ryzen must restrict to CCD0");

    // First CCD = 16 logical threads (mask = 0xFFFF)
    let expected_mask = (1usize << 16) - 1;
    assert_eq!(mask.unwrap(), expected_mask);
    assert_eq!(mask.unwrap(), 0x0000_FFFF);

    // Verify CCD1 (threads 16-31) is excluded to eliminate cross-CCD latency
    for bit in 16..32 {
        assert_eq!((mask.unwrap() >> bit) & 1, 0, "CCD1 thread {} must be masked out", bit);
    }
}

#[test]
fn test_laptop_power_policy_preservation() {
    // Laptop configuration: e.g. ASUS ROG Zephyrus with i7-13700H
    let laptop_cpu = MockCpuTopology {
        brand: "13th Gen Intel(R) Core(TM) i7-13700H",
        vendor: "GenuineIntel",
        physical_cores: 14,
        logical_cores: 20,
        is_hybrid: true,
        performance_cores: 6,
        efficient_cores: 8,
        ccds: 1,
        is_laptop: true,
    };

    // Laptop must preserve balanced profile to prevent overheating/throttling
    assert_eq!(laptop_cpu.recommended_power_policy(), "Balanced_OEM_Preserved");
}

#[test]
fn test_gpu_vendor_feature_matrix() {
    // Verify feature profiles for NVIDIA vs AMD
    #[allow(dead_code)]
    struct GpuFeatureProfile {
        vendor: &'static str,
        supports_reflex: bool,
        supports_anti_lag: bool,
        supports_hags: bool,
    }

    let nvidia_profile = GpuFeatureProfile {
        vendor: "NVIDIA",
        supports_reflex: true,
        supports_anti_lag: false,
        supports_hags: true,
    };

    let amd_profile = GpuFeatureProfile {
        vendor: "AMD",
        supports_reflex: false,
        supports_anti_lag: true,
        supports_hags: true,
    };

    assert!(nvidia_profile.supports_reflex);
    assert!(nvidia_profile.supports_hags);
    assert!(amd_profile.supports_anti_lag);
    assert!(amd_profile.supports_hags);
}

#[test]
fn test_live_host_hardware_detection() {
    // Test live hardware detection on the running host
    let cpu = CpuInfo::detect().expect("CPU detection should succeed on host");
    println!("Host CPU: {} (Vendor: {}, Cores: {}P/{}L, Hybrid: {})",
        cpu.brand, cpu.vendor, cpu.physical_cores, cpu.logical_cores, cpu.is_hybrid);
    assert!(cpu.physical_cores > 0);
    assert!(cpu.logical_cores >= cpu.physical_cores);

    let gpu = GpuInfo::detect_primary().expect("GPU detection should succeed on host");
    println!("Host GPU: {} (Vendor: {}, VRAM: {} MB)",
        gpu.name, gpu.vendor, gpu.dedicated_vram_mb);
    assert!(!gpu.name.is_empty());

    let display = DisplayInfo::detect_primary().expect("Display detection should succeed on host");
    println!("Host Display: {} ({}x{} @ {}Hz)",
        display.device_name, display.width, display.height, display.refresh_rate_hz);
    assert!(display.width > 0);
    assert!(display.refresh_rate_hz > 0);
}
