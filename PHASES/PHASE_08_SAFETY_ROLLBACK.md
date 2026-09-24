# Phase 8: Safety, State Snapshot & Auto-Rollback Engine

---

## 1. Phase Objective
Harden the system's fault-tolerance, disaster recovery, and anti-cheat compliance. This includes atomic JSON state snapshotting with SHA-256 integrity validation, an automated Windows boot orphan crash recovery service, pre-flight Riot Vanguard integrity checks, and a standalone emergency rollback CLI tool.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-08`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 7 complete.
- **Target Deliverable:** Fully crash-resilient rollback system guaranteeing zero orphaned settings under sudden power cuts, BSODs, or application crashes.
- **QA File:** `QA/QA_PHASE_08.md`

---

## 3. Tasks Breakdown

### `TASK-P08-001`: Atomic State Snapshot Serializer
- **Objective:** Build atomic snapshot engine recording complete baseline system state prior to modifications.
- **Files Involved:**
  - `crates/val-opt-core/src/state/snapshot_engine.rs`
- **Requirements:** Capture power schemes, audio settings, paused services, closed processes, and NIC configurations into `%ProgramData%\ValorantOptimizer\snapshot.json` with SHA-256 integrity hash.
- **Verification Method:** Unit test verifying serialized state can be written, hashed, and read back identically.
- **Completion Criteria:** Complete snapshot serialized in < 50ms without disk corruption risk.
- **Status:** `COMPLETE`

### `TASK-P08-002`: Windows Boot Orphan Crash Recovery Service
- **Objective:** Implement auto-recovery mechanism that restores paused services if the PC experiences a crash or sudden power cut mid-game.
- **Files Involved:**
  - `crates/val-opt-core/src/state/crash_recovery.rs`
- **Requirements:** On daemon startup, check for uncommitted `snapshot.json`; if found, execute immediate full restoration, log recovery event, and delete snapshot.
- **Verification Method:** Apply tweaks, simulate hard process termination via `taskkill /F`, restart daemon, and verify all services restore cleanly.
- **Completion Criteria:** 100% automated recovery from abnormal system shutdowns.
- **Status:** `COMPLETE`

### `TASK-P08-003`: Riot Vanguard Active Compliance Checker
- **Objective:** Implement pre-flight anti-cheat validator ensuring all Vanguard requirements are strictly satisfied before launch.
- **Files Involved:**
  - `crates/val-opt-core/src/safety/vanguard_check.rs`
- **Requirements:** Verify `vgc` service is running, `vgk.sys` driver is loaded, Windows test signing is OFF, Secure Boot is ON, and VBS/HVCI is intact.
- **Verification Method:** Run validator on test system; verify warnings fire if test signing is enabled or `vgc` is stopped.
- **Completion Criteria:** Rejects optimization launch and alerts user if any Vanguard integrity prerequisite is compromised.
- **Status:** `COMPLETE`

### `TASK-P08-004`: One-Click Manual Emergency Rollback Trigger
- **Objective:** Provide a fail-safe standalone CLI and GUI rollback command (`val-opt-cli rollback`).
- **Files Involved:**
  - `crates/val-opt-cli/src/main.rs`
- **Requirements:** Standalone executable that reads `snapshot.json` and restores all modified system settings without requiring GUI or daemon.
- **Verification Method:** Run `val-opt-cli rollback` from administrator PowerShell; verify immediate reversion of all settings.
- **Completion Criteria:** Immediate, failsafe restoration under all operational conditions.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_08.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 9.
