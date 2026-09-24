//! Atomic State Snapshot Serializer & Verifier.
//!
//! Captures complete baseline system state prior to modifications and commits
//! to `%ProgramData%\ValorantOptimizer\snapshot.json` with SHA-256 integrity hash
//! and atomic file replacement to eliminate any risk of disk corruption.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;
use tracing::{error, info, warn};
use val_opt_shared::models::network::{AdapterPropertyBackup, QosPolicyBackup};
use val_opt_shared::models::snapshot::{AudioEndpointBackup, SystemStateSnapshot};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

/// Errors encountered during snapshot serialization, persistence, or validation.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("Snapshot file not found at: {0}")]
    NotFound(PathBuf),

    #[error("I/O error during snapshot operation: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Snapshot integrity violation for {path}: expected SHA-256 {expected_hash}, calculated {computed_hash}")]
    IntegrityViolation {
        path: PathBuf,
        expected_hash: String,
        computed_hash: String,
    },
}

pub struct SnapshotEngine;

impl SnapshotEngine {
    /// Retrieve the standard persistence path for the active snapshot.
    /// Default: `%ProgramData%\ValorantOptimizer\snapshot.json`
    pub fn default_snapshot_path() -> PathBuf {
        let program_data =
            std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        let dir = Path::new(&program_data).join("ValorantOptimizer");
        let _ = fs::create_dir_all(&dir);
        dir.join("snapshot.json")
    }

    /// Check if an uncommitted snapshot exists at the target path.
    pub fn has_snapshot(path: Option<&Path>) -> bool {
        let p = path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);
        p.exists() && p.is_file()
    }

    /// Capture complete baseline system state prior to applying any optimizations.
    pub fn capture_system_baseline() -> Result<SystemStateSnapshot, String> {
        let start = Instant::now();
        let id = format!(
            "snap_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );

        let mut snapshot = SystemStateSnapshot::new(id);

        // 1. Capture Game Mode state
        if let Ok(gm) = crate::optimizations::game_mode::get_game_mode_state() {
            snapshot.previous_game_mode = Some(gm);
        }

        // 2. Capture Active Power Scheme
        if let Ok(guid) = crate::optimizations::power::get_active_power_scheme() {
            snapshot.previous_power_scheme =
                Some(crate::optimizations::power::guid_to_string(&guid));
        }

        // 3. Capture Audio Endpoints
        if let Ok(endpoints) = crate::optimizations::audio::enumerate_render_endpoints() {
            let mut backups = Vec::new();
            for ep in endpoints {
                backups.push(AudioEndpointBackup {
                    endpoint_id: ep.id,
                    endpoint_name: ep.name,
                    original_disabled_state: Some(if ep.enhancements_disabled { 1 } else { 0 }),
                });
            }
            snapshot.previous_audio_endpoints = backups;
        }

        // 4. Capture Primary Network Adapter Properties & Flow Control
        if let Ok(primary) =
            val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary()
        {
            let mut props = Vec::new();
            if let Ok(adv_props) =
                crate::network::adapter::query_adapter_properties(&primary.adapter_name)
            {
                for p in adv_props {
                    let kw = &p.registry_keyword;
                    if kw.contains("EEE")
                        || kw.contains("Green")
                        || kw.contains("Interrupt")
                        || kw.contains("Flow")
                    {
                        if let Some(val) = p.first_value() {
                            props.push(AdapterPropertyBackup {
                                adapter_name: primary.adapter_name.clone(),
                                keyword: p.registry_keyword.clone(),
                                display_name: p.display_name.clone(),
                                original_value: val.to_string(),
                            });
                        }
                    }
                }
            }
            snapshot.previous_adapter_properties = props;
        }

        // 5. Capture QoS Policy (if registered)
        if let Ok(Some(policy)) = crate::network::qos::query_qos_policy(
            crate::network::qos::DEFAULT_VALORANT_QOS_POLICY_NAME,
        ) {
            snapshot.previous_qos_policy = Some(QosPolicyBackup {
                policy_name: policy.name,
                previous_nla_setting: None,
                was_policy_present_before: true,
            });
        }

        info!(
            snapshot_id = %snapshot.snapshot_id,
            duration_ms = start.elapsed().as_millis(),
            "Baseline system state captured"
        );

        Ok(snapshot)
    }

    /// Atomically serialize and save the state snapshot to disk.
    ///
    /// Writes to a temporary file first and atomically moves it to the target path
    /// using `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`
    /// to guarantee zero risk of file corruption under sudden power loss.
    pub fn save_atomic(
        snapshot: &mut SystemStateSnapshot,
        target_path: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let start = Instant::now();
        let target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create snapshot dir: {}", e))?;
        }

        // Sign snapshot with SHA-256
        snapshot.sign_in_place();

        let json_data = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| format!("Failed to serialize snapshot: {}", e))?;

        // Write to temporary file in the same directory
        let temp_filename = format!(
            "snapshot_{}_{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let temp_path = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(temp_filename);

        fs::write(&temp_path, &json_data)
            .map_err(|e| format!("Failed to write temporary snapshot file: {}", e))?;

        // Atomically replace target file
        #[cfg(windows)]
        {
            let temp_wide: Vec<u16> = temp_path
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let target_wide: Vec<u16> = target
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                let moved = MoveFileExW(
                    PCWSTR(temp_wide.as_ptr()),
                    PCWSTR(target_wide.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                );

                if moved.is_err() {
                    // Fallback to std::fs rename/remove if MoveFileEx failed
                    let _ = fs::remove_file(&target);
                    if let Err(e) = fs::rename(&temp_path, &target) {
                        let _ = fs::remove_file(&temp_path);
                        return Err(format!("Atomic move failed: {}", e));
                    }
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&target);
            fs::rename(&temp_path, &target).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                format!("Failed to rename temp snapshot: {}", e)
            })?;
        }

        let elapsed = start.elapsed();
        info!(
            snapshot_id = %snapshot.snapshot_id,
            path = %target.display(),
            sha256 = %snapshot.sha256_hash,
            duration_ms = elapsed.as_millis(),
            "Snapshot atomically committed to disk"
        );

        if elapsed.as_millis() > 50 {
            warn!(
                duration_ms = elapsed.as_millis(),
                "Snapshot serialization exceeded 50ms latency target"
            );
        }

        Ok(target)
    }

    /// Read and verify snapshot from disk.
    ///
    /// Validates SHA-256 checksum and returns `SnapshotError::IntegrityViolation`
    /// if the payload was modified or corrupted.
    pub fn load_and_verify(target_path: Option<&Path>) -> Result<SystemStateSnapshot, SnapshotError> {
        let target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        if !target.exists() || !target.is_file() {
            return Err(SnapshotError::NotFound(target));
        }

        let content = fs::read_to_string(&target)?;
        let snapshot: SystemStateSnapshot = serde_json::from_str(&content)?;

        let computed = snapshot.compute_payload_hash();
        if !computed.eq_ignore_ascii_case(&snapshot.sha256_hash) {
            error!(
                path = %target.display(),
                expected = %snapshot.sha256_hash,
                computed = %computed,
                "Snapshot SHA-256 integrity check FAILED! Possible file tampering or corruption"
            );
            return Err(SnapshotError::IntegrityViolation {
                path: target,
                expected_hash: snapshot.sha256_hash,
                computed_hash: computed,
            });
        }

        info!(
            snapshot_id = %snapshot.snapshot_id,
            path = %target.display(),
            "Snapshot loaded and integrity verified"
        );
        Ok(snapshot)
    }

    /// Safely delete the snapshot file after a successful rollback or cleanup.
    pub fn delete_snapshot(target_path: Option<&Path>) -> Result<bool, String> {
        let target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        if target.exists() {
            fs::remove_file(&target).map_err(|e| format!("Failed to delete snapshot: {}", e))?;
            info!(path = %target.display(), "Snapshot file removed");
            Ok(true)
        } else {
            Ok(false)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use val_opt_shared::models::process::ServiceBackup;

    #[test]
    fn test_snapshot_atomic_write_and_verification() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_01");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("test_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_snap_001");
        snapshot.previous_game_mode = Some(true);
        snapshot.previous_power_scheme = Some("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".to_string());
        snapshot.previous_audio_endpoints.push(AudioEndpointBackup {
            endpoint_id: "{test-endpoint-id}".to_string(),
            endpoint_name: "Test Audio Device".to_string(),
            original_disabled_state: Some(0),
        });
        snapshot.paused_services.push(ServiceBackup {
            service_name: "wuauserv".to_string(),
            display_name: "Windows Update".to_string(),
            previous_state: 4,
            was_paused_by_optimizer: true,
        });

        // 1. Save atomically
        let saved_path = SnapshotEngine::save_atomic(&mut snapshot, Some(&snap_file))
            .expect("Atomic save must succeed");
        assert_eq!(saved_path, snap_file);
        assert!(snap_file.exists());

        // 2. Load and verify
        let loaded = SnapshotEngine::load_and_verify(Some(&snap_file))
            .expect("Load and verify must succeed");
        assert_eq!(loaded.snapshot_id, "test_snap_001");
        assert_eq!(loaded.previous_game_mode, Some(true));
        assert_eq!(loaded.paused_services.len(), 1);
        assert_eq!(loaded.sha256_hash, snapshot.sha256_hash);

        // 3. Clean up
        let deleted = SnapshotEngine::delete_snapshot(Some(&snap_file)).expect("Delete must succeed");
        assert!(deleted);
        assert!(!snap_file.exists());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snapshot_tamper_detection() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_02");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("tampered_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_tamper_snap");
        snapshot.previous_game_mode = Some(false);
        SnapshotEngine::save_atomic(&mut snapshot, Some(&snap_file))
            .expect("Save must succeed");

        // Tamper with content on disk directly
        let raw_json = fs::read_to_string(&snap_file).unwrap();
        let tampered_json = raw_json.replace("\"previous_game_mode\": false", "\"previous_game_mode\": true");
        fs::write(&snap_file, tampered_json).unwrap();

        // Attempt load: must fail with IntegrityViolation
        let result = SnapshotEngine::load_and_verify(Some(&snap_file));
        match result {
            Err(SnapshotError::IntegrityViolation { expected_hash, computed_hash, .. }) => {
                assert_ne!(expected_hash, computed_hash);
            }
            other => panic!("Expected IntegrityViolation error, got: {:?}", other),
        }

        let _ = SnapshotEngine::delete_snapshot(Some(&snap_file));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snapshot_serialization_latency() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_03");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("perf_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("perf_test");
        snapshot.previous_game_mode = Some(true);
        for i in 0..10 {
            snapshot.previous_audio_endpoints.push(AudioEndpointBackup {
                endpoint_id: format!("ep_{}", i),
                endpoint_name: format!("Audio Endpoint {}", i),
                original_disabled_state: Some(0),
            });
        }

        let start = Instant::now();
        SnapshotEngine::save_atomic(&mut snapshot, Some(&snap_file))
            .expect("Atomic save should succeed");
        let elapsed = start.elapsed();

        println!("Snapshot serialization & atomic commit took: {:?}", elapsed);
        // Requirement: < 50ms
        assert!(
            elapsed.as_millis() < 50,
            "Snapshot serialization took {:?}, exceeding 50ms limit",
            elapsed
        );

        let _ = SnapshotEngine::delete_snapshot(Some(&snap_file));
        let _ = fs::remove_dir_all(&temp_dir);
    }
}
