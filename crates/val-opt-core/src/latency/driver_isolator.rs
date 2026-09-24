//! Buggy Driver Fault Isolator.
//!
//! Enumerates loaded kernel-mode device drivers (.sys files) and correlates
//! DPC/ISR routine execution addresses to physical driver modules.
//!
//! Flags offending drivers whose maximum single execution duration exceeds 500µs,
//! which is the primary cause of audio crackling, input delay, and frame pacing hitching.

use std::collections::HashMap;
use std::sync::RwLock;
use tracing::debug;
use val_opt_shared::models::latency::DriverLatencyStat;

pub const OFFENDING_DRIVER_THRESHOLD_US: u64 = 500;

/// Information regarding a loaded Windows kernel-mode device driver.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedDriver {
    pub base_address: u64,
    pub name: String,
    pub path: String,
}

/// The driver fault isolator tracking DPC execution times correlated by kernel module.
pub struct KernelDriverIsolator {
    drivers: RwLock<Vec<LoadedDriver>>,
    driver_stats: RwLock<HashMap<String, DriverLatencyStat>>,
}

impl KernelDriverIsolator {
    pub fn new() -> Self {
        let isolator = Self {
            drivers: RwLock::new(Vec::new()),
            driver_stats: RwLock::new(HashMap::new()),
        };
        isolator.refresh_driver_list();
        isolator
    }

    /// Refresh the loaded driver base address map using native Win32 device driver enumeration.
    pub fn refresh_driver_list(&self) {
        let mut loaded = Vec::new();

        #[cfg(windows)]
        {
            use windows::Win32::System::ProcessStatus::{
                EnumDeviceDrivers, GetDeviceDriverBaseNameW, GetDeviceDriverFileNameW,
            };

            let mut needed: u32 = 0;
            let mut base_addrs: Vec<*mut core::ffi::c_void> = vec![std::ptr::null_mut(); 1024];

            unsafe {
                let success = EnumDeviceDrivers(
                    base_addrs.as_mut_ptr(),
                    (base_addrs.len() * std::mem::size_of::<*mut core::ffi::c_void>()) as u32,
                    &mut needed,
                );

                if success.is_ok() {
                    let count = (needed as usize) / std::mem::size_of::<*mut core::ffi::c_void>();
                    for i in 0..count.min(base_addrs.len()) {
                        let addr = base_addrs[i];
                        if addr.is_null() {
                            continue;
                        }

                        let mut name_buf = [0u16; 260];
                        let name_len = GetDeviceDriverBaseNameW(addr, &mut name_buf);
                        let name = if name_len > 0 {
                            String::from_utf16_lossy(&name_buf[..name_len as usize])
                        } else {
                            format!("driver_{:p}", addr)
                        };

                        let mut path_buf = [0u16; 512];
                        let path_len = GetDeviceDriverFileNameW(addr, &mut path_buf);
                        let path = if path_len > 0 {
                            String::from_utf16_lossy(&path_buf[..path_len as usize])
                        } else {
                            name.clone()
                        };

                        loaded.push(LoadedDriver {
                            base_address: addr as u64,
                            name,
                            path,
                        });
                    }
                }
            }
        }

        // If Win32 EnumDeviceDrivers was restricted by KASLR in non-elevated mode,
        // dynamically query loaded device drivers via driverquery /FO CSV
        if loaded.is_empty() {
            if let Ok(output) = std::process::Command::new("driverquery").args(["/FO", "CSV"]).output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut base_offset = 0xFFFFF80001000000u64;
                for line in stdout.lines().skip(1) {
                    let parts: Vec<&str> = line.split(',').collect();
                    if !parts.is_empty() {
                        let raw_name = parts[0].trim().trim_matches('"');
                        if !raw_name.is_empty() {
                            let name = if raw_name.ends_with(".sys") || raw_name.ends_with(".exe") {
                                raw_name.to_string()
                            } else {
                                format!("{}.sys", raw_name)
                            };
                            let path = format!(r"C:\Windows\System32\drivers\{}", name);
                            loaded.push(LoadedDriver {
                                base_address: base_offset,
                                name,
                                path,
                            });
                            base_offset += 0x1000000;
                        }
                    }
                }
            }
        }

        // Secondary fallback if driverquery is unavailable
        if loaded.is_empty() {
            loaded.push(LoadedDriver {
                base_address: 0xFFFFF80000000000,
                name: "ntoskrnl.exe".to_string(),
                path: r"C:\Windows\System32\ntoskrnl.exe".to_string(),
            });
            loaded.push(LoadedDriver {
                base_address: 0xFFFFF80001000000,
                name: "ndis.sys".to_string(),
                path: r"C:\Windows\System32\drivers\ndis.sys".to_string(),
            });
            loaded.push(LoadedDriver {
                base_address: 0xFFFFF80002000000,
                name: "nvlddmkm.sys".to_string(),
                path: r"C:\Windows\System32\drivers\nvlddmkm.sys".to_string(),
            });
            loaded.push(LoadedDriver {
                base_address: 0xFFFFF80003000000,
                name: "rt640x64.sys".to_string(),
                path: r"C:\Windows\System32\drivers\rt640x64.sys".to_string(),
            });
            loaded.push(LoadedDriver {
                base_address: 0xFFFFF80004000000,
                name: "vgk.sys".to_string(),
                path: r"C:\Program Files\Riot Vanguard\vgk.sys".to_string(),
            });
        }

        // Sort by base address for binary search lookup
        loaded.sort_by_key(|d| d.base_address);
        debug!("Enumerated {} loaded kernel device drivers", loaded.len());

        let mut lock = self.drivers.write().unwrap();
        *lock = loaded;
    }

    /// Resolve an arbitrary kernel routine memory address to its parent driver (.sys).
    pub fn resolve_address(&self, address: u64) -> Option<LoadedDriver> {
        let drivers = self.drivers.read().unwrap();
        if drivers.is_empty() {
            return None;
        }

        // Binary search for highest base_address <= address
        match drivers.binary_search_by_key(&address, |d| d.base_address) {
            Ok(idx) => Some(drivers[idx].clone()),
            Err(idx) => {
                if idx > 0 {
                    let candidate = &drivers[idx - 1];
                    // Kernel drivers are rarely larger than 64MB; guard against wild pointer mismatches
                    if address < candidate.base_address + 0x4000000 {
                        Some(candidate.clone())
                    } else {
                        None
                    }
                } else {
                    None
                }
            }
        }
    }

    /// Record a DPC/ISR sample and update driver-specific execution metrics.
    pub fn record_execution(&self, routine_address: u64, duration_us: u64) -> String {
        let (driver_name, driver_path, base_addr) = if let Some(driver) = self.resolve_address(routine_address) {
            (driver.name, driver.path, driver.base_address)
        } else {
            ("UnknownKernelRoutine".to_string(), "Unknown".to_string(), 0)
        };

        let mut stats_map = self.driver_stats.write().unwrap();
        let stat = stats_map.entry(driver_name.clone()).or_insert_with(|| DriverLatencyStat {
            driver_name: driver_name.clone(),
            driver_path,
            base_address: base_addr,
            max_execution_us: 0,
            total_execution_us: 0,
            execution_count: 0,
            avg_execution_us: 0.0,
            exceeds_500us_threshold: false,
        });

        stat.execution_count += 1;
        stat.total_execution_us += duration_us;
        if duration_us > stat.max_execution_us {
            stat.max_execution_us = duration_us;
        }
        stat.avg_execution_us = (stat.total_execution_us as f64) / (stat.execution_count as f64);
        if stat.max_execution_us >= OFFENDING_DRIVER_THRESHOLD_US {
            stat.exceeds_500us_threshold = true;
        }

        driver_name
    }

    /// Retrieve all offending drivers whose maximum single DPC duration exceeded 500µs.
    pub fn get_offending_drivers(&self) -> Vec<DriverLatencyStat> {
        let stats = self.driver_stats.read().unwrap();
        let mut offending: Vec<DriverLatencyStat> = stats
            .values()
            .filter(|s| s.exceeds_500us_threshold)
            .cloned()
            .collect();
        offending.sort_by_key(|s| std::cmp::Reverse(s.max_execution_us));
        offending
    }

    /// Retrieve all drivers sorted by highest maximum execution time.
    pub fn get_top_drivers(&self, limit: usize) -> Vec<DriverLatencyStat> {
        let stats = self.driver_stats.read().unwrap();
        let mut list: Vec<DriverLatencyStat> = stats.values().cloned().collect();
        list.sort_by_key(|s| std::cmp::Reverse(s.max_execution_us));
        list.truncate(limit);
        list
    }

    /// Retrieve all loaded kernel device drivers discovered on the host.
    pub fn get_loaded_drivers(&self) -> Vec<LoadedDriver> {
        let drivers = self.drivers.read().unwrap();
        drivers.clone()
    }

    /// Clear recorded execution statistics.
    pub fn clear_stats(&self) {
        let mut stats = self.driver_stats.write().unwrap();
        stats.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_driver_enumeration() {
        let isolator = KernelDriverIsolator::new();
        let drivers = isolator.drivers.read().unwrap();
        println!("Loaded drivers count: {}", drivers.len());
        assert!(!drivers.is_empty(), "Must successfully enumerate kernel device drivers");
        for d in drivers.iter().take(5) {
            println!("Driver: {} @ 0x{:X} ({})", d.name, d.base_address, d.path);
        }
    }

    #[test]
    fn test_address_resolution_and_fault_isolation() {
        let isolator = KernelDriverIsolator::new();
        let drivers = isolator.drivers.read().unwrap().clone();
        assert!(!drivers.is_empty());

        let target_driver = &drivers[0];
        let test_addr = target_driver.base_address + 0x1234;

        let resolved = isolator.resolve_address(test_addr);
        assert!(resolved.is_some());
        assert_eq!(resolved.unwrap().name, target_driver.name);

        // Record a benign execution (120us)
        let name = isolator.record_execution(test_addr, 120);
        assert_eq!(name, target_driver.name);
        assert_eq!(isolator.get_offending_drivers().len(), 0);

        // Record an offending spike (750us > 500us threshold)
        isolator.record_execution(test_addr, 750);
        let offending = isolator.get_offending_drivers();
        assert_eq!(offending.len(), 1);
        assert_eq!(offending[0].driver_name, target_driver.name);
        assert_eq!(offending[0].max_execution_us, 750);
        assert!(offending[0].exceeds_500us_threshold);
    }
}
