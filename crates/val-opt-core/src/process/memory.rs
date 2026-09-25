//! Process & System Memory Diagnostics (Read-Only).
//!
//! Provides observational memory telemetry for host and process performance profiling.
//!
//! NOTE: Forced working-set trimming (`EmptyWorkingSet` / `K32EmptyWorkingSet` /
//! `SetProcessWorkingSetSize`) was PERMANENTLY REMOVED in Phase P3 (`TASK-OPT-01`).
//! Forcing pages out of process working sets induces soft page faults, disk I/O to the
//! paging/standby list, and frame hitching when shell components or background apps resume.
//! Modern Windows dynamically and efficiently manages the system working set.

use tracing::debug;
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::ProcessStatus::{K32GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::System::Threading::{GetCurrentProcessId, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};

/// Snapshot of process memory usage (read-only diagnostic).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessMemorySnapshot {
    pub pid: u32,
    pub working_set_bytes: usize,
    pub peak_working_set_bytes: usize,
    pub pagefile_usage_bytes: usize,
    pub peak_pagefile_usage_bytes: usize,
}

/// Query process memory usage information without altering its working set (read-only diagnostic).
pub fn query_process_memory_info(pid: u32) -> Result<ProcessMemorySnapshot, String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid)
            .map_err(|e| format!("Failed to open PID {} for memory query: {}", pid, e))?;

        if handle.is_invalid() {
            return Err(format!("Invalid handle for PID {}", pid));
        }

        let mut counters = PROCESS_MEMORY_COUNTERS::default();
        let success = K32GetProcessMemoryInfo(
            handle,
            &mut counters,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        );

        let _ = CloseHandle(handle);

        if !success.as_bool() {
            return Err(format!("K32GetProcessMemoryInfo failed for PID {}", pid));
        }

        let snapshot = ProcessMemorySnapshot {
            pid,
            working_set_bytes: counters.WorkingSetSize,
            peak_working_set_bytes: counters.PeakWorkingSetSize,
            pagefile_usage_bytes: counters.PagefileUsage,
            peak_pagefile_usage_bytes: counters.PeakPagefileUsage,
        };

        debug!(pid = pid, working_set = snapshot.working_set_bytes, "Sampled process memory diagnostics");
        Ok(snapshot)
    }
}

/// Query memory usage of the current optimizer process.
pub fn query_current_process_memory() -> Result<ProcessMemorySnapshot, String> {
    let pid = unsafe { GetCurrentProcessId() };
    query_process_memory_info(pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_query_process_memory_info_read_only() {
        let snapshot = query_current_process_memory().expect("Should query current process memory");
        assert!(snapshot.pid > 0, "PID must be non-zero");
        assert!(snapshot.working_set_bytes > 0, "Working set must be non-zero");
        assert!(snapshot.peak_working_set_bytes >= snapshot.working_set_bytes);
        println!(
            "Diagnostic memory telemetry: PID {} WorkingSet: {:.2} MB (Peak: {:.2} MB)",
            snapshot.pid,
            snapshot.working_set_bytes as f64 / (1024.0 * 1024.0),
            snapshot.peak_working_set_bytes as f64 / (1024.0 * 1024.0)
        );
    }

    #[test]
    fn test_no_empty_working_set_invoked() {
        // Sample before
        let before = query_current_process_memory().expect("Memory before");
        // Ensure that querying memory multiple times is purely observational and does not flush working set
        let after = query_current_process_memory().expect("Memory after");
        assert_eq!(before.pid, after.pid);
        // Working set must remain stable and not drop to near-zero as EmptyWorkingSet would cause
        assert!(after.working_set_bytes > 1_000_000, "Working set must not be forcefully flushed");
    }
}
