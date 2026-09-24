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
    PROCESS_QUERY_INFORMATION, PROCESS_SET_INFORMATION, PROCESS_SYNCHRONIZE,
};

use super::terminator::terminate_process_gracefully;

pub const VALORANT_BINARY_NAME: &str = "VALORANT-Win64-Shipping.exe";
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

    /// Search the running process list for a target executable name.
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

    /// Terminate background Riot Client CEF UI processes (`RiotClientUx.exe`).
    /// Crucial: Preserves `RiotClientServices.exe` (which maintains Vanguard session).
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
                        info!(pid = entry.th32ProcessID, name = %exe, "Terminating Riot Client CEF frontend");
                        let _ = terminate_process_gracefully(entry.th32ProcessID, &exe, 1000);
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
    pub fn wait_for_game_exit(&self, pid: u32) -> Result<(), String> {
        info!(pid = pid, "Entering low-overhead passive supervision state for game process");
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE | PROCESS_QUERY_INFORMATION, false, pid)
                .map_err(|e| format!("Failed to open game PID {} for synchronization: {}", pid, e))?;

            // Wait in 1000ms increments to allow cancellation checks while using 0% CPU
            loop {
                let wait = WaitForSingleObject(handle, 1000);
                if wait.0 == 0 {
                    // Object signaled: Process exited!
                    info!(pid = pid, "Target game process terminated cleanly");
                    break;
                }

                let mut exit_code: u32 = 0;
                let _ = GetExitCodeProcess(handle, &mut exit_code);
                if exit_code != (STILL_ACTIVE.0 as u32) {
                    info!(pid = pid, exit_code = exit_code, "Target game process exited");
                    break;
                }
            }

            let _ = CloseHandle(handle);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use windows::Win32::System::Threading::GetPriorityClass;

    #[test]
    fn test_optimize_process_priority() {
        // Spawn a test child process
        let child = std::process::Command::new("notepad.exe")
            .spawn()
            .expect("Failed to launch test notepad.exe");

        let pid = child.id();
        std::thread::sleep(Duration::from_millis(200));

        let config = GameSupervisorConfig {
            target_binary: "notepad.exe".to_string(),
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
        let _ = terminate_process_gracefully(pid, "notepad.exe", 1000);
    }
}
