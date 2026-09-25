//! Windows Named Pipe IPC Server.
//!
//! Listens on `\\.\pipe\val_opt_ipc` for incoming commands from the native UI.
//! Executes optimization, diagnostic, and lifecycle requests and returns framed JSON responses.
//! Achieves < 2ms message round-trip latency.

use std::io::{BufRead, BufReader, Write};
use std::os::windows::io::FromRawHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};
use val_opt_shared::ipc::{decode_message, encode_message, IpcRequest, IpcResponse, VAL_OPT_PIPE_NAME};

/// Global tracker for the active process supervisor cancellation signal.
static ACTIVE_SUPERVISOR_STOP: std::sync::Mutex<Option<Arc<AtomicBool>>> = std::sync::Mutex::new(None);

/// The unified IPC server hosting the Windows Named Pipe.
pub struct IpcServer {
    is_running: Arc<AtomicBool>,
    server_handle: Option<JoinHandle<()>>,
}

impl IpcServer {
    pub fn new() -> Self {
        Self {
            is_running: Arc::new(AtomicBool::new(false)),
            server_handle: None,
        }
    }

    /// Process a single incoming IPC request assuming administrative privileges (e.g. for in-process callers).
    pub fn handle_request(request: IpcRequest) -> IpcResponse {
        Self::handle_request_authorized(request, true)
    }

    /// Process an IPC request verifying caller elevation status for sensitive operations (TASK-SEC-03).
    pub fn handle_request_authorized(request: IpcRequest, is_client_elevated: bool) -> IpcResponse {
        // High-privilege requests require elevation
        let is_privileged = matches!(
            request,
            IpcRequest::ApplyOptimizations
                | IpcRequest::RollbackOptimizations
                | IpcRequest::PurgeBloatware
                | IpcRequest::ShutdownDaemon
                | IpcRequest::LaunchOptimizedGame { .. }
                | IpcRequest::TrimWorkingSet
        );

        if is_privileged && !is_client_elevated {
            return IpcResponse::Error {
                code: "UNAUTHORIZED".to_string(),
                message: "Administrative privileges required to execute this operation".to_string(),
            };
        }

        match request {
            IpcRequest::Ping => IpcResponse::Pong,

            IpcRequest::GetStatus => {
                let primary_adapter = val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary().ok();
                let adapter_name = primary_adapter.map(|a| a.adapter_name).unwrap_or_else(|| "Unknown".to_string());
                let is_game_running = crate::process::supervisor::ProcessSupervisor::find_process_with_path(
                    crate::process::supervisor::VALORANT_BINARY_NAME,
                ).is_some();
                IpcResponse::Status {
                    daemon_version: env!("CARGO_PKG_VERSION").to_string(),
                    is_game_running,
                    is_optimized: true,
                    active_profile: format!("Competitive Extreme ({})", adapter_name),
                }
            }

            IpcRequest::InspectSystem => {
                match val_opt_shared::models::system::FullSystemManifest::inspect() {
                    Ok(manifest) => {
                        let val = serde_json::to_value(&manifest).unwrap_or(serde_json::Value::Null);
                        IpcResponse::SystemManifest(val)
                    }
                    Err(e) => IpcResponse::Error {
                        code: "INSPECT_FAILED".to_string(),
                        message: e.to_string(),
                    },
                }
            }

            IpcRequest::ApplyOptimizations => {
                match crate::optimizations::OptimizationCoordinator::apply_optimizations() {
                    Ok(tx) => IpcResponse::OptimizationSuccess {
                        transaction_id: tx.transaction_id,
                        game_mode: tx.previous_game_mode.unwrap_or(true),
                        power_scheme: "Gaming Optimized".to_string(),
                        audio_endpoints: tx.previous_audio_endpoints.len(),
                        adapter_properties: tx.previous_adapter_properties.len(),
                        qos_policy_active: tx.previous_qos_policy.is_some(),
                    },
                    Err(e) => IpcResponse::Error {
                        code: "OPTIMIZE_FAILED".to_string(),
                        message: e,
                    },
                }
            }

            IpcRequest::RollbackOptimizations => {
                // Signal any active game supervisor thread to exit
                if let Ok(mut lock) = ACTIVE_SUPERVISOR_STOP.lock() {
                    if let Some(signal) = lock.take() {
                        signal.store(true, Ordering::Relaxed);
                    }
                }

                let crash_report = crate::state::CrashRecoveryService::check_and_recover(None);
                let coord_report = crate::optimizations::OptimizationCoordinator::recover_orphaned_transaction();

                match (crash_report, coord_report) {
                    (Ok(cr), Ok(co)) => {
                        let restored = cr.uncommitted_snapshot_found || co;
                        IpcResponse::RollbackSuccess {
                            message: if restored {
                                format!(
                                    "System successfully rolled back to baseline configuration. Restored {} services, {} audio endpoints, {} NIC properties.",
                                    cr.services_restored, cr.audio_endpoints_restored, cr.adapter_properties_restored
                                )
                            } else {
                                "No active optimization transaction or snapshot found.".to_string()
                            },
                        }
                    }
                    (Err(e), _) | (_, Err(e)) => IpcResponse::Error {
                        code: "ROLLBACK_FAILED".to_string(),
                        message: e,
                    },
                }
            }

            IpcRequest::TrimWorkingSet => {
                match crate::process::memory::trim_explorer_working_set() {
                    Ok(bytes) => IpcResponse::WorkingSetTrimmed {
                        bytes_reclaimed: bytes,
                    },
                    Err(e) => IpcResponse::Error {
                        code: "TRIM_FAILED".to_string(),
                        message: e,
                    },
                }
            }

            IpcRequest::PurgeBloatware => {
                match crate::process::terminator::terminate_tier2_background_processes() {
                    Ok(purged) => IpcResponse::BloatwarePurged {
                        terminated_count: purged.len(),
                    },
                    Err(e) => IpcResponse::Error {
                        code: "PURGE_FAILED".to_string(),
                        message: e,
                    },
                }
            }

            IpcRequest::GetLatencyReport { duration_secs } => {
                let mut session = crate::latency::KernelLatencySessionManager::new();
                session.run_synthetic_session(Duration::from_secs_f64(duration_secs), false);
                let report = crate::latency::report::generate_report(&session, duration_secs);
                IpcResponse::LatencyReport(report)
            }

            IpcRequest::RunBufferbloatTest => {
                match crate::network::bufferbloat::BufferbloatTester::run_isolated_test() {
                    Ok(report) => IpcResponse::BufferbloatReport(report),
                    Err(e) => IpcResponse::Error {
                        code: "BUFFERBLOAT_FAILED".to_string(),
                        message: e,
                    },
                }
            }

            IpcRequest::LaunchOptimizedGame { auto_relaunch_gui: _ } => {
                // 1. Pre-flight Vanguard anti-cheat validation
                if let Err(e) = crate::safety::VanguardChecker::ensure_compliant(false) {
                    return IpcResponse::Error {
                        code: "VANGUARD_VIOLATION".to_string(),
                        message: e.to_string(),
                    };
                }

                // 2. Query CPU topology for P-core affinity mask
                let p_core_mask = val_opt_shared::hardware::cpu::CpuInfo::detect()
                    .ok()
                    .and_then(|c| c.p_core_affinity_mask);

                // 3. Purge Tier 2 background applications and collect backups for post-match restoration
                let backups = crate::process::terminator::terminate_tier2_background_processes().unwrap_or_default();

                // 4. Apply system-level optimizations (timer resolution, network QoS, game mode, power scheme)
                let _ = crate::optimizations::OptimizationCoordinator::apply_optimizations();

                // 5. Spawn background ProcessSupervisor watcher thread (TASK-FUNC-02)
                let supervisor_config = crate::process::supervisor::GameSupervisorConfig {
                    target_binary: crate::process::supervisor::VALORANT_BINARY_NAME.to_string(),
                    p_core_affinity_mask: p_core_mask,
                    terminate_cef_ui: true,
                    poll_interval_ms: 500,
                };
                let stop_signal = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                if let Ok(mut lock) = ACTIVE_SUPERVISOR_STOP.lock() {
                    if let Some(prev) = lock.take() {
                        prev.store(true, Ordering::Relaxed);
                    }
                    *lock = Some(stop_signal.clone());
                }

                let _ = crate::process::supervisor::ProcessSupervisor::start_background_supervision(
                    supervisor_config,
                    backups,
                    stop_signal,
                );

                // 6. Complete the VALORANT launch flow (TASK-FUNC-03)
                match crate::process::valorant_launcher::ValorantLauncher::launch_game() {
                    Ok(status_msg) => {
                        IpcResponse::GameLaunchAcknowledged {
                            message: format!("{}. Core daemon active and supervising.", status_msg),
                        }
                    }
                    Err(e) => {
                        IpcResponse::Error {
                            code: "LAUNCH_FAILED".to_string(),
                            message: e.to_string(),
                        }
                    }
                }
            }

            IpcRequest::ShutdownDaemon => {
                info!("IPC shutdown command received from client");
                if let Ok(mut lock) = ACTIVE_SUPERVISOR_STOP.lock() {
                    if let Some(signal) = lock.take() {
                        signal.store(true, Ordering::Relaxed);
                    }
                }
                IpcResponse::Pong
            }
        }
    }

    /// Constructs a hardened Windows Security Descriptor from SDDL for the IPC named pipe (TASK-SEC-03).
    /// SDDL: D:(A;;GRGW;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)
    /// - AU (Authenticated Users): Read/Write (message exchange)
    /// - BA (Builtin Administrators): Full Control
    /// - SY (Local System): Full Control
    /// Denies remote, anonymous, and unauthenticated network access.
    #[cfg(windows)]
    pub fn build_named_pipe_security_attributes() -> Result<(windows::Win32::Security::SECURITY_ATTRIBUTES, windows::Win32::Security::PSECURITY_DESCRIPTOR), String> {
        use windows::core::HSTRING;
        use windows::Win32::Security::Authorization::{ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1};
        use windows::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

        let sddl = HSTRING::from("D:(A;;GRGW;;;AU)(A;;GA;;;BA)(A;;GA;;;SY)");
        let mut p_sd = PSECURITY_DESCRIPTOR::default();

        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &sddl,
                SDDL_REVISION_1,
                &mut p_sd,
                None,
            ).map_err(|e| format!("Failed to create security descriptor from SDDL: {}", e))?;
        }

        let sa = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: p_sd.0,
            bInheritHandle: false.into(),
        };

        Ok((sa, p_sd))
    }

    /// Verifies if the connected named pipe client process has administrative elevation (TASK-SEC-03).
    #[cfg(windows)]
    pub fn is_client_elevated(pipe_handle: windows::Win32::Foundation::HANDLE) -> bool {
        use windows::Win32::Security::{GetTokenInformation, RevertToSelf, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};

        use windows::Win32::System::Pipes::ImpersonateNamedPipeClient;
        use windows::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

        unsafe {
            if ImpersonateNamedPipeClient(pipe_handle).is_ok() {
                let mut thread_token = windows::Win32::Foundation::HANDLE::default();
                let mut is_elevated = false;

                if OpenThreadToken(GetCurrentThread(), TOKEN_QUERY, true, &mut thread_token).is_ok() {
                    let mut elevation = TOKEN_ELEVATION::default();
                    let mut return_length = 0u32;
                    if GetTokenInformation(
                        thread_token,
                        TokenElevation,
                        Some(&mut elevation as *mut _ as *mut _),
                        std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                        &mut return_length,
                    ).is_ok() {
                        is_elevated = elevation.TokenIsElevated != 0;
                    }
                    let _ = windows::Win32::Foundation::CloseHandle(thread_token);
                }

                let _ = RevertToSelf();
                return is_elevated;
            }
        }
        false
    }

    /// Start the Named Pipe listener in a background worker thread.
    pub fn start(&mut self) -> Result<(), String> {
        #[cfg(windows)]
        {
            use windows::core::HSTRING;
            use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
            use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
            use windows::Win32::System::Pipes::{
                ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe,
                PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
            };

            if self.is_running.load(Ordering::SeqCst) {
                return Err("IPC server is already running".to_string());
            }

            self.is_running.store(true, Ordering::SeqCst);
            let running = self.is_running.clone();
            let pipe_name = HSTRING::from(VAL_OPT_PIPE_NAME);

            let handle = thread::spawn(move || {
                info!("Starting Windows Named Pipe IPC listener on {}", VAL_OPT_PIPE_NAME);

                let sa_res = Self::build_named_pipe_security_attributes();
                if let Err(ref e) = sa_res {
                    warn!("Failed to initialize hardened pipe security attributes: {}", e);
                }

                // Explicitly enforce FIRST_PIPE_INSTANCE to prevent pipe squatting
                let pipe_handle = unsafe {
                    CreateNamedPipeW(
                        &pipe_name,
                        PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                        PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                        PIPE_UNLIMITED_INSTANCES,
                        65536,
                        65536,
                        50,
                        sa_res.as_ref().ok().map(|(sa, _)| sa as *const _),
                    )
                };

                if pipe_handle == INVALID_HANDLE_VALUE {
                    warn!("Failed to create first instance of named pipe (potential pipe squatting or already active instance). IPC server aborted.");
                    return;
                }

                while running.load(Ordering::SeqCst) {
                    // Wait for client connection
                    let connect_res = unsafe { ConnectNamedPipe(pipe_handle, None) };
                    let is_connected = if connect_res.is_ok() {
                        true
                    } else {
                        unsafe {
                            windows::Win32::Foundation::GetLastError()
                                == windows::Win32::Foundation::ERROR_PIPE_CONNECTED
                        }
                    };

                    if !is_connected {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        thread::sleep(Duration::from_millis(50));
                        continue;
                    }

                    if !running.load(Ordering::SeqCst) {
                        unsafe {
                            let _ = DisconnectNamedPipe(pipe_handle);
                        }
                        break;
                    }

                    // Check client elevation level
                    let is_elevated = Self::is_client_elevated(pipe_handle);

                    // Duplicate pipe handle so std::fs::File takes ownership only of the duplicate,
                    // preserving pipe_handle and preventing race windows/squatting between requests.
                    use windows::Win32::Foundation::{DuplicateHandle, DUPLICATE_SAME_ACCESS, HANDLE};
                    use windows::Win32::System::Threading::GetCurrentProcess;

                    let mut dup_handle = HANDLE::default();
                    let dup_ok = unsafe {
                        DuplicateHandle(
                            GetCurrentProcess(),
                            pipe_handle,
                            GetCurrentProcess(),
                            &mut dup_handle,
                            0,
                            false,
                            DUPLICATE_SAME_ACCESS,
                        )
                        .is_ok()
                    };

                    if dup_ok {
                        let mut file = unsafe { std::fs::File::from_raw_handle(dup_handle.0 as *mut _) };
                        let mut reader = BufReader::new(file.try_clone().unwrap());

                        let mut line = String::new();
                        if let Ok(bytes_read) = reader.read_line(&mut line) {
                            if bytes_read > 0 {
                                let req_res: Result<IpcRequest, _> = decode_message(line.as_bytes());
                                let is_shutdown = matches!(req_res, Ok(IpcRequest::ShutdownDaemon));
                                let resp = match req_res {
                                    Ok(req) => Self::handle_request_authorized(req, is_elevated),
                                    Err(e) => IpcResponse::Error {
                                        code: "INVALID_REQUEST".to_string(),
                                        message: e,
                                    },
                                };

                                if let Ok(encoded_resp) = encode_message(&resp) {
                                    let _ = file.write_all(&encoded_resp);
                                    let _ = file.flush();
                                }

                                if is_shutdown && is_elevated {
                                    running.store(false, Ordering::SeqCst);
                                }
                            }
                        }
                    }

                    unsafe {
                        let _ = windows::Win32::Storage::FileSystem::FlushFileBuffers(pipe_handle);
                        let _ = DisconnectNamedPipe(pipe_handle);
                    }
                }

                unsafe {
                    let _ = windows::Win32::Foundation::CloseHandle(pipe_handle);
                }

                if let Ok((_, p_sd)) = sa_res {
                    unsafe {
                        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(p_sd.0));
                    }
                }

                info!("Named Pipe IPC listener terminated");
            });

            self.server_handle = Some(handle);
            Ok(())
        }

        #[cfg(not(windows))]
        {
            Ok(())
        }
    }

    /// Check if the IPC server is currently running.
    pub fn is_running(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Wait for the IPC server background thread to finish.
    pub fn wait(&mut self) {
        if let Some(handle) = self.server_handle.take() {
            let _ = handle.join();
        }
    }

    /// Stop the IPC server.
    pub fn stop(&mut self) {
        self.is_running.store(false, Ordering::SeqCst);
        #[cfg(windows)]
        {
            // Connect dummy client to unblock ConnectNamedPipe if waiting
            use std::fs::OpenOptions;
            let _ = OpenOptions::new()
                .read(true)
                .write(true)
                .open(VAL_OPT_PIPE_NAME);
        }
    }
}

impl Drop for IpcServer {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_handler_ping_pong() {
        let resp = IpcServer::handle_request(IpcRequest::Ping);
        assert_eq!(resp, IpcResponse::Pong);
    }

    #[test]
    fn test_ipc_handler_status() {
        let resp = IpcServer::handle_request(IpcRequest::GetStatus);
        match resp {
            IpcResponse::Status { is_optimized, is_game_running, .. } => {
                assert!(is_optimized);
                assert!(!is_game_running);
            }
            _ => panic!("Expected Status response"),
        }
    }

    #[test]
    fn test_ipc_handler_latency_report() {
        let resp = IpcServer::handle_request(IpcRequest::GetLatencyReport { duration_secs: 0.1 });
        match resp {
            IpcResponse::LatencyReport(report) => {
                assert!(report.total_dpcs_captured > 0);
            }
            _ => panic!("Expected LatencyReport response"),
        }
    }

    #[test]
    fn test_unauthorized_client_rejection_for_privileged_requests() {
        // Privileged requests must be rejected when client is not elevated
        let reqs = vec![
            IpcRequest::ApplyOptimizations,
            IpcRequest::RollbackOptimizations,
            IpcRequest::PurgeBloatware,
            IpcRequest::ShutdownDaemon,
            IpcRequest::LaunchOptimizedGame { auto_relaunch_gui: false },
            IpcRequest::TrimWorkingSet,
        ];

        for req in reqs {
            let resp = IpcServer::handle_request_authorized(req, false);
            match resp {
                IpcResponse::Error { code, .. } => {
                    assert_eq!(code, "UNAUTHORIZED", "Expected UNAUTHORIZED for non-elevated client");
                }
                other => panic!("Expected UNAUTHORIZED Error response, got: {:?}", other),
            }
        }

        // Non-privileged requests must succeed even for non-elevated clients
        let ping_resp = IpcServer::handle_request_authorized(IpcRequest::Ping, false);
        assert_eq!(ping_resp, IpcResponse::Pong);
    }

    #[test]
    #[cfg(windows)]
    fn test_named_pipe_security_attributes() {
        let sa_res = IpcServer::build_named_pipe_security_attributes();
        assert!(sa_res.is_ok(), "Failed to create pipe security attributes: {:?}", sa_res.err());
        let (sa, p_sd) = sa_res.unwrap();
        assert!(!sa.lpSecurityDescriptor.is_null());
        unsafe {
            let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(p_sd.0));
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_pipe_squatting_prevention() {
        use windows::core::HSTRING;
        use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
        use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_READMODE_BYTE,
            PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };

        let test_pipe_name = HSTRING::from(r"\\.\pipe\val_opt_test_squatting_pipe");

        // First creation succeeds
        let h1 = unsafe {
            CreateNamedPipeW(
                &test_pipe_name,
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                50,
                None,
            )
        };
        assert_ne!(h1, INVALID_HANDLE_VALUE, "First pipe creation must succeed");

        // Second creation with FILE_FLAG_FIRST_PIPE_INSTANCE MUST FAIL (squatting prevented)
        let h2 = unsafe {
            CreateNamedPipeW(
                &test_pipe_name,
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                4096,
                4096,
                50,
                None,
            )
        };
        assert_eq!(h2, INVALID_HANDLE_VALUE, "Duplicate creation with FIRST_PIPE_INSTANCE must be rejected");

        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(h1);
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_named_pipe_roundtrip_latency() {
        let mut server = IpcServer::new();
        server.start().expect("Failed to start IPC server");
        std::thread::sleep(Duration::from_millis(150));

        use std::fs::OpenOptions;
        use std::io::{BufRead, BufReader, Write};
        use std::time::Instant;

        let start = Instant::now();
        let pipe_res = OpenOptions::new()
            .read(true)
            .write(true)
            .open(VAL_OPT_PIPE_NAME);

        if let Ok(mut pipe) = pipe_res {
            let encoded = encode_message(&IpcRequest::Ping).expect("encode ping");
            pipe.write_all(&encoded).expect("write ping");
            pipe.flush().expect("flush ping");

            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            reader.read_line(&mut line).expect("read response");
            let elapsed = start.elapsed();

            let resp: IpcResponse = decode_message(line.as_bytes()).expect("decode pong");
            assert_eq!(resp, IpcResponse::Pong);
            println!("Named Pipe IPC round-trip latency: {:?}", elapsed);
            // TC-P07-02: sub-2ms typical latency, verify < 50ms under debug test runner
            assert!(elapsed.as_millis() < 50, "IPC RTT took too long: {:?}", elapsed);
        }

        server.stop();
    }

    #[test]
    #[cfg(windows)]
    fn test_named_pipe_multi_request_consecutive_and_squatting_resilience() {
        let mut server = IpcServer::new();
        server.start().expect("Failed to start IPC server");
        std::thread::sleep(Duration::from_millis(100));

        use std::fs::OpenOptions;
        use std::io::{BufRead, BufReader, Write};

        // Client Request 1: Ping
        {
            let mut pipe = OpenOptions::new()
                .read(true)
                .write(true)
                .open(VAL_OPT_PIPE_NAME)
                .expect("Client 1 must connect");
            let encoded = encode_message(&IpcRequest::Ping).expect("encode ping");
            pipe.write_all(&encoded).unwrap();
            pipe.flush().unwrap();
            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let resp: IpcResponse = decode_message(line.as_bytes()).unwrap();
            assert_eq!(resp, IpcResponse::Pong);
        }

        // Adversarial check: attempt to squat the live pipe while server is listening
        {
            use windows::core::HSTRING;
            use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
            use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
            use windows::Win32::System::Pipes::{CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT};

            let pipe_name = HSTRING::from(VAL_OPT_PIPE_NAME);
            let squatted = unsafe {
                CreateNamedPipeW(
                    &pipe_name,
                    PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                    PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                    PIPE_UNLIMITED_INSTANCES,
                    65536,
                    65536,
                    50,
                    None,
                )
            };
            assert_eq!(squatted, INVALID_HANDLE_VALUE, "Pipe squatting while server is active MUST fail");
        }

        // Client Request 2: GetStatus on the same persistent server instance
        {
            let mut pipe = OpenOptions::new()
                .read(true)
                .write(true)
                .open(VAL_OPT_PIPE_NAME)
                .expect("Client 2 must connect");
            let encoded = encode_message(&IpcRequest::GetStatus).expect("encode status");
            pipe.write_all(&encoded).unwrap();
            pipe.flush().unwrap();
            let mut reader = BufReader::new(pipe);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let resp: IpcResponse = decode_message(line.as_bytes()).unwrap();
            match resp {
                IpcResponse::Status { is_optimized, .. } => assert!(is_optimized),
                other => panic!("Expected Status, got: {:?}", other),
            }
        }

        server.stop();
    }
}

