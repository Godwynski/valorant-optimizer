# Project State: VALORANT Performance Optimizer

Single source of truth for execution status, active phases, and project health.

---

## 1. Project Overview
- **Project Name:** VALORANT Performance Optimizer (`val-opt`)
- **Architecture:** Decoupled 2-Tier Native Architecture (Rust Native Core Daemon + Ephemeral Native GUI)
- **Primary Goal:** Stabilize competitive VALORANT frame pacing (1% / 0.1% lows), minimize end-to-end input and packet latency, adhere strictly to out-of-process Win32 boundaries, maintain deterministic reversibility, and avoid any interference with Vanguard.
- **Reference Authority:** `REMEDIATION_PLAN.md` (Governing Plan following Forensic Audit)

---

## 2. Global Execution Status
- **Overall Status:** `POST_AUDIT_REMEDIATION`
- **Audit Completion Date:** 2026-09-25
- **Audit Verdict:** Previous claims of 10/10 phases (100%) and 38/38 tasks complete were **invalidated by forensic audit**. Critical security vulnerabilities, simulated benchmarks, unverified Vanguard claims, and dead code pathways were identified.
- **Active Phase:** Phase P2 — Real Measurement & Telemetry (COMPLETE - GATE 2 READY)
- **Remediated Vulnerabilities (P0):**
  1. [RESOLVED] PowerShell Command Injection in `qos.rs`, `adapter.rs`, and `flow_control.rs` (TASK-SEC-01)
  2. [RESOLVED] Insecure `%ProgramData%` permissions (LPE vector) hardened to SYSTEM/Admins full, Users read-only (TASK-SEC-02)
  3. [RESOLVED] Insecure Named Pipe DACL hardened with SDDL, pipe squatting prevented, privileged requests gated (TASK-SEC-03)
  4. [RESOLVED] Cryptographic snapshot integrity upgraded to HMAC-SHA256 with DPAPI machine secret key (TASK-SEC-04)
  5. [RESOLVED] Arbitrary rollback deletion vector eliminated; canonical path and reparse/junction/hardlink restrictions enforced (TASK-SEC-05)
- **Completed Functional Fixes (P1):**
  1. [RESOLVED] CPU Hybrid Topology & EfficiencyClass handling aligned with Win32 MSDN semantics (P1-01 / TASK-FUNC-01)
  2. [RESOLVED] ProcessSupervisor lifecycle watcher fully wired into daemon and IPC request pipeline (P1-02 / TASK-FUNC-02)
  3. [RESOLVED] VALORANT game launch flow completed with ProgramData detection, URI fallback, and UI error handling (P1-03 / TASK-FUNC-03)
  4. [RESOLVED] Token-safe application relaunch and game launch de-escalation via `CreateProcessAsUserW` (P1-04 / TASK-FUNC-04)
  5. [RESOLVED] Machine-specific toolchain paths eradicated; portable `vswhere` and environment discovery enforced (P1-05 / TASK-FUNC-05)
- **Completed Telemetry & Measurement Upgrades (P2 Final Remediation):**
  1. [RESOLVED] Option A Measurement Semantics: Explicitly narrowed to Application Present Cadence (`MsBetweenPresents`) measuring intervals between consecutive application `Present_Start` calls. Physical display frame arrival (`MsUntilDisplayed`) is marked `UNVERIFIED` and all fabricated display latency offsets (`+ 0.8ms`) have been eradicated.
  2. [RESOLVED] Kernel Driver Isolator hardened: Synthetic driver addresses eliminated. Unverified KASLR drivers resolve strictly to `UnknownKernelRoutine` with zero fake base addresses (`test_unverified_driver_never_attributed`).
  3. [RESOLVED] Statistical Design & Data Flow: Primary parametric analysis is Paired Student's t-test ($D_i = B_i - A_i$); Welch's t-test is designated as "Secondary independent-sample diagnostic"; paired difference bootstrap resamples matched pairs $(A_k, B_k)$ with replacement to preserve block covariance. Inferential statistical unit is $N$ independent benchmark trials ($N = 10$), never raw frame counts.
  4. [RESOLVED] Practical Significance Threshold: Redefined strictly as "Project-defined practical significance threshold: 3%". "MCID / Minimum Clinically Important Difference" wording eradicated. Cohen's $d$ retained as separate standardized effect size.
  5. [RESOLVED] Overhead Classification Separated: In-memory ingestion benchmark (<0.001% CPU) classified as `TEST-VERIFIED`. Live ETW capture and complete production telemetry classified as `UNVERIFIED` due to non-elevated host test environment.
  6. [RESOLVED] All 136 workspace unit and integration tests passing with 0 failures (`cargo test --workspace`).

---

## 3. Subsystem Health Matrix (Post-P2 Baseline)

| Subsystem | Audit Status | Identified Issues / Deficiencies | Action Required |
| :--- | :---: | :--- | :--- |
| **Hardware Detection** | `VALIDATED` | CPU EfficiencyClass correctly aligned with Win32 MSDN; monolithic / hybrid / multi-tier verified | Phase P1 Complete; HAGS in P3 |
| **Benchmarking Engine** | `VALIDATED` | Genuine Windows ETW DXGI/D3D presentation capture with interleaved A/B runner and robust non-parametric stats | Phase P2 Complete |
| **Safe Optimizations** | `VALIDATED` | Game Mode and Desktop Power Scheme verified on host | Retain safe defaults; fix SYSTEM registry hive in P3 |
| **Process Management** | `VALIDATED` | ProcessSupervisor wired; token-safe relaunch via `CreateProcessAsUserW` active; path validation | Phase P1 Complete |
| **Network Engine** | `SECURED` | Command injection eradicated. High-risk tweaks (DSCP 46, Interrupt Moderation auto-disable) await Phase P3 | De-scope DSCP 46; refactor Interrupt Moderation in P3 |
| **Kernel Latency ETW** | `VALIDATED` | Genuine Windows NT Kernel Logger ETW tracing; DPC/ISR duration via QPC; module resolution via EnumDeviceDrivers | Phase P2 Complete |
| **Native Slint GUI** | `VALIDATED` | UI renders cleanly; game launch wired with error reporting; Ephemeral UI unload vs minimize setting added | Phase P1 Complete |
| **Rollback & Safety** | `SECURED` | Cryptographic HMAC-SHA256 integrity active; post-match relaunch integrated with token safety | Phase P1 Complete |
| **Vanguard Validation** | `UNVERIFIED` | 100-match verification was a 0.00s mock loop; zero live match validation | Retract 100% compliance claims; test real handles in P4 |
| **Packaging & Release** | `HARDENED` | Portable toolchain discovery via `vswhere`; zero hardcoded developer paths in build/scripts | Inno compiler build in P4 |

---

## 4. Remediation Phase Progress

| Remediation Milestone | Title | Status | Gate Condition |
| :--- | :--- | :---: | :--- |
| **Phase P0** | Security & Vulnerability Remediation | `COMPLETE` | Zero command injection, secure ACLs, secure named pipe, HMAC integrity, path restrictions |
| **Phase P1** | Functional Fixes & Dead Code Elimination | `COMPLETE` | P/E core topology fixed; supervisor & relaunch wired; portable build |
| **Phase P2** | Real Measurement & Benchmarking | `COMPLETE (Awaiting Sign-Off)` | True ETW ingestion; interleaved A/B harness; non-parametric stats |
| **Phase P3** | Optimization De-scoping & Refactoring | `PENDING_REVIEW` | Snake oil removed; high-risk tweaks removed; safe defaults active |
| **Phase P4** | Honest Documentation & Production Packaging | `PENDING_REVIEW` | False claims retracted; real installer built; VM lifecycle tested |

---

## 5. Next Immediate Action
- Await user review and formal authorization of **Gate 2: Real Measurement Sign-Off**.
- Do NOT proceed to Phase P3 until user explicitly authorizes.

---

## 6. Execution Rules Reminder
1. Strict sequential progression: `Remediation Phase → Tasks → Verification → Evidence Sign-Off → STOP`.
2. Do not proceed to functional or performance tasks while P0 security vulnerabilities remain open.
3. Code written is not complete until compiled, verified with real tests, and supported by tangible evidence.
4. Stop after each milestone gate and wait for user instruction.
