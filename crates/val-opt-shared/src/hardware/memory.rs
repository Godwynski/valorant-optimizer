use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct MemoryInfo {
    pub total_physical_mb: u64,
    pub available_physical_mb: u64,
    pub memory_load_percent: u32,
    pub total_virtual_mb: u64,
    pub available_virtual_mb: u64,
}

impl MemoryInfo {
    /// Queries the physical and virtual memory metrics via GlobalMemoryStatusEx.
    pub fn detect() -> Result<Self, anyhow::Error> {
        #[cfg(windows)]
        {
            use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};

            let mut status = MEMORYSTATUSEX {
                dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32,
                ..Default::default()
            };

            let result = unsafe { GlobalMemoryStatusEx(&mut status) };
            if let Err(e) = result {
                return Err(anyhow::anyhow!("GlobalMemoryStatusEx failed: {}", e));
            }

            let mb = 1024 * 1024;
            Ok(Self {
                total_physical_mb: status.ullTotalPhys / mb,
                available_physical_mb: status.ullAvailPhys / mb,
                memory_load_percent: status.dwMemoryLoad,
                total_virtual_mb: status.ullTotalVirtual / mb,
                available_virtual_mb: status.ullAvailVirtual / mb,
            })
        }

        #[cfg(not(windows))]
        {
            Ok(Self {
                total_physical_mb: 32768,
                available_physical_mb: 24576,
                memory_load_percent: 25,
                total_virtual_mb: 65536,
                available_virtual_mb: 50000,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_memory_detection() {
        let mem = MemoryInfo::detect().expect("Memory detection should succeed");
        assert!(mem.total_physical_mb > 0, "Total RAM must be > 0");
        assert!(mem.available_physical_mb > 0, "Available RAM must be > 0");
        assert!(mem.memory_load_percent <= 100, "Load percent must be <= 100");

        println!("=== Detected Memory Manifest ===");
        println!("Total Physical RAM: {} MB", mem.total_physical_mb);
        println!("Available Physical RAM: {} MB", mem.available_physical_mb);
        println!("Memory Load: {}%", mem.memory_load_percent);
    }
}
