# Phase 7: High-Performance Native UI

---

## 1. Phase Objective
Build the native desktop operator dashboard using **Rust + Slint** (or native C++). The UI provides hardware visualization, one-click profile activation, network diagnostics, and benchmark graphing. Crucially, it implements the **Ephemeral UI pattern**: upon game launch, the GUI completely exits and unloads from RAM, leaving only the zero-overhead daemon active.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-07`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 6 complete.
- **Target Deliverable:** Responsive, GPU-accelerated desktop UI consuming < 25 MB RAM when active and 0 MB during gameplay.
- **QA File:** `QA/QA_PHASE_07.md`

---

## 3. Tasks Breakdown

### `TASK-P07-001`: Native Desktop GUI Skeleton (Rust + Slint)
- **Objective:** Initialize the desktop graphical interface using the Slint framework, compiled to native machine code.
- **Files Involved:**
  - `crates/val-opt-gui/Cargo.toml`
  - `crates/val-opt-gui/src/main.rs`
  - `crates/val-opt-gui/ui/appwindow.slint`
- **Requirements:** Clean, dark-mode competitive FPS theme; zero web technologies or Chromium dependencies; memory footprint < 25 MB.
- **Verification Method:** Launch application; check Task Manager for memory footprint and GPU swapchain allocations.
- **Completion Criteria:** Window renders at native monitor refresh rate with < 25 MB RAM and instant responsiveness.
- **Status:** `COMPLETE`

### `TASK-P07-002`: Named Pipe IPC Client-Server Implementation
- **Objective:** Implement inter-process communication between `val-opt-core` daemon and `val-opt-gui`.
- **Files Involved:**
  - `crates/val-opt-shared/src/ipc/mod.rs`
  - `crates/val-opt-core/src/ipc_server.rs`
  - `crates/val-opt-gui/src/ipc_client.rs`
- **Requirements:** Asynchronous Windows Named Pipe (`\\.\pipe\val_opt_ipc`), structured JSON message framing, automatic reconnection handling.
- **Verification Method:** Send mock scan and optimize commands from GUI; verify daemon receives and responds correctly.
- **Completion Criteria:** Bidirectional IPC operates with < 2ms message round-trip time.
- **Status:** `COMPLETE`

### `TASK-P07-003`: Dashboard, Settings & Diagnostics Views
- **Objective:** Implement GUI views for Hardware Status, Optimization Profile, Network Health, and Benchmark Graphs.
- **Files Involved:**
  - `crates/val-opt-gui/ui/dashboard.slint`
  - `crates/val-opt-gui/ui/settings.slint`
  - `crates/val-opt-gui/ui/benchmarks.slint`
- **Requirements:** Real-time hardware overview, one-click "Launch Optimized" trigger, toggleable tweak categories, visual latency graphs.
- **Verification Method:** Navigate all views and test interactive toggles.
- **Completion Criteria:** Fully functional user interface with all settings linked to IPC commands.
- **Status:** `COMPLETE`

### `TASK-P07-004`: Ephemeral UI Unload & Relaunch Coordinator
- **Objective:** Implement logic to terminate the GUI process completely upon game launch and relaunch it post-game.
- **Files Involved:**
  - `crates/val-opt-gui/src/lifecycle.rs`
  - `crates/val-opt-core/src/process/supervisor.rs`
- **Requirements:** When "Launch Optimized" is triggered, GUI sends handoff message to core daemon and cleanly exits (`std::process::exit(0)`); core daemon optionally respawns GUI when VALORANT exits.
- **Verification Method:** Launch game through GUI; verify `val-opt-gui.exe` is completely absent from Task Manager while VALORANT is running.
- **Completion Criteria:** Zero GUI memory or GPU footprint exists during gameplay.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate (56/56 workspace tests passing).
3. Checklist in `QA/QA_PHASE_07.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 8.
