//! Windows Boot Orphan Crash Recovery Service.
//!
//! Automatically detects uncommitted state snapshots upon daemon startup
//! (indicating an abnormal exit, power outage, or BSOD mid-game) and executes
//! a full transactional rollback of all paused services, network tweaks,
//! audio APOs, and power policies before clearing the snapshot.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Instant;
use tracing::{error, info, warn};
use val_opt_shared::models::snapshot::SystemStateSnapshot;

use super::snapshot_engine::{SnapshotEngine, SnapshotError};

/// Summary report of an orphan crash recovery operation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RecoveryReport {
    pub uncommitted_snapshot_found: bool,
    pub snapshot_id: Option<String>,
    pub snapshot_timestamp: Option<String>,
    pub services_restored: usize,
    pub audio_endpoints_restored: usize,
    pub adapter_properties_restored: usize,
    pub qos_policy_restored: bool,
    pub power_scheme_restored: bool,
    pub game_mode_restored: bool,
    pub error: Option<String>,
    pub duration_ms: u128,
}

pub struct CrashRecoveryService;

impl CrashRecoveryService {
    /// Retrieve path to the persistent recovery audit log.
    /// Default: `%ProgramData%\ValorantOptimizer\recovery.log`
    pub fn default_recovery_log_path() -> PathBuf {
        let program_data =
            std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        let dir = Path::new(&program_data).join("ValorantOptimizer");
        let _ = fs::create_dir_all(&dir);
        dir.join("recovery.log")
    }

    /// Check if an uncommitted snapshot exists from an ungraceful shutdown and restore it within canonical storage.
    pub fn check_and_recover(snapshot_path: Option<&Path>) -> Result<RecoveryReport, String> {
        Self::check_and_recover_with_base(snapshot_path, None)
    }

    /// Check if an uncommitted snapshot exists within an authorized base directory and restore it.
    pub fn check_and_recover_with_base(
        snapshot_path: Option<&Path>,
        allowed_base: Option<&Path>,
    ) -> Result<RecoveryReport, String> {
        let raw_target = snapshot_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(SnapshotEngine::default_snapshot_path);

        // Security check: validate snapshot path against canonical directory and reject reparse points/symlinks/hardlinks (TASK-SEC-05)
        let target = SnapshotEngine::validate_snapshot_path(&raw_target, allowed_base)
            .map_err(|e| format!("Security validation failed for rollback path: {}", e))?;

        if !target.exists() {
            return Ok(RecoveryReport::default());
        }

        warn!(
            path = %target.display(),
            "Found uncommitted state snapshot from previous session; initiating auto-recovery"
        );

        let start = Instant::now();

        // 1. Load and verify SHA-256 and HMAC integrity
        let snapshot = match SnapshotEngine::load_and_verify_with_base(Some(&target), allowed_base) {
            Ok(s) => s,
            Err(SnapshotError::IntegrityViolation {
                expected_hash,
                computed_hash,
                ..
            }) => {
                let err_msg = format!(
                    "Snapshot integrity check failed! Expected {}, found {}",
                    expected_hash, computed_hash
                );
                error!("{}", err_msg);
                let report = RecoveryReport {
                    uncommitted_snapshot_found: true,
                    error: Some(err_msg.clone()),
                    duration_ms: start.elapsed().as_millis(),
                    ..Default::default()
                };
                let _ = Self::append_recovery_log(&report, None);
                return Err(err_msg);
            }
            Err(e) => {
                let err_msg = format!("Failed to read orphaned snapshot: {}", e);
                error!("{}", err_msg);
                return Err(err_msg);
            }
        };

        // 2. Perform full rollback of all system subsystems
        let mut report = Self::rollback_snapshot(&snapshot)?;
        report.duration_ms = start.elapsed().as_millis();

        // 3. Delete uncommitted snapshot
        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&target), allowed_base);

        // 4. Record to recovery log
        let _ = Self::append_recovery_log(&report, None);

        info!(
            snapshot_id = ?report.snapshot_id,
            duration_ms = report.duration_ms,
            services = report.services_restored,
            "Crash auto-recovery completed successfully"
        );

        Ok(report)
    }

    /// Execute a complete rollback across all recorded subsystems.
    pub fn rollback_snapshot(snapshot: &SystemStateSnapshot) -> Result<RecoveryReport, String> {
        let mut report = RecoveryReport {
            uncommitted_snapshot_found: true,
            snapshot_id: Some(snapshot.snapshot_id.clone()),
            snapshot_timestamp: Some(snapshot.timestamp_utc.clone()),
            ..Default::default()
        };

        info!(snapshot_id = %snapshot.snapshot_id, "Executing full system rollback");

        // 1. Unregister QoS Policy
        if let Some(ref qos_backup) = snapshot.previous_qos_policy {
            match crate::network::unregister_qos_policy(qos_backup) {
                Ok(_) => {
                    info!("Restored QoS policy: uninstalled {}", qos_backup.policy_name);
                    report.qos_policy_restored = true;
                }
                Err(e) => warn!("Failed to unregister QoS policy during rollback: {}", e),
            }
        }

        // 2. Restore Network Adapter Properties
        if !snapshot.previous_adapter_properties.is_empty() {
            match crate::network::restore_adapter_properties(&snapshot.previous_adapter_properties) {
                Ok(_) => {
                    info!(
                        "Restored {} network adapter properties",
                        snapshot.previous_adapter_properties.len()
                    );
                    report.adapter_properties_restored = snapshot.previous_adapter_properties.len();
                }
                Err(e) => warn!("Failed to restore network adapter properties: {}", e),
            }
        }

        // 3. Restore Audio Endpoints APO state
        if !snapshot.previous_audio_endpoints.is_empty() {
            match crate::optimizations::audio::restore_audio_enhancements(
                &snapshot.previous_audio_endpoints,
            ) {
                Ok(_) => {
                    info!(
                        "Restored {} audio endpoint DSP configurations",
                        snapshot.previous_audio_endpoints.len()
                    );
                    report.audio_endpoints_restored = snapshot.previous_audio_endpoints.len();
                }
                Err(e) => warn!("Failed to restore audio endpoint enhancements: {}", e),
            }
        }

        // 4. Restore Power Scheme
        if let Some(ref scheme_str) = snapshot.previous_power_scheme {
            if let Ok(guid) = crate::optimizations::power::string_to_guid(scheme_str) {
                match crate::optimizations::power::restore_power_scheme(guid) {
                    Ok(_) => {
                        info!("Restored original power scheme: {}", scheme_str);
                        report.power_scheme_restored = true;
                    }
                    Err(e) => warn!("Failed to restore power scheme: {}", e),
                }
            }
        }

        // 5. Restore Windows Game Mode
        if let Some(prev_gm) = snapshot.previous_game_mode {
            match crate::optimizations::game_mode::restore_game_mode(prev_gm) {
                Ok(_) => {
                    info!("Restored original Game Mode state: {}", prev_gm);
                    report.game_mode_restored = true;
                }
                Err(e) => warn!("Failed to restore Game Mode: {}", e),
            }
        }

        // 6. Restore Paused Tier 3 Services
        if !snapshot.paused_services.is_empty() {
            crate::process::services::restore_tier3_services(&snapshot.paused_services);
            info!(
                count = snapshot.paused_services.len(),
                "Restored paused Tier 3 Windows services"
            );
            report.services_restored = snapshot.paused_services.len();
        }

        Ok(report)
    }

    /// Append a structured entry to the persistent recovery log.
    pub fn append_recovery_log(
        report: &RecoveryReport,
        log_path: Option<&Path>,
    ) -> Result<(), std::io::Error> {
        let path = log_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_recovery_log_path);

        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }

        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;

        let timestamp = chrono_now_iso();
        let log_entry = format!(
            "[{}] [CRASH_RECOVERY] Snapshot: {:?} | Restored: (services={}, audio={}, nic={}, qos={}, power={}, game_mode={}) | Duration: {}ms | Error: {:?}\n",
            timestamp,
            report.snapshot_id,
            report.services_restored,
            report.audio_endpoints_restored,
            report.adapter_properties_restored,
            report.qos_policy_restored,
            report.power_scheme_restored,
            report.game_mode_restored,
            report.duration_ms,
            report.error
        );

        file.write_all(log_entry.as_bytes())?;
        Ok(())
    }
}

fn chrono_now_iso() -> String {
    let now = std::time::SystemTime::now();
    format!("{:?}", now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_crash_recovery_from_orphaned_snapshot() {
        let temp_dir = std::env::temp_dir().join(format!("val_opt_test_recovery_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("orphaned_snapshot.json");
        let log_file = temp_dir.join("test_recovery.log");

        // 1. Create a simulated orphaned snapshot
        let mut snapshot = SystemStateSnapshot::new("snap_crash_test");
        snapshot.previous_game_mode = Some(true);
        snapshot.paused_services.push(val_opt_shared::models::process::ServiceBackup {
            service_name: "wuauserv".to_string(),
            display_name: "Windows Update".to_string(),
            previous_state: 4,
            was_paused_by_optimizer: true,
        });

        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir))
            .expect("Must save test snapshot");
        assert!(snap_file.exists());

        // 2. Execute crash recovery within authorized test base
        let report = CrashRecoveryService::check_and_recover_with_base(Some(&snap_file), Some(&temp_dir))
            .expect("Crash recovery should succeed");

        assert!(report.uncommitted_snapshot_found);
        assert_eq!(report.snapshot_id.as_deref(), Some("snap_crash_test"));
        assert_eq!(report.services_restored, 1);
        assert!(report.game_mode_restored);

        // 3. Verify snapshot was deleted after recovery
        assert!(!snap_file.exists(), "Snapshot must be deleted after recovery");

        // 4. Test recovery logging
        let _ = CrashRecoveryService::append_recovery_log(&report, Some(&log_file));
        assert!(log_file.exists());
        let log_content = fs::read_to_string(&log_file).unwrap();
        assert!(log_content.contains("snap_crash_test"));

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_crash_recovery_no_snapshot() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_no_recovery");
        let _ = fs::create_dir_all(&temp_dir);
        let nonexistent_file = temp_dir.join("does_not_exist.json");

        let report = CrashRecoveryService::check_and_recover_with_base(Some(&nonexistent_file), Some(&temp_dir))
            .expect("Should handle non-existent snapshot gracefully");

        assert!(!report.uncommitted_snapshot_found);
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_crash_recovery_rejects_external_file_outside_canonical_dir() {
        let temp_dir = std::env::temp_dir().join("val_opt_sec05_ext_recovery");
        let _ = fs::create_dir_all(&temp_dir);
        let external_file = temp_dir.join("orphaned_snapshot.json");
        fs::write(&external_file, b"{\"test\": \"data\"}").unwrap();

        // Attempting recovery on external path with default canonical base
        let res = CrashRecoveryService::check_and_recover(Some(&external_file));
        assert!(res.is_err(), "Must reject rollback on external path outside canonical dir");
        let err = res.unwrap_err();
        assert!(
            err.contains("Security validation failed") || err.contains("outside authorized root"),
            "Unexpected error message: {}",
            err
        );

        // Crucial verification: external file MUST NOT be deleted
        assert!(external_file.exists(), "External file must NOT be deleted by recovery service");

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(windows)]
    fn test_crash_recovery_rejects_directory_junction() {
        let temp_dir = std::env::temp_dir().join(format!("val_opt_sec05_rec_junc_{}", std::process::id()));
        let base_dir = temp_dir.join("allowed_base");
        let target_dir = temp_dir.join("target_dir");
        let junction_dir = base_dir.join("junction_link");

        let _ = fs::create_dir_all(&base_dir);
        let _ = fs::create_dir_all(&target_dir);

        let target_snap = target_dir.join("snapshot.json");
        fs::write(&target_snap, b"{\"snapshot_id\": \"junc_snap\"}").unwrap();

        // Create junction: junction_dir -> target_dir
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", junction_dir.to_str().unwrap(), target_dir.to_str().unwrap()])
            .output()
            .expect("mklink /J must execute");

        if status.status.success() {
            let snap_via_junction = junction_dir.join("snapshot.json");

            let res = CrashRecoveryService::check_and_recover_with_base(Some(&snap_via_junction), Some(&base_dir));
            assert!(res.is_err(), "Crash recovery must reject path through directory junction");
            let err = res.unwrap_err();
            assert!(
                err.contains("Reparse point") || err.contains("junction") || err.contains("symlink"),
                "Unexpected error message: {}",
                err
            );

            // Crucial verification: file in target directory must NOT be deleted!
            assert!(target_snap.exists(), "Target file pointed to by junction must NOT be deleted");

            // Cleanup junction
            let _ = std::process::Command::new("cmd")
                .args(["/C", "rmdir", junction_dir.to_str().unwrap()])
                .output();
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}
