# Phase 2: Benchmarking & Telemetry Infrastructure

---

## 1. Phase Objective
Build an empirical measurement infrastructure to objectively test optimizations against stock Windows baselines. This includes high-precision frame-time ingestion (ETW / PresentMon API), statistical distribution analysis (1% / 0.1% lows, variance), automated multi-trial A/B testing with Student's t-test verification, and UDP jitter tracking.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-02`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 1 complete.
- **Target Deliverable:** Headless benchmark runner capable of recording 10 repeated trials, computing statistical significance ($p$-values), and rejecting placebo tweaks.
- **QA File:** `QA/QA_PHASE_02.md`

---

## 3. Tasks Breakdown

### `TASK-P02-001`: PresentMon / ETW Frame-Time Ingestion Engine
- **Objective:** Build a headless frame-time telemetry collector utilizing the Intel PresentMon API or Windows ETW D3D events.
- **Files Involved:**
  - `crates/val-opt-core/src/benchmarking/etw_capture.rs`
  - `crates/val-opt-core/src/benchmarking/frametimes.rs`
  - `crates/val-opt-core/src/benchmarking/synthetic.rs`
- **Requirements:** Capture individual application frame presentation intervals (`MsBetweenPresents`, Option A: Application Present Cadence; physical display timing `MsUntilDisplayed` remains UNVERIFIED) without injecting hooks into the game process.
- **Verification Method:** Run synthetic DirectX sample app and verify frame capture stream with microsecond precision.
- **Completion Criteria:** In-memory collector ingestion operates with < 0.001% CPU overhead (TEST-VERIFIED) and zero DLL injection into game memory. (Live ETW collection requires Administrator elevation; UNVERIFIED in standard test runner).
- **Status:** `COMPLETE`

### `TASK-P02-002`: Statistical Metrics & Percentile Calculation Engine
- **Objective:** Implement statistical computation module for Average Present Rate (FPS), 1% Low (99th percentile), 0.1% Low (99.9th percentile), and present cadence standard deviation.
- **Files Involved:**
  - `crates/val-opt-shared/src/benchmarking/stats.rs`
  - `crates/val-opt-shared/src/benchmarking/models.rs`
- **Requirements:** Compute rolling and total statistical distributions over recorded present timestamps, with configurable warm-up frame trimming.
- **Verification Method:** Unit test on synthetic timestamp arrays with known statistical percentiles.
- **Completion Criteria:** Accurately computes average present rate, 1% low, 0.1% low, and variance matching reference datasets.
- **Status:** `COMPLETE`

### `TASK-P02-003`: Automated Multi-Trial A/B Testing Harness
- **Objective:** Create an automated harness that executes N repeated interleaved benchmark runs (Baseline vs Optimized) and computes paired statistical tests.
- **Files Involved:**
  - `crates/val-opt-core/src/benchmarking/ab_runner.rs`
- **Requirements:** Execute configurable interleaved test cycles ($A_1 \to B_1 \to A_2 \to B_2 \dots$), evaluating trial-level observations ($N = 10$) using primary Paired Student's t-test ($D_i = B_i - A_i$), secondary Welch t-test diagnostic, paired difference bootstrap 95% CI, and project-defined practical significance threshold of 3.0% ($\Delta \ge 3.0\%$).
- **Verification Method:** Run mock benchmark trials and verify test statistics against statistical references.
- **Completion Criteria:** Automatically generates comparative markdown/JSON report indicating whether an optimization produces statistically significant gains.
- **Status:** `COMPLETE`

### `TASK-P02-004`: UDP Jitter & Ping Telemetry Collector
- **Objective:** Implement network latency telemetry tool that measures round-trip time, packet jitter, and packet loss against Riot edge nodes.
- **Files Involved:**
  - `crates/val-opt-core/src/network/probe.rs`
  - `crates/val-opt-core/src/network/mod.rs`
- **Requirements:** High-precision UDP probe transmitter sending time-stamped datagrams and recording return jitter.
- **Verification Method:** Probe local gateway and public edge endpoints; verify jitter and RTT metrics against Wireshark traces.
- **Completion Criteria:** Telemetry module records min/avg/max ping, standard deviation jitter, and packet loss percentage.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_02.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 3.
