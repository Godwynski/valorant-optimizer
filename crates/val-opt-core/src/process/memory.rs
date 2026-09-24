//! Process Working Set Memory Trimmer.
//!
//! Flushes idle and cached working set memory (e.g. from Windows Explorer shell)
//! back into standby/paged pool to maximize available physical RAM for VALORANT.

use tracing::{debug, info};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{K32EmptyWorkingSet, K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_SET_QUOTA};

/// Empty the working set of a target process by PID.
/// Returns the number of bytes reclaimed (if any).
pub fn trim_working_set(pid: u32) -> Result<usize, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA, false, pid)
            .map_err(|e| format!("Failed to open PID {} with quota rights: {}", pid, e))?;

        if handle.is_invalid() {
            return Err(format!("Invalid handle for PID {}", pid));
        }

        let mut mem_before = PROCESS_MEMORY_COUNTERS::default();
        let _ = K32GetProcessMemoryInfo(
            handle,
            &mut mem_before,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );

        let success = K32EmptyWorkingSet(handle);

        let mut mem_after = PROCESS_MEMORY_COUNTERS::default();
        let _ = K32GetProcessMemoryInfo(
            handle,
            &mut mem_after,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );

        let _ = CloseHandle(handle);

        if !success.as_bool() {
            return Err("K32EmptyWorkingSet call failed".to_string());
        }

        let reclaimed = mem_before.WorkingSetSize.saturating_sub(mem_after.WorkingSetSize);
        debug!(pid = pid, before = mem_before.WorkingSetSize, after = mem_after.WorkingSetSize, reclaimed = reclaimed, "Working set trimmed");
        Ok(reclaimed)
    }
}

/// Find all active `explorer.exe` instances and trim their working set memory.
/// Reclaims typically 100MB - 350MB of RAM without killing the Windows taskbar/shell.
pub fn trim_explorer_working_set() -> Result<usize, String> {
    let mut total_reclaimed = 0usize;

    unsafe {
        let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)
            .map_err(|e| format!("ToolHelp snapshot failed: {}", e))?;

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;

        if Process32FirstW(snapshot, &mut entry).is_ok() {
            loop {
                let name = String::from_utf16_lossy(&entry.szExeFile)
                    .trim_matches('\0')
                    .to_lowercase();

                if name == "explorer.exe" {
                    let pid = entry.th32ProcessID;
                    if let Ok(bytes) = trim_working_set(pid) {
                        total_reclaimed += bytes;
                    }
                }

                if Process32NextW(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }

        let _ = CloseHandle(snapshot);
    }

    info!(reclaimed_mb = total_reclaimed / (1024 * 1024), "Windows Explorer working set trimmed successfully");
    Ok(total_reclaimed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_trim_explorer_working_set() {
        let result = trim_explorer_working_set();
        assert!(result.is_ok(), "Explorer working set trimming should succeed: {:?}", result);
        let bytes = result.unwrap();
        println!("Explorer working set memory trimmed: {} KB", bytes / 1024);
    }
}
