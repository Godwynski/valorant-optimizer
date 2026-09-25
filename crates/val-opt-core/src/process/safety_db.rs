//! In-memory Process Safety Database and Classification Engine.
//!
//! Enforces strict whitelisting to protect Windows kernel subsystems, Vanguard (`vgc.exe`),
//! and critical background services from accidental termination or interference.

use std::collections::HashMap;
use std::sync::OnceLock;
use thiserror::Error;
use val_opt_shared::models::process::ProcessTier;

/// Safety violation errors when an operation attempts to touch a protected subsystem.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SafetyViolationError {
    #[error("Safety violation: Process '{0}' is a Tier 0 protected system/Vanguard binary and CANNOT be terminated or modified")]
    ProtectedProcess(String),

    #[error("Safety violation: Service '{0}' is a Tier 0 critical system/Vanguard service and CANNOT be stopped")]
    ProtectedService(String),
}

/// The core in-memory process and service safety classification database.
pub struct ProcessSafetyDb {
    process_tiers: HashMap<String, ProcessTier>,
    service_tiers: HashMap<String, ProcessTier>,
}

static INSTANCE: OnceLock<ProcessSafetyDb> = OnceLock::new();

impl ProcessSafetyDb {
    /// Retrieve global singleton instance of the safety database.
    pub fn get() -> &'static Self {
        INSTANCE.get_or_init(Self::new)
    }

    /// Construct and initialize the default process safety taxonomy.
    pub fn new() -> Self {
        let mut process_tiers = HashMap::new();
        let mut service_tiers = HashMap::new();

        // -------------------------------------------------------------
        // TIER 0: PROTECTED CORE (HARD INVARIANT: NEVER TOUCH / NEVER TERMINATE)
        // -------------------------------------------------------------
        let tier0_procs = [
            "system",
            "smss.exe",
            "csrss.exe",
            "wininit.exe",
            "services.exe",
            "lsass.exe",
            "winlogon.exe",
            "dwm.exe",
            "fontdrvhost.exe",
            "audiodg.exe",
            "ctfmon.exe",
            "svchost.exe",
            "registry",
            "memory compression",
            "explorer.exe", // Trimmed only, never killed
            // Riot Anti-Cheat & Essential Services
            "vgc.exe",
            "vgk.sys",
            "riotclientservices.exe", // Heartbeat provider - MUST REMAIN ALIVE
            "valorant.exe",
            "valorant-win64-shipping.exe",
            // Optimizer binaries
            "val-opt-core.exe",
            "val-opt-cli.exe",
            "val-opt-gui.exe",
        ];
        for proc in tier0_procs {
            process_tiers.insert(proc.to_lowercase(), ProcessTier::Tier0Protected);
        }

        let tier0_services = [
            "vgc",
            "vgk",
            "cryptsvc",
            "rpcss",
            "rpceptmapper",
            "bfe",
            "dnscache",
            "dhcp",
            "audioendpointbuilder",
            "audiosrv",
            "mpssvc", // Windows Defender Firewall
            "windefend",
            "securityhealthservice",
            "wuauserv", // Windows Update (MUST_NOT_MODIFY per Phase P3 safety specification)
        ];
        for svc in tier0_services {
            service_tiers.insert(svc.to_lowercase(), ProcessTier::Tier0Protected);
        }

        // -------------------------------------------------------------
        // TIER 1: HARDWARE DRIVERS (PRESERVE OR ADJUST PRIORITY)
        // -------------------------------------------------------------
        let tier1_procs = [
            "amdfendrsr.exe",
            "radeonsoftware.exe",
            "amdow.exe",
            "nvcontainer.exe",
            "nvidia web helper.exe",
            "nvdisplay.container.exe",
            "lghub.exe",
            "lghub_agent.exe",
            "razersynapse.exe",
            "rzsynapse.exe",
        ];
        for proc in tier1_procs {
            process_tiers.insert(proc.to_lowercase(), ProcessTier::Tier1Driver);
        }

        // -------------------------------------------------------------
        // TIER 2: SAFE TO TERMINATE FOR GAMING SESSIONS
        // -------------------------------------------------------------
        let tier2_procs = [
            // Browsers
            "brave.exe",
            "chrome.exe",
            "msedge.exe",
            "firefox.exe",
            "opera.exe",
            "vivaldi.exe",
            // Communication & Media
            "discord.exe",
            "slack.exe",
            "spotify.exe",
            "teams.exe",
            // Launchers & Overlays
            "steam.exe",
            "steamwebhelper.exe",
            "epicgameslauncher.exe",
            "battle.net.exe",
            "galaxyclient.exe",
            "overwolf.exe",
            "medal.exe",
            // Riot Chromium UI (CEF) - Safe to purge post-game launch
            "riotclientux.exe",
            "riotclientuxrender.exe",
            // Background cloud sync & indexing
            "onedrive.exe",
            "dropbox.exe",
            "googledrivefs.exe",
            "searchindexer.exe",
        ];
        for proc in tier2_procs {
            process_tiers.insert(proc.to_lowercase(), ProcessTier::Tier2SafeTerminate);
        }

        // -------------------------------------------------------------
        // TIER 3: NON-ESSENTIAL PAUSABLE SERVICES
        // -------------------------------------------------------------
        let tier3_services = [
            "sysmain",   // Superfetch / Prefetch page defrag
            "diagtrack", // Telemetry / Diagnostics
            "spooler",   // Print Spooler
        ];
        for svc in tier3_services {
            service_tiers.insert(svc.to_lowercase(), ProcessTier::Tier3ServicePause);
        }

        Self {
            process_tiers,
            service_tiers,
        }
    }

    /// Classify a process name into its assigned safety tier.
    pub fn classify_process(&self, name: &str) -> ProcessTier {
        let clean = name.to_lowercase();
        self.process_tiers.get(&clean).copied().unwrap_or(ProcessTier::Unknown)
    }

    /// Classify a Windows service name into its assigned safety tier.
    pub fn classify_service(&self, name: &str) -> ProcessTier {
        let clean = name.to_lowercase();
        self.service_tiers.get(&clean).copied().unwrap_or(ProcessTier::Unknown)
    }

    /// Hard invariant safety gate: Verifies a process can be legally targeted for termination.
    /// Returns `Err(SafetyViolationError)` if the process is Tier 0 protected.
    pub fn assert_safe_to_kill(&self, name: &str) -> Result<(), SafetyViolationError> {
        let tier = self.classify_process(name);
        if tier == ProcessTier::Tier0Protected {
            return Err(SafetyViolationError::ProtectedProcess(name.to_string()));
        }
        Ok(())
    }

    /// Hard invariant safety gate: Verifies a service can be safely paused.
    /// Returns `Err(SafetyViolationError)` if the service is Tier 0 protected.
    pub fn assert_safe_to_stop_service(&self, name: &str) -> Result<(), SafetyViolationError> {
        let tier = self.classify_service(name);
        if tier == ProcessTier::Tier0Protected {
            return Err(SafetyViolationError::ProtectedService(name.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vanguard_and_system_protection() {
        let db = ProcessSafetyDb::get();

        // Tier 0 Process Invariants
        assert_eq!(db.classify_process("vgc.exe"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_process("VGC.EXE"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_process("csrss.exe"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_process("dwm.exe"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_process("System"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_process("RiotClientServices.exe"), ProcessTier::Tier0Protected);

        // Attempting to kill Tier 0 MUST return hard error
        assert!(db.assert_safe_to_kill("vgc.exe").is_err());
        assert!(db.assert_safe_to_kill("dwm.exe").is_err());
        assert!(db.assert_safe_to_kill("csrss.exe").is_err());
        assert!(db.assert_safe_to_kill("RiotClientServices.exe").is_err());

        // Tier 0 Service Invariants (Vanguard, Core OS, Defender, Windows Update)
        assert_eq!(db.classify_service("vgc"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_service("CryptSvc"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_service("WinDefend"), ProcessTier::Tier0Protected);
        assert_eq!(db.classify_service("wuauserv"), ProcessTier::Tier0Protected);
        assert!(db.assert_safe_to_stop_service("vgc").is_err());
        assert!(db.assert_safe_to_stop_service("CryptSvc").is_err());
        assert!(db.assert_safe_to_stop_service("WinDefend").is_err());
        assert!(db.assert_safe_to_stop_service("wuauserv").is_err());
    }

    #[test]
    fn test_tier2_and_tier3_classification() {
        let db = ProcessSafetyDb::get();

        // Safe Tier 2 targets
        assert_eq!(db.classify_process("brave.exe"), ProcessTier::Tier2SafeTerminate);
        assert_eq!(db.classify_process("discord.exe"), ProcessTier::Tier2SafeTerminate);
        assert_eq!(db.classify_process("RiotClientUx.exe"), ProcessTier::Tier2SafeTerminate);
        assert_eq!(db.classify_process("steam.exe"), ProcessTier::Tier2SafeTerminate);

        assert!(db.assert_safe_to_kill("brave.exe").is_ok());
        assert!(db.assert_safe_to_kill("RiotClientUx.exe").is_ok());

        // Optional Tier 3 services (user-controlled)
        assert_eq!(db.classify_service("SysMain"), ProcessTier::Tier3ServicePause);
        assert_eq!(db.classify_service("DiagTrack"), ProcessTier::Tier3ServicePause);
        assert_eq!(db.classify_service("spooler"), ProcessTier::Tier3ServicePause);

        assert!(db.assert_safe_to_stop_service("SysMain").is_ok());
        assert!(db.assert_safe_to_stop_service("spooler").is_ok());
    }
}
