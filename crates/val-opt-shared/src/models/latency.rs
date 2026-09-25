use crate::benchmarking::models::TelemetryProvenance;
use serde::{Deserialize, Serialize};

/// Status and provenance classification for a DPC/ISR latency measurement.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LatencyMeasurementStatus {
    /// Telemetry collected directly from genuine Windows NT Kernel Logger ETW stream.
    RealEtwCollected,
    /// Kernel ETW tracing requires administrator elevation which was not present.
    UnsupportedElevationRequired,
    /// NT Kernel Logger was already locked/active by an external system profiler or WPR.
    UnsupportedSessionInUse,
    /// An internal Win32 ETW session error occurred.
    SessionError,
    /// Synthetic fixture used strictly for offline testing / CI environments.
    SyntheticTestFixture,
}

impl LatencyMeasurementStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RealEtwCollected => "REAL_ETW_COLLECTED",
            Self::UnsupportedElevationRequired => "UNSUPPORTED_ELEVATION_REQUIRED",
            Self::UnsupportedSessionInUse => "UNSUPPORTED_SESSION_IN_USE",
            Self::SessionError => "SESSION_ERROR",
            Self::SyntheticTestFixture => "SYNTHETIC_TEST_FIXTURE",
        }
    }
}

/// High-resolution DPC or ISR execution sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DpcIsrSample {
    pub timestamp_us: u64,
    pub duration_us: u64,
    pub routine_address: u64,
    pub is_dpc: bool, // true = DPC, false = ISR
    pub driver_name: Option<String>,
}

/// Aggregated latency statistics for a specific loaded kernel driver.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DriverLatencyStat {
    pub driver_name: String,
    pub driver_path: String,
    pub base_address: u64,
    pub max_execution_us: u64,
    pub total_execution_us: u64,
    pub execution_count: u64,
    pub avg_execution_us: f64,
    pub exceeds_500us_threshold: bool,
}

/// Severity classification of real-time latency anomalies.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum LatencySeverity {
    Normal,
    Warning,  // 500us - 1000us
    Critical, // >= 1000us (Severe frame-drop hazard)
}

impl LatencySeverity {
    pub fn from_duration_us(duration_us: u64) -> Self {
        if duration_us >= 1000 {
            LatencySeverity::Critical
        } else if duration_us >= 500 {
            LatencySeverity::Warning
        } else {
            LatencySeverity::Normal
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            LatencySeverity::Normal => "NORMAL",
            LatencySeverity::Warning => "WARNING",
            LatencySeverity::Critical => "CRITICAL",
        }
    }
}

/// Real-time warning event emitted when DPC/ISR execution exceeds safe gaming thresholds.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatencyWarningEvent {
    pub timestamp_us: u64,
    pub duration_us: u64,
    pub driver_name: String,
    pub routine_address: u64,
    pub is_dpc: bool,
    pub threshold_us: u64,
    pub severity: LatencySeverity,
    pub message: String,
}

/// Comprehensive hardware & driver latency diagnostic report.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LatencyReport {
    pub provenance: TelemetryProvenance,
    pub status: LatencyMeasurementStatus,
    pub total_dpcs_captured: u64,
    pub total_isrs_captured: u64,
    pub highest_dpc_us: u64,
    pub highest_isr_us: u64,
    pub highest_dpc_driver: Option<String>,
    pub highest_isr_driver: Option<String>,
    pub offending_drivers: Vec<DriverLatencyStat>,
    pub top_drivers: Vec<DriverLatencyStat>,
    pub system_suitable_for_competitive: bool,
    pub recommendations: Vec<String>,
    pub test_duration_secs: f64,
}
