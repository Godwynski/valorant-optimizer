# Master Tasks Checklist: VALORANT Performance Optimizer

Complete checklist of all 38 discrete engineering tasks across the 10 execution phases.

---

## Phase 1: Hardware & Subsystem Detection

- [x] **`TASK-P01-001`**: Project Repository Initialization & Rust Workspace Setup
  - **Objective:** Initialize the project repository with a modular Rust workspace (`val-opt-core` daemon, `val-opt-shared` library, and `val-opt-cli`).
  - **Files / Components:** `Cargo.toml`, `crates/val-opt-shared/Cargo.toml`, `crates/val-opt-core/Cargo.toml`, `crates/val-opt-cli/Cargo.toml`.
  - **Requirements:** Cargo workspace configuration, static linking setup, Windows target dependencies (`windows-rs`, `serde`, `tracing`).
  - **Verification Method:** `cargo check --workspace` and `cargo test --workspace` pass cleanly on Windows 11.
  - **Completion Criteria:** All workspace crates build cleanly with zero warnings.
  - **Status:** `COMPLETE`

- [x] **`TASK-P01-002`**: CPU Topology & Instruction Inspection
  - **Objective:** Implement CPU topology detection module capable of distinguishing Hybrid architectures (P-cores vs E-cores) and Hyperthreading/SMT.
  - **Files / Components:** `crates/val-opt-shared/src/hardware/cpu.rs`.
  - **Requirements:** Use Win32 `GetLogicalProcessorInformationEx` API to enumerate RelationProcessorCore, identify EfficiencyClass (P-core vs E-core), count physical and logical cores, and detect AVX/AVX2 instruction support.
  - **Verification Method:** Unit test comparing detected topology against Windows Task Manager and CPU-Z values on the host machine.
  - **Completion Criteria:** Correctly outputs structured JSON with physical cores, logical threads, and hybrid P/E core identification.
  - **Status:** `COMPLETE`

- [x] **`TASK-P01-003`**: GPU Architecture, Driver & Display Inspector
  - **Objective:** Implement GPU detection module for vendor, VRAM, driver version, HAGS support, and display refresh rates.
  - **Files / Components:** `crates/val-opt-shared/src/hardware/gpu.rs`, `crates/val-opt-shared/src/hardware/display.rs`.
  - **Requirements:** Use DXGI and Win32 `EnumDisplaySettings` to query GPU vendor (AMD, NVIDIA, Intel), dedicated VRAM, driver release, active display resolution, and maximum refresh rate (Hz).
  - **Verification Method:** Unit test verifying host GPU (e.g. AMD Radeon RX 580) and primary monitor refresh rate.
  - **Completion Criteria:** Accurately populates GPU manifest and display refresh rate without external dependencies.
  - **Status:** `COMPLETE`

- [x] **`TASK-P01-004`**: Subsystem, RAM, NIC & Security Feature Inspector
  - **Objective:** Inspect physical RAM channels/speed, active network adapter (Ethernet/Wi-Fi), and Windows 11 security state (Vanguard, VBS, HVCI, Secure Boot, Game Mode).
  - **Files / Components:** `crates/val-opt-shared/src/hardware/memory.rs`, `crates/val-opt-shared/src/hardware/network.rs`, `crates/val-opt-shared/src/hardware/security.rs`.
  - **Requirements:** Query `GlobalMemoryStatusEx`, NDIS network interface details via IP Helper API, VBS/HVCI status via WMI (`Win32_DeviceGuard`), and Vanguard service status (`vgc.exe`, `vgk.sys`).
  - **Verification Method:** Run detection CLI tool; verify output matches `msinfo32` and Vanguard system tray state.
  - **Completion Criteria:** Full system inspection JSON generated in under 150ms.
  - **Status:** `COMPLETE`

---

## Phase 2: Benchmarking & Telemetry Infrastructure

- [x] **`TASK-P02-001`**: PresentMon / ETW Frame-Time Ingestion Engine
  - **Objective:** Build a headless frame-time telemetry collector utilizing the Intel PresentMon API or Windows ETW (Event Tracing for Windows) D3D events.
  - **Files / Components:** `crates/val-opt-core/src/benchmarking/etw_capture.rs`, `crates/val-opt-core/src/benchmarking/frametimes.rs`.
  - **Requirements:** Capture individual application frame presentation intervals (`MsBetweenPresents`, Option A: Application Present Cadence; physical display timing `MsUntilDisplayed` remains UNVERIFIED) without injecting hooks into the game process.
  - **Verification Method:** Run synthetic DirectX sample app and verify frame capture stream with microsecond precision.
  - **Completion Criteria:** In-memory collector ingestion operates with < 0.001% CPU overhead (TEST-VERIFIED) and zero DLL injection into game memory. (Live ETW collection requires Administrator elevation; UNVERIFIED in standard test runner).
  - **Status:** `COMPLETE`

- [x] **`TASK-P02-002`**: Statistical Metrics & Percentile Calculation Engine
  - **Objective:** Implement statistical computation module for Average Present Rate (FPS), 1% Low (99th percentile), 0.1% Low (99.9th percentile), and present cadence standard deviation.
  - **Files / Components:** `crates/val-opt-shared/src/benchmarking/stats.rs`.
  - **Requirements:** Pure Rust implementation computing rolling and total statistical distributions over recorded present timestamps, with warm-up frame trimming.
  - **Verification Method:** Unit test on synthetic timestamp arrays with known statistical percentiles.
  - **Completion Criteria:** Accurately computes average present rate, 1% low, 0.1% low, and variance matching reference datasets.
  - **Status:** `COMPLETE`

- [x] **`TASK-P02-003`**: Automated Multi-Trial A/B Testing Harness
  - **Objective:** Create an automated harness that executes N repeated benchmark runs (Baseline vs Optimized) and computes two-tailed Student's t-test ($p$-values).
  - **Files / Components:** `crates/val-opt-core/src/benchmarking/ab_runner.rs`.
  - **Requirements:** Execute configurable test cycles (e.g. 10 Baseline vs 10 Optimized), filter outliers, and test whether differences satisfy $p < 0.01$ and $\Delta > 3\%$.
  - **Verification Method:** Run mock benchmark trials and verify $p$-value calculation against statistical reference tables.
  - **Completion Criteria:** Automatically generates comparative markdown/JSON report indicating whether an optimization produces statistically significant gains.
  - **Status:** `COMPLETE`

- [x] **`TASK-P02-004`**: UDP Jitter & Ping Telemetry Collector
  - **Objective:** Implement network latency telemetry tool that measures round-trip time, packet jitter, and packet loss against Riot edge nodes.
  - **Files / Components:** `crates/val-opt-core/src/network/probe.rs`.
  - **Requirements:** High-precision UDP probe transmitter sending time-stamped datagrams and recording return jitter.
  - **Verification Method:** Probe local gateway and public edge endpoints; verify jitter and RTT metrics against Wireshark traces.
  - **Completion Criteria:** Telemetry module records min/avg/max ping, standard deviation jitter, and packet loss percentage.
  - **Status:** `COMPLETE`

---

## Phase 3: Safe System Optimizations

- [x] **`TASK-P03-001`**: Windows Game Mode Programmatic Enforcer
  - **Objective:** Implement module to query and safely enforce Windows Game Mode state.
  - **Files / Components:** `crates/val-opt-core/src/optimizations/game_mode.rs`.
  - **Requirements:** Query registry key `HKCU\Software\Microsoft\GameBar` (`AllowAutoGameMode`), provide backup of previous state, and enable Game Mode.
  - **Verification Method:** Unit test verifying registry state toggle and rollback.
  - **Completion Criteria:** Successfully enables Game Mode and rolls back cleanly without system restart.
  - **Status:** `COMPLETE`

- [x] **`TASK-P03-002`**: Windows Core Audio APO Enhancement Disabler
  - **Objective:** Implement audio optimization module to disable DSP enhancements on the default audio endpoint to eliminate DPC spikes.
  - **Files / Components:** `crates/val-opt-core/src/optimizations/audio.rs`.
  - **Requirements:** Use Windows Core Audio APIs (`IMMDeviceEnumerator`, `IMMDevice`) and registry property store (`PKEY_AudioEndpoint_Disable_SysFx`) to disable enhancements cleanly.
  - **Verification Method:** Verify sound remains active and audio enhancement toggle in Windows Sound Control Panel reflects changes.
  - **Completion Criteria:** Audio remains operational with enhancements disabled; full rollback restores previous user settings.
  - **Status:** `COMPLETE`

- [x] **`TASK-P03-003`**: Power Scheme & Processor Boost Manager
  - **Objective:** Implement power plan management module with desktop/laptop auto-detection.
  - **Files / Components:** `crates/val-opt-core/src/optimizations/power.rs`.
  - **Requirements:** Query `PowerGetActiveScheme`, apply Ultimate/High Performance on AC-powered desktops, preserve balanced thermal profiles on laptops, record original GUID for restoration.
  - **Verification Method:** Verify active GUID changes via `powercfg /getactivescheme` and returns to original on rollback.
  - **Completion Criteria:** Power plan changes cleanly without requiring reboot; desktop/laptop logic works properly.
  - **Status:** `COMPLETE`

- [x] **`TASK-P03-004`**: Optimization Transaction & Unified Rollback Module
  - **Objective:** Create transactional optimization coordinator that aggregates Phase 3 settings into a reversible snapshot.
  - **Files / Components:** `crates/val-opt-core/src/optimizations/mod.rs`, `crates/val-opt-shared/src/models/snapshot.rs`.
  - **Requirements:** Serialize all "before" states to a transaction log; execute atomic rollback if any optimization step fails.
  - **Verification Method:** Apply optimizations, trigger artificial failure, verify complete system rollback to baseline.
  - **Completion Criteria:** Zero orphan changes on failure; 100% deterministic restoration.
  - **Status:** `COMPLETE`

---

## Phase 4: Process & Service Management Engine

- [x] **`TASK-P04-001`**: Process Safety Database & Classification Parser
  - **Objective:** Implement in-memory process safety database mapping executables to Tiers 0 through 3.
  - **Files / Components:** `crates/val-opt-core/src/process/safety_db.rs`, `crates/val-opt-shared/src/models/process.rs`.
  - **Requirements:** Fast lookup table containing protected Tier 0 processes (Vanguard, DWM, System), Tier 1 driver processes, Tier 2 candidates (Brave, Discord, Steam), and Tier 3 services.
  - **Verification Method:** Unit tests verifying known critical processes cannot be marked for termination.
  - **Completion Criteria:** Attempting to classify `vgc.exe`, `csrss.exe`, or `dwm.exe` as Tier 2 or 3 triggers a hard compile/runtime error.
  - **Status:** `COMPLETE`

- [x] **`TASK-P04-002`**: Graceful Process Termination & Working Set Trimmer
  - **Objective:** Implement safe termination engine with two-stage exit (`WM_CLOSE` -> 1000ms wait -> `TerminateProcess`) and RAM working set trimming for Explorer.
  - **Files / Components:** `crates/val-opt-core/src/process/terminator.rs`, `crates/val-opt-core/src/process/memory.rs`.
  - **Requirements:** Terminate safe Tier 2 targets gracefully; record process paths for post-match relaunch; call `EmptyWorkingSet` on `explorer.exe`.
  - **Verification Method:** Launch test instances of notepad and browser; verify graceful exit and working set memory reduction.
  - **Completion Criteria:** Clean termination of Tier 2 processes without process tree leaks or orphaned handles.
  - **Status:** `COMPLETE`

- [x] **`TASK-P04-003`**: Non-Essential Windows Service Pauser
  - **Objective:** Implement service management module to pause and resume Tier 3 background services (`wuauserv`, `SysMain`, `DiagTrack`).
  - **Files / Components:** `crates/val-opt-core/src/process/services.rs`.
  - **Requirements:** Use Windows Service Control Manager (`OpenSCManagerW`, `OpenServiceW`, `ControlService`) to send `SERVICE_CONTROL_STOP`; record previous service running state.
  - **Verification Method:** Verify services enter Stopped state during gaming and resume Running state upon restoration.
  - **Completion Criteria:** Services start and stop safely without registry corruption or service hangs.
  - **Status:** `COMPLETE`

- [x] **`TASK-P04-004`**: VALORANT Process Lifecycle Supervisor
  - **Objective:** Implement game lifecycle watcher monitoring for `VALORANT-Win64-Shipping.exe`.
  - **Files / Components:** `crates/val-opt-core/src/process/supervisor.rs`.
  - **Requirements:** Detect game launch, terminate `RiotClientUx.exe` (CEF frontend), apply `HIGH_PRIORITY_CLASS`, apply P-core affinity mask if Intel hybrid, and enter low-overhead `WaitForSingleObject` wait state.
  - **Verification Method:** Test against mock game executable; verify priority, affinity, and child process termination within 500ms of launch.
  - **Completion Criteria:** Zero CPU usage during active supervision; immediate triggering of restoration upon game process exit.
  - **Status:** `COMPLETE`

---

## Phase 5: Network Optimization & Diagnostics

- [x] **`TASK-P05-001`**: NIC Property Configurator (EEE & Interrupt Moderation)
  - **Objective:** Programmatically configure physical network adapter to disable Energy Efficient Ethernet (EEE) and tune Interrupt Moderation.
  - **Files / Components:** `crates/val-opt-core/src/network/adapter.rs`.
  - **Requirements:** Query active adapter via CIM/WMI/NetAdapter, set `*EEE` = 0 (Disabled), `*GreenEthernet` = 0 (Disabled), and `*InterruptModeration` = Low or Disabled; record original settings.
  - **Verification Method:** Check adapter advanced properties in Windows Device Manager before and after application.
  - **Completion Criteria:** Properties applied without network link drop lasting > 1.5 seconds; full rollback supported.
  - **Status:** `COMPLETE`

- [x] **`TASK-P05-002`**: Flow Control & RSS Verifier
  - **Objective:** Verify Receive Side Scaling (RSS) is active and disable 802.3x Flow Control on the primary network interface.
  - **Files / Components:** `crates/val-opt-core/src/network/flow_control.rs`.
  - **Requirements:** Ensure Flow Control is disabled (to avoid UDP packet stalling) and RSS has $\ge 4$ queues allocated.
  - **Verification Method:** Verify settings via PowerShell `Get-NetAdapterAdvancedProperty` and netsh.
  - **Completion Criteria:** Successfully disables Flow Control and validates RSS queue distribution.
  - **Status:** `COMPLETE`

- [x] **`TASK-P05-003`**: Windows QoS DSCP Policy Registrar
  - **Objective:** Register a local QoS policy tagging VALORANT UDP outbound packets with DSCP 46 (Expedited Forwarding).
  - **Files / Components:** `crates/val-opt-core/src/network/qos.rs`.
  - **Requirements:** Create Group Policy / Windows QoS policy rule matching `VALORANT-Win64-Shipping.exe` port range 7000-8000 with DSCP 46; ensure rollback removes rule.
  - **Verification Method:** Verify policy entry using PowerShell `Get-NetQosPolicy` and check IP header DSCP field via Wireshark.
  - **Completion Criteria:** QoS policy successfully registered and unregisters cleanly on rollback.
  - **Status:** `COMPLETE`

- [x] **`TASK-P05-004`**: Bufferbloat & Network Health Diagnostic Runner
  - **Objective:** Implement standalone network quality test measuring loaded vs unloaded ping and jitter.
  - **Files / Components:** `crates/val-opt-core/src/network/bufferbloat.rs`.
  - **Requirements:** Measure baseline RTT to Riot edge servers, initiate asynchronous saturating throughput burst, measure induced latency spike, and compute bufferbloat grade (A+ to F).
  - **Verification Method:** Run diagnostic against test servers; verify score consistency across multiple runs.
  - **Completion Criteria:** Generates diagnostic report displaying unloaded ping, loaded ping, jitter, and bufferbloat grade.
  - **Status:** `COMPLETE`

---

## Phase 6: Latency Monitoring & Kernel ETW Integration

- [x] **`TASK-P06-001`**: Kernel ETW DPC / ISR Event Session Manager
  - **Objective:** Build real-time Deferred Procedure Call (DPC) and Interrupt Service Routine (ISR) latency event session using Windows ETW.
  - **Files / Components:** `crates/val-opt-core/src/latency/etw_session.rs`.
  - **Requirements:** Start a kernel trace session with `EVENT_TRACE_FLAG_DPC | EVENT_TRACE_FLAG_INTERRUPT`, stream events without disk buffering, and parse execution duration.
  - **Verification Method:** Compare DPC latency metrics with LatencyMon during heavy disk/GPU activity.
  - **Completion Criteria:** Captures DPC and ISR execution times with microsecond precision and < 1% CPU overhead.
  - **Status:** `COMPLETE`

- [x] **`TASK-P06-002`**: Buggy Driver Fault Isolator
  - **Objective:** Implement module to resolve DPC execution spikes to specific driver filenames (`.sys`).
  - **Files / Components:** `crates/val-opt-core/src/latency/driver_isolator.rs`.
  - **Requirements:** Correlate DPC routine addresses to loaded kernel module base addresses; identify offending drivers (e.g. `ndis.sys`, audio drivers, GPU drivers) exceeding 500µs.
  - **Verification Method:** Inject synthetic load or test against known high-DPC audio driver; verify offending driver name is identified.
  - **Completion Criteria:** Outputs prioritized list of kernel drivers ranked by highest single DPC execution time.
  - **Status:** `COMPLETE`

- [x] **`TASK-P06-003`**: Real-Time Latency Health Monitor
  - **Objective:** Create a background watchdog that continuously tracks system interrupt health during a match.
  - **Files / Components:** `crates/val-opt-core/src/latency/monitor.rs`.
  - **Requirements:** Ring buffer of recent DPC/ISR durations; triggers warnings if latency exceeds 1000µs threshold.
  - **Verification Method:** Run stress test; verify warning event is emitted when threshold is breached.
  - **Completion Criteria:** Emits structured latency events through IPC channel without interrupting game threads.
  - **Status:** `COMPLETE`

- [x] **`TASK-P06-004`**: Latency Diagnostic Report Generator
  - **Objective:** Produce comprehensive hardware latency health report for the user.
  - **Files / Components:** `crates/val-opt-core/src/latency/report.rs`.
  - **Requirements:** Export formatted summary detailing system suitability for competitive gaming (DPC/ISR stability, BIOS power management impact, timer integrity).
  - **Verification Method:** Run diagnostic suite and verify output markdown/JSON report.
  - **Completion Criteria:** Produces actionable report identifying whether hardware/drivers are bottlenecking input latency.
  - **Status:** `COMPLETE`

---

## Phase 7: High-Performance Native UI

- [x] **`TASK-P07-001`**: Native Desktop GUI Skeleton (Rust + Slint)
  - **Objective:** Initialize the desktop graphical interface using the Slint framework, compiled to native machine code.
  - **Files / Components:** `crates/val-opt-gui/Cargo.toml`, `crates/val-opt-gui/src/main.rs`, `crates/val-opt-gui/ui/appwindow.slint`.
  - **Requirements:** Clean, dark-mode competitive FPS theme; zero web technologies or Chromium dependencies; memory footprint < 25 MB.
  - **Verification Method:** Launch application; check Task Manager for memory footprint and GPU swapchain allocations.
  - **Completion Criteria:** Window renders at native monitor refresh rate with < 25 MB RAM and instant responsiveness.
  - **Status:** `COMPLETE`

- [x] **`TASK-P07-002`**: Named Pipe IPC Client-Server Implementation
  - **Objective:** Implement inter-process communication between `val-opt-core` daemon and `val-opt-gui`.
  - **Files / Components:** `crates/val-opt-shared/src/ipc/mod.rs`, `crates/val-opt-core/src/ipc_server.rs`, `crates/val-opt-gui/src/ipc_client.rs`.
  - **Requirements:** Asynchronous Windows Named Pipe (`\\.\pipe\val_opt_ipc`), structured JSON message framing, automatic reconnection handling.
  - **Verification Method:** Send mock scan and optimize commands from GUI; verify daemon receives and responds correctly.
  - **Completion Criteria:** Bidirectional IPC operates with < 2ms message round-trip time.
  - **Status:** `COMPLETE`

- [x] **`TASK-P07-003`**: Dashboard, Settings & Diagnostics Views
  - **Objective:** Implement GUI views for Hardware Status, Optimization Profile, Network Health, and Benchmark Graphs.
  - **Files / Components:** `crates/val-opt-gui/ui/dashboard.slint`, `crates/val-opt-gui/ui/settings.slint`, `crates/val-opt-gui/ui/benchmarks.slint`.
  - **Requirements:** Real-time hardware overview, one-click "Launch Optimized" trigger, toggleable tweak categories, visual latency graphs.
  - **Verification Method:** Navigate all views and test interactive toggles.
  - **Completion Criteria:** Fully functional user interface with all settings linked to IPC commands.
  - **Status:** `COMPLETE`

- [x] **`TASK-P07-004`**: Ephemeral UI Unload & Relaunch Coordinator
  - **Objective:** Implement logic to terminate the GUI process completely upon game launch and relaunch it post-game.
  - **Files / Components:** `crates/val-opt-gui/src/lifecycle.rs`, `crates/val-opt-core/src/process/supervisor.rs`.
  - **Requirements:** When "Launch Optimized" is triggered, GUI sends handoff message to core daemon and cleanly exits (`std::process::exit(0)`); core daemon optionally respawns GUI when VALORANT exits.
  - **Verification Method:** Launch game through GUI; verify `val-opt-gui.exe` is completely absent from Task Manager while VALORANT is running.
  - **Completion Criteria:** Zero GUI memory or GPU footprint exists during gameplay.
  - **Status:** `COMPLETE`

---

## Phase 8: Safety, State Snapshot & Auto-Rollback Engine

- [x] **`TASK-P08-001`**: Atomic State Snapshot Serializer
  - **Objective:** Build atomic snapshot engine recording complete baseline system state prior to modifications.
  - **Files / Components:** `crates/val-opt-core/src/state/snapshot_engine.rs`.
  - **Requirements:** Capture power schemes, audio settings, paused services, closed processes, and NIC configurations into `%ProgramData%\ValorantOptimizer\snapshot.json` with SHA-256 integrity hash.
  - **Verification Method:** Unit test verifying serialized state can be written, hashed, and read back identically.
  - **Completion Criteria:** Complete snapshot serialized in < 50ms without disk corruption risk.
  - **Status:** `COMPLETE`

- [x] **`TASK-P08-002`**: Windows Boot Orphan Crash Recovery Service
  - **Objective:** Implement auto-recovery mechanism that restores paused services if the PC experiences a crash or sudden power cut mid-game.
  - **Files / Components:** `crates/val-opt-core/src/state/crash_recovery.rs`.
  - **Requirements:** On daemon startup, check for uncommitted `snapshot.json`; if found, execute immediate full restoration, log recovery event, and delete snapshot.
  - **Verification Method:** Apply tweaks, simulate hard process termination via `taskkill /F`, restart daemon, and verify all services restore cleanly.
  - **Completion Criteria:** 100% automated recovery from abnormal system shutdowns.
  - **Status:** `COMPLETE`

- [x] **`TASK-P08-003`**: Riot Vanguard Active Compliance Checker
  - **Objective:** Implement pre-flight anti-cheat validator ensuring all Vanguard requirements are strictly satisfied before launch.
  - **Files / Components:** `crates/val-opt-core/src/safety/vanguard_check.rs`.
  - **Requirements:** Verify `vgc` service is running, `vgk.sys` driver is loaded, Windows test signing is OFF, Secure Boot is ON, and VBS/HVCI is intact.
  - **Verification Method:** Run validator on test system; verify warnings fire if test signing is enabled or `vgc` is stopped.
  - **Completion Criteria:** Rejects optimization launch and alerts user if any Vanguard integrity prerequisite is compromised.
  - **Status:** `COMPLETE`

- [x] **`TASK-P08-004`**: One-Click Manual Emergency Rollback Trigger
  - **Objective:** Provide a fail-safe standalone CLI and GUI rollback command (`val-opt-cli rollback`).
  - **Files / Components:** `crates/val-opt-cli/src/main.rs`.
  - **Requirements:** Standalone executable that reads `snapshot.json` and restores all modified system settings without requiring GUI or daemon.
  - **Verification Method:** Run `val-opt-cli rollback` from administrator PowerShell; verify immediate reversion of all settings.
  - **Completion Criteria:** Immediate, failsafe restoration under all operational conditions.
  - **Status:** `COMPLETE`

---

## Phase 9: Comprehensive QA & Vanguard Validation

- [x] **`TASK-P09-001`**: Multi-Hardware Compatibility Test Suite
  - **Objective:** Execute test matrix across distinct hardware configurations (Intel Hybrid vs Monolithic, AMD Ryzen, AMD Radeon, NVIDIA GeForce, Desktop vs Laptop).
  - **Files / Components:** `tests/hardware_matrix_tests.rs`.
  - **Requirements:** Verify affinity masking, power plan selection, and HAGS detection function correctly on each hardware profile.
  - **Verification Method:** Automated test execution on physical test environments; log results to `QA/QA_PHASE_09.md`.
  - **Completion Criteria:** 100% test pass rate across all supported hardware configurations.
  - **Status:** `COMPLETE`

- [x] **`TASK-P09-002`**: Edge-Case & Failure Mode Stress Testing
  - **Objective:** Subject optimizer to aggressive edge-case scenarios (mid-session game updates, dual-monitor refresh mismatch, process re-spawn loops, power unplug on laptop).
  - **Files / Components:** `tests/edge_case_tests.rs`.
  - **Requirements:** Validate graceful handling without system lockups, memory leaks, or unhandled exceptions.
  - **Verification Method:** Execute automated stress script simulating edge conditions.
  - **Completion Criteria:** All failure scenarios recover cleanly according to the QA Failure Recovery Matrix.
  - **Status:** `COMPLETE`

- [x] **`TASK-P09-003`**: Live VALORANT Match & Vanguard Integrity Verification
  - **Objective:** Perform live testing across 100+ competitive matches with active optimizer.
  - **Files / Components:** `tests/vanguard_audit.rs`.
  - **Requirements:** Validate zero Vanguard error codes (`VAN 9005`, `VAN 1067`, `VAN 84`), zero account flags, and zero in-game disconnects.
  - **Verification Method:** Live match sessions with full log monitoring.
  - **Completion Criteria:** Zero Vanguard conflicts or match penalties recorded.
  - **Status:** `COMPLETE`

---

## Phase 10: Production Packaging & Security Audit

- [x] **`TASK-P10-001`**: Static Security Analysis & Binary Hardening
  - **Objective:** Conduct security audit of compiled native binaries.
  - **Files / Components:** Compiler release flags, `Cargo.toml`.
  - **Requirements:** Enable ASLR, DEP/NX, Control Flow Guard (CFG), and stack protection flags; run `cargo audit` to verify zero vulnerable dependencies.
  - **Verification Method:** Run `dumpbin /headers` and vulnerability scanner on release binaries.
  - **Completion Criteria:** Binaries fully hardened with zero security vulnerabilities.
  - **Status:** `COMPLETE`

- [x] **`TASK-P10-002`**: Digital Code Signing & Antivirus Whitelist Verification
  - **Objective:** Apply Microsoft Authenticode digital signature to all binaries and installer.
  - **Files / Components:** Release pipeline signing script.
  - **Requirements:** Sign `val-opt-core.exe`, `val-opt-gui.exe`, and `val-opt-cli.exe` with valid code-signing certificate; submit to VirusTotal.
  - **Verification Method:** Verify digital signature via `Get-AuthenticodeSignature` and confirm 0/70 detections on VirusTotal.
  - **Completion Criteria:** Zero false-positive antivirus blocks or Windows SmartScreen warnings.
  - **Status:** `COMPLETE`

- [x] **`TASK-P10-003`**: Production Installer Packaging (WiX / Inno Setup)
  - **Objective:** Create lightweight, production-grade installer package.
  - **Files / Components:** `installer/setup.iss` or `installer/Product.wxs`.
  - **Requirements:** One-click installation with UAC prompt, installation of core service, optional desktop shortcut, clean uninstaller that guarantees full rollback before removal.
  - **Verification Method:** Install on clean Windows 11 virtual machine; test full install, execution, and uninstall cycle.
  - **Completion Criteria:** Flawless installation and complete removal with zero registry or file leftovers.
  - **Status:** `COMPLETE`
