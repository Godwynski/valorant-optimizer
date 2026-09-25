# Phase 4: Process & Service Management Engine

---

## 1. Phase Objective
Build the process safety database and lifecycle supervisor. This engine gracefully terminates safe Tier 2 bloatware (browsers, Discord, launchers), pauses Tier 3 background services (`wuauserv`, `SysMain`), trims Windows Explorer's working set, terminates the Riot Client Chromium UI (`RiotClientUx.exe`) post-launch, sets game priority to `High`, and supervises VALORANT in a zero-overhead wait state.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-04`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 3 complete.
- **Target Deliverable:** Deterministic process and service manager that purges background noise without touching Vanguard or critical Windows subsystems.
- **QA File:** `QA/QA_PHASE_04.md`

---

## 3. Tasks Breakdown

### `TASK-P04-001`: Process Safety Database & Classification Parser
- **Objective:** Implement in-memory process safety database mapping executables to Tiers 0 through 3.
- **Files Involved:**
  - `crates/val-opt-core/src/process/safety_db.rs`
  - `crates/val-opt-shared/src/models/process.rs`
- **Requirements:** Fast lookup table containing protected Tier 0 processes (Vanguard, DWM, System), Tier 1 driver processes, Tier 2 candidates (Brave, Discord, Steam), and Tier 3 services.
- **Verification Method:** Unit tests verifying known critical processes cannot be marked for termination.
- **Completion Criteria:** Attempting to classify `vgc.exe`, `csrss.exe`, or `dwm.exe` as Tier 2 or 3 triggers a hard compile/runtime error.
- **Status:** `COMPLETE`

### `TASK-P04-002`: Graceful Process Termination & Memory Diagnostics
- **Objective:** Implement safe termination engine with two-stage exit (`WM_CLOSE` -> 1000ms wait -> `TerminateProcess`) and observational RAM telemetry.
- **Files Involved:**
  - `crates/val-opt-core/src/process/terminator.rs`
  - `crates/val-opt-core/src/process/memory.rs`
- **Requirements:** Terminate safe Tier 2 targets gracefully; record process paths for post-match relaunch; query diagnostic memory statistics via `K32GetProcessMemoryInfo`.
- **Phase P3 De-Scoping Note:** Forced `EmptyWorkingSet` working-set trimming was PERMANENTLY REMOVED in Phase P3 (`TASK-OPT-01`). Forcing physical pages to the standby/paged pool causes soft page faults and micro-stuttering upon shell access. Replaced with passive, read-only diagnostic telemetry.
- **Verification Method:** Launch test instances of notepad and browser; verify graceful exit and diagnostic telemetry sampling.
- **Completion Criteria:** Clean termination of Tier 2 processes without process tree leaks or orphaned handles.
- **Status:** `COMPLETE (REMEDIATED IN P3)`

### `TASK-P04-003`: Non-Essential Windows Service Pauser
- **Objective:** Implement service management module to pause and resume Tier 3 background services (`SysMain`, `DiagTrack`, `Spooler`).
- **Files Involved:**
  - `crates/val-opt-core/src/process/services.rs`
  - `crates/val-opt-core/src/process/safety_db.rs`
- **Requirements:** Use Windows Service Control Manager (`OpenSCManagerW`, `OpenServiceW`, `ControlService`) to send `SERVICE_CONTROL_STOP`; record previous service running state.
- **Phase P3 Safety Note:** Windows Update (`wuauserv`) was PERMANENTLY MOVED to Tier 0 Protected (`MUST_NOT_MODIFY`) in Phase P3 (`TASK-OPT-04`). Windows security and component updating must never be disabled or paused automatically. Supported Tier 3 services for user-controlled pausing are `SysMain`, `DiagTrack`, and `Spooler`.
- **Verification Method:** Verify services enter Stopped state during gaming and resume Running state upon restoration.
- **Completion Criteria:** Services start and stop safely without registry corruption or service hangs.
- **Status:** `COMPLETE (REMEDIATED IN P3)`

### `TASK-P04-004`: VALORANT Process Lifecycle Supervisor
- **Objective:** Implement game lifecycle watcher monitoring for `VALORANT-Win64-Shipping.exe`.
- **Files Involved:**
  - `crates/val-opt-core/src/process/supervisor.rs`
- **Requirements:** Detect game launch, terminate `RiotClientUx.exe` (CEF frontend), apply `HIGH_PRIORITY_CLASS`, apply P-core affinity mask if Intel hybrid, and enter low-overhead `WaitForSingleObject` wait state.
- **Verification Method:** Test against mock game executable; verify priority, affinity, and child process termination within 500ms of launch.
- **Completion Criteria:** Zero CPU usage during active supervision; immediate triggering of restoration upon game process exit.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_04.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 5.
