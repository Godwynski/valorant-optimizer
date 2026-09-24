# Phase 3: Safe System Optimizations

---

## 1. Phase Objective
Implement the core, universally safe Windows subsystem optimizations: programmatic Game Mode enforcement, Windows Core Audio APO enhancement disabler (to flatten DPC spikes), and power scheme management with desktop vs laptop auto-detection. Every optimization must be 100% reversible through an atomic transaction snapshot.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-03`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 2 complete.
- **Target Deliverable:** Modular optimization engine with atomic rollback capability and verified zero audio/system regressions.
- **QA File:** `QA/QA_PHASE_03.md`

---

## 3. Tasks Breakdown

### `TASK-P03-001`: Windows Game Mode Programmatic Enforcer
- **Objective:** Implement module to query and safely enforce Windows Game Mode state.
- **Files Involved:**
  - `crates/val-opt-core/src/optimizations/game_mode.rs`
- **Requirements:** Query registry key `HKCU\Software\Microsoft\GameBar` (`AllowAutoGameMode`), provide backup of previous state, and enable Game Mode.
- **Verification Method:** Unit test verifying registry state toggle and rollback.
- **Completion Criteria:** Successfully enables Game Mode and rolls back cleanly without system restart.
- **Status:** `COMPLETE`

### `TASK-P03-002`: Windows Core Audio APO Enhancement Disabler
- **Objective:** Implement audio optimization module to disable DSP enhancements on the default audio endpoint to eliminate DPC spikes.
- **Files Involved:**
  - `crates/val-opt-core/src/optimizations/audio.rs`
- **Requirements:** Use Windows Core Audio APIs (`IMMDeviceEnumerator`, `IMMDevice`) and registry property store (`PKEY_AudioEndpoint_Disable_SysFx`) to disable enhancements cleanly.
- **Verification Method:** Verify sound remains active and audio enhancement toggle in Windows Sound Control Panel reflects changes.
- **Completion Criteria:** Audio remains operational with enhancements disabled; full rollback restores previous user settings.
- **Status:** `COMPLETE`

### `TASK-P03-003`: Power Scheme & Processor Boost Manager
- **Objective:** Implement power plan management module with desktop/laptop auto-detection.
- **Files Involved:**
  - `crates/val-opt-core/src/optimizations/power.rs`
- **Requirements:** Query `PowerGetActiveScheme`, apply Ultimate/High Performance on AC-powered desktops, preserve balanced thermal profiles on laptops, record original GUID for restoration.
- **Verification Method:** Verify active GUID changes via `powercfg /getactivescheme` and returns to original on rollback.
- **Completion Criteria:** Power plan changes cleanly without requiring reboot; desktop/laptop logic works properly.
- **Status:** `COMPLETE`

### `TASK-P03-004`: Optimization Transaction & Unified Rollback Module
- **Objective:** Create transactional optimization coordinator that aggregates Phase 3 settings into a reversible snapshot.
- **Files Involved:**
  - `crates/val-opt-core/src/optimizations/mod.rs`
  - `crates/val-opt-shared/src/models/snapshot.rs`
- **Requirements:** Serialize all "before" states to a transaction log; execute atomic rollback if any optimization step fails.
- **Verification Method:** Apply optimizations, trigger artificial failure, verify complete system rollback to baseline.
- **Completion Criteria:** Zero orphan changes on failure; 100% deterministic restoration.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_03.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 4.
