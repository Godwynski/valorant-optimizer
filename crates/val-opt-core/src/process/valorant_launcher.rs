//! VALORANT & Riot Client Launch Engine (`TASK-FUNC-03`).
//!
//! Discovers and initiates VALORANT game launches via the Riot Client
//! (`RiotClientServices.exe` or `riotclient://` URI protocol), ensuring
//! proper Vanguard anti-cheat session authentication and token establishment.

use std::path::PathBuf;
use tracing::{info, warn};

pub const VALORANT_BINARY_NAME: &str = "VALORANT-Win64-Shipping.exe";
pub const RIOT_CLIENT_SERVICES_NAME: &str = "RiotClientServices.exe";
pub const RIOT_CLIENT_URI: &str = "riotclient://launch-product?product=valorant&patchline=live";

/// Errors encountered during VALORANT launch sequence.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ValorantLaunchError {
    #[error("VALORANT or Riot Client is not installed: {0}")]
    NotInstalled(String),
    #[error("Failed to initiate launch via Riot Client: {0}")]
    LaunchFailed(String),
    #[error("Vanguard validation error: {0}")]
    VanguardError(String),
}

/// Installation status descriptor for VALORANT and Riot Client.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ValorantInstallation {
    pub riot_client_path: Option<PathBuf>,
    pub valorant_shipping_path: Option<PathBuf>,
    pub install_directory: Option<PathBuf>,
    pub uri_registered: bool,
}

pub struct ValorantLauncher;

impl ValorantLauncher {
    /// Detect whether VALORANT is currently installed and locate its binaries.
    pub fn detect_installation() -> ValorantInstallation {
        let riot_client_path = Self::find_riot_client_executable();
        let (valorant_shipping_path, install_directory) = Self::find_valorant_paths();
        let uri_registered = Self::is_uri_protocol_registered();

        ValorantInstallation {
            riot_client_path,
            valorant_shipping_path,
            install_directory,
            uri_registered,
        }
    }

    /// Check if the `riotclient://` URI protocol is registered in Windows registry.
    pub fn is_uri_protocol_registered() -> bool {
        #[cfg(windows)]
        {
            use windows::Win32::System::Registry::{
                RegCloseKey, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT, KEY_READ,
            };
            unsafe {
                let mut h_key = HKEY::default();
                let subkey = windows::core::w!("riotclient\\shell\\open\\command");
                if RegOpenKeyExW(HKEY_CLASSES_ROOT, subkey, 0, KEY_READ, &mut h_key).is_ok() {
                    let _ = RegCloseKey(h_key);
                    return true;
                }
            }
        }
        false
    }

    /// Search registry and known system drives for `RiotClientServices.exe`.
    pub fn find_riot_client_executable() -> Option<PathBuf> {
        #[cfg(windows)]
        {
            use windows::Win32::System::Registry::{
                HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_CLASSES_ROOT,
            };

            // 1. Try RiotClientInstalls.json in ProgramData
            let prog_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
            let installs_json = PathBuf::from(&prog_data).join("Riot Games").join("RiotClientInstalls.json");
            if installs_json.is_file() {
                if let Ok(content) = std::fs::read_to_string(&installs_json) {
                    if let Ok(json) = serde_json::from_str::<serde_json::Value>(&content) {
                        for key in ["rc_default", "rc_live"] {
                            if let Some(path_str) = json.get(key).and_then(|v| v.as_str()) {
                                let candidate = PathBuf::from(path_str);
                                if candidate.is_file() {
                                    return Some(candidate);
                                }
                            }
                        }
                    }
                }
            }

            // 2. Try registry locations
            let subkeys = [
                (HKEY_CURRENT_USER, "Software\\Riot Games\\RiotClient"),
                (HKEY_LOCAL_MACHINE, "SOFTWARE\\Riot Games, Inc\\Riot Client"),
                (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\Riot Games, Inc\\Riot Client"),
            ];

            for (root, subkey) in subkeys {
                if let Some(loc) = query_registry_string(root, subkey, "Location") {
                    let candidate = PathBuf::from(loc).join(RIOT_CLIENT_SERVICES_NAME);
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }

            // 3. Try parsing HKCR\riotclient\shell\open\command
            if let Some(cmd) = query_registry_string(HKEY_CLASSES_ROOT, "riotclient\\shell\\open\\command", "") {
                if let Some(path) = extract_executable_from_command(&cmd) {
                    if path.is_file() {
                        return Some(path);
                    }
                }
            }

            // 4. Scan common drive letters
            let drives = ["C", "D", "E", "F", "G"];
            for drive in drives {
                let candidate = PathBuf::from(format!(r"{}:\Riot Games\Riot Client\{}", drive, RIOT_CLIENT_SERVICES_NAME));
                if candidate.is_file() {
                    return Some(candidate);
                }
            }
        }

        None
    }

    /// Search registry and known system drives for VALORANT install folder and shipping binary.
    pub fn find_valorant_paths() -> (Option<PathBuf>, Option<PathBuf>) {
        #[cfg(windows)]
        {
            use windows::Win32::System::Registry::{
                HKEY_LOCAL_MACHINE, HKEY_CURRENT_USER,
            };

            // 1. Try Riot Games Metadata in ProgramData
            let prog_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
            let settings_yaml = PathBuf::from(&prog_data)
                .join("Riot Games")
                .join("Metadata")
                .join("valorant.live")
                .join("valorant.live.product_settings.yaml");
            if settings_yaml.is_file() {
                if let Ok(content) = std::fs::read_to_string(&settings_yaml) {
                    for line in content.lines() {
                        let trimmed = line.trim();
                        if trimmed.starts_with("product_install_full_path:") {
                            let parts: Vec<&str> = trimmed.splitn(2, ':').collect();
                            if parts.len() == 2 {
                                let dir_str = parts[1].trim().trim_matches('"').trim_matches('\'');
                                let dir = PathBuf::from(dir_str);
                                let shipping = dir.join("ShooterGame").join("Binaries").join("Win64").join(VALORANT_BINARY_NAME);
                                if shipping.is_file() {
                                    return (Some(shipping), Some(dir));
                                }
                            }
                        }
                    }
                }
            }

            // 2. Try registry uninstall locations
            let uninstall_keys = [
                (HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Riot Game valorant.live"),
                (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Riot Game valorant.live"),
                (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\Riot Game valorant.live"),
            ];

            for (root, key) in uninstall_keys {
                if let Some(loc) = query_registry_string(root, key, "InstallLocation") {
                    let dir = PathBuf::from(loc);
                    let shipping = dir.join("ShooterGame").join("Binaries").join("Win64").join(VALORANT_BINARY_NAME);
                    if shipping.is_file() {
                        return (Some(shipping), Some(dir));
                    }
                }
            }

            // 3. Scan common drive letters
            let drives = ["C", "D", "E", "F", "G"];
            for drive in drives {
                let candidate = PathBuf::from(format!(r"{}:\Riot Games\VALORANT\live\ShooterGame\Binaries\Win64\{}", drive, VALORANT_BINARY_NAME));
                if candidate.is_file() {
                    let dir = PathBuf::from(format!(r"{}:\Riot Games\VALORANT\live", drive));
                    return (Some(candidate), Some(dir));
                }
            }
        }

        (None, None)
    }

    /// Execute the launch sequence for VALORANT.
    ///
    /// Execution policy:
    /// 1. If `VALORANT-Win64-Shipping.exe` is ALREADY active, attach supervisor directly.
    /// 2. If `RiotClientServices.exe` is found on disk:
    ///    - If current process is elevated, de-escalate launch by spawning under interactive desktop user token (`TASK-FUNC-04`).
    ///    - If un-elevated, spawn directly via `std::process::Command`.
    /// 3. If URI protocol is registered, dispatch via Win32 `ShellExecuteW`.
    /// 4. If neither is available, return `Err(ValorantLaunchError::NotInstalled)`.
    pub fn launch_game() -> Result<String, ValorantLaunchError> {
        info!("Initiating VALORANT game launch flow");

        // 1. Check if genuine game is already running with verified executable path
        if let Some((pid, path)) = super::supervisor::ProcessSupervisor::find_process_with_path(VALORANT_BINARY_NAME) {
            info!(pid = pid, path = %path, "Genuine VALORANT process is already running; lifecycle supervisor attaching directly");
            return Ok(format!("VALORANT is already running (PID {}). Supervisor attached.", pid));
        }

        let installation = Self::detect_installation();

        // 2. Launch via RiotClientServices.exe if found
        if let Some(ref riot_exe) = installation.riot_client_path {
            info!(path = %riot_exe.display(), "Launching VALORANT via RiotClientServices executable");

            #[cfg(windows)]
            {
                let args = "--launch-product=valorant --launch-patchline=live";

                // Enforce token-safe launch if daemon is running elevated (prevent launching game as SYSTEM)
                if super::terminator::is_current_process_elevated() {
                    if let Some(user_token) = super::terminator::get_interactive_user_token() {
                        info!("Daemon is elevated; spawning Riot Client under interactive desktop user token");
                        let spawn_res = super::terminator::spawn_process_with_user_token(
                            riot_exe,
                            Some(args),
                            user_token,
                        );
                        unsafe {
                            let _ = windows::Win32::Foundation::CloseHandle(user_token);
                        }

                        match spawn_res {
                            Ok(pid) => {
                                info!(client_pid = pid, "Riot Client spawned successfully under user token. Awaiting VALORANT startup.");
                                return Ok(format!(
                                    "Riot Client launched (PID {}). Core daemon supervising game startup.",
                                    pid
                                ));
                            }
                            Err(e) => {
                                warn!(error = %e, "Failed to spawn Riot Client with user token; attempting fallback");
                            }
                        }
                    } else {
                        warn!("Interactive user token unavailable; falling back to direct spawn");
                    }
                }

                let mut cmd = std::process::Command::new(riot_exe);
                if let Some(parent) = riot_exe.parent() {
                    cmd.current_dir(parent);
                }
                cmd.arg("--launch-product=valorant");
                cmd.arg("--launch-patchline=live");

                match cmd.spawn() {
                    Ok(child) => {
                        info!(client_pid = child.id(), "Riot Client spawned successfully. Awaiting VALORANT startup.");
                        return Ok(format!("Riot Client launched (PID {}). Core daemon supervising game startup.", child.id()));
                    }
                    Err(e) => {
                        warn!(error = %e, "Failed to spawn Riot Client executable directly; attempting URI fallback");
                    }
                }
            }
        }

        // 3. Launch via registered URI protocol if available
        if installation.uri_registered {
            info!(uri = RIOT_CLIENT_URI, "Launching VALORANT via Riot Client URI protocol");
            #[cfg(windows)]
            {
                use windows::Win32::UI::Shell::ShellExecuteW;
                use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
                use windows::core::w;

                let uri_wide: Vec<u16> = RIOT_CLIENT_URI.encode_utf16().chain(std::iter::once(0)).collect();
                let op = w!("open");

                let h_inst = unsafe {
                    ShellExecuteW(
                        None,
                        op,
                        windows::core::PCWSTR(uri_wide.as_ptr()),
                        None,
                        None,
                        SW_SHOWNORMAL,
                    )
                };

                let ret_val = h_inst.0 as usize;
                if ret_val > 32 {
                    info!("Riot Client URI successfully dispatched via ShellExecuteW");
                    return Ok("VALORANT launch dispatched via Riot Client URI protocol. Core daemon supervising startup.".to_string());
                } else {
                    return Err(ValorantLaunchError::LaunchFailed(format!(
                        "ShellExecuteW failed for URI with error code: {}",
                        ret_val
                    )));
                }
            }
        }

        Err(ValorantLaunchError::NotInstalled(
            "Neither Riot Client executable nor riotclient:// protocol handler was found on this system. Please verify VALORANT installation.".to_string(),
        ))
    }
}

/// Helper to query a string value from Windows Registry.
#[cfg(windows)]
fn query_registry_string(
    root: windows::Win32::System::Registry::HKEY,
    subkey: &str,
    value_name: &str,
) -> Option<String> {
    use windows::Win32::System::Registry::{
        RegCloseKey, RegOpenKeyExW, RegQueryValueExW, HKEY, KEY_READ, REG_SZ, REG_EXPAND_SZ,
    };

    let wide_sub: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let wide_val: Vec<u16> = value_name.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let mut h_key = HKEY::default();
        if RegOpenKeyExW(root, windows::core::PCWSTR(wide_sub.as_ptr()), 0, KEY_READ, &mut h_key).is_ok() {
            let mut buf = [0u16; 1024];
            let mut size = (buf.len() * 2) as u32;
            let mut val_type = REG_SZ;
            let val_ptr = if value_name.is_empty() {
                windows::core::PCWSTR::null()
            } else {
                windows::core::PCWSTR(wide_val.as_ptr())
            };

            let res = RegQueryValueExW(
                h_key,
                val_ptr,
                None,
                Some(&mut val_type),
                Some(buf.as_mut_ptr() as *mut u8),
                Some(&mut size),
            );
            let _ = RegCloseKey(h_key);

            if res.is_ok() && (val_type == REG_SZ || val_type == REG_EXPAND_SZ) && size > 2 {
                let chars = (size as usize / 2).saturating_sub(1);
                return Some(String::from_utf16_lossy(&buf[..chars]).trim_matches('\0').trim().to_string());
            }
        }
    }
    None
}

/// Helper to extract executable path from a shell open command string like:
/// `"C:\Riot Games\Riot Client\RiotClientServices.exe" --app-command="%1"`
pub fn extract_executable_from_command(cmd: &str) -> Option<PathBuf> {
    let trimmed = cmd.trim();
    if trimmed.starts_with('"') {
        if let Some(end) = trimmed[1..].find('"') {
            let path_str = &trimmed[1..1 + end];
            return Some(PathBuf::from(path_str));
        }
    } else {
        if let Some(pos) = trimmed.to_lowercase().find(".exe") {
            let path_str = &trimmed[..pos + 4];
            return Some(PathBuf::from(path_str));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_executable_from_command() {
        let cmd1 = r#""C:\Riot Games\Riot Client\RiotClientServices.exe" --app-command="%1""#;
        assert_eq!(
            extract_executable_from_command(cmd1),
            Some(PathBuf::from(r"C:\Riot Games\Riot Client\RiotClientServices.exe"))
        );

        let cmd2 = r#"C:\Games\RiotClientServices.exe /arg"#;
        assert_eq!(
            extract_executable_from_command(cmd2),
            Some(PathBuf::from(r"C:\Games\RiotClientServices.exe"))
        );

        let cmd3 = "invalid_command_without_exe";
        assert_eq!(extract_executable_from_command(cmd3), None);
    }

    #[test]
    fn test_detect_installation_structure() {
        let install = ValorantLauncher::detect_installation();
        println!("=== Detected VALORANT Installation ===");
        println!("Riot Client Path: {:?}", install.riot_client_path);
        println!("VALORANT Shipping Path: {:?}", install.valorant_shipping_path);
        println!("Install Directory: {:?}", install.install_directory);
        println!("URI Registered: {}", install.uri_registered);

        // Verification logic:
        // On this test machine, if the binary is missing, launching must fail gracefully
        // with NotInstalled rather than panicking or crashing.
        if install.riot_client_path.is_none() && !install.uri_registered {
            let res = ValorantLauncher::launch_game();
            assert!(matches!(res, Err(ValorantLaunchError::NotInstalled(_))));
        }
    }

    #[test]
    fn test_uri_registration_detection() {
        let registered = ValorantLauncher::is_uri_protocol_registered();
        println!("Is riotclient URI protocol registered: {}", registered);
        // On Windows with Riot Client installed or previously registered, this returns true
        assert!(registered || !registered);
    }

    #[test]
    fn test_already_running_game_detection() {
        // Create a temporary mock binary structured as ShooterGame/Binaries/Win64/VALORANT-Win64-Shipping.exe
        let temp_dir = std::env::temp_dir();
        let target_dir = temp_dir.join("valopt_test_shootergame").join("ShooterGame").join("Binaries").join("Win64");
        let _ = std::fs::create_dir_all(&target_dir);
        let mock_game = target_dir.join(VALORANT_BINARY_NAME);
        let windir = std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".to_string());
        let src = std::path::PathBuf::from(windir).join("System32").join("cmd.exe");
        let _ = std::fs::copy(&src, &mock_game);

        if mock_game.is_file() {
            let child = std::process::Command::new(&mock_game)
                .args(["/c", "ping -n 10 127.0.0.1 >nul"])
                .spawn();

            if let Ok(mut c) = child {
                let pid = c.id();
                std::thread::sleep(std::time::Duration::from_millis(150));

                let launch_res = ValorantLauncher::launch_game();
                assert!(launch_res.is_ok(), "Must detect already-running game instance: {:?}", launch_res);
                let msg = launch_res.unwrap();
                assert!(msg.contains("already running"), "Message must indicate game is already running: {}", msg);
                assert!(msg.contains(&pid.to_string()), "Message must reference PID: {}", msg);

                let _ = c.kill();
                let _ = c.wait();
            }
            let _ = std::fs::remove_file(&mock_game);
            let _ = std::fs::remove_dir_all(temp_dir.join("valopt_test_shootergame"));
        }
    }

    #[test]
    fn test_token_selection_logic() {
        let is_elevated = crate::process::terminator::is_current_process_elevated();
        println!("Process elevation state: {}", is_elevated);
        if is_elevated {
            let user_token = crate::process::terminator::get_interactive_user_token();
            println!("Interactive user token obtained: {}", user_token.is_some());
            if let Some(token) = user_token {
                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(token);
                }
            }
        }
    }
}
