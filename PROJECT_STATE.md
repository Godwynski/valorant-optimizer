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
- **Active Phase:** Phase P0 — Security & Vulnerability Remediation (COMPLETE - GATE 0 READY)
- **Remediated Vulnerabilities:**
  1. [RESOLVED] PowerShell Command Injection in `qos.rs`, `adapter.rs`, and `flow_control.rs` (TASK-SEC-01)
  2. [RESOLVED] Insecure `%ProgramData%` permissions (LPE vector) hardened to SYSTEM/Admins full, Users read-only (TASK-SEC-02)
  3. [RESOLVED] Insecure Named Pipe DACL hardened with SDDL, pipe squatting prevented, privileged requests gated (TASK-SEC-03)
  4. [RESOLVED] Cryptographic snapshot integrity upgraded to HMAC-SHA256 with DPAPI machine secret key (TASK-SEC-04)
  5. [RESOLVED] Arbitrary rollback deletion vector eliminated; canonical path and reparse/junction/hardlink restrictions enforced (TASK-SEC-05)

---

## 3. Subsystem Health Matrix (Post-Audit Baseline)

| Subsystem | Audit Status | Identified Issues / Deficiencies | Action Required |
| :--- | :---: | :--- | :--- |
| **Hardware Detection** | `PARTIALLY VALIDATED` | CPU EfficiencyClass inverted (P/E cores swapped); HAGS missing | Fix `cpu.rs` logic; implement real HAGS query |
| **Benchmarking Engine** | `UNVERIFIED` | Zero ETW capture; A/B runner tests against synthetic mock | Implement real ETW tracing; interleaved A/B harness |
| **Safe Optimizations** | `VALIDATED` | Game Mode and Desktop Power Scheme verified on host | Retain safe defaults; fix SYSTEM registry hive |
| **Process Management** | `PARTIALLY VALIDATED` | Supervisor is dead code; app relaunch is dead code; name spoofing | Wire supervisor; implement token-safe app relaunch |
| **Network Engine** | `SECURED` | Command injection eradicated. High-risk tweaks (DSCP 46, Interrupt Moderation auto-disable) await Phase P3 | De-scope DSCP 46; refactor Interrupt Moderation in P3 |
| **Kernel Latency ETW** | `UNVERIFIED` | Zero ETW APIs in code; synthetic simulation generator | Implement real `StartTraceW` for kernel DPC/ISR |
| **Native Slint GUI** | `VALIDATED` | UI renders cleanly; standalone fallback secured against broken pipe | Wire actual game launch trigger; provide minimize option in P1 |
| **Rollback & Safety** | `SECURED` | Cryptographic HMAC-SHA256 integrity active; canonical path & reparse checks active | App relaunch integration in P1 |
| **Vanguard Validation** | `UNVERIFIED` | 100-match verification was a 0.00s mock loop; zero live match validation | Retract 100% compliance claims; test real handles in P4 |
| **Packaging & Release** | `HARDENED` | Inno Setup & install.ps1 ACLs hardened; Inno compiler build in P4 | Portable toolchain in P1; real installer in P4 |

---

## 4. Remediation Phase Progress

| Remediation Milestone | Title | Status | Gate Condition |
| :--- | :--- | :---: | :--- |
| **Phase P0** | Security & Vulnerability Remediation | `COMPLETE (Awaiting Sign-Off)` | Zero command injection, secure ACLs, secure named pipe, HMAC integrity, path restrictions |
| **Phase P1** | Functional Fixes & Dead Code Elimination | `PENDING_REVIEW` | P/E core topology fixed; supervisor & relaunch wired; portable build |
| **Phase P2** | Real Measurement & Benchmarking | `PENDING_REVIEW` | True ETW ingestion; interleaved A/B harness; non-parametric stats |
| **Phase P3** | Optimization De-scoping & Refactoring | `PENDING_REVIEW` | Snake oil removed; high-risk tweaks removed; safe defaults active |
| **Phase P4** | Honest Documentation & Production Packaging | `PENDING_REVIEW` | False claims retracted; real installer built; VM lifecycle tested |

---

## 5. Next Immediate Action
- Await user review and formal authorization of **Gate 0: Security Sign-Off**.
- Do NOT proceed to Phase P1 until user explicitly authorizes.

---

## 6. Execution Rules Reminder
1. Strict sequential progression: `Remediation Phase → Tasks → Verification → Evidence Sign-Off → STOP`.
2. Do not proceed to functional or performance tasks while P0 security vulnerabilities remain open.
3. Code written is not complete until compiled, verified with real tests, and supported by tangible evidence.
4. Stop after each milestone gate and wait for user instruction.
