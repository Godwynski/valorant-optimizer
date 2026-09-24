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

/// Relaunch closed applications from backups upon match completion.
pub fn relaunch_applications(backups: &[TerminatedAppBackup]) -> Result<(), String> {
    for app in backups {
        info!(name = %app.name, path = %app.executable_path, "Relaunching user application post-match");
        let _ = std::process::Command::new(&app.executable_path).spawn();
    }
    Ok(())
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
}
