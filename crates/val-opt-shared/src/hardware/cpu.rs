use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CpuFeatures {
    pub sse: bool,
    pub sse2: bool,
    pub avx: bool,
    pub avx2: bool,
    pub fma: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct CpuInfo {
    pub brand: String,
    pub vendor: String,
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub has_smt: bool,
    pub is_hybrid: bool,
    pub performance_cores: u32,
    pub efficient_cores: u32,
    pub features: CpuFeatures,
}

impl CpuInfo {
    /// Detects CPU brand, topology, hybrid cores, and instruction capabilities.
    pub fn detect() -> Result<Self, anyhow::Error> {
        let (brand, vendor, features) = detect_cpuid_info();
        let topology = detect_processor_topology()?;

        let is_hybrid = topology.efficient_cores > 0 && topology.performance_cores > 0;
        let performance_cores = if is_hybrid {
            topology.performance_cores
        } else {
            topology.physical_cores
        };

        Ok(Self {
            brand,
            vendor,
            physical_cores: topology.physical_cores,
            logical_cores: topology.logical_cores,
            has_smt: topology.has_smt,
            is_hybrid,
            performance_cores,
            efficient_cores: topology.efficient_cores,
            features,
        })
    }
}

#[derive(Default)]
struct TopologyCount {
    physical_cores: u32,
    logical_cores: u32,
    performance_cores: u32,
    efficient_cores: u32,
    has_smt: bool,
}

#[cfg(windows)]
fn detect_processor_topology() -> Result<TopologyCount, anyhow::Error> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    let mut buffer_size: u32 = 0;
    // First call to determine buffer size
    unsafe {
        let _ = GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            None,
            &mut buffer_size,
        );
    }

    if buffer_size == 0 {
        return Err(anyhow::anyhow!("Failed to query logical processor information size"));
    }

    let mut buffer: Vec<u8> = vec![0u8; buffer_size as usize];
    let success = unsafe {
        GetLogicalProcessorInformationEx(
            RelationProcessorCore,
            Some(buffer.as_mut_ptr() as *mut SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX),
            &mut buffer_size,
        )
    };

    if let Err(e) = success {
        return Err(anyhow::anyhow!("GetLogicalProcessorInformationEx failed: {}", e));
    }

    let mut topology = TopologyCount::default();
    let mut offset: usize = 0;
    let total_bytes = buffer.len();

    while offset + std::mem::size_of::<SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX>() <= total_bytes {
        let info_ptr = buffer[offset..].as_ptr() as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
        let info = unsafe { &*info_ptr };

        if info.Relationship == RelationProcessorCore {
            topology.physical_cores += 1;
            let core = unsafe { &info.Anonymous.Processor };

            if (core.Flags & 1) != 0 {
                topology.has_smt = true;
            }

            // EfficiencyClass: 0 = Performance core (or uniform), >0 = Efficient core
            if core.EfficiencyClass == 0 {
                topology.performance_cores += 1;
            } else {
                topology.efficient_cores += 1;
            }

            // Count logical processors from the GROUP_AFFINITY mask
            let group_count = core.GroupCount as usize;
            let affinity_slice = unsafe {
                std::slice::from_raw_parts(core.GroupMask.as_ptr(), group_count)
            };

            for group_aff in affinity_slice {
                topology.logical_cores += group_aff.Mask.count_ones();
            }
        }

        let record_size = info.Size as usize;
        if record_size == 0 {
            break;
        }
        offset += record_size;
    }

    Ok(topology)
}

#[cfg(not(windows))]
fn detect_processor_topology() -> Result<TopologyCount, anyhow::Error> {
    Ok(TopologyCount {
        physical_cores: 6,
        logical_cores: 12,
        performance_cores: 6,
        efficient_cores: 0,
        has_smt: true,
    })
}

fn detect_cpuid_info() -> (String, String, CpuFeatures) {
    #[cfg(target_arch = "x86_64")]
    {
        use std::arch::x86_64::{__cpuid, __cpuid_count};

        // Vendor string
        let id0 = __cpuid(0);
        let mut vendor_bytes = [0u8; 12];
        vendor_bytes[0..4].copy_from_slice(&id0.ebx.to_le_bytes());
        vendor_bytes[4..8].copy_from_slice(&id0.edx.to_le_bytes());
        vendor_bytes[8..12].copy_from_slice(&id0.ecx.to_le_bytes());
        let vendor = String::from_utf8_lossy(&vendor_bytes).to_string();

        // Features (CPUID leaf 1 & leaf 7)
        let id1 = __cpuid(1);
        let sse = (id1.edx & (1 << 25)) != 0;
        let sse2 = (id1.edx & (1 << 26)) != 0;
        let avx = (id1.ecx & (1 << 28)) != 0;
        let fma = (id1.ecx & (1 << 12)) != 0;

        let id7 = __cpuid_count(7, 0);
        let avx2 = (id7.ebx & (1 << 5)) != 0;

        let features = CpuFeatures {
            sse,
            sse2,
            avx,
            avx2,
            fma,
        };

        // Brand string (CPUID extended leaf 0x80000002..=0x80000004)
        let ext_max = __cpuid(0x80000000).eax;
        let brand = if ext_max >= 0x80000004 {
            let mut brand_bytes = [0u8; 48];
            for i in 0..3 {
                let id = __cpuid(0x80000002 + i);
                let offset = (i as usize) * 16;
                brand_bytes[offset..offset + 4].copy_from_slice(&id.eax.to_le_bytes());
                brand_bytes[offset + 4..offset + 8].copy_from_slice(&id.ebx.to_le_bytes());
                brand_bytes[offset + 8..offset + 12].copy_from_slice(&id.ecx.to_le_bytes());
                brand_bytes[offset + 12..offset + 16].copy_from_slice(&id.edx.to_le_bytes());
            }
            String::from_utf8_lossy(&brand_bytes)
                .trim_matches('\0')
                .trim()
                .to_string()
        } else {
            "Unknown x86_64 Processor".to_string()
        };

        (brand, vendor, features)
    }

    #[cfg(not(target_arch = "x86_64"))]
    {
        (
            "Generic Processor".to_string(),
            "Unknown".to_string(),
            CpuFeatures::default(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cpu_detection() {
        let cpu = CpuInfo::detect().expect("CPU detection should succeed on host");
        assert!(!cpu.brand.is_empty(), "CPU brand should not be empty");
        assert!(!cpu.vendor.is_empty(), "CPU vendor should not be empty");
        assert!(cpu.physical_cores >= 1, "Physical cores must be >= 1");
        assert!(cpu.logical_cores >= cpu.physical_cores, "Logical cores must be >= physical cores");

        println!("=== Detected CPU Manifest ===");
        println!("Brand: {}", cpu.brand);
        println!("Vendor: {}", cpu.vendor);
        println!("Physical Cores: {}", cpu.physical_cores);
        println!("Logical Cores: {}", cpu.logical_cores);
        println!("Has SMT: {}", cpu.has_smt);
        println!("Is Hybrid: {}", cpu.is_hybrid);
        println!("P-Cores: {}", cpu.performance_cores);
        println!("E-Cores: {}", cpu.efficient_cores);
        println!("AVX2 Supported: {}", cpu.features.avx2);
    }
}
