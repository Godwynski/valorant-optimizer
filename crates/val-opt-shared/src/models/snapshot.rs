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

/// Constant-time byte slice comparison to mitigate timing attacks.
pub fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Standard RFC 2104 HMAC-SHA256 implementation.
pub fn compute_hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let mut hasher = Sha256::new();
        hasher.update(key);
        let hash = hasher.finalize();
        k[..32].copy_from_slice(&hash);
    } else {
        k[..key.len()].copy_from_slice(key);
    }

    let mut k_ipad = [0u8; 64];
    let mut k_opad = [0u8; 64];
    for i in 0..64 {
        k_ipad[i] = k[i] ^ 0x36;
        k_opad[i] = k[i] ^ 0x5c;
    }

    let mut inner_hasher = Sha256::new();
    inner_hasher.update(&k_ipad);
    inner_hasher.update(message);
    let inner_hash = inner_hasher.finalize();

    let mut outer_hasher = Sha256::new();
    outer_hasher.update(&k_opad);
    outer_hasher.update(&inner_hash);
    let outer_hash = outer_hasher.finalize();

    let mut result = [0u8; 32];
    result.copy_from_slice(&outer_hash);
    result
}

/// Complete atomic baseline system snapshot recording all modified parameters,
/// protected by DPAPI-backed HMAC-SHA256 cryptographic signature (TASK-SEC-04).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SystemStateSnapshot {
    pub snapshot_id: String,
    pub version: u32,
    pub timestamp_utc: String,
    #[serde(default)]
    pub sha256_hash: String,
    #[serde(default)]
    pub hmac_signature: String,
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
            hmac_signature: String::new(),
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

    /// Prepares canonical bytes for signing/verification (excluding signatures).
    fn canonical_payload_bytes(&self) -> Vec<u8> {
        let mut canonical = self.clone();
        canonical.sha256_hash.clear();
        canonical.hmac_signature.clear();
        serde_json::to_vec(&canonical).unwrap_or_default()
    }

    /// Computes SHA-256 over canonical serialized state.
    pub fn compute_payload_hash(&self) -> String {
        let bytes = self.canonical_payload_bytes();
        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        format!("{:x}", hasher.finalize())
    }

    /// Computes RFC 2104 HMAC-SHA256 signature using the machine-bound secret key.
    pub fn compute_payload_hmac(&self, key: &[u8]) -> String {
        let bytes = self.canonical_payload_bytes();
        let hmac_bytes = compute_hmac_sha256(key, &bytes);
        let mut hex = String::with_capacity(64);
        for b in hmac_bytes {
            use std::fmt::Write;
            let _ = write!(hex, "{:02x}", b);
        }
        hex
    }

    /// Validates that the recorded `sha256_hash` strictly matches the payload content.
    pub fn verify_integrity(&self) -> bool {
        if self.sha256_hash.is_empty() {
            return false;
        }
        let computed = self.compute_payload_hash();
        computed.eq_ignore_ascii_case(&self.sha256_hash)
    }

    /// Cryptographically validates the HMAC-SHA256 signature using the provided secret key.
    /// Employs constant-time byte comparison to prevent side-channel timing attacks.
    pub fn verify_hmac(&self, key: &[u8]) -> bool {
        if self.hmac_signature.is_empty() {
            return false;
        }
        let computed = self.compute_payload_hmac(key);
        constant_time_eq(self.hmac_signature.as_bytes(), computed.as_bytes())
    }

    /// Signs the snapshot in place with unkeyed SHA-256 digest (legacy fallback).
    pub fn sign_in_place(&mut self) {
        self.sha256_hash = self.compute_payload_hash();
    }

    /// Cryptographically signs the snapshot in place with both SHA-256 and HMAC-SHA256 (TASK-SEC-04).
    pub fn sign_with_key(&mut self, key: &[u8]) {
        self.sha256_hash = self.compute_payload_hash();
        self.hmac_signature = self.compute_payload_hmac(key);
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
    fn test_rfc2104_hmac_sha256_test_vector() {
        // RFC 4231 / RFC 2104 Test Case 2:
        // Key: "Jefe" (4 bytes)
        // Data: "what do ya want for nothing?" (28 bytes)
        // HMAC-SHA256: 5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let hmac = compute_hmac_sha256(key, data);
        let mut hex_str = String::new();
        for b in hmac {
            use std::fmt::Write;
            let _ = write!(hex_str, "{:02x}", b);
        }
        assert_eq!(hex_str, "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843");
    }

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

        // Sign snapshot legacy
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

    #[test]
    fn test_hmac_snapshot_tamper_detection_even_if_hash_recalculated() {
        let key = b"my-super-secret-machine-key-32b!";
        let mut snap = SystemStateSnapshot::new("snap_hmac_test");
        snap.previous_game_mode = Some(false);
        snap.sign_with_key(key);

        assert!(!snap.hmac_signature.is_empty());
        assert!(snap.verify_hmac(key));

        // Attacker tampers with game mode AND recalculates unkeyed sha256_hash
        let mut attacker_snap = snap.clone();
        attacker_snap.previous_game_mode = Some(true);
        // Attacker updates sha256_hash to match the tampered payload
        attacker_snap.sha256_hash = attacker_snap.compute_payload_hash();
        assert!(attacker_snap.verify_integrity(), "SHA-256 alone matches attacker payload");

        // BUT HMAC verification MUST FAIL because attacker lacks secret key!
        assert!(!attacker_snap.verify_hmac(key), "HMAC check must reject tampered snapshot!");

        // Wrong key must also fail
        let wrong_key = b"wrong-key-different-machine-32b!";
        assert!(!snap.verify_hmac(wrong_key), "Wrong key must fail HMAC verification");
    }
}

