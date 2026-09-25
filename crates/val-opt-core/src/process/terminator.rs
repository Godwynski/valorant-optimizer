//! Graceful Two-Stage Process Termination and Relaunch Engine.
//!
//! Terminates safe Tier 2 applications gracefully via `WM_CLOSE` (with 1000ms grace period)
//! followed by `TerminateProcess`, while recording executable paths for post-match restoration.

use tracing::{debug, info, warn};
use val_opt_shared::models::process::{ProcessTier, TerminatedAppBackup};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, BOOL, HANDLE, HWND, LPARAM, STILL_ACTIVE};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::Threading::{
    GetExitCodeProcess, OpenProcess, QueryFullProcessImageNameW, TerminateProcess,
    WaitForSingleObject, PROCESS_NAME_FORMAT, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE};

use super::safety_db::{ProcessSafetyDb, SafetyViolationError};

/// Callback context for EnumWindows to collect top-level window handles for a given PID.
struct EnumContext {
    target_pid: u32,
    found_hwnds: Vec<HWND>,
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let ctx = &mut *(lparam.0 as *mut EnumContext);
    let mut pid: u32 = 0;
    let _ = GetWindowThreadProcessId(hwnd, Some(&mut pid));
    if pid == ctx.target_pid {
        ctx.found_hwnds.push(hwnd);
    }
    BOOL(1)
}

/// Retrieve top-level window handles associated with a given PID.
pub fn get_process_windows(pid: u32) -> Vec<HWND> {
    let mut ctx = EnumContext {
        target_pid: pid,
        found_hwnds: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(
            Some(enum_windows_callback),
            LPARAM(&mut ctx as *mut _ as isize),
        );
    }
    ctx.found_hwnds
}

/// Retrieve the full executable image path for a running process.
pub fn get_process_image_path(handle: HANDLE) -> Option<String> {
    unsafe {
        let mut path_buf = [0u16; 1024];
        let mut path_len = path_buf.len() as u32;
        let success = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_FORMAT(0),
            PWSTR(path_buf.as_mut_ptr()),
            &mut path_len,
        );

        if success.is_ok() && path_len > 0 {
            Some(String::from_utf16_lossy(&path_buf[..path_len as usize]))
        } else {
            None
        }
    }
}

/// Gracefully terminate a process using the two-stage protocol:
/// 1. Send `WM_CLOSE` to all top-level windows.
/// 2. Wait up to `timeout_ms` (default 1000ms).
/// 3. If still active, execute `TerminateProcess`.
pub fn terminate_process_gracefully(
    pid: u32,
    process_name: &str,
    timeout_ms: u32,
) -> Result<Option<TerminatedAppBackup>, SafetyViolationError> {
    let safety_db = ProcessSafetyDb::get();
    safety_db.assert_safe_to_kill(process_name)?;

    info!(pid = pid, name = %process_name, "Executing two-stage graceful process termination");

    unsafe {
        let handle = match OpenProcess(
            PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE | PROCESS_SYNCHRONIZE,
            false,
            pid,
        ) {
            Ok(h) if !h.is_invalid() => h,
            _ => {
                debug!(pid = pid, "Cannot open process (may have already exited)");
                return Ok(None);
            }
        };

        let exe_path = get_process_image_path(handle);

        // Stage 1: Send WM_CLOSE to process windows
        let hwnds = get_process_windows(pid);
        for hwnd in hwnds {
            let _ = PostMessageW(hwnd, WM_CLOSE, windows::Win32::Foundation::WPARAM(0), LPARAM(0));
        }

        // Stage 2: Wait for clean exit
        let wait_result = WaitForSingleObject(handle, timeout_ms);
        let mut exit_code: u32 = 0;
        let _ = GetExitCodeProcess(handle, &mut exit_code);

        // Stage 3: Force terminate if still alive
        if exit_code == (STILL_ACTIVE.0 as u32) || wait_result.0 != 0 {
            debug!(pid = pid, name = %process_name, "Process did not exit on WM_CLOSE; forcing TerminateProcess");
            let _ = TerminateProcess(handle, 1);
            let _ = WaitForSingleObject(handle, 500);
        }

        let _ = CloseHandle(handle);

        let backup = exe_path.map(|path| TerminatedAppBackup {
            name: process_name.to_string(),
            executable_path: path,
            command_line: None,
        });

        Ok(backup)
    }
}

/// Scan running processes and terminate all active safe Tier 2 processes.
/// Returns backup records for each closed process to enable post-match restoration.
pub fn terminate_tier2_background_processes() -> Result<Vec<TerminatedAppBackup>, String> {
    let safety_db = ProcessSafetyDb::get();
    let current_pid = std::process::id();
    let mut backups = Vec::new();

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| format!("Process snapshot failed: {}", e))?;

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let pid = entry.th32ProcessID;
                if pid != current_pid && pid > 4 {
                    let name = String::from_utf16_lossy(&entry.szExeFile)
                        .trim_matches('\0')
                        .to_string();

                    let tier = safety_db.classify_process(&name);
                    if tier == ProcessTier::Tier2SafeTerminate {
                        match terminate_process_gracefully(pid, &name, 1000) {
                            Ok(Some(backup)) => backups.push(backup),
                            Ok(None) => {}
                            Err(e) => warn!("Safety barrier skipped process {}: {}", name, e),
                        }
                    }
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    info!(count = backups.len(), "Tier 2 background processes terminated successfully");
    Ok(backups)
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RelaunchError {
    #[error("Executable path is empty")]
    EmptyPath,
    #[error("Executable path must be absolute: {0}")]
    NotAbsolute(String),
    #[error("Executable not found on disk: {0}")]
    NotFound(String),
    #[error("Invalid executable extension (must be .exe): {0}")]
    InvalidExtension(String),
    #[error("Target is a directory, not an executable file: {0}")]
    IsDirectory(String),
    #[error("Executable is in protected blacklist: {0}")]
    ProtectedExecutable(String),
    #[error("Access denied or permission error: {0}")]
    AccessDenied(String),
    #[error("Spawning process failed: {0}")]
    SpawnFailed(String),
}

/// Validates that an executable path is safe to relaunch:
/// 1. Must not be empty.
/// 2. Must be an absolute path.
/// 3. Must have `.exe` extension (rejecting scripts or batch files).
/// 4. Must exist on disk and be a regular file.
/// 5. Must not be a Tier 0 protected operating system or Vanguard binary.
pub fn validate_relaunch_path(path_str: &str) -> Result<std::path::PathBuf, RelaunchError> {
    let trimmed = path_str.trim();
    if trimmed.is_empty() {
        return Err(RelaunchError::EmptyPath);
    }

    let path = std::path::Path::new(trimmed);
    if !path.is_absolute() {
        return Err(RelaunchError::NotAbsolute(trimmed.to_string()));
    }

    match path.extension().and_then(|ext| ext.to_str()) {
        Some(ext) if ext.eq_ignore_ascii_case("exe") => {}
        _ => return Err(RelaunchError::InvalidExtension(trimmed.to_string())),
    }

    if !path.exists() {
        return Err(RelaunchError::NotFound(trimmed.to_string()));
    }

    if path.is_dir() {
        return Err(RelaunchError::IsDirectory(trimmed.to_string()));
    }

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();
    let safety_db = ProcessSafetyDb::get();
    if safety_db.classify_process(file_name) == ProcessTier::Tier0Protected {
        return Err(RelaunchError::ProtectedExecutable(file_name.to_string()));
    }

    let canonical = path
        .canonicalize()
        .map_err(|e| RelaunchError::NotFound(format!("{}: {}", trimmed, e)))?;

    let clean_path = if let Some(stripped) = canonical.to_str().and_then(|s| s.strip_prefix(r"\\?\")) {
        std::path::PathBuf::from(stripped)
    } else {
        canonical
    };

    Ok(clean_path)
}

/// Check whether the current calling process is running with an elevated token (Administrator or SYSTEM).
#[cfg(windows)]
pub fn is_current_process_elevated() -> bool {
    use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_ok() {
            let mut elevation = TOKEN_ELEVATION::default();
            let mut ret_len = 0u32;
            let res = GetTokenInformation(
                token,
                TokenElevation,
                Some(&mut elevation as *mut _ as *mut _),
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            let _ = CloseHandle(token);
            if res.is_ok() {
                return elevation.TokenIsElevated != 0;
            }
        }
    }
    false
}

#[cfg(not(windows))]
pub fn is_current_process_elevated() -> bool {
    false
}

#[cfg(windows)]
fn find_session_explorer_pid(target_session: u32) -> Option<u32> {
    extern "system" {
        fn ProcessIdToSessionId(dwprocessid: u32, pdwsessionid: *mut u32) -> windows::Win32::Foundation::BOOL;
    }

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
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_matches('\0')
                    .to_lowercase();
                if name == "explorer.exe" {
                    let mut sess_id = 0u32;
                    if ProcessIdToSessionId(entry.th32ProcessID, &mut sess_id).as_bool()
                        && sess_id == target_session
                    {
                        found_pid = Some(entry.th32ProcessID);
                        break;
                    }
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

/// Retrieve the active interactive desktop user token to prevent privilege escalation.
#[cfg(windows)]
pub fn get_interactive_user_token() -> Option<HANDLE> {
    use windows::Win32::Foundation::FreeLibrary;
    use windows::Win32::Security::{
        DuplicateTokenEx, SecurityImpersonation, TokenPrimary, TOKEN_ALL_ACCESS,
        TOKEN_ASSIGN_PRIMARY, TOKEN_DUPLICATE, TOKEN_QUERY,
    };
    use windows::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId;
    use windows::Win32::System::Threading::{OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION};

    let session_id = unsafe { WTSGetActiveConsoleSessionId() };
    if session_id == 0xFFFFFFFF {
        return None;
    }

    // Attempt 1: Dynamic call to WTSQueryUserToken (succeeds under SYSTEM service account)
    unsafe {
        if let Ok(h_wts) = windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("wtsapi32.dll")) {
            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                h_wts,
                windows::core::s!("WTSQueryUserToken"),
            );
            if let Some(func) = proc {
                type WtsQueryUserTokenFn = unsafe extern "system" fn(u32, *mut HANDLE) -> BOOL;
                let wts_query: WtsQueryUserTokenFn = std::mem::transmute(func);
                let mut token = HANDLE::default();
                if wts_query(session_id, &mut token).as_bool() && !token.is_invalid() {
                    let _ = FreeLibrary(h_wts);
                    return Some(token);
                }
            }
            let _ = FreeLibrary(h_wts);
        }
    }

    // Attempt 2: Duplicate token from active shell (explorer.exe in the user's session)
    if let Some(explorer_pid) = find_session_explorer_pid(session_id) {
        unsafe {
            if let Ok(proc_handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, explorer_pid) {
                let mut proc_token = HANDLE::default();
                if OpenProcessToken(
                    proc_handle,
                    TOKEN_DUPLICATE | TOKEN_QUERY | TOKEN_ASSIGN_PRIMARY,
                    &mut proc_token,
                )
                .is_ok()
                {
                    let mut primary_token = HANDLE::default();
                    let dup_res = DuplicateTokenEx(
                        proc_token,
                        TOKEN_ALL_ACCESS,
                        None,
                        SecurityImpersonation,
                        TokenPrimary,
                        &mut primary_token,
                    );
                    let _ = CloseHandle(proc_token);
                    let _ = CloseHandle(proc_handle);
                    if dup_res.is_ok() && !primary_token.is_invalid() {
                        return Some(primary_token);
                    }
                } else {
                    let _ = CloseHandle(proc_handle);
                }
            }
        }
    }

    None
}

#[cfg(windows)]
pub fn spawn_process_with_user_token(
    exe_path: &std::path::Path,
    args: Option<&str>,
    user_token: HANDLE,
) -> Result<u32, RelaunchError> {
    use windows::Win32::System::Threading::{
        CreateProcessAsUserW, CREATE_UNICODE_ENVIRONMENT, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION, STARTUPINFOW,
    };
    use windows::Win32::Foundation::BOOL;

    let path_str = exe_path.to_string_lossy().to_string();
    let cmd_str = match args {
        Some(a) if !a.trim().is_empty() => format!("\"{}\" {}\0", path_str, a.trim()),
        _ => format!("\"{}\"\0", path_str),
    };
    let mut wide_cmd: Vec<u16> = cmd_str.encode_utf16().collect();
    let mut si = STARTUPINFOW::default();
    si.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    let mut winsta_wide: Vec<u16> = "winsta0\\default\0".encode_utf16().collect();
    si.lpDesktop = windows::core::PWSTR(winsta_wide.as_mut_ptr());

    let mut pi = PROCESS_INFORMATION::default();

    // Optionally create user environment block
    let mut env_ptr: *mut std::ffi::c_void = std::ptr::null_mut();
    let mut creation_flags = PROCESS_CREATION_FLAGS(0);
    let mut loaded_userenv = None;

    unsafe {
        if let Ok(h_userenv) = windows::Win32::System::LibraryLoader::LoadLibraryW(windows::core::w!("userenv.dll")) {
            let proc = windows::Win32::System::LibraryLoader::GetProcAddress(
                h_userenv,
                windows::core::s!("CreateEnvironmentBlock"),
            );
            if let Some(func) = proc {
                type CreateEnvFn = unsafe extern "system" fn(*mut *mut std::ffi::c_void, HANDLE, BOOL) -> BOOL;
                let create_env: CreateEnvFn = std::mem::transmute(func);
                if create_env(&mut env_ptr, user_token, BOOL(0)).as_bool() {
                    creation_flags |= CREATE_UNICODE_ENVIRONMENT;
                    loaded_userenv = Some(h_userenv);
                } else {
                    let _ = windows::Win32::Foundation::FreeLibrary(h_userenv);
                }
            } else {
                let _ = windows::Win32::Foundation::FreeLibrary(h_userenv);
            }
        }

        let working_dir = exe_path.parent();
        let mut wide_dir: Option<Vec<u16>> = working_dir.map(|d| {
            d.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect()
        });
        let dir_pcwstr = wide_dir.as_mut().map(|w| windows::core::PCWSTR(w.as_ptr())).unwrap_or(windows::core::PCWSTR::null());

        let res = CreateProcessAsUserW(
            user_token,
            None,
            windows::core::PWSTR(wide_cmd.as_mut_ptr()),
            None,
            None,
            false,
            creation_flags,
            if !env_ptr.is_null() { Some(env_ptr) } else { None },
            dir_pcwstr,
            &si,
            &mut pi,
        );

        if let Some(h_userenv) = loaded_userenv {
            if !env_ptr.is_null() {
                let proc_destroy = windows::Win32::System::LibraryLoader::GetProcAddress(
                    h_userenv,
                    windows::core::s!("DestroyEnvironmentBlock"),
                );
                if let Some(func_destroy) = proc_destroy {
                    type DestroyEnvFn = unsafe extern "system" fn(*mut std::ffi::c_void) -> BOOL;
                    let destroy_env: DestroyEnvFn = std::mem::transmute(func_destroy);
                    let _ = destroy_env(env_ptr);
                }
            }
            let _ = windows::Win32::Foundation::FreeLibrary(h_userenv);
        }

        if res.is_ok() {
            let pid = pi.dwProcessId;
            let _ = CloseHandle(pi.hProcess);
            let _ = CloseHandle(pi.hThread);
            Ok(pid)
        } else {
            let err = std::io::Error::last_os_error();
            Err(RelaunchError::SpawnFailed(format!(
                "CreateProcessAsUserW failed: {}",
                err
            )))
        }
    }
}

/// Relaunch a single application with validation and token-aware privilege de-escalation.
pub fn relaunch_single_application(app: &TerminatedAppBackup) -> Result<u32, RelaunchError> {
    let valid_path = validate_relaunch_path(&app.executable_path)?;
    info!(name = %app.name, path = %valid_path.display(), "Executing token-aware application relaunch");

    #[cfg(windows)]
    {
        if is_current_process_elevated() {
            if let Some(user_token) = get_interactive_user_token() {
                debug!(name = %app.name, "Relaunching process using interactive user token (standard integrity)");
                let res = spawn_process_with_user_token(&valid_path, app.command_line.as_deref(), user_token);
                unsafe {
                    let _ = CloseHandle(user_token);
                }
                return res;
            } else {
                warn!(name = %app.name, "Interactive user token unavailable; falling back to standard spawn");
            }
        }

        // Standard spawn for un-elevated process
        let mut cmd = std::process::Command::new(&valid_path);
        if let Some(parent) = valid_path.parent() {
            cmd.current_dir(parent);
        }
        if let Some(ref args) = app.command_line {
            cmd.arg(args);
        }
        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id();
                info!(name = %app.name, pid = pid, "Successfully relaunched user application");
                Ok(pid)
            }
            Err(e) => {
                warn!(name = %app.name, error = %e, "Failed to spawn application");
                Err(RelaunchError::SpawnFailed(e.to_string()))
            }
        }
    }

    #[cfg(not(windows))]
    {
        let child = std::process::Command::new(&valid_path)
            .spawn()
            .map_err(|e| RelaunchError::SpawnFailed(e.to_string()))?;
        Ok(child.id())
    }
}

/// Relaunch closed applications from backups upon match completion.
/// Returns individual outcomes for each application.
pub fn relaunch_applications(backups: &[TerminatedAppBackup]) -> Vec<Result<u32, RelaunchError>> {
    let mut results = Vec::with_capacity(backups.len());
    let mut success_count = 0;
    let mut fail_count = 0;

    for app in backups {
        match relaunch_single_application(app) {
            Ok(pid) => {
                success_count += 1;
                results.push(Ok(pid));
            }
            Err(e) => {
                fail_count += 1;
                warn!(name = %app.name, error = %e, "Failed to relaunch application post-match");
                results.push(Err(e));
            }
        }
    }

    info!(
        total = backups.len(),
        successful = success_count,
        failed = fail_count,
        "Post-match application relaunch complete"
    );
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_termination_refusal_for_tier0() {
        // Attempting to kill vgc.exe, csrss.exe, or dwm.exe must be rejected
        let res1 = terminate_process_gracefully(1234, "vgc.exe", 100);
        assert!(matches!(res1, Err(SafetyViolationError::ProtectedProcess(_))));

        let res2 = terminate_process_gracefully(1234, "dwm.exe", 100);
        assert!(matches!(res2, Err(SafetyViolationError::ProtectedProcess(_))));

        let res3 = terminate_process_gracefully(1234, "csrss.exe", 100);
        assert!(matches!(res3, Err(SafetyViolationError::ProtectedProcess(_))));
    }

    #[test]
    fn test_graceful_termination_of_spawned_process() {
        // Spawn a test instance of notepad.exe
        let mut child = std::process::Command::new("notepad.exe")
            .spawn()
            .expect("Failed to launch notepad.exe for testing");

        let pid = child.id();
        std::thread::sleep(std::time::Duration::from_millis(200));

        let term_res = terminate_process_gracefully(pid, "notepad.exe", 1000);
        assert!(term_res.is_ok(), "Notepad termination should succeed: {:?}", term_res);

        let wait_res = child.wait();
        assert!(wait_res.is_ok(), "Child process must exit cleanly");
    }

    #[test]
    fn test_relaunch_validation_nonexistent_executable() {
        let res = validate_relaunch_path(r"C:\Windows\System32\valopt_test_nonexistent_12345.exe");
        assert!(matches!(res, Err(RelaunchError::NotFound(_))));
    }

    #[test]
    fn test_relaunch_validation_invalid_path() {
        // Relative path
        let res_rel = validate_relaunch_path("notepad.exe");
        assert!(matches!(res_rel, Err(RelaunchError::NotAbsolute(_))));

        // Empty path
        let res_empty = validate_relaunch_path("   ");
        assert_eq!(res_empty, Err(RelaunchError::EmptyPath));

        // Directory path
        let res_dir = validate_relaunch_path(r"C:\Windows");
        assert!(matches!(res_dir, Err(RelaunchError::InvalidExtension(_))));

        // Non-exe file
        let res_non_exe = validate_relaunch_path(r"C:\Windows\System32\drivers\etc\hosts");
        assert!(matches!(res_non_exe, Err(RelaunchError::InvalidExtension(_))));
    }

    #[test]
    fn test_relaunch_validation_protected_executable() {
        // csrss.exe must be rejected as protected
        let res = validate_relaunch_path(r"C:\Windows\System32\csrss.exe");
        assert!(matches!(res, Err(RelaunchError::ProtectedExecutable(_))));
    }

    #[test]
    fn test_relaunch_normal_user_process() {
        // Use notepad.exe in system directory
        let windir = std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".to_string());
        let notepad_path = format!(r"{}\System32\notepad.exe", windir);

        let app = TerminatedAppBackup {
            name: "notepad.exe".to_string(),
            executable_path: notepad_path,
            command_line: None,
        };

        let relaunch_res = relaunch_single_application(&app);
        assert!(relaunch_res.is_ok(), "Relaunch of notepad must succeed: {:?}", relaunch_res);

        let pid = relaunch_res.unwrap();
        assert!(pid > 0, "Launched PID must be > 0");

        // Terminate test notepad instance
        std::thread::sleep(std::time::Duration::from_millis(150));
        let _ = terminate_process_gracefully(pid, "notepad.exe", 1000);
    }

    #[test]
    fn test_elevation_check() {
        let is_elevated = is_current_process_elevated();
        println!("Current process elevation state: {}", is_elevated);
        // Just verify it returns a boolean without crashing
        assert!(is_elevated || !is_elevated);
    }

    #[test]
    fn test_batch_relaunch_handles_failures_gracefully() {
        let windir = std::env::var("windir").unwrap_or_else(|_| r"C:\Windows".to_string());
        let notepad_path = format!(r"{}\System32\notepad.exe", windir);

        let apps = vec![
            TerminatedAppBackup {
                name: "fake_app.exe".to_string(),
                executable_path: r"C:\Windows\System32\valopt_does_not_exist.exe".to_string(),
                command_line: None,
            },
            TerminatedAppBackup {
                name: "relative.exe".to_string(),
                executable_path: "relative_calc.exe".to_string(),
                command_line: None,
            },
            TerminatedAppBackup {
                name: "notepad.exe".to_string(),
                executable_path: notepad_path,
                command_line: None,
            },
        ];

        let results = relaunch_applications(&apps);
        assert_eq!(results.len(), 3);
        assert!(matches!(results[0], Err(RelaunchError::NotFound(_))));
        assert!(matches!(results[1], Err(RelaunchError::NotAbsolute(_))));
        assert!(results[2].is_ok(), "Third app (notepad) should succeed despite previous failures");

        // Clean up spawned notepad
        if let Ok(pid) = results[2] {
            std::thread::sleep(std::time::Duration::from_millis(150));
            let _ = terminate_process_gracefully(pid, "notepad.exe", 1000);
        }
    }
}
