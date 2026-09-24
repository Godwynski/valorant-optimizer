//! Process and service data models and safety classification tiers.

use serde::{Deserialize, Serialize};

/// Process safety and optimization classification tier.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub enum ProcessTier {
    /// Tier 0: Critical OS subsystems & Riot Vanguard. NEVER TOUCH / TERMINATION FORBIDDEN.
    Tier0Protected,
    /// Tier 1: Hardware peripheral & GPU driver software. Preserve or adjust priority only.
    Tier1Driver,
    /// Tier 2: Non-essential third-party bloatware (browsers, Discord, secondary launchers). Safe to terminate.
    Tier2SafeTerminate,
    /// Tier 3: Non-essential background services (Windows Update, SysMain, DiagTrack). Safe to pause.
    Tier3ServicePause,
    /// Unclassified / unknown process.
    Unknown,
}

/// Metadata and state for a running Windows process.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub path: Option<String>,
    pub tier: ProcessTier,
    pub working_set_kb: usize,
    pub priority_class: u32,
}

/// Backup record for an application terminated for a gaming session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TerminatedAppBackup {
    pub name: String,
    pub executable_path: String,
    pub command_line: Option<String>,
}

/// Windows background service optimization status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ServiceBackup {
    pub service_name: String,
    pub display_name: String,
    pub previous_state: u32, // SERVICE_RUNNING, SERVICE_STOPPED, etc.
    pub was_paused_by_optimizer: bool,
}
