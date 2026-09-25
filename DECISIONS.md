# Architectural Decision Records (ADR): VALORANT Performance Optimizer

This document records the foundational architectural decisions, justifications, tradeoffs, and compliance boundaries governing this project.

---

## ADR-001: Decoupled 2-Tier Native Architecture (Rust Core + Ephemeral UI)
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  Most gaming "optimizers" are built with Electron (Discord, Riot Client itself, Razer Synapse, etc.). Electron runs Node.js and a full Chromium browser instance, consuming 400 MB–900 MB RAM, spawning 5–8 helper processes, allocating active D3D11 swapchains, and executing unpredictable V8 Mark-Sweep Garbage Collection cycles. These GC sweeps steal CPU slices and evict L3 cache lines during gameplay, degrading 1% and 0.1% lows.
- **Decision:**
  Build a **2-Tier Architecture**:
  1. `val-opt-core`: A native daemon written in **Rust** (using `windows-rs`). Single binary, < 6 MB physical RAM, 0% CPU at idle, zero garbage collection pauses.
  2. `val-opt-gui`: A native desktop UI (Rust + Slint or C++).
  3. **The Ephemeral GUI Rule:** The GUI unloads completely from memory when VALORANT launches. Only the headless native daemon remains active in a passive wait state (`WaitForSingleObject`).
- **Consequences:**
  - *Positive:* Zero memory bloat, zero VRAM fragmentation, zero GC interruptions, deterministic Win32/NT API access.
  - *Negative:* UI cannot use web CSS frameworks; requires native desktop GUI frameworks (Slint/egui).

---

## ADR-002: Strict Riot Vanguard Ring-0 Out-of-Process Compliance
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  Riot Vanguard (`vgk.sys`, `vgc.exe`) enforces kernel-level anti-cheat security. Any tool that injects DLLs, opens handles with `PROCESS_ALL_ACCESS` to the game, modifies game memory, hooks DirectX/Present calls, or disables critical security features (VBS/HVCI, TPM 2.0) risks triggering anti-cheat bans (`VAN 9005`, `VAN 1067`, `VAN 84`, or account suspensions).
- **Decision:**
  The optimizer is strictly an **out-of-process system stabilizer**.
  - **Permitted:** Setting process priority via standard Win32 `SetPriorityClass(HIGH_PRIORITY_CLASS)`, setting core affinity via `SetProcessAffinityMask`, terminating safe user-space bloatware (browsers, Discord), configuring NIC properties via NDIS/CIM, and adjusting Windows power plans.
  - **Prohibited:** Memory reading/writing of `VALORANT-Win64-Shipping.exe`, API hooking, kernel driver installation, tampering with `vgk.sys` or `vgc.exe`, or altering driver signing enforcement.
- **Consequences:**
  - Guarantees 100% Vanguard safety and zero risk of anti-cheat penalties.

---

## ADR-003: Rejection of VBS / HVCI Disabling
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  Legacy gaming optimization guides frequently recommend disabling Virtualization-Based Security (VBS) and Hypervisor-Protected Code Integrity (HVCI / Memory Integrity) to gain 5% CPU throughput.
- **Decision:**
  **Do NOT disable VBS or HVCI.** Modern Riot Vanguard on Windows 11 mandates TPM 2.0, Secure Boot, and VBS/HVCI for competitive match integrity. Disabling VBS/HVCI locks the player out of the game with `VAN: RESTRICTION: 5` or `VAN 9005`.
- **Consequences:**
  - Preserves game eligibility and platform security while avoiding false optimizations.

---

## ADR-004: Rejection of Realtime Process Priority & Standby Memory Purging
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  - Setting process priority to `Realtime` (priority 24) causes user threads to run ahead of Windows kernel threads, mouse input processing, `audiodg.exe`, and Vanguard heartbeats, leading to audio stutter and anti-cheat timeouts.
  - Standby memory purgers (`EmptyStandbyList`) flush cached file blocks from RAM to disk, causing micro-freezes when the game engine demands those assets again.
- **Decision:**
  - Process priority ceiling is strictly enforced at `HIGH_PRIORITY_CLASS` (priority 13).
  - Standby memory flushing is completely omitted; Windows Memory Manager's native page eviction is trusted.
- **Consequences:**
  - Prevents audio crackling, input drops, anti-cheat watchdog timeouts, and storage hitching.

---

## ADR-005: Hardware-Adaptive CPU Affinity Policy
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  Intel 12th/13th/14th Gen processors feature hybrid architecture (P-cores and E-cores). If Windows Thread Director assigns VALORANT rendering threads to E-cores, frame pacing drops significantly. However, on monolithic CPUs (e.g. Intel i5-12400F with 6P/12T, AMD Ryzen 5600X/7800X3D), restricting affinity can choke Unreal Engine 4's task graph.
- **Decision:**
  - The optimizer dynamically queries CPU topology.
  - **Hybrid CPUs (P+E):** Affinity masking is applied to restrict VALORANT strictly to P-cores.
  - **Monolithic CPUs ($\le 6$ physical cores without E-cores):** Affinity masking is NOT enforced by default to preserve the full thread pool for the game engine.
- **Consequences:**
  - Prevents scheduler misallocations without restricting core availability on non-hybrid platforms.

---

## ADR-006: Reversible State & Automatic Orphan Crash Recovery
- **Date:** 2026-09-24
- **Status:** `ACCEPTED`
- **Context:**
  If the computer crashes, loses power, or encounters a BSOD while the optimizer has paused Windows services (e.g. Windows Update, SysMain), the machine could remain permanently altered.
- **Decision:**
  All pre-optimization states are serialized into an atomic disk snapshot (`%ProgramData%\ValorantOptimizer\snapshot_<timestamp>.json`). On system startup, the native core daemon checks for uncommitted snapshots and automatically executes a rollback before resuming idle state.
- **Consequences:**
  - Absolute reversibility and crash immunity.

---

## ADR-007: DPAPI Machine-Bound HMAC-SHA256 Snapshot Integrity Model
- **Date:** 2026-09-25
- **Status:** `ACCEPTED`
- **Context:**
  An unkeyed SHA-256 hash allows standard users or malware to alter serialized snapshot state and recalculate the hash, creating a Local Privilege Escalation (LPE) vector during elevated daemon recovery or CLI rollback.
- **Decision:**
  Enforce machine-bound HMAC-SHA256 snapshot integrity (`docs/DPAPI_SNAPSHOT_SECURITY_MODEL.md`). Secrets are generated using CSPRNG and encrypted via Windows DPAPI with `CRYPTPROTECT_LOCAL_MACHINE` and application entropy. The encrypted key is stored at `%ProgramData%\ValorantOptimizer\snapshot.key` with hardened DACL (`SYSTEM` and `Administrators` only). Any integrity, DPAPI, or cross-machine mismatch fails secure without applying changes.
- **Consequences:**
  - Guarantees tamper-proof snapshot persistence; eliminates LPE vector; bounds recovery strictly to the local physical computer.

