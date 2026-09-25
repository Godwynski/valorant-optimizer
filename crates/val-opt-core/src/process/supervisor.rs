//! VALORANT Process Lifecycle Supervisor.
//!
//! Supervises the game process (`VALORANT-Win64-Shipping.exe`), sets `HIGH_PRIORITY_CLASS`,
//! terminates the memory-heavy Riot Client CEF UI (`RiotClientUx.exe`), and maintains
//! a zero-CPU wait state during match play.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use tracing::{info, warn};
use windows::Win32::Foundation::{CloseHandle, STILL_ACTIVE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, SetPriorityClass,
    SetProcessAffinityMask, WaitForSingleObject, HIGH_PRIORITY_CLASS,
    PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SYNCHRONIZE,
};

use super::terminator::{get_process_image_path, relaunch_applications, terminate_process_gracefully};
pub use super::valorant_launcher::VALORANT_BINARY_NAME;
use val_opt_shared::models::process::TerminatedAppBackup;

pub const RIOT_CLIENT_UX_NAME: &str = "RiotClientUx.exe";
pub const RIOT_CLIENT_UX_RENDER_NAME: &str = "RiotClientUxRender.exe";

/// Game supervision configuration.
#[derive(Debug, Clone)]
pub struct GameSupervisorConfig {
    pub target_binary: String,
    pub p_core_affinity_mask: Option<usize>,
    pub terminate_cef_ui: bool,
    pub poll_interval_ms: u64,
}

impl Default for GameSupervisorConfig {
    fn default() -> Self {
        Self {
            target_binary: VALORANT_BINARY_NAME.to_string(),
            p_core_affinity_mask: None,
            terminate_cef_ui: true,
            poll_interval_ms: 500,
        }
    }
}

/// The game lifecycle supervisor.
pub struct ProcessSupervisor {
    config: GameSupervisorConfig,
    #[allow(dead_code)]
    is_running: Arc<AtomicBool>,
}

impl ProcessSupervisor {
    pub fn new(config: GameSupervisorConfig) -> Self {
        Self {
            config,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Search running processes for a target executable name and return its PID.
    pub fn find_process_by_name(name: &str) -> Option<u32> {
        let clean_target = name.to_lowercase();
        unsafe {
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return None,
            };

            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            let mut found_pid = None;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let exe = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_lowercase();

                    if exe == clean_target {
                        found_pid = Some(entry.th32ProcessID);
                        break;
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }

            let _ = CloseHandle(snapshot);
            found_pid
        }
    }

    /// Verifies if an image path belongs to the genuine game binary.
    /// Specifically protects against malicious impersonation of `VALORANT-Win64-Shipping.exe`.
    pub fn is_legitimate_game_path(target_binary: &str, image_path: &str) -> bool {
        if target_binary.eq_ignore_ascii_case(VALORANT_BINARY_NAME) {
            let lower = image_path.to_lowercase();
            lower.ends_with(r"\shootergame\binaries\win64\valorant-win64-shipping.exe")
                || (lower.contains("valorant") && lower.contains("shootergame") && lower.ends_with("valorant-win64-shipping.exe"))
        } else {
            true
        }
    }

    /// Search running processes for target executable name and return both PID and full image path.
    /// Validates full executable image path for genuine game installations.
    pub fn find_process_with_path(name: &str) -> Option<(u32, String)> {
        let clean_target = name.to_lowercase();
        unsafe {
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return None,
            };

            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            let mut found = None;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let exe = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_lowercase();

                    if exe == clean_target {
                        let pid = entry.th32ProcessID;
                        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) {
                            let path = get_process_image_path(handle).unwrap_or_default();
                            let _ = CloseHandle(handle);
                            if Self::is_legitimate_game_path(name, &path) {
                                found = Some((pid, path));
                                break;
                            } else {
                                warn!(pid = pid, path = %path, "Process matched target binary name but failed full path validation");
                            }
                        }
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }

            let _ = CloseHandle(snapshot);
            found
        }
    }

    /// Terminate background Riot Client CEF UI processes (`RiotClientUx.exe`).
    /// Crucial:
    /// - Verifies full executable path is inside genuine Riot Client folder.
    /// - Preserves `RiotClientServices.exe` (which maintains Vanguard session).
    /// - Never terminates `vgc.exe` or `vgk.sys`.
    pub fn purge_riot_client_cef() {
        unsafe {
            let snapshot = match CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                Ok(h) => h,
                Err(_) => return,
            };

            let mut entry = PROCESSENTRY32W::default();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

            if Process32FirstW(snapshot, &mut entry).is_ok() {
                loop {
                    let exe = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_lowercase();

                    if exe == "riotclientux.exe" || exe == "riotclientuxrender.exe" {
                        // Verify full path to ensure it is the genuine Riot Client CEF frontend
                        if let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID) {
                            if let Some(path) = get_process_image_path(handle) {
                                let path_lower = path.to_lowercase();
                                if path_lower.contains("riot client") || path_lower.contains("riotclient") {
                                    info!(pid = entry.th32ProcessID, name = %exe, path = %path, "Terminating Riot Client CEF frontend");
                                    let _ = terminate_process_gracefully(entry.th32ProcessID, &exe, 1000);
                                } else {
                                    warn!(pid = entry.th32ProcessID, path = %path, "Skipping process matching CEF name outside Riot Client directory");
                                }
                            }
                            let _ = CloseHandle(handle);
                        }
                    }

                    if Process32NextW(snapshot, &mut entry).is_err() {
                        break;
                    }
                }
            }

            let _ = CloseHandle(snapshot);
        }
    }

    /// Apply `HIGH_PRIORITY_CLASS` and optional P-Core CPU affinity to the game process.
    pub fn optimize_game_process(&self, pid: u32) -> Result<(), String> {
        unsafe {
            let handle = OpenProcess(
                PROCESS_SET_INFORMATION | PROCESS_QUERY_INFORMATION,
                false,
                pid,
            ).map_err(|e| format!("Failed to open game PID {} for optimization: {}", pid, e))?;

            // 1. Set High Process Priority Class
            let prio_res = SetPriorityClass(handle, HIGH_PRIORITY_CLASS);
            if prio_res.is_err() {
                warn!("Failed to set HIGH_PRIORITY_CLASS on PID {}: {:?}", pid, prio_res);
            } else {
                info!(pid = pid, "Successfully applied HIGH_PRIORITY_CLASS (13) to game process");
            }

            // 2. Set P-Core Affinity if configured
            if let Some(mask) = self.config.p_core_affinity_mask {
                let aff_res = SetProcessAffinityMask(handle, mask);
                if aff_res.is_err() {
                    warn!("Failed to set affinity mask {:#x} on PID {}: {:?}", mask, pid, aff_res);
                } else {
                    info!(pid = pid, mask = format!("{:#x}", mask), "Set P-core CPU affinity mask");
                }
            }

            let _ = CloseHandle(handle);
        }

        // 3. Purge Riot Client CEF UI if requested
        if self.config.terminate_cef_ui {
            Self::purge_riot_client_cef();
        }

        Ok(())
    }

    /// Supervise the active game process until exit. Consumes 0.0% CPU by using `WaitForSingleObject`.
    /// Periodically verifies `stop_signal` (every 1000ms) to allow cancellation without leaks.
    /// Returns `Ok(true)` if process exited normally, `Ok(false)` if cancelled by `stop_signal`.
    pub fn wait_for_game_exit(&self, pid: u32, stop_signal: &Arc<AtomicBool>) -> Result<bool, String> {
        info!(pid = pid, "Entering low-overhead passive supervision state for game process");
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open game PID {} for synchronization: {}", pid, e))?;

            // Wait in 1000ms increments to allow cancellation checks while using 0% CPU
            loop {
                if stop_signal.load(std::sync::atomic::Ordering::Relaxed) {
                    info!(pid = pid, "Passive game supervision cancelled by stop signal");
                    let _ = CloseHandle(handle);
                    return Ok(false);
                }

                let wait = WaitForSingleObject(handle, 1000);
                if wait.0 == 0 {
                    // Object signaled: Process exited!
                    info!(pid = pid, "Target game process terminated cleanly");
                    let _ = CloseHandle(handle);
                    return Ok(true);
                }

                let mut exit_code: u32 = 0;
                let _ = GetExitCodeProcess(handle, &mut exit_code);
                if exit_code != (STILL_ACTIVE.0 as u32) {
                    info!(pid = pid, exit_code = exit_code, "Target game process exited");
                    let _ = CloseHandle(handle);
                    return Ok(true);
                }
            }
        }
    }

    /// Spawn an asynchronous background watcher thread to supervise the game process lifecycle (`TASK-FUNC-02`).
    ///
    /// The watcher:
    /// 1. Polls for target binary until active.
    /// 2. Verifies process identity and path.
    /// 3. Applies `HIGH_PRIORITY_CLASS` (13) and P-core affinity mask.
    /// 4. Optionally purges Riot Client CEF frontend.
    /// 5. Enters passive 0.0% CPU wait state via `WaitForSingleObject`.
    /// 6. Upon game exit, triggers post-match application relaunch and restores baseline optimizations.
    pub fn start_background_supervision(
        config: GameSupervisorConfig,
        backups: Vec<TerminatedAppBackup>,
        stop_signal: Arc<AtomicBool>,
    ) -> std::thread::JoinHandle<()> {
        std::thread::spawn(move || {
            let supervisor = ProcessSupervisor::new(config.clone());
            info!(target = %config.target_binary, "Game lifecycle supervisor active; polling for process startup");

            // 1. Wait for game process to appear
            let mut detected_pid = None;
            while !stop_signal.load(std::sync::atomic::Ordering::Relaxed) {
                if let Some((pid, path)) = Self::find_process_with_path(&config.target_binary) {
                    info!(pid = pid, path = %path, target = %config.target_binary, "Target game process detected!");
                    detected_pid = Some(pid);
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(config.poll_interval_ms));
            }

            let pid = match detected_pid {
                Some(p) => p,
                None => {
                    info!("Supervision cancelled before target process appeared");
                    return;
                }
            };

            // 2. Apply game optimizations (priority, affinity, CEF purge)
            if let Err(e) = supervisor.optimize_game_process(pid) {
                warn!(pid = pid, error = %e, "Failed to apply full optimizations to game process");
            }

            // 3. Passive wait state for game exit
            let clean_exit = match supervisor.wait_for_game_exit(pid, &stop_signal) {
                Ok(status) => status,
                Err(e) => {
                    warn!(pid = pid, error = %e, "Passive synchronization error during game supervision");
                    false
                }
            };

            if !clean_exit || stop_signal.load(std::sync::atomic::Ordering::Relaxed) {
                info!("Game supervision cancelled or stopped prematurely. Skipping post-match restoration.");
                return;
            }

            // 4. Game has exited! Post-match cleanup and restoration
            info!("Target game process has exited. Initiating post-match restoration sequence");

            // Relaunch closed Tier 2 apps
            if !backups.is_empty() {
                info!(count = backups.len(), "Relaunching user applications post-match");
                let relaunch_outcomes = relaunch_applications(&backups);
                let successes = relaunch_outcomes.iter().filter(|r| r.is_ok()).count();
                info!(successful = successes, total = backups.len(), "Post-match applications relaunched");
            }

            // Trigger optimization rollback / restore baseline
            match crate::optimizations::OptimizationCoordinator::recover_orphaned_transaction() {
                Ok(restored) => {
                    info!(restored = restored, "Post-match system optimization rollback completed");
                }
                Err(e) => {
                    warn!(error = %e, "Error recovering post-match optimization state");
                }
            }

            info!("Game lifecycle supervision complete. State returned to baseline.");
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows::Win32::System::Threading::GetPriorityClass;

    fn get_unique_mock_target(name: &str) -> std::path::PathBuf {
        let temp_exe = std::env::temp_dir().join(name);
        let windir = std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".to_string());
        let src = std::path::PathBuf::from(windir).join("System32").join("cmd.exe");
        let _ = std::fs::copy(&src, &temp_exe);
        temp_exe
    }

    #[test]
    fn test_optimize_process_priority() {
        let test_name = "valopt_supervisor_test_prio.exe";
        let test_exe = get_unique_mock_target(test_name);

        let child = std::process::Command::new(&test_exe)
            .args(["/c", "ping -n 10 127.0.0.1 >nul"])
            .spawn()
            .expect("Failed to launch test mock process");

        let pid = child.id();
        std::thread::sleep(Duration::from_millis(200));

        let config = GameSupervisorConfig {
            target_binary: test_name.to_string(),
            p_core_affinity_mask: None,
            terminate_cef_ui: false,
            poll_interval_ms: 100,
        };
        let supervisor = ProcessSupervisor::new(config);

        // Optimize priority
        supervisor.optimize_game_process(pid).expect("Optimization should succeed");

        // Verify priority changed to HIGH_PRIORITY_CLASS
        unsafe {
            let handle = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid).unwrap();
            let prio = GetPriorityClass(handle);
            let _ = CloseHandle(handle);
            assert_eq!(prio, HIGH_PRIORITY_CLASS.0, "Priority must be HIGH_PRIORITY_CLASS");
        }

        // Clean up
        let _ = terminate_process_gracefully(pid, test_name, 1000);
        let _ = std::fs::remove_file(&test_exe);
    }

    #[test]
    fn test_find_process_with_path() {
        let test_name = "valopt_supervisor_test_path.exe";
        let test_exe = get_unique_mock_target(test_name);

        let child = std::process::Command::new(&test_exe)
            .args(["/c", "ping -n 10 127.0.0.1 >nul"])
            .spawn()
            .expect("Failed to spawn mock process");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(150));

        let found = ProcessSupervisor::find_process_with_path(test_name);
        assert!(found.is_some(), "Must discover running mock target with path");

        let (found_pid, path) = found.unwrap();
        assert_eq!(found_pid, pid);
        assert!(path.to_lowercase().ends_with(test_name));
        assert!(std::path::Path::new(&path).is_file());

        let _ = terminate_process_gracefully(pid, test_name, 1000);
        let _ = std::fs::remove_file(&test_exe);
    }

    #[test]
    fn test_background_supervision_lifecycle() {
        let test_name = "valopt_supervisor_test_lifecycle.exe";
        let test_exe = get_unique_mock_target(test_name);
        let stop_signal = Arc::new(AtomicBool::new(false));

        // Start background supervision watching for mock target
        let config = GameSupervisorConfig {
            target_binary: test_name.to_string(),
            p_core_affinity_mask: None,
            terminate_cef_ui: false,
            poll_interval_ms: 50,
        };

        let backups = Vec::new();
        let handle = ProcessSupervisor::start_background_supervision(
            config,
            backups,
            stop_signal.clone(),
        );

        // Allow supervisor thread to start polling
        std::thread::sleep(Duration::from_millis(100));

        // Spawn mock target
        let child = std::process::Command::new(&test_exe)
            .args(["/c", "ping -n 10 127.0.0.1 >nul"])
            .spawn()
            .expect("Failed to spawn mock target for supervision test");
        let pid = child.id();

        // Wait for supervisor to detect and apply priority
        std::thread::sleep(Duration::from_millis(300));
        unsafe {
            if let Ok(p_handle) = OpenProcess(PROCESS_QUERY_INFORMATION, false, pid) {
                let prio = GetPriorityClass(p_handle);
                let _ = CloseHandle(p_handle);
                assert_eq!(prio, HIGH_PRIORITY_CLASS.0, "Supervisor must have applied HIGH_PRIORITY_CLASS");
            }
        }

        // Terminate mock target
        let _ = terminate_process_gracefully(pid, test_name, 1000);

        // Supervisor thread must detect exit and join cleanly
        let join_res = handle.join();
        assert!(join_res.is_ok(), "Supervisor thread must terminate cleanly after process exit");
        let _ = std::fs::remove_file(&test_exe);
    }

    #[test]
    fn test_wait_for_game_exit_aborts_on_stop_signal() {
        let test_name = "valopt_supervisor_test_cancel.exe";
        let test_exe = get_unique_mock_target(test_name);

        let child = std::process::Command::new(&test_exe)
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .expect("Failed to launch test mock process");
        let pid = child.id();
        std::thread::sleep(Duration::from_millis(150));

        let config = GameSupervisorConfig {
            target_binary: test_name.to_string(),
            p_core_affinity_mask: None,
            terminate_cef_ui: false,
            poll_interval_ms: 100,
        };
        let supervisor = ProcessSupervisor::new(config);
        let stop_signal = Arc::new(AtomicBool::new(false));

        let stop_clone = stop_signal.clone();
        let cancel_thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1100));
            stop_clone.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        let wait_start = std::time::Instant::now();
        let clean_exit = supervisor.wait_for_game_exit(pid, &stop_signal).expect("Wait must succeed");
        let wait_dur = wait_start.elapsed();

        assert!(!clean_exit, "Must report cancellation (false) instead of normal exit");
        assert!(wait_dur.as_millis() < 3000, "Must return shortly after stop_signal, elapsed: {:?}", wait_dur);

        let _ = cancel_thread.join();
        let _ = terminate_process_gracefully(pid, test_name, 1000);
        let _ = std::fs::remove_file(&test_exe);
    }

    #[test]
    fn test_spoofed_valorant_binary_rejected_by_path_validation() {
        let temp_dir = std::env::temp_dir();
        let spoofed_exe = temp_dir.join(VALORANT_BINARY_NAME);
        let windir = std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".to_string());
        let src = std::path::PathBuf::from(windir).join("System32").join("cmd.exe");
        let _ = std::fs::copy(&src, &spoofed_exe);

        if spoofed_exe.is_file() {
            let child = std::process::Command::new(&spoofed_exe)
                .args(["/c", "ping -n 10 127.0.0.1 >nul"])
                .spawn();

            if let Ok(mut c) = child {
                let spoofed_pid = c.id();
                std::thread::sleep(Duration::from_millis(150));

                // 1. ProcessSupervisor MUST reject the spoofed binary PID
                if let Some((detected_pid, detected_path)) = ProcessSupervisor::find_process_with_path(VALORANT_BINARY_NAME) {
                    assert_ne!(
                        detected_pid, spoofed_pid,
                        "Spoofed VALORANT binary (PID {}) in temp dir was accepted! Path: {}",
                        spoofed_pid, detected_path
                    );
                }

                // 2. Directly verify path validator rejects temp binary path
                let spoofed_path_str = spoofed_exe.to_string_lossy();
                assert!(
                    !ProcessSupervisor::is_legitimate_game_path(VALORANT_BINARY_NAME, &spoofed_path_str),
                    "is_legitimate_game_path must reject spoofed binary at {}",
                    spoofed_path_str
                );

                let _ = c.kill();
                let _ = c.wait();
            }
            let _ = std::fs::remove_file(&spoofed_exe);
        }
    }
}

