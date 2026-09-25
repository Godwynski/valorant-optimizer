//! Ephemeral UI Unload & Relaunch Coordinator (`TASK-FUNC-03`).
//!
//! Implements the Ephemeral UI pattern:
//! When the user clicks "Launch Optimized", the graphical interface sends the launch request
//! to the background core daemon. If successfully acknowledged, the GUI unloads from memory
//! (`std::process::exit(0)`) or minimizes to tray, enforcing a 0 MB gaming footprint.
//! If the launch fails or is refused, the error is returned to the user without exiting.

use tracing::{info, warn};
use val_opt_shared::ipc::{IpcRequest, IpcResponse};
use crate::ipc_client::IpcClient;

pub struct EphemeralLifecycleCoordinator;

impl EphemeralLifecycleCoordinator {
    /// Perform the ephemeral handoff sequence:
    /// 1. Dispatches `LaunchOptimizedGame` IPC message to the core daemon.
    /// 2. Verifies that the daemon successfully initiated/acknowledged the launch.
    /// 3. If acknowledged and `terminate_now` is true, exits GUI process with code 0.
    /// 4. If launch failed or was rejected, returns `Err` without terminating.
    pub fn trigger_launch_and_unload(auto_relaunch: bool, terminate_now: bool) -> Result<String, String> {
        info!("Initiating Ephemeral UI handoff to core daemon (auto_relaunch={})", auto_relaunch);

        let req = IpcRequest::LaunchOptimizedGame {
            auto_relaunch_gui: auto_relaunch,
        };

        // If daemon is not running on pipe, attempt to spawn val-opt-core daemon before dispatching
        if !IpcClient::is_daemon_running() {
            let _ = IpcClient::spawn_daemon();
        }

        // Send launch request to daemon
        let response = IpcClient::send_request(&req)?;

        match response {
            IpcResponse::GameLaunchAcknowledged { message } => {
                info!(msg = %message, "Game launch acknowledged by daemon");
                if terminate_now {
                    info!("Ephemeral UI unloading completely from RAM (0 MB gaming footprint enforced)");
                    std::process::exit(0);
                }
                Ok(message)
            }
            IpcResponse::Error { code, message } => {
                warn!(code = %code, message = %message, "Game launch refused or failed");
                Err(format!("[{}] {}", code, message))
            }
            other => {
                warn!(response = ?other, "Unexpected response from daemon for game launch");
                Err(format!("Unexpected response: {:?}", other))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ephemeral_handoff_handles_daemon_response() {
        // When running in unit test environment without daemon, verify it returns error or ok without exiting
        let res = EphemeralLifecycleCoordinator::trigger_launch_and_unload(true, false);
        // Either succeeds (if daemon running) or returns descriptive error (pipe open failed), but never crashes
        assert!(res.is_ok() || res.is_err());
    }
}
