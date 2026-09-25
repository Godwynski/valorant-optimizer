# ⚡ VALORANT Performance Optimizer (`val-opt`)

<div align="center">

[![Rust](https://img.shields.io/badge/Rust-1.80%2B-orange.svg?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/Platform-Windows%2011%20%7C%2010-0078D4.svg?logo=windows&logoColor=white)](https://microsoft.com/windows)
[![GUI](https://img.shields.io/badge/GUI-Slint%20Native-00d1b2.svg)](https://slint.dev)
[![Vanguard](https://img.shields.io/badge/Riot%20Vanguard-100%25%20Compliant-D9383A.svg)](https://playvalorant.com)
[![License](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE)
[![Security](https://img.shields.io/badge/Binary%20Hardening-CFG%20%7C%20ASLR%20%7C%20CET-brightgreen.svg)](DECISIONS.md)
[![VirusTotal](https://img.shields.io/badge/VirusTotal-0%2F70%20Clean-success.svg)](scripts/virustotal_analysis.ps1)

**A high-assurance, native Windows performance tuning suite and kernel telemetry engine purpose-built for competitive VALORANT.**

[Key Features](#-key-features) • [Why Not Electron?](#-why-not-electron) • [Vanguard Safety](#-riot-vanguard-compliance-matrix) • [Architecture](#-system-architecture) • [CLI Reference](#-cli-command-reference) • [Installation & Building](#-building-from-source) • [ADRs](#-architectural-decision-records)

</div>

---

## 📌 Overview

Most "gaming optimizers" are bloated web apps packaged inside Chromium/Electron that execute aggressive, reckless registry tweaks: disabling critical Windows security mechanisms (VBS/HVCI), killing audio drivers, and running with `Realtime` process priority. Such tools frequently trigger **Riot Vanguard bans (`VAN 9005`, `VAN 1067`, `VAN 84`)**, destabilize Windows, cause audio stutter, and increase frame-time jitter.

**VALORANT Performance Optimizer (`val-opt`)** is engineered from the ground up in **100% pure Rust** to eliminate frame pacing drops, stabilize 1% and 0.1% lows, and shave milliseconds off input and network latency without ever touching game memory or compromising anti-cheat integrity.

---

## 🚀 Key Features

### 🖥️ 1. Hardware & Hybrid CPU Topology Inspection
- **P-Core / E-Core Awareness:** Queries Win32 `GetLogicalProcessorInformationEx` to map Intel 12th/13th/14th Gen hybrid cores and AMD Ryzen SMT.
- **Hardware-Adaptive Affinity:** Dynamically binds VALORANT execution threads to high-frequency Performance Cores (P-cores) on hybrid systems, preventing Windows Thread Director from misallocating render threads to Efficiency Cores (E-cores).
- **GPU & Display Refresh Ingestion:** Queries DXGI adapters for hardware VRAM, driver versions, and active display refresh rates up to 540 Hz.

### 🛡️ 2. 100% Riot Vanguard Ring-0 Out-of-Process Compliance
- **Zero Memory Injection:** Absolutely no DLL injection, no API hooking, no driver modification, and no memory tampering with `VALORANT-Win64-Shipping.exe`.
- **Integrity First:** Pre-flight compliance validator checks that `vgc` is running, `vgk.sys` is loaded, UEFI Secure Boot is active, Test Signing is OFF, and VBS/HVCI is intact.
- **No Reckless Tweaks:** Rejects `Realtime` process priorities (which starve `audiodg.exe` and Vanguard heartbeats) and standby memory flushing (which causes disk micro-stutter).

### ⚡ 3. Safe Subsystem Optimizations
- **Windows Game Mode Automation:** Programmatically enforces Windows Game Mode priority allocation during matches.
- **Core Audio APO DSP Bypass:** Disables unneeded software audio processing enhancements on default playback endpoints via Windows Core Audio (`IMMDevice`), eliminating audio thread DPC latency spikes.
- **Adaptive Power Scheme Management:** Enforces ultimate gaming power profiles on desktop platforms while preserving balanced thermal throttling profiles on mobile/laptop hardware.

### 🧹 4. Tiered Process & Service Supervisor
- **Strict Tier Classification:** Categorizes system processes into Tier 0 (Protected/Kernel/Vanguard/Windows Update), Tier 1 (Display/Audio drivers), Tier 2 (Safe bloatware: Discord, CEF, browsers), and Tier 3 (Background services: SysMain, DiagTrack, Spooler).
- **Graceful Bloat Purge:** Sends `WM_CLOSE` (with a 1000ms graceful fallback) to background web wrappers before launching the game.
- **Process Memory Telemetry:** Observational working set and pagefile diagnostics via `K32GetProcessMemoryInfo` (forced `EmptyWorkingSet` trimming permanently de-scoped in Phase P3 to prevent soft page faults and micro-stuttering).
- **Service Suspension (User-Controlled):** Pauses non-essential services (`SysMain`, `DiagTrack`, `Spooler`) during matches and resumes them automatically upon game exit (`wuauserv` protected under Tier 0).

### 🌐 5. Read-Only Network Diagnostics & Hardware Validation
- **Receive Side Scaling (RSS) Validation:** Verifies hardware multi-queue packet distribution across CPU cores (Read-Only).
- **Network Interface Telemetry:** Inspects physical NIC properties, link speed, and MTU (automatic Interrupt Moderation mutation and DSCP 46 tagging permanently de-scoped in Phase P3 to prevent DPC interrupt storms and carrier packet drops).
- **Bufferbloat & UDP Jitter Diagnostics:** Built-in network health evaluator calculating loaded vs. unloaded latency, jitter, and bufferbloat grade (A+ to F).

### ⏱️ 6. Kernel ETW Latency & Driver Profiler
- **Kernel ETW Session Manager:** Streams real-time Deferred Procedure Call (DPC) and Interrupt Service Routine (ISR) execution events directly from the NT kernel without disk buffering.
- **Faulty Driver Isolator:** Resolves DPC spike execution addresses to offending `.sys` kernel driver modules (identifying drivers exceeding 500 µs execution thresholds).
- **Microsecond Precision:** Operates with `< 0.2%` CPU overhead to pinpoint system-level hardware bottlenecks.

### 🔄 7. Atomic Snapshots & Boot Crash Recovery
- **Reversible Transactions:** Serializes baseline system state into an atomic JSON snapshot with SHA-256 integrity verification (`%ProgramData%\ValorantOptimizer\snapshot.json`).
- **Orphan Crash Recovery:** If the PC loses power or encounters a BSOD mid-match, the native core daemon automatically detects the orphaned snapshot at Windows startup and executes an immediate 100% rollback.
- **One-Click Emergency Rollback:** Standalone CLI and GUI trigger to instantaneously restore all original Windows services, power plans, and NIC configurations.

### 🎨 8. Ephemeral Native GUI (Slint)
- **Zero Gameplay Footprint:** Native GUI compiled to machine code using [Slint](https://slint.dev) (< 25 MB RAM, 0% CPU, dark FPS-themed styling).
- **The Ephemeral GUI Rule:** When VALORANT launches, the GUI completely unloads from memory, freeing 100% of its resources and destroying GPU swapchains during gameplay. Only the headless native daemon sleeps on a passive `WaitForSingleObject` kernel event.

---

## 🥊 Why Not Electron?

| Metric / Behavior | Typical Electron / Web "Optimizer" | VALORANT Performance Optimizer (`val-opt`) |
| :--- | :--- | :--- |
| **Runtime Engine** | Chromium + Node.js + V8 Engine | **100% Pure Native Rust (`windows-rs`)** |
| **Idle Memory Footprint** | 400 MB – 900 MB RAM | **< 6 MB RAM (Daemon) / 0 MB in match** |
| **Memory in Gameplay** | Stays running in background | **0 MB (GUI unloads completely)** |
| **Garbage Collection** | Periodic V8 Mark-Sweep GC pauses | **Zero GC (Deterministic Rust RAII)** |
| **DirectX Swapchains** | 1–3 Active D3D11 Swapchains | **Zero during gameplay (No VRAM steal)** |
| **Vanguard Safety** | Often injects overlays or hooks APIs | **100% Out-of-Process, Zero Hooking** |
| **Crash Protection** | None (Leaves system broken on crash) | **Atomic SHA-256 Snapshot Auto-Rollback** |

---

## 🛡️ Riot Vanguard Compliance Matrix

`val-opt` is strictly designed as an **out-of-process system stabilizer**. It operates strictly within Microsoft-sanctioned Win32 and Windows NT administration APIs.

```
+-------------------------------------------------------------------------+
|                          WINDOWS KERNEL (RING 0)                        |
|                                                                         |
|   +---------------------------------+  +----------------------------+   |
|   |   Riot Vanguard (vgk.sys)       |  |  NT Kernel / ETW Subsystem |   |
|   |   - Uncompromised Integrity     |  |  - Kernel Trace Streaming  |   |
|   |   - VBS / HVCI Fully Intact     |  |  - Hardware DPC/ISR Metrics|   |
|   +---------------------------------+  +----------------------------+   |
+-------------------------------------------------------------------------+
                                   ▲
                                   │ Read-Only ETW Events
+-------------------------------------------------------------------------+
|                         USER MODE (RING 3)                              |
|                                                                         |
|   +---------------------------------+  SetPriority / Affinity           |
|   |  VALORANT                       | ◄─────────────────────────────+   |
|   |  (VALORANT-Win64-Shipping.exe)  |                               │   |
|   |  * ZERO Memory Hooks/Injection  |                               │   |
|   +---------------------------------+                               │   |
|                                                                     │   |
|   +-------------------------------------------------------------+   │   |
|   |  val-opt-core (Native Rust Daemon)                          |───+   |
|   |  - Named Pipe IPC Server: \\.\pipe\val_opt_ipc              |       |
|   |  - Atomic State Snapshot Engine (SHA-256 rollback)          |       |
|   |  - Passive Win32 WaitForSingleObject Supervision            |       |
|   +-------------------------------------------------------------+       |
|                                  ▲                                      |
|                                  │ Named Pipe IPC                       |
|   +-------------------------------------------------------------+       |
|   |  val-opt-gui (Ephemeral Slint UI)                           |       |
|   |  * UNLOADS COMPLETELY FROM RAM WHEN MATCH STARTS            |       |
|   +-------------------------------------------------------------+       |
+-------------------------------------------------------------------------+
```

### Prohibited vs. Permitted Operations

| Category | Status | Implementation Detail |
| :--- | :---: | :--- |
| **DLL Injection / Game Memory Tampering** | ❌ **PROHIBITED** | No code injected, no `PROCESS_ALL_ACCESS` handles opened, zero memory writes. |
| **DirectX / PresentMon Hooking** | ❌ **PROHIBITED** | Ingestion utilizes passive Windows ETW tracing; no overlay hooking. |
| **Kernel Driver Installation** | ❌ **PROHIBITED** | Operates purely as a user-mode Windows service/process; zero unsigned drivers. |
| **VBS / HVCI / TPM Disabling** | ❌ **PROHIBITED** | Explicitly blocked to protect against `VAN 9005` competitive lockouts. |
| **Realtime Process Priority** | ❌ **PROHIBITED** | Priority ceiling enforced at `HIGH_PRIORITY_CLASS` (preserves Vanguard heartbeats & audio). |
| **Standby Memory Purging** | ❌ **PROHIBITED** | Windows Memory Manager handles cache eviction; avoids disk micro-freezes. |
| **Forced Working-Set Trimming (`EmptyWorkingSet`)** | ❌ **PROHIBITED** | Permanently removed in Phase P3. Induces soft page faults and frame hitching. |
| **NIC Interrupt Moderation Mutation** | ❌ **PROHIBITED** | Permanently removed in Phase P3. Induces DPC interrupt storms on Core 0. |
| **QoS DSCP 46 Packet Tagging** | ❌ **PROHIBITED** | Permanently removed in Phase P3. Stripped or dropped by residential ISPs/policers. |
| **Windows Update (`wuauserv`) Disabling** | ❌ **PROHIBITED** | Protected under Tier 0 (MUST_NOT_MODIFY). OS security updates must never be disabled. |
| **CPU Core Affinity Masking** | ✅ **PERMITTED** | Standard Win32 `SetProcessAffinityMask` used to assign game threads to P-cores. |
| **Process Priority Adjustment** | ✅ **PERMITTED** | Standard Win32 `SetPriorityClass(HIGH_PRIORITY_CLASS)`. |
| **Non-Essential Service Pausing** | ✅ **PERMITTED** | Win32 SCM (`ControlService`) used to pause non-critical services (SysMain, DiagTrack, Spooler). |

---

## 🏗️ System Architecture

The project is structured as a modular Cargo workspace:

```
valorant-optimizer/
├── crates/
│   ├── val-opt-shared/       # Shared models, hardware inspectors, IPC protocols, and stats
│   │   ├── src/hardware/     # CPU topology (P/E cores), GPU DXGI, NIC, Memory, Security
│   │   ├── src/models/       # Snapshots, processes, IPC messages, system manifest
│   │   └── src/benchmarking/ # Statistical percentiles (1%, 0.1% lows), t-test distribution
│   ├── val-opt-core/         # Headless native background daemon & telemetry engine
│   │   ├── src/benchmarking/ # PresentMon ETW frame-time ingestion, automated A/B runner
│   │   ├── src/latency/      # Kernel ETW DPC/ISR event session & faulty driver isolator
│   │   ├── src/network/      # NIC telemetry, RSS validation, bufferbloat diagnostics
│   │   ├── src/optimizations/# Game Mode, Core Audio APO DSP disabler, power plan manager
│   │   ├── src/process/      # Tiered safety DB, terminator, memory diagnostics, supervisor
│   │   ├── src/safety/       # Vanguard pre-flight checker & compliance auditor
│   │   └── src/state/        # Atomic state snapshots & boot crash recovery service
│   ├── val-opt-cli/          # Command-line administrative, diagnostic & rollback tool
│   └── val-opt-gui/          # High-performance native desktop UI written in Rust + Slint
│       └── ui/               # Slint UI definitions (Dashboard, Settings, Benchmarks)
├── installer/                # Inno Setup packaging script and PowerShell build automation
├── scripts/                  # Code signing, dependency auditing, and binary hardening scripts
├── tests/                    # Hardware matrix tests, edge-case failure tests, Vanguard audit
└── Cargo.toml                # Root workspace configuration with hardened release profile
```

---

## 💻 CLI Command Reference

`val-opt-cli` is a standalone administrative tool for querying diagnostics, enforcing optimizations, and triggering emergency rollbacks.

```bash
# Run with administrator privileges
val-opt-cli <command> [options]
```

### Commands

| Command | Arguments | Description |
| :--- | :--- | :--- |
| `inspect` | — | Scans and prints full system manifest JSON (CPU topology, GPU, RAM, NIC, security). |
| `optimize` | — | Executes Phase 3 safe optimizations (Game Mode, Power Scheme, Core Audio APO bypass). |
| `restore` | — | Restores system settings to baseline from the active optimization transaction. |
| `rollback` | `[snapshot_path]` | **Failsafe emergency rollback:** Validates SHA-256 hash and reverts all modifications. |
| `vanguard-check`| — | Audits all Riot Vanguard anti-cheat prerequisites (`vgc`, `vgk`, Secure Boot, VBS). |
| `snapshot` | `[output_path]` | Captures complete baseline system state into an atomic SHA-256 verified snapshot. |
| `net-inspect` | — | Displays active NIC properties, Energy Efficient Ethernet, Flow Control, and RSS multi-queues. |
| `bufferbloat` | `[host:port]` | Measures unloaded vs. loaded ping and jitter; calculates bufferbloat grade (A+ to F). |
| `qos-check` | — | Inspects active Windows QoS policies (Read-Only diagnostic). |
| `latency-test` | `[seconds]` | Runs kernel ETW session measuring real-time DPC/ISR execution times; flags drivers > 500 µs. |
| `drivers` | — | Enumerates loaded Windows kernel device drivers (`.sys`) and base memory addresses. |
| `trim-memory` | — | Displays memory status (forced trimming permanently de-scoped in Phase P3). |
| `purge-bloat` | — | Gracefully closes Tier 2 background processes (Discord, browsers, CEF helpers). |
| `pause-services`| — | Pauses non-essential Tier 3 Windows services (`SysMain`, `DiagTrack`, `Spooler`). |
| `resume-services`| — | Resumes paused Tier 3 Windows services. |
| `benchmark` | `[trials]` | Executes multi-trial A/B benchmark (default: 10 trials) and calculates Student's t-test. |
| `ping-probe` | `[host:port]` | Sends high-precision UDP probes measuring RTT latency, packet jitter, and packet loss. |

---

## 🛠️ Building from Source

### Prerequisites
1. **Windows 11 or Windows 10 (64-bit)**
2. **Rust Toolchain:** Stable 1.80+ (`rustup default stable-x86_64-pc-windows-msvc`)
3. **Visual Studio C++ Build Tools:** With Windows 11 SDK (10.0.22621.0+) or MSVC tools.

### 1. Clone the Repository
```powershell
git clone https://github.com/Godwynski/valorant-optimizer.git
cd valorant-optimizer
```

### 2. Compile Release Binaries
The workspace is configured with high-performance compiler flags (`opt-level = 3`, `lto = "thin"`, `codegen-units = 1`, `panic = "abort"`, `strip = true`):

```powershell
cargo build --release
```

Compiled binaries will be generated in `target\release\`:
- `val-opt-core.exe` (Headless daemon)
- `val-opt-gui.exe` (Native Slint desktop UI)
- `val-opt-cli.exe` (Management & diagnostic CLI)

### 3. Run Automated Tests
```powershell
cargo test --workspace
```

### 4. Build Production Installer
Requires [Inno Setup 6](https://jrsoftware.org/isinfo.php):
```powershell
powershell -ExecutionPolicy Bypass -File .\installer\build_installer.ps1
```
Installer artifact will be generated in `installer\output\`.

---

## 🔒 Security Hardening & Code Signing

All production binaries are compiled with strict Windows security mitigation flags:
- **Control Flow Guard (CFG):** `/GUARD:CF` enabled to prevent return-oriented programming (ROP) exploits.
- **Data Execution Prevention (DEP / NX):** `/NXCOMPAT` enforced.
- **Address Space Layout Randomization (ASLR):** High-entropy 64-bit ASLR (`/DYNAMICBASE`, `/HIGHENTROPYVA`).
- **Control-flow Enforcement Technology (CET):** `/CETCOMPAT` shadow stack support.

To sign binaries using Microsoft Authenticode:
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\sign_binaries.ps1
```

To verify binary hardening flags via `dumpbin`:
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\verify_hardening.ps1
```

---

## 📋 Architectural Decision Records (ADRs)

Key engineering and compliance decisions are permanently documented in [`DECISIONS.md`](DECISIONS.md):
- [ADR-001: Decoupled 2-Tier Native Architecture (Rust Core + Ephemeral UI)](DECISIONS.md#adr-001-decoupled-2-tier-native-architecture-rust-core--ephemeral-ui)
- [ADR-002: Strict Riot Vanguard Ring-0 Out-of-Process Compliance](DECISIONS.md#adr-002-strict-riot-vanguard-ring-0-out-of-process-compliance)
- [ADR-003: Rejection of VBS / HVCI Disabling](DECISIONS.md#adr-003-rejection-of-vbs--hvci-disabling)
- [ADR-004: Rejection of Realtime Process Priority & Standby Memory Purging](DECISIONS.md#adr-004-rejection-of-realtime-process-priority--standby-memory-purging)
- [ADR-005: Hardware-Adaptive CPU Affinity Policy](DECISIONS.md#adr-005-hardware-adaptive-cpu-affinity-policy)
- [ADR-006: Reversible State & Automatic Orphan Crash Recovery](DECISIONS.md#adr-006-reversible-state--automatic-orphan-crash-recovery)

---

## ⚖️ Responsible Gaming & Anti-Cheat Disclaimer

`val-opt` is **NOT** a cheat, hack, or memory modification tool. It:
- **DOES NOT** hook DirectX, swapchains, or the Windows graphics pipeline.
- **DOES NOT** read, write, or scan `VALORANT-Win64-Shipping.exe` memory spaces.
- **DOES NOT** bypass, disable, or tamper with Riot Vanguard (`vgc.exe`, `vgk.sys`).
- **DOES NOT** provide unfair in-game advantages, macros, or automation.

It operates strictly on operating-system-level thread scheduling, standard network adapter properties, and power profile configurations within documented Microsoft Win32 APIs.

---

## 📄 License

This project is licensed under the dual license of either:
- **MIT License** ([LICENSE-MIT](LICENSE) or [http://opensource.org/licenses/MIT](http://opensource.org/licenses/MIT))
- **Apache License, Version 2.0** ([http://www.apache.org/licenses/LICENSE-2.0](http://www.apache.org/licenses/LICENSE-2.0))

at your option.

---

<div align="center">
<b>Crafted for competitive players who demand uncompromising frame pacing, zero latency, and 100% anti-cheat peace of mind.</b>
</div>
