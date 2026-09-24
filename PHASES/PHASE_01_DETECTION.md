# Phase 1: Hardware & Subsystem Detection

---

## 1. Phase Objective
Build the foundational system inspection engine (`HardwareInspector`) to programmatically discover CPU topology, GPU architecture, RAM speed, active NIC properties, and Windows 11 / Vanguard security states. This guarantees the optimizer will adapt dynamically rather than applying blind, hardcoded tweaks.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-01`
- **Status:** `COMPLETE`
- **Dependencies:** None
- **Target Deliverable:** CLI inspection tool generating comprehensive, structured JSON system manifests in < 150ms.
- **QA File:** `QA/QA_PHASE_01.md`

---

## 3. Tasks Breakdown

### `TASK-P01-001`: Project Repository Initialization & Rust Workspace Setup
- **Objective:** Initialize the project repository with a modular Rust workspace (`val-opt-core` daemon, `val-opt-shared` library, and `val-opt-cli`).
- **Files Involved:**
  - `Cargo.toml` (root workspace file)
  - `crates/val-opt-shared/Cargo.toml`
  - `crates/val-opt-core/Cargo.toml`
  - `crates/val-opt-cli/Cargo.toml`
- **Requirements:**
  - Configure root virtual manifest `[workspace]`.
  - Add core dependencies: `windows` (with features for System, Hardware, Process, Security), `serde`, `serde_json`, `tracing`, `tracing-subscriber`.
  - Ensure MSVC static runtime compilation flags.
- **Verification Method:** Run `cargo check --workspace` and `cargo test --workspace`.
- **Completion Criteria:** All crates compile cleanly with zero warnings.
- **Status:** `COMPLETE`

### `TASK-P01-002`: CPU Topology & Instruction Inspection
- **Objective:** Implement CPU topology detection module capable of distinguishing Hybrid architectures (P-cores vs E-cores) and Hyperthreading/SMT.
- **Files Involved:**
  - `crates/val-opt-shared/src/hardware/cpu.rs`
  - `crates/val-opt-shared/src/hardware/mod.rs`
- **Requirements:**
  - Call Win32 `GetLogicalProcessorInformationEx` (`RelationProcessorCore`).
  - Extract `EfficiencyClass` (Class 0 = P-core, Class 1 = E-core on Intel Alder/Raptor Lake).
  - Count physical cores, logical processors, and L3 cache size.
  - Query CPU brand string and instruction capabilities (AVX2, FMA).
- **Verification Method:** Execute unit test verifying CPU model and core counts against Windows Task Manager.
- **Completion Criteria:** Structured CPU manifest accurately reflecting host CPU architecture.
- **Status:** `COMPLETE`

### `TASK-P01-003`: GPU Architecture, Driver & Display Inspector
- **Objective:** Implement GPU detection module for vendor, dedicated VRAM, driver version, HAGS support, and display refresh rates.
- **Files Involved:**
  - `crates/val-opt-shared/src/hardware/gpu.rs`
  - `crates/val-opt-shared/src/hardware/display.rs`
- **Requirements:**
  - Query DXGI adapter description for GPU vendor ID, device ID, and dedicated video memory.
  - Query registry/WMI for installed driver version.
  - Call Win32 `EnumDisplaySettingsW` to retrieve active resolution, color depth, and refresh rate (Hz).
- **Verification Method:** Unit test comparing detected GPU model (e.g. AMD Radeon RX 580) and refresh rate against display settings.
- **Completion Criteria:** Accurately populates GPU manifest and display refresh rate without external dependencies.
- **Status:** `COMPLETE`

### `TASK-P01-004`: Subsystem, RAM, NIC & Security Feature Inspector
- **Objective:** Inspect physical RAM channels/speed, active network adapter (Ethernet/Wi-Fi), and Windows 11 security state (Vanguard, VBS, HVCI, Secure Boot, Game Mode).
- **Files Involved:**
  - `crates/val-opt-shared/src/hardware/memory.rs`
  - `crates/val-opt-shared/src/hardware/network.rs`
  - `crates/val-opt-shared/src/hardware/security.rs`
- **Requirements:**
  - Query `GlobalMemoryStatusEx` for installed RAM.
  - Call IP Helper API (`GetAdaptersAddresses`) to identify active network interface, connection speed, and adapter name.
  - Query WMI `Win32_DeviceGuard` to verify VBS and HVCI (Memory Integrity) status.
  - Query Windows Service Control Manager to verify `vgc` (Vanguard) running state.
  - Query Game Mode registry state (`HKCU\Software\Microsoft\GameBar`).
- **Verification Method:** Run detection CLI tool; verify output matches `msinfo32` and Vanguard system tray state.
- **Completion Criteria:** Full system inspection JSON generated in under 150ms.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_01.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 2.
