use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SecurityInfo {
    pub vbs_enabled: bool,
    pub hvci_enabled: bool,
    pub secure_boot_enabled: bool,
    pub game_mode_enabled: bool,
    pub vanguard_installed: bool,
    pub vanguard_service_running: bool,
}

impl SecurityInfo {
    /// Detects Windows 11 platform security and Vanguard anti-cheat operational status.
    pub fn detect() -> Result<Self, anyhow::Error> {
        #[cfg(windows)]
        {
            let (vbs_enabled, hvci_enabled) = query_vbs_and_hvci();
            let secure_boot_enabled = query_secure_boot();
            let game_mode_enabled = query_game_mode();
            let (vanguard_installed, vanguard_service_running) = query_vanguard_status();

            Ok(Self {
                vbs_enabled,
                hvci_enabled,
                secure_boot_enabled,
                game_mode_enabled,
                vanguard_installed,
                vanguard_service_running,
            })
        }

        #[cfg(not(windows))]
        {
            Ok(Self {
                vbs_enabled: true,
                hvci_enabled: true,
                secure_boot_enabled: true,
                game_mode_enabled: true,
                vanguard_installed: true,
                vanguard_service_running: true,
            })
        }
    }
}

#[cfg(windows)]
fn query_vbs_and_hvci() -> (bool, bool) {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD,
    };

    let mut vbs = false;
    let mut hvci = false;

    unsafe {
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
                vbs = val == 1;
            }

            let _ = RegCloseKey(h_key);
        }

        let hvci_key = windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\DeviceGuard\\Scenarios\\HypervisorEnforcedCodeIntegrity");
        let mut h_hvci_key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, hvci_key, 0, KEY_READ, &mut h_hvci_key).is_ok() {
            let mut val: u32 = 0;
            let mut size = std::mem::size_of::<u32>() as u32;
            let mut val_type = REG_DWORD;
            let enabled_val = windows::core::w!("Enabled");

            if RegQueryValueExW(
                h_hvci_key,
                enabled_val,
                None,
                Some(&mut val_type),
                Some(&mut val as *mut u32 as *mut u8),
                Some(&mut size),
            )
            .is_ok()
            {
                hvci = val == 1;
            }

            let _ = RegCloseKey(h_hvci_key);
        }
    }

    (vbs, hvci)
}

#[cfg(windows)]
fn query_secure_boot() -> bool {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, REG_DWORD,
    };

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
    }

    false
}

#[cfg(windows)]
fn query_game_mode() -> bool {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, KEY_READ, REG_DWORD,
    };

    unsafe {
        let gb_key = windows::core::w!("Software\\Microsoft\\GameBar");
        let mut h_key = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, gb_key, 0, KEY_READ, &mut h_key).is_ok() {
            let mut val: u32 = 0;
            let mut size = std::mem::size_of::<u32>() as u32;
            let mut val_type = REG_DWORD;
            let gm_val = windows::core::w!("AllowAutoGameMode");

            let res = RegQueryValueExW(
                h_key,
                gm_val,
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
    }

    // Game mode is enabled by default in Windows 11
    true
}

#[cfg(windows)]
fn query_vanguard_status() -> (bool, bool) {
    use windows::Win32::System::Services::{
        CloseServiceHandle, OpenSCManagerW, OpenServiceW, QueryServiceStatusEx,
        SC_MANAGER_CONNECT, SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_STATUS_PROCESS,
        SC_STATUS_PROCESS_INFO,
    };

    let vgk_path = std::path::Path::new("C:\\Program Files\\Riot Vanguard\\vgk.sys");
    let driver_installed = vgk_path.exists();

    let mut service_running = false;
    unsafe {
        if let Ok(scm) = OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            let service_name = windows::core::w!("vgc");
            if let Ok(service) = OpenServiceW(scm, service_name, SERVICE_QUERY_STATUS) {
                let mut status_process: SERVICE_STATUS_PROCESS = std::mem::zeroed();
                let mut bytes_needed = 0;
                let buf = std::slice::from_raw_parts_mut(
                    &mut status_process as *mut _ as *mut u8,
                    std::mem::size_of::<SERVICE_STATUS_PROCESS>(),
                );

                if QueryServiceStatusEx(
                    service,
                    SC_STATUS_PROCESS_INFO,
                    Some(buf),
                    &mut bytes_needed,
                )
                .is_ok()
                {
                    service_running = status_process.dwCurrentState == SERVICE_RUNNING;
                }
                let _ = CloseServiceHandle(service);
            }
            let _ = CloseServiceHandle(scm);
        }
    }

    let installed = driver_installed || service_running;
    (installed, service_running)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_security_detection() {
        let sec = SecurityInfo::detect().expect("Security detection should succeed");
        println!("=== Detected Security & Vanguard State ===");
        println!("VBS Enabled: {}", sec.vbs_enabled);
        println!("HVCI (Memory Integrity): {}", sec.hvci_enabled);
        println!("Secure Boot: {}", sec.secure_boot_enabled);
        println!("Windows Game Mode: {}", sec.game_mode_enabled);
        println!("Vanguard Installed: {}", sec.vanguard_installed);
        println!("Vanguard Service (vgc) Running: {}", sec.vanguard_service_running);
    }
}
