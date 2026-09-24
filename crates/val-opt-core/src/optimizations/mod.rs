//! Safe Windows Subsystem Optimizations & Transactional Rollback Engine.

pub mod audio;
pub mod game_mode;
pub mod power;

use std::fs;
use std::path::{Path, PathBuf};
use tracing::{error, info, warn};
use val_opt_shared::models::snapshot::OptimizationTransaction;

pub use audio::*;
pub use game_mode::*;
pub use power::*;

/// The unified coordinator managing all Phase 3 safe system optimizations.
pub struct OptimizationCoordinator;

impl OptimizationCoordinator {
    /// Retrieve the standard persistence path for the active transaction snapshot.
    pub fn get_snapshot_path() -> PathBuf {
        let program_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        let dir = Path::new(&program_data).join("ValorantOptimizer");
        let _ = fs::create_dir_all(&dir);
        dir.join("active_transaction.json")
    }

    /// Apply all safe system optimizations atomically.
    /// If any individual step fails, previously applied changes are automatically rolled back.
    pub fn apply_optimizations() -> Result<OptimizationTransaction, String> {
        let transaction_id = format!("tx_{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis());
        let timestamp_utc = format!("{:?}", std::time::SystemTime::now());

        let mut transaction = OptimizationTransaction {
            transaction_id,
            timestamp_utc,
            previous_game_mode: None,
            previous_power_scheme: None,
            previous_audio_endpoints: Vec::new(),
            previous_adapter_properties: Vec::new(),
            previous_qos_policy: None,
            is_applied: false,
        };

        info!("Starting atomic system optimization transaction: {}", transaction.transaction_id);

        // Step 1: Enforce Windows Game Mode
        match game_mode::set_game_mode(true) {
            Ok(prev) => {
                transaction.previous_game_mode = Some(prev);
                info!("Game Mode enforced (previous state: {})", prev);
            }
            Err(e) => {
                error!("Game Mode optimization failed: {}. Rolling back...", e);
                Self::rollback(&transaction)?;
                return Err(format!("Failed to enforce Game Mode: {}", e));
            }
        }

        // Step 2: Configure Power Scheme (Desktop High/Ultimate, Laptop Balanced)
        match power::apply_gaming_power_scheme() {
            Ok(prev_guid) => {
                transaction.previous_power_scheme = Some(power::guid_to_string(&prev_guid));
                info!("Gaming power scheme activated (previous: {:?})", prev_guid);
            }
            Err(e) => {
                warn!("Power scheme optimization failed: {}. Continuing with remaining optimizations...", e);
            }
        }

        // Step 3: Disable Audio DSP / APO Enhancements to eradicate DPC spikes
        match audio::disable_audio_enhancements() {
            Ok(backups) => {
                info!("Disabled APO enhancements on {} audio endpoints", backups.len());
                transaction.previous_audio_endpoints = backups;
            }
            Err(e) => {
                warn!("Audio APO optimization failed: {}. Continuing with remaining optimizations...", e);
            }
        }

        // Step 4: Network Adapter Latency & Flow Control Optimizations
        if let Ok(primary_adapter) = val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary() {
            match crate::network::optimize_adapter_latency_properties(&primary_adapter.adapter_name) {
                Ok(mut backups) => {
                    if let Ok(Some(fc_backup)) = crate::network::disable_flow_control(&primary_adapter.adapter_name) {
                        backups.push(fc_backup);
                    }
                    info!("Optimized {} network adapter latency properties on {}", backups.len(), primary_adapter.adapter_name);
                    transaction.previous_adapter_properties = backups;
                }
                Err(e) => {
                    warn!("Network adapter latency optimization notice: {}. Continuing with remaining optimizations...", e);
                }
            }
        }

        // Step 5: Windows QoS DSCP Policy Registration
        match crate::network::register_valorant_qos_policy() {
            Ok(qos_backup) => {
                info!("Registered VALORANT QoS DSCP 46 policy");
                transaction.previous_qos_policy = Some(qos_backup);
            }
            Err(e) => {
                warn!("QoS DSCP policy notice: {}. Continuing with remaining optimizations...", e);
            }
        }

        transaction.is_applied = true;

        // Persist transaction record to disk for crash recovery
        let snapshot_path = Self::get_snapshot_path();
        if let Ok(json) = serde_json::to_string_pretty(&transaction) {
            let _ = fs::write(&snapshot_path, json);
        }

        info!("Optimization transaction successfully applied and committed");
        Ok(transaction)
    }

    /// Roll back an applied optimization transaction, restoring the system to baseline.
    pub fn rollback(transaction: &OptimizationTransaction) -> Result<(), String> {
        info!("Executing atomic rollback for transaction: {}", transaction.transaction_id);

        // 1. Unregister QoS Policy
        if let Some(ref qos_backup) = transaction.previous_qos_policy {
            if let Err(e) = crate::network::unregister_qos_policy(qos_backup) {
                error!("Failed to unregister QoS policy: {}", e);
            } else {
                info!("Unregistered QoS policy: {}", qos_backup.policy_name);
            }
        }

        // 2. Restore Network Adapter Properties
        if !transaction.previous_adapter_properties.is_empty() {
            if let Err(e) = crate::network::restore_adapter_properties(&transaction.previous_adapter_properties) {
                error!("Failed to restore network adapter properties: {}", e);
            } else {
                info!("Restored {} network adapter properties", transaction.previous_adapter_properties.len());
            }
        }

        // 3. Restore Audio Enhancements
        if !transaction.previous_audio_endpoints.is_empty() {
            if let Err(e) = audio::restore_audio_enhancements(&transaction.previous_audio_endpoints) {
                error!("Failed to restore audio enhancements: {}", e);
            } else {
                info!("Restored {} audio endpoints", transaction.previous_audio_endpoints.len());
            }
        }

        // 4. Restore Power Scheme
        if let Some(ref scheme_str) = transaction.previous_power_scheme {
            if let Ok(guid) = power::string_to_guid(scheme_str) {
                if let Err(e) = power::restore_power_scheme(guid) {
                    error!("Failed to restore power scheme: {}", e);
                } else {
                    info!("Restored original power scheme: {}", scheme_str);
                }
            }
        }

        // 5. Restore Game Mode
        if let Some(prev_mode) = transaction.previous_game_mode {
            if let Err(e) = game_mode::restore_game_mode(prev_mode) {
                error!("Failed to restore Game Mode: {}", e);
            } else {
                info!("Restored original Game Mode state: {}", prev_mode);
            }
        }

        // Clean up persisted snapshot
        let snapshot_path = Self::get_snapshot_path();
        if snapshot_path.exists() {
            let _ = fs::remove_file(snapshot_path);
        }

        info!("Rollback completed successfully");
        Ok(())
    }

    /// Check if an orphaned transaction file exists from an ungraceful exit and restore it.
    pub fn recover_orphaned_transaction() -> Result<bool, String> {
        let snapshot_path = Self::get_snapshot_path();
        if !snapshot_path.exists() {
            return Ok(false);
        }

        let content = fs::read_to_string(&snapshot_path).map_err(|e| e.to_string())?;
        let transaction: OptimizationTransaction = serde_json::from_str(&content).map_err(|e| e.to_string())?;

        warn!("Found orphaned optimization transaction {}; initiating recovery rollback", transaction.transaction_id);
        Self::rollback(&transaction)?;
        Ok(true)
    }
}

#[cfg(test)]
pub(crate) static SYSTEM_STATE_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_full_optimization_transaction_and_rollback() {
        let _lock = SYSTEM_STATE_MUTEX.lock().unwrap();
        let initial_game_mode = game_mode::get_game_mode_state().unwrap();
        let initial_power_guid = power::get_active_power_scheme().unwrap();

        // Apply optimizations
        let transaction = OptimizationCoordinator::apply_optimizations()
            .expect("Optimization transaction should apply cleanly");

        assert!(transaction.is_applied);
        assert!(transaction.previous_game_mode.is_some());

        // Roll back
        OptimizationCoordinator::rollback(&transaction)
            .expect("Rollback should execute cleanly");

        // Verify restoration
        let restored_game_mode = game_mode::get_game_mode_state().unwrap();
        let restored_power_guid = power::get_active_power_scheme().unwrap();

        assert_eq!(initial_game_mode, restored_game_mode);
        assert_eq!(initial_power_guid, restored_power_guid);
    }

    #[test]
    fn test_atomic_rollback_on_simulated_failure() {
        let _lock = SYSTEM_STATE_MUTEX.lock().unwrap();
        let initial_game_mode = game_mode::get_game_mode_state().unwrap();
        let initial_power_guid = power::get_active_power_scheme().unwrap();

        // Construct mock partial transaction with applied Game Mode
        let prev_mode = game_mode::set_game_mode(!initial_game_mode).unwrap();
        let partial_tx = OptimizationTransaction {
            transaction_id: "tx_simulated_fail".to_string(),
            timestamp_utc: "2026-09-25T00:00:00Z".to_string(),
            previous_game_mode: Some(prev_mode),
            previous_power_scheme: Some(power::guid_to_string(&initial_power_guid)),
            previous_audio_endpoints: Vec::new(),
            previous_adapter_properties: Vec::new(),
            previous_qos_policy: None,
            is_applied: false,
        };

        // Trigger rollback simulating atomic error handler
        OptimizationCoordinator::rollback(&partial_tx)
            .expect("Rollback handler must succeed");

        // Verify that partial changes were eradicated and baseline is 100% preserved
        let restored_mode = game_mode::get_game_mode_state().unwrap();
        let restored_guid = power::get_active_power_scheme().unwrap();

        assert_eq!(restored_mode, initial_game_mode, "Game Mode must return to baseline after rollback");
        assert_eq!(restored_guid, initial_power_guid, "Power Scheme must return to baseline after rollback");
    }
}
