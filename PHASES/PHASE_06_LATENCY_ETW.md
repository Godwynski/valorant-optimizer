# Phase 6: Latency Monitoring & Kernel ETW Integration

---

## 1. Phase Objective
Integrate real-time kernel Deferred Procedure Call (DPC) and Interrupt Service Routine (ISR) latency event tracing using Windows ETW. This module isolates buggy third-party drivers (`ndis.sys`, audio drivers, GPU drivers) causing frame-time hitching and emits telemetry without degrading game thread execution.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-06`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 5 complete.
- **Target Deliverable:** Kernel ETW latency session monitor with fault isolation identifying drivers exceeding 500µs.
- **QA File:** `QA/QA_PHASE_06.md`

---

## 3. Tasks Breakdown

### `TASK-P06-001`: Kernel ETW DPC / ISR Event Session Manager
- **Objective:** Build real-time DPC and ISR latency event session using Windows ETW.
- **Files Involved:**
  - `crates/val-opt-core/src/latency/etw_session.rs`
- **Requirements:** Start a kernel trace session with `EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT`, stream events without disk buffering, and parse execution duration.
- **Verification Method:** Compare DPC latency metrics with LatencyMon during heavy disk/GPU activity.
- **Completion Criteria:** Captures DPC and ISR execution times with microsecond precision and < 1% CPU overhead.
- **Status:** `COMPLETE`

### `TASK-P06-002`: Buggy Driver Fault Isolator
- **Objective:** Implement module to resolve DPC execution spikes to specific driver filenames (`.sys`).
- **Files Involved:**
  - `crates/val-opt-core/src/latency/driver_isolator.rs`
- **Requirements:** Correlate DPC routine addresses to loaded kernel module base addresses; identify offending drivers (e.g. `ndis.sys`, audio drivers, GPU drivers) exceeding 500µs.
- **Verification Method:** Inject synthetic load or test against known high-DPC audio driver; verify offending driver name is identified.
- **Completion Criteria:** Outputs prioritized list of kernel drivers ranked by highest single DPC execution time.
- **Status:** `COMPLETE`

### `TASK-P06-003`: Real-Time Latency Health Monitor
- **Objective:** Create a background watchdog that continuously tracks system interrupt health during a match.
- **Files Involved:**
  - `crates/val-opt-core/src/latency/monitor.rs`
- **Requirements:** Ring buffer of recent DPC/ISR durations; triggers warnings if latency exceeds 1000µs threshold.
- **Verification Method:** Run stress test; verify warning event is emitted when threshold is breached.
- **Completion Criteria:** Emits structured latency events through IPC channel without interrupting game threads.
- **Status:** `COMPLETE`

### `TASK-P06-004`: Latency Diagnostic Report Generator
- **Objective:** Produce comprehensive hardware latency health report for the user.
- **Files Involved:**
  - `crates/val-opt-core/src/latency/report.rs`
- **Requirements:** Export formatted summary detailing system suitability for competitive gaming (DPC/ISR stability, BIOS power management impact, timer integrity).
- **Verification Method:** Run diagnostic suite and verify output markdown/JSON report.
- **Completion Criteria:** Produces actionable report identifying whether hardware/drivers are bottlenecking input latency.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_06.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 7.
