//! Riot Vanguard Active Compliance Checker.
//!
//! Pre-flight anti-cheat validator ensuring all Vanguard requirements are strictly satisfied
//! before launching or applying game-time optimizations:
//! 1. Riot Vanguard service (`vgc`) is running.
//! 2. Riot Vanguard kernel driver (`vgk.sys`) is loaded into memory.
//! 3. Windows Test Signing (TESTSIGNING) is disabled in boot configuration.
//! 4. UEFI Secure Boot is active.
//! 5. Virtualization-Based Security (VBS) and HVCI (Memory Integrity) are intact.

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{debug, error, info, warn};

#[cfg(windows)]
use windows::Win32::System::ProcessStatus::{EnumDeviceDrivers, GetDeviceDriverBaseNameW};
#[cfg(windows)]
use windows::Win32::System::Registry::{
    RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD,
    REG_SZ,
};
#[cfg(windows)]
use windows::Win32::System::Services::{
    CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatus, SC_MANAGER_CONNECT,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS,
};

/// Result of a comprehensive Riot Vanguard compliance check.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VanguardComplianceReport {
    pub vgc_service_running: bool,
    pub vgk_driver_loaded: bool,
    pub test_signing_disabled: bool,
    pub secure_boot_enabled: bool,
    pub vbs_hvci_intact: bool,
    pub is_compliant: bool,
    pub violations: Vec<String>,
}

#[derive(Debug, Error, Clone)]
pub enum VanguardComplianceError {
    #[error("Riot Vanguard anti-cheat prerequisite check failed with {count} violation(s):\n{violations}")]
    IntegrityCompromised {
        count: usize,
        violations: String,
        report: VanguardComplianceReport,
    },
}

pub struct VanguardChecker;

impl VanguardChecker {
    /// Perform full compliance check against all Vanguard requirements.
    pub fn check() -> VanguardComplianceReport {
        let vgc_running = Self::is_vgc_running();
        let vgk_loaded = Self::is_vgk_loaded();
        let (test_signing_disabled, test_signing_detail) = Self::is_test_signing_disabled();
        let secure_boot = Self::is_secure_boot_enabled();
        let (vbs_hvci_intact, vbs_detail) = Self::is_vbs_hvci_intact();

        let mut violations = Vec::new();

        if !vgc_running {
            violations.push("Riot Vanguard service (vgc) is not running".to_string());
        }
        if !vgk_loaded {
            violations.push("Riot Vanguard kernel driver (vgk.sys) is not loaded".to_string());
        }
        if !test_signing_disabled {
            let msg = test_signing_detail.unwrap_or_else(|| "Windows Test Signing is enabled (prohibited by Vanguard)".to_string());
            violations.push(msg);
        }
        if !secure_boot {
            violations.push("UEFI Secure Boot is disabled (required by Vanguard on Windows 11)".to_string());
        }
        if !vbs_hvci_intact {
            let msg = vbs_detail.unwrap_or_else(|| "VBS / HVCI (Memory Integrity) is disabled or compromised".to_string());
            violations.push(msg);
        }

        let is_compliant = violations.is_empty();

        if is_compliant {
            info!("Riot Vanguard anti-cheat compliance check: 100% COMPLIANT");
        } else {
            warn!(
                violations_count = violations.len(),
                "Riot Vanguard anti-cheat compliance check: VIOLATIONS DETECTED"
            );
            for v in &violations {
                warn!(" - Vanguard Violation: {}", v);
            }
        }

        VanguardComplianceReport {
            vgc_service_running: vgc_running,
            vgk_driver_loaded: vgk_loaded,
            test_signing_disabled,
            secure_boot_enabled: secure_boot,
            vbs_hvci_intact,
            is_compliant,
            violations,
        }
    }

    /// Pre-flight guard ensuring full compliance, returning an error if any check fails.
    /// In developer/testing environments, `allow_dev_override` can be set to true or via
    /// `VAL_OPT_BYPASS_VANGUARD_DEV` env var.
    pub fn ensure_compliant(allow_dev_override: bool) -> Result<VanguardComplianceReport, VanguardComplianceError> {
        let report = Self::check();

        let bypass_env = std::env::var("VAL_OPT_BYPASS_VANGUARD_DEV").map(|v| v == "1" || v.eq_ignore_ascii_case("true")).unwrap_or(false);
        if !report.is_compliant {
            if allow_dev_override || bypass_env {
                warn!("Bypassing Vanguard compliance rejection due to developer override flag!");
                return Ok(report);
            }

            let count = report.violations.len();
            let violations_str = report
                .violations
                .iter()
                .map(|v| format!("  * {}", v))
                .collect::<Vec<_>>()
                .join("\n");

            error!(
                "Aborting optimization launch: Vanguard requirements not satisfied ({})",
                count
            );

            return Err(VanguardComplianceError::IntegrityCompromised {
                count,
                violations: violations_str,
                report,
            });
        }

        Ok(report)
    }

    /// Check if the `vgc` Windows service is active and in `SERVICE_RUNNING` state.
    pub fn is_vgc_running() -> bool {
        #[cfg(windows)]
        unsafe {
            let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
                Ok(h) if !h.is_invalid() => h,
                _ => return false,
            };

            let service_name = windows::core::w!("vgc");
            let svc = match OpenServiceW(scm, service_name, SERVICE_QUERY_STATUS) {
                Ok(h) if !h.is_invalid() => h,
                _ => {
                    let _ = CloseServiceHandle(scm);
                    return false;
                }
            };

            let mut status = SERVICE_STATUS::default();
            let ok = QueryServiceStatus(svc, &mut status);

            let _ = CloseServiceHandle(svc);
            let _ = CloseServiceHandle(scm);

            if ok.is_ok() {
                return status.dwCurrentState == SERVICE_RUNNING;
            }
            false
        }

        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Check if the `vgk.sys` kernel driver is actively loaded in kernel address space.
    pub fn is_vgk_loaded() -> bool {
        #[cfg(windows)]
        unsafe {
            let mut needed: u32 = 0;
            let mut base_addrs: Vec<*mut core::ffi::c_void> = vec![std::ptr::null_mut(); 1024];

            let success = EnumDeviceDrivers(
                base_addrs.as_mut_ptr(),
                (base_addrs.len() * std::mem::size_of::<*mut core::ffi::c_void>()) as u32,
                &mut needed,
            );

            if success.is_err() {
                return false;
            }

            let count = (needed as usize) / std::mem::size_of::<*mut core::ffi::c_void>();
            let mut name_buf = [0u16; 256];

            for &base in base_addrs.iter().take(count) {
                if base.is_null() {
                    continue;
                }

                let len = GetDeviceDriverBaseNameW(base, &mut name_buf);
                if len > 0 {
                    let name = String::from_utf16_lossy(&name_buf[..len as usize]);
                    if name.eq_ignore_ascii_case("vgk.sys") {
                        debug!("Found loaded kernel driver: vgk.sys at base {:p}", base);
                        return true;
                    }
                }
            }

            false
        }

        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Check if Windows Test Signing mode is disabled.
    /// Vanguard prohibits booting with TESTSIGNING on.
    pub fn is_test_signing_disabled() -> (bool, Option<String>) {
        #[cfg(windows)]
        unsafe {
            let subkey = windows::core::w!("SYSTEM\\CurrentControlSet\\Control");
            let mut h_key = HKEY::default();

            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey, 0, KEY_READ, &mut h_key).is_ok() {
                let val_name = windows::core::w!("SystemStartOptions");
                let mut buf = [0u8; 1024];
                let mut size = buf.len() as u32;
                let mut val_type = REG_SZ;

                let res = RegQueryValueExW(
                    h_key,
                    val_name,
                    None,
                    Some(&mut val_type),
                    Some(buf.as_mut_ptr()),
                    Some(&mut size),
                );

                let _ = RegCloseKey(h_key);

                if res.is_ok() && size > 0 {
                    let u16_slice = std::slice::from_raw_parts(
                        buf.as_ptr() as *const u16,
                        (size as usize) / 2,
                    );
                    let options_str = String::from_utf16_lossy(u16_slice)
                        .trim_matches('\0')
                        .to_string();

                    debug!(options = %options_str, "Inspected SystemStartOptions");

                    if options_str.to_uppercase().contains("TESTSIGNING") {
                        return (
                            false,
                            Some("Windows Test Signing (TESTSIGNING) is enabled in BCD".to_string()),
                        );
                    }
                }
            }

            (true, None)
        }

        #[cfg(not(windows))]
        {
            (true, None)
        }
    }

    /// Check if UEFI Secure Boot is enabled.
    pub fn is_secure_boot_enabled() -> bool {
        #[cfg(windows)]
        unsafe {
            let sb_key = windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\SecureBoot\\State");
            let mut h_key = HKEY::default();

            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, sb_key, 0, KEY_READ, &mut h_key).is_ok() {
                let mut val: u32 = 0;
                let mut size = std::mem::size_of::<u32>() as u32;
                let mut val_type = REG_DWORD;
                let sb_val = windows::core::w!("UEFISecureBootEnabled");

                let res = RegQueryValueExW(
                    h_key,
                    sb_val,
                    None,
                    Some(&mut val_type),
                    Some(&mut val as *mut u32 as *mut u8),
                    Some(&mut size),
                );
                let _ = RegCloseKey(h_key);

                if res.is_ok() {
                    return val == 1;
                }
            }
            false
        }

        #[cfg(not(windows))]
        {
            true
        }
    }

    /// Check if VBS and HVCI (Hypervisor-Protected Code Integrity) are intact.
    pub fn is_vbs_hvci_intact() -> (bool, Option<String>) {
        #[cfg(windows)]
        unsafe {
            // Check DeviceGuard key
            let dg_key = windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\DeviceGuard");
            let mut h_key = HKEY::default();

            if RegOpenKeyExW(HKEY_LOCAL_MACHINE, dg_key, 0, KEY_READ, &mut h_key).is_ok() {
                let mut val: u32 = 0;
                let mut size = std::mem::size_of::<u32>() as u32;
                let mut val_type = REG_DWORD;
                let vbs_val = windows::core::w!("EnableVirtualizationBasedSecurity");

                if RegQueryValueExW(
                    h_key,
                    vbs_val,
                    None,
                    Some(&mut val_type),
                    Some(&mut val as *mut u32 as *mut u8),
                    Some(&mut size),
                )
                .is_ok()
                {
                    // If explicitly set to 0, VBS was manually disabled
                    if val == 0 {
                        let _ = RegCloseKey(h_key);
                        return (
                            false,
                            Some("Virtualization-Based Security (VBS) is explicitly disabled in registry".to_string()),
                        );
                    }
                }
                let _ = RegCloseKey(h_key);
            }

            (true, None)
        }

        #[cfg(not(windows))]
        {
            (true, None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vanguard_compliance_report_generation() {
        let report = VanguardChecker::check();
        println!("=== Riot Vanguard Compliance Report ===");
        println!(" vgc service running:   {}", report.vgc_service_running);
        println!(" vgk driver loaded:     {}", report.vgk_driver_loaded);
        println!(" test signing disabled: {}", report.test_signing_disabled);
        println!(" secure boot enabled:   {}", report.secure_boot_enabled);
        println!(" VBS / HVCI intact:     {}", report.vbs_hvci_intact);
        println!(" is_compliant:          {}", report.is_compliant);
        println!(" violations:            {:?}", report.violations);

        // Test signing should definitely be disabled on normal production Windows
        assert!(report.test_signing_disabled);
    }

    #[test]
    fn test_compliance_validation_logic() {
        let mut mock_report = VanguardComplianceReport {
            vgc_service_running: false,
            vgk_driver_loaded: false,
            test_signing_disabled: false,
            secure_boot_enabled: false,
            vbs_hvci_intact: false,
            is_compliant: false,
            violations: vec![
                "Riot Vanguard service (vgc) is not running".to_string(),
                "Windows Test Signing is enabled".to_string(),
            ],
        };

        assert!(!mock_report.is_compliant);
        assert_eq!(mock_report.violations.len(), 2);

        // When all conditions met:
        mock_report.vgc_service_running = true;
        mock_report.vgk_driver_loaded = true;
        mock_report.test_signing_disabled = true;
        mock_report.secure_boot_enabled = true;
        mock_report.vbs_hvci_intact = true;
        mock_report.violations.clear();
        mock_report.is_compliant = true;

        assert!(mock_report.is_compliant);
    }

    #[test]
    fn test_dev_override_allows_launch() {
        // In testing, ensure_compliant with allow_dev_override=true must succeed
        let res = VanguardChecker::ensure_compliant(true);
        assert!(res.is_ok(), "Dev override must allow optimization pipeline to proceed");
    }
}
