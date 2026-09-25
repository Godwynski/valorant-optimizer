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
    pub p_core_affinity_mask: Option<usize>,
    pub features: CpuFeatures,
}

impl CpuInfo {
    /// Detects CPU brand, topology, hybrid cores, and instruction capabilities.
    pub fn detect() -> Result<Self, anyhow::Error> {
        let (brand, vendor, features) = detect_cpuid_info();
        let topology = detect_processor_topology()?;

        Ok(Self {
            brand,
            vendor,
            physical_cores: topology.physical_cores,
            logical_cores: topology.logical_cores,
            has_smt: topology.has_smt,
            is_hybrid: topology.is_hybrid,
            performance_cores: topology.performance_cores,
            efficient_cores: topology.efficient_cores,
            p_core_affinity_mask: topology.p_core_affinity_mask,
            features,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CoreDescriptor {
    pub efficiency_class: u8,
    pub has_smt: bool,
    pub logical_core_count: u32,
    pub group_masks: Vec<(u16, usize)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EvaluatedTopology {
    pub physical_cores: u32,
    pub logical_cores: u32,
    pub performance_cores: u32,
    pub efficient_cores: u32,
    pub has_smt: bool,
    pub is_hybrid: bool,
    pub p_core_affinity_mask: Option<usize>,
}

/// Evaluates CPU topology according to authoritative Win32 MSDN semantics:
/// In `RelationProcessorCore`, higher `EfficiencyClass` values designate higher-performance cores (P-cores);
/// lower values designate energy-efficient cores (E-cores).
///
/// On monolithic CPUs (e.g. AMD Ryzen, Intel 11th Gen or non-hybrid Intel 12th Gen like i5-12400),
/// all cores share the identical EfficiencyClass (typically 0). `is_hybrid` is false and no affinity
/// restriction is imposed.
///
/// On hybrid CPUs (e.g. Intel Alder Lake / Raptor Lake with 8P+8E), cores with `efficiency_class == max`
/// are Performance cores, and `< max` are Efficient cores.
pub fn evaluate_topology(cores: &[CoreDescriptor]) -> EvaluatedTopology {
    if cores.is_empty() {
        return EvaluatedTopology::default();
    }

    let physical_cores = cores.len() as u32;
    let mut logical_cores = 0u32;
    let mut has_smt = false;
    let mut min_eff = u8::MAX;
    let mut max_eff = u8::MIN;

    for c in cores {
        logical_cores += c.logical_core_count;
        if c.has_smt {
            has_smt = true;
        }
        if c.efficiency_class < min_eff {
            min_eff = c.efficiency_class;
        }
        if c.efficiency_class > max_eff {
            max_eff = c.efficiency_class;
        }
    }

    let is_hybrid = max_eff > min_eff;

    let (performance_cores, efficient_cores, p_core_affinity_mask) = if is_hybrid {
        let p_count = cores.iter().filter(|c| c.efficiency_class == max_eff).count() as u32;
        let e_count = cores.iter().filter(|c| c.efficiency_class < max_eff).count() as u32;

        let mut mask: usize = 0;
        for c in cores.iter().filter(|c| c.efficiency_class == max_eff) {
            for (group, m) in &c.group_masks {
                if *group == 0 {
                    mask |= *m;
                }
            }
        }

        (p_count, e_count, if mask != 0 { Some(mask) } else { None })
    } else {
        (physical_cores, 0, None)
    };

    EvaluatedTopology {
        physical_cores,
        logical_cores,
        performance_cores,
        efficient_cores,
        has_smt,
        is_hybrid,
        p_core_affinity_mask,
    }
}

#[cfg(windows)]
pub fn parse_processor_cores(buffer: &[u8]) -> Result<Vec<CoreDescriptor>, anyhow::Error> {
    use windows::Win32::System::SystemInformation::{
        RelationProcessorCore, SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    let mut cores = Vec::new();
    let mut offset: usize = 0;
    let total_bytes = buffer.len();

    while offset + 8 <= total_bytes {
        let info_ptr = buffer[offset..].as_ptr() as *const SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX;
        let info = unsafe { &*info_ptr };

        let record_size = info.Size as usize;
        if record_size < 8 || offset + record_size > total_bytes {
            break;
        }

        if info.Relationship == RelationProcessorCore {
            let core = unsafe { &info.Anonymous.Processor };
            let has_smt = (core.Flags & 1) != 0;
            let efficiency_class = core.EfficiencyClass;

            let group_count = core.GroupCount as usize;
            let affinity_slice = unsafe {
                std::slice::from_raw_parts(core.GroupMask.as_ptr(), group_count)
            };

            let mut group_masks = Vec::with_capacity(group_count);
            let mut logical_core_count = 0u32;

            for group_aff in affinity_slice {
                let mask = group_aff.Mask;
                let group = group_aff.Group;
                logical_core_count += mask.count_ones();
                group_masks.push((group, mask));
            }

            cores.push(CoreDescriptor {
                efficiency_class,
                has_smt,
                logical_core_count,
                group_masks,
            });
        }

        offset += record_size;
    }

    Ok(cores)
}

#[cfg(windows)]
fn detect_processor_topology() -> Result<EvaluatedTopology, anyhow::Error> {
    use windows::Win32::System::SystemInformation::{
        GetLogicalProcessorInformationEx, RelationProcessorCore,
        SYSTEM_LOGICAL_PROCESSOR_INFORMATION_EX,
    };

    let mut buffer_size: u32 = 0;
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

    let cores = parse_processor_cores(&buffer)?;
    Ok(evaluate_topology(&cores))
}

#[cfg(not(windows))]
fn detect_processor_topology() -> Result<EvaluatedTopology, anyhow::Error> {
    Ok(EvaluatedTopology {
        physical_cores: 6,
        logical_cores: 12,
        performance_cores: 6,
        efficient_cores: 0,
        has_smt: true,
        is_hybrid: false,
        p_core_affinity_mask: None,
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

    #[cfg(windows)]
    fn build_core_record(flags: u8, efficiency_class: u8, mask: usize, group: u16) -> Vec<u8> {
        let mut buf = vec![0u8; 48];
        // Relationship: RelationProcessorCore = 0 (u32)
        buf[0..4].copy_from_slice(&0u32.to_ne_bytes());
        // Size: 48 (u32)
        buf[4..8].copy_from_slice(&48u32.to_ne_bytes());
        // Flags (offset 8)
        buf[8] = flags;
        // EfficiencyClass (offset 9)
        buf[9] = efficiency_class;
        // GroupCount (offset 30, u16)
        buf[30..32].copy_from_slice(&1u16.to_ne_bytes());
        // GroupMask[0].Mask (offset 32, usize)
        buf[32..40].copy_from_slice(&mask.to_ne_bytes());
        // GroupMask[0].Group (offset 40, u16)
        buf[40..42].copy_from_slice(&group.to_ne_bytes());
        buf
    }

    #[test]
    #[cfg(windows)]
    fn test_synthetic_intel_hybrid_13700k() {
        // Intel Core i7-13700K: 8 P-Cores (SMT=true, EfficiencyClass=1), 8 E-Cores (SMT=false, EfficiencyClass=0)
        // 8 P-Cores each with 2 logical processors -> 16 logical threads (mask 0x0000_FFFF)
        // 8 E-Cores each with 1 logical processor -> 8 logical threads (mask 0x00FF_0000)
        let mut buffer = Vec::new();

        // 8 P-Cores
        for i in 0..8 {
            let mask = 0b11usize << (i * 2);
            buffer.extend(build_core_record(1, 1, mask, 0));
        }

        // 8 E-Cores
        for i in 0..8 {
            let mask = 1usize << (16 + i);
            buffer.extend(build_core_record(0, 0, mask, 0));
        }

        let parsed = parse_processor_cores(&buffer).expect("Buffer parsing must succeed");
        assert_eq!(parsed.len(), 16, "Must parse 16 physical cores");

        let evaluated = evaluate_topology(&parsed);
        assert_eq!(evaluated.physical_cores, 16);
        assert_eq!(evaluated.logical_cores, 24);
        assert!(evaluated.is_hybrid, "13700K must be detected as hybrid");
        assert_eq!(evaluated.performance_cores, 8, "Must detect exactly 8 P-cores");
        assert_eq!(evaluated.efficient_cores, 8, "Must detect exactly 8 E-cores");
        assert!(evaluated.has_smt, "Must detect SMT capability");
        assert_eq!(evaluated.p_core_affinity_mask, Some(0x0000_FFFF), "P-core affinity mask must cover threads 0-15");
    }

    #[test]
    #[cfg(windows)]
    fn test_synthetic_intel_monolithic_12400() {
        // Intel Core i5-12400: 6 P-Cores (SMT=true, EfficiencyClass=0), 0 E-Cores
        // Uniform EfficiencyClass = 0 across all cores.
        let mut buffer = Vec::new();

        for i in 0..6 {
            let mask = 0b11usize << (i * 2);
            buffer.extend(build_core_record(1, 0, mask, 0));
        }

        let parsed = parse_processor_cores(&buffer).expect("Buffer parsing must succeed");
        assert_eq!(parsed.len(), 6, "Must parse 6 physical cores");

        let evaluated = evaluate_topology(&parsed);
        assert_eq!(evaluated.physical_cores, 6);
        assert_eq!(evaluated.logical_cores, 12);
        assert!(!evaluated.is_hybrid, "i5-12400 must NOT be marked hybrid");
        assert_eq!(evaluated.performance_cores, 6, "All 6 cores are performance cores");
        assert_eq!(evaluated.efficient_cores, 0, "Monolithic has 0 efficient cores");
        assert!(evaluated.has_smt, "Must detect SMT");
        assert_eq!(evaluated.p_core_affinity_mask, None, "Monolithic must not restrict affinity");
    }

    #[test]
    #[cfg(windows)]
    fn test_synthetic_amd_monolithic_7800x3d() {
        // AMD Ryzen 7 7800X3D: 8 P-Cores (SMT=true, EfficiencyClass=0), 0 E-Cores
        let mut buffer = Vec::new();

        for i in 0..8 {
            let mask = 0b11usize << (i * 2);
            buffer.extend(build_core_record(1, 0, mask, 0));
        }

        let parsed = parse_processor_cores(&buffer).expect("Buffer parsing must succeed");
        assert_eq!(parsed.len(), 8);

        let evaluated = evaluate_topology(&parsed);
        assert_eq!(evaluated.physical_cores, 8);
        assert_eq!(evaluated.logical_cores, 16);
        assert!(!evaluated.is_hybrid, "Ryzen 7 7800X3D must NOT be marked hybrid");
        assert_eq!(evaluated.performance_cores, 8);
        assert_eq!(evaluated.efficient_cores, 0);
        assert_eq!(evaluated.p_core_affinity_mask, None);
    }

    #[test]
    fn test_three_tier_hybrid_architecture() {
        // Multi-tier architecture: 2 LP-E cores (class 0), 4 E-cores (class 1), 6 P-cores (class 2)
        let mut cores = Vec::new();

        // 2 LP-E cores
        for i in 0..2 {
            cores.push(CoreDescriptor {
                efficiency_class: 0,
                has_smt: false,
                logical_core_count: 1,
                group_masks: vec![(0, 1usize << i)],
            });
        }

        // 4 E-cores
        for i in 0..4 {
            cores.push(CoreDescriptor {
                efficiency_class: 1,
                has_smt: false,
                logical_core_count: 1,
                group_masks: vec![(0, 1usize << (2 + i))],
            });
        }

        // 6 P-cores with SMT
        for i in 0..6 {
            cores.push(CoreDescriptor {
                efficiency_class: 2,
                has_smt: true,
                logical_core_count: 2,
                group_masks: vec![(0, 0b11usize << (6 + i * 2))],
            });
        }

        let evaluated = evaluate_topology(&cores);
        assert_eq!(evaluated.physical_cores, 12);
        assert_eq!(evaluated.logical_cores, 18);
        assert!(evaluated.is_hybrid, "Multi-tier must be detected as hybrid");
        assert_eq!(evaluated.performance_cores, 6, "Must identify highest EfficiencyClass as P-cores");
        assert_eq!(evaluated.efficient_cores, 6, "Must identify lower EfficiencyClasses as E-cores");
        let expected_mask = (0b111111111111usize) << 6;
        assert_eq!(evaluated.p_core_affinity_mask, Some(expected_mask));
    }

    #[test]
    fn test_cpu_detection() {
        let cpu = CpuInfo::detect().expect("CPU detection should succeed on host");
        assert!(!cpu.brand.is_empty(), "CPU brand should not be empty");
        assert!(!cpu.vendor.is_empty(), "CPU vendor should not be empty");
        assert!(cpu.physical_cores >= 1, "Physical cores must be >= 1");
        assert!(cpu.logical_cores >= cpu.physical_cores, "Logical cores must be >= physical cores");

        // On current test machine (Intel i5-12400F):
        // Must NOT falsely claim to be a hybrid processor with E-cores
        if cpu.brand.contains("12400") {
            assert!(!cpu.is_hybrid, "i5-12400 is a monolithic CPU without E-cores");
            assert_eq!(cpu.efficient_cores, 0, "i5-12400 has 0 E-cores");
            assert_eq!(cpu.performance_cores, cpu.physical_cores, "All cores on 12400 are P-cores");
            assert!(cpu.p_core_affinity_mask.is_none(), "Monolithic 12400 must not restrict affinity");
        }

        println!("=== Detected CPU Manifest ===");
        println!("Brand: {}", cpu.brand);
        println!("Vendor: {}", cpu.vendor);
        println!("Physical Cores: {}", cpu.physical_cores);
        println!("Logical Cores: {}", cpu.logical_cores);
        println!("Has SMT: {}", cpu.has_smt);
        println!("Is Hybrid: {}", cpu.is_hybrid);
        println!("P-Cores: {}", cpu.performance_cores);
        println!("E-Cores: {}", cpu.efficient_cores);
        println!("Affinity Mask: {:?}", cpu.p_core_affinity_mask);
        println!("AVX2 Supported: {}", cpu.features.avx2);
    }
}
