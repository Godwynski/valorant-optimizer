//! Inter-Process Communication (IPC) Protocol Definition.
//!
//! Defines the structured message framing and commands communicated over the
//! Windows Named Pipe `\\.\pipe\val_opt_ipc` between `val-opt-core` and `val-opt-gui`.

use serde::{Deserialize, Serialize};
use crate::models::latency::LatencyReport;
use crate::models::network::BufferbloatReport;

/// The standard Windows Named Pipe endpoint for the VALORANT Performance Optimizer.
pub const VAL_OPT_PIPE_NAME: &str = r"\\.\pipe\val_opt_ipc";

/// Standard requests sent from the GUI client to the Core daemon.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum IpcRequest {
    Ping,
    GetStatus,
    InspectSystem,
    ApplyOptimizations,
    RollbackOptimizations,
    TrimWorkingSet,
    PurgeBloatware,
    GetLatencyReport { duration_secs: f64 },
    RunBufferbloatTest,
    LaunchOptimizedGame { auto_relaunch_gui: bool },
    ShutdownDaemon,
}

/// Standard responses returned by the Core daemon to the GUI client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", content = "payload")]
pub enum IpcResponse {
    Pong,
    Status {
        daemon_version: String,
        is_game_running: bool,
        is_optimized: bool,
        active_profile: String,
    },
    SystemManifest(serde_json::Value),
    OptimizationSuccess {
        transaction_id: String,
        game_mode: bool,
        power_scheme: String,
        audio_endpoints: usize,
        adapter_properties: usize,
        qos_policy_active: bool,
    },
    RollbackSuccess {
        message: String,
    },
    WorkingSetTrimmed {
        bytes_reclaimed: usize,
    },
    BloatwarePurged {
        terminated_count: usize,
    },
    LatencyReport(LatencyReport),
    BufferbloatReport(BufferbloatReport),
    GameLaunchAcknowledged {
        message: String,
    },
    Error {
        code: String,
        message: String,
    },
}

/// Serialize a message to a newline-terminated JSON byte vector.
pub fn encode_message<T: Serialize>(message: &T) -> Result<Vec<u8>, String> {
    let mut json = serde_json::to_vec(message).map_err(|e| format!("Serialization error: {}", e))?;
    json.push(b'\n');
    Ok(json)
}

/// Deserialize a message from a JSON byte slice.
pub fn decode_message<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> Result<T, String> {
    let s = std::str::from_utf8(bytes).map_err(|e| format!("UTF-8 error: {}", e))?;
    let trimmed = s.trim();
    serde_json::from_str(trimmed).map_err(|e| format!("Deserialization error: {} for '{}'", e, trimmed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ipc_request_serialization_roundtrip() {
        let req = IpcRequest::GetLatencyReport { duration_secs: 2.5 };
        let encoded = encode_message(&req).unwrap();
        assert_eq!(encoded.last(), Some(&b'\n'));

        let decoded: IpcRequest = decode_message(&encoded).unwrap();
        assert_eq!(req, decoded);
    }

    #[test]
    fn test_ipc_response_serialization_roundtrip() {
        let resp = IpcResponse::Status {
            daemon_version: "0.1.0".to_string(),
            is_game_running: false,
            is_optimized: true,
            active_profile: "Competitive Extreme".to_string(),
        };
        let encoded = encode_message(&resp).unwrap();
        let decoded: IpcResponse = decode_message(&encoded).unwrap();
        assert_eq!(resp, decoded);
    }
}
