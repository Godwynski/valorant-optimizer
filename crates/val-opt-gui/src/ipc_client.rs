//! Named Pipe IPC Client for the Native Slint GUI.
//!
//! Connects to `\\.\pipe\val_opt_ipc` to dispatch commands and receive status updates.
//! Features automatic daemon discovery/spawning and seamless fallback to standalone in-process execution.

use std::io::{BufRead, BufReader, Write};
use std::time::Instant;
use tracing::{debug, info, warn};
use val_opt_shared::ipc::{decode_message, encode_message, IpcRequest, IpcResponse, VAL_OPT_PIPE_NAME};

pub struct IpcClient;

impl IpcClient {
    /// Check if the core optimizer daemon is running and responding on the Named Pipe.
    pub fn is_daemon_running() -> bool {
        #[cfg(windows)]
        {
            use std::fs::OpenOptions;
            OpenOptions::new()
                .read(true)
                .write(true)
                .open(VAL_OPT_PIPE_NAME)
                .is_ok()
        }
        #[cfg(not(windows))]
        {
            false
        }
    }

    /// Attempt to spawn the background daemon executable if found on disk.
    pub fn spawn_daemon() -> bool {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            use std::process::Command;

            const CREATE_NO_WINDOW: u32 = 0x08000000;
            const DETACHED_PROCESS: u32 = 0x00000008;

            let mut candidates = Vec::new();

            // Candidate 1: Same directory as current running executable
            if let Ok(current_exe) = std::env::current_exe() {
                if let Some(parent) = current_exe.parent() {
                    candidates.push(parent.join("val-opt-core.exe"));
                }
            }

            // Candidate 2: target/debug or target/release relative to CWD
            candidates.push(std::path::PathBuf::from("target/release/val-opt-core.exe"));
            candidates.push(std::path::PathBuf::from("target/debug/val-opt-core.exe"));
            candidates.push(std::path::PathBuf::from(r"C:\Program Files\ValorantOptimizer\val-opt-core.exe"));

            for path in candidates {
                if path.exists() {
                    info!("Spawning background daemon from {}", path.display());
                    if Command::new(&path)
                        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS)
                        .spawn()
                        .is_ok()
                    {
                        // Give daemon a short window to initialize named pipe
                        std::thread::sleep(std::time::Duration::from_millis(200));
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Send an IPC request over the Windows Named Pipe and wait for the response frame.
    /// If the background daemon is not running, attempts auto-spawning and falls back
    /// seamlessly to in-process standalone execution.
    pub fn send_request(request: &IpcRequest) -> Result<IpcResponse, String> {
        let start = Instant::now();

        #[cfg(windows)]
        {
            use std::fs::OpenOptions;

            // Connect to named pipe with write/read access
            let mut pipe_res = OpenOptions::new()
                .read(true)
                .write(true)
                .open(VAL_OPT_PIPE_NAME);

            // If connection failed, attempt to auto-spawn the background daemon once
            if pipe_res.is_err() {
                if Self::spawn_daemon() {
                    pipe_res = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(VAL_OPT_PIPE_NAME);
                }
            }

            match pipe_res {
                Ok(mut pipe) => {
                    let encoded = encode_message(request)?;
                    pipe.write_all(&encoded)
                        .map_err(|e| format!("Failed to send IPC request: {}", e))?;
                    pipe.flush()
                        .map_err(|e| format!("Failed to flush pipe buffer: {}", e))?;

                    let mut reader = BufReader::new(pipe);
                    let mut line = String::new();
                    reader
                        .read_line(&mut line)
                        .map_err(|e| format!("Failed to read IPC response: {}", e))?;

                    let elapsed = start.elapsed();
                    debug!(rtt = ?elapsed, "IPC message round-trip completed");

                    decode_message(line.as_bytes())
                }
                Err(e) => {
                    // Daemon not running or inaccessible: fall back to standalone in-process execution
                    warn!("Named pipe connection to daemon unavailable ({}). Executing via standalone in-process dispatch.", e);
                    Ok(val_opt_core::ipc_server::IpcServer::handle_request(request.clone()))
                }
            }
        }

        #[cfg(not(windows))]
        {
            Ok(val_opt_core::ipc_server::IpcServer::handle_request(request.clone()))
        }
    }

    /// Check if the core optimizer daemon is running and responding.
    pub fn ping() -> bool {
        match Self::send_request(&IpcRequest::Ping) {
            Ok(IpcResponse::Pong) => true,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_client_standalone_dispatch_fallback() {
        // When daemon pipe is not active, client must fall back to standalone dispatch
        // and return a valid response rather than failing or panicking
        let res = IpcClient::send_request(&IpcRequest::Ping);
        assert!(res.is_ok(), "Standalone fallback should return Ok");
        assert_eq!(res.unwrap(), IpcResponse::Pong);
    }
}
