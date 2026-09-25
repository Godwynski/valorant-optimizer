//! Non-Essential Windows Service Pauser and Restorer.
//!
//! Pauses Tier 3 non-essential services (`SysMain`, `DiagTrack`, `Spooler`) during matches
//! to eliminate background CPU spikes and disk contention, then restores them post-match.
//! NOTE: Windows Update (`wuauserv`) is protected under Tier 0 (MUST_NOT_MODIFY).

use std::time::{Duration, Instant};
use tracing::{info, warn};
use val_opt_shared::models::process::ServiceBackup;
use windows::core::PCWSTR;
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, OpenSCManagerW, OpenServiceW, QueryServiceStatus,
    StartServiceW, SC_MANAGER_CONNECT, SERVICE_CONTROL_STOP, SERVICE_QUERY_STATUS,
    SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STOPPED,
};

use super::safety_db::{ProcessSafetyDb, SafetyViolationError};

/// Target Tier 3 services to pause during gaming sessions (explicit user opt-in only).
pub const TARGET_TIER3_SERVICES: [&str; 3] = [
    "SysMain",   // Superfetch / Prefetch memory defrag
    "DiagTrack", // Connected User Experiences and Telemetry
    "Spooler",   // Print Spooler
];

/// Query the current running status of a Windows service.
pub fn query_service_state(service_name: &str) -> Result<u32, String> {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT)
            .map_err(|e| format!("Failed to open SCM: {}", e))?;

        let wide_name: Vec<u16> = service_name.encode_utf16().chain(std::iter::once(0)).collect();
        let svc = OpenServiceW(scm, PCWSTR(wide_name.as_ptr()), SERVICE_QUERY_STATUS)
            .map_err(|e| {
                let _ = CloseServiceHandle(scm);
                format!("Failed to open service '{}': {}", service_name, e)
            })?;

        let mut status = SERVICE_STATUS::default();
        let query_ok = QueryServiceStatus(svc, &mut status);

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if query_ok.is_ok() {
            Ok(status.dwCurrentState.0)
        } else {
            Err(format!("QueryServiceStatus failed for '{}'", service_name))
        }
    }
}

/// Stop a Windows service with clean timeout waiting.
pub fn stop_service(service_name: &str) -> Result<ServiceBackup, SafetyViolationError> {
    let safety_db = ProcessSafetyDb::get();
    safety_db.assert_safe_to_stop_service(service_name)?;

    let mut backup = ServiceBackup {
        service_name: service_name.to_string(),
        display_name: service_name.to_string(),
        previous_state: SERVICE_STOPPED.0,
        was_paused_by_optimizer: false,
    };

    unsafe {
        let scm = match OpenSCManagerW(None, None, SC_MANAGER_CONNECT) {
            Ok(h) if !h.is_invalid() => h,
            _ => return Ok(backup),
        };

        let wide_name: Vec<u16> = service_name.encode_utf16().chain(std::iter::once(0)).collect();
        let svc = match OpenServiceW(
            scm,
            PCWSTR(wide_name.as_ptr()),
            SERVICE_QUERY_STATUS | SERVICE_CONTROL_STOP,
        ) {
            Ok(h) if !h.is_invalid() => h,
            _ => {
                let _ = CloseServiceHandle(scm);
                return Ok(backup);
            }
        };

        let mut status = SERVICE_STATUS::default();
        if QueryServiceStatus(svc, &mut status).is_ok() {
            backup.previous_state = status.dwCurrentState.0;

            if status.dwCurrentState == SERVICE_RUNNING {
                info!(service = %service_name, "Pausing non-essential Windows service");
                let mut control_status = SERVICE_STATUS::default();
                let _ = ControlService(svc, SERVICE_CONTROL_STOP, &mut control_status);

                // Wait up to 2 seconds for clean stop
                let start = Instant::now();
                while start.elapsed() < Duration::from_millis(2000) {
                    if QueryServiceStatus(svc, &mut status).is_ok() && status.dwCurrentState == SERVICE_STOPPED {
                        backup.was_paused_by_optimizer = true;
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
            }
        }

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);
    }

    Ok(backup)
}

/// Resume a previously paused Windows service.
pub fn start_service(service_name: &str) -> Result<(), String> {
    unsafe {
        let scm = OpenSCManagerW(None, None, SC_MANAGER_CONNECT)
            .map_err(|e| format!("Failed to open SCM: {}", e))?;

        let wide_name: Vec<u16> = service_name.encode_utf16().chain(std::iter::once(0)).collect();
        let svc = OpenServiceW(scm, PCWSTR(wide_name.as_ptr()), SERVICE_START | SERVICE_QUERY_STATUS)
            .map_err(|e| {
                let _ = CloseServiceHandle(scm);
                format!("Failed to open service '{}' for start: {}", service_name, e)
            })?;

        let start_ok = StartServiceW(svc, None);

        let _ = CloseServiceHandle(svc);
        let _ = CloseServiceHandle(scm);

        if start_ok.is_ok() {
            info!(service = %service_name, "Resumed Windows service");
            Ok(())
        } else {
            Err(format!("Failed to start service '{}'", service_name))
        }
    }
}

/// Pause all non-essential Tier 3 background services and return backup records.
pub fn pause_tier3_services() -> Vec<ServiceBackup> {
    let mut backups = Vec::new();

    for svc_name in TARGET_TIER3_SERVICES {
        match stop_service(svc_name) {
            Ok(backup) => {
                if backup.was_paused_by_optimizer {
                    backups.push(backup);
                }
            }
            Err(e) => warn!("Safety barrier prohibited pausing service {}: {}", svc_name, e),
        }
    }

    info!(count = backups.len(), "Tier 3 background services paused");
    backups
}

/// Restore all services that were paused by the optimizer.
pub fn restore_tier3_services(backups: &[ServiceBackup]) {
    for svc in backups {
        if svc.was_paused_by_optimizer {
            let _ = start_service(&svc.service_name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_service_protection_barrier() {
        // Must reject stopping Tier 0 critical services
        assert!(matches!(
            stop_service("vgc"),
            Err(SafetyViolationError::ProtectedService(_))
        ));

        assert!(matches!(
            stop_service("CryptSvc"),
            Err(SafetyViolationError::ProtectedService(_))
        ));

        assert!(matches!(
            stop_service("wuauserv"),
            Err(SafetyViolationError::ProtectedService(_))
        ));
    }

    #[test]
    fn test_query_tier3_service_state() {
        // Query SysMain without crashing or error
        let status = query_service_state("SysMain");
        println!("SysMain state: {:?}", status);
        assert!(status.is_ok());
    }
}
