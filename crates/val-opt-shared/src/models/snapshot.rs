use serde::{Deserialize, Serialize};

/// Backup record for an individual audio endpoint's DSP / APO enhancement configuration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AudioEndpointBackup {
    pub endpoint_id: String,
    pub endpoint_name: String,
    pub original_disabled_state: Option<u32>,
}

use super::network::{AdapterPropertyBackup, QosPolicyBackup};
use super::process::{ServiceBackup, TerminatedAppBackup};
use sha2::{Digest, Sha256};

/// Transactional snapshot recording state prior to applying optimizations,
/// enabling 100% deterministic, atomic rollback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptimizationTransaction {
    pub transaction_id: String,
    pub timestamp_utc: String,
    pub previous_game_mode: Option<bool>,
    pub previous_power_scheme: Option<String>,
    pub previous_audio_endpoints: Vec<AudioEndpointBackup>,
    #[serde(default)]
    pub previous_adapter_properties: Vec<AdapterPropertyBackup>,
    #[serde(default)]
    pub previous_qos_policy: Option<QosPolicyBackup>,
    pub is_applied: bool,
}

impl Default for OptimizationTransaction {
    fn default() -> Self {
        Self {
            transaction_id: String::new(),
            timestamp_utc: String::new(),
            previous_game_mode: None,
            previous_power_scheme: None,
            previous_audio_endpoints: Vec::new(),
            previous_adapter_properties: Vec::new(),
            previous_qos_policy: None,
            is_applied: false,
        }
    }
}

/// Complete atomic baseline system snapshot recording all modified parameters,
/// protected by a SHA-256 integrity hash for failsafe crash recovery and rollback.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemStateSnapshot {
    pub snapshot_id: String,
    pub version: u32,
    pub timestamp_utc: String,
    #[serde(default)]
    pub sha256_hash: String,
    pub previous_game_mode: Option<bool>,
    pub previous_power_scheme: Option<String>,
    pub previous_audio_endpoints: Vec<AudioEndpointBackup>,
    #[serde(default)]
    pub previous_adapter_properties: Vec<AdapterPropertyBackup>,
    #[serde(default)]
    pub previous_qos_policy: Option<QosPolicyBackup>,
    #[serde(default)]
    pub paused_services: Vec<ServiceBackup>,
    #[serde(default)]
    pub terminated_processes: Vec<TerminatedAppBackup>,
    #[serde(default)]
    pub is_applied: bool,
}

impl Default for SystemStateSnapshot {
    fn default() -> Self {
        Self {
            snapshot_id: String::new(),
            version: 1,
            timestamp_utc: String::new(),
            sha256_hash: String::new(),
            previous_game_mode: None,
            previous_power_scheme: None,
            previous_audio_endpoints: Vec::new(),
            previous_adapter_properties: Vec::new(),
            previous_qos_policy: None,
            paused_services: Vec::new(),
            terminated_processes: Vec::new(),
            is_applied: false,
        }
    }
}

impl SystemStateSnapshot {
    pub const CURRENT_VERSION: u32 = 1;

    /// Generates a new snapshot initialized with UTC timestamp and version.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            snapshot_id: id.into(),
            version: Self::CURRENT_VERSION,
            timestamp_utc: format!("{:?}", std::time::SystemTime::now()),
            ..Default::default()
        }
    }

    /// Computes SHA-256 over canonical serialized state (excluding the sha256_hash field itself).
    pub fn compute_payload_hash(&self) -> String {
        let mut canonical = self.clone();
        canonical.sha256_hash.clear();
        let bytes = serde_json::to_vec(&canonical).unwrap_or_default();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Validates that the recorded `sha256_hash` strictly matches the payload content.
    pub fn verify_integrity(&self) -> bool {
        if self.sha256_hash.is_empty() {
            return false;
        }
        let computed = self.compute_payload_hash();
        computed.eq_ignore_ascii_case(&self.sha256_hash)
    }

    /// Signs the snapshot by computing and storing the SHA-256 hash.
    pub fn sign_in_place(&mut self) {
        self.sha256_hash = self.compute_payload_hash();
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemSnapshot {
    pub id: String,
    pub created_at: String,
    pub active_power_scheme: Option<String>,
    pub game_mode_enabled: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_state_snapshot_hashing_and_tampering() {
        let mut snap = SystemStateSnapshot::new("snap_test_001");
        snap.previous_game_mode = Some(true);
        snap.previous_power_scheme = Some("381b4222-f694-41f0-9685-ff5bb260df2e".to_string());
        snap.previous_audio_endpoints.push(AudioEndpointBackup {
            endpoint_id: "{0.0.0.00000000}.{test}".to_string(),
            endpoint_name: "Realtek High Definition Audio".to_string(),
            original_disabled_state: Some(0),
        });

        // Sign snapshot
        snap.sign_in_place();
        assert!(!snap.sha256_hash.is_empty());
        assert_eq!(snap.sha256_hash.len(), 64);
        assert!(snap.verify_integrity(), "Signed snapshot must pass integrity check");

        // Simulate tampering with state
        let mut tampered = snap.clone();
        tampered.previous_game_mode = Some(false);
        assert!(!tampered.verify_integrity(), "Tampered snapshot must fail integrity check");

        // Simulate tampering with hash
        let mut bad_hash = snap.clone();
        bad_hash.sha256_hash = "0000000000000000000000000000000000000000000000000000000000000000".to_string();
        assert!(!bad_hash.verify_integrity(), "Altered hash must fail integrity check");
    }
}
