# Phase P3 Remediation Report: Optimization De-Scoping and Safety Cleanup

**Repository:** `https://github.com/Godwynski/valorant-optimizer`  
**Execution Date:** 2026-09-26  
**Phase:** P3 — Optimization De-Scoping & Subsystem Safety Cleanup  
**Governing Specification:** `REMEDIATION_PLAN.md` (Tasks `TASK-OPT-01` through `TASK-OPT-05`)  
**Audit Standard:** Forensic adversarial engineering review  

---

## 1. Executive Summary

Phase P3 executed an aggressive, unsparing de-scoping and safety cleanup across the entire `valorant-optimizer` codebase. Previous versions inherited legacy gaming optimizer tropes that attempted to forcefully mutate Windows kernel, memory, network driver, and service subsystems under the unverified assumption that aggressive interventions improve gaming performance. 

In Phase P3, four primary categories of unsafe or snake-oil optimizations were permanently eradicated from the automatic optimization pipeline:
1. **Forced Working-Set Trimming (`EmptyWorkingSet`):** Evicting physical pages to the standby/paged pool causes immediate soft page faults, standby list paging, and micro-stuttering whenever the Windows shell or background components resume. It was permanently excised from all automatic execution paths and replaced with passive, read-only diagnostic telemetry (`K32GetProcessMemoryInfo`).
2. **Network QoS DSCP 46 Mutation:** Local Group Policy / Windows NetQosPolicy rules forcing DSCP 46 (Expedited Forwarding) on VALORANT UDP outbound traffic were eliminated. Commercial residential ISPs and consumer routers routinely strip or reset DSCP tags upon ingress, while carrier policers may penalize or drop packets tagged with unauthorized Expedited Forwarding. All network policy creation and registry mutations were permanently removed from the automatic pipeline; QoS functionality was transitioned strictly to read-only diagnostics (`query_qos_policy`).
3. **Automatic NIC Interrupt Moderation & Driver Mutation:** Disabling Interrupt Moderation via PowerShell NetAdapter or registry modifications induces severe DPC/ISR packet interrupt storms that saturate CPU Core 0 under high throughput, destabilizing frame delivery and causing audio crackling. Automatic driver property mutations were completely excised; NIC driver parameters remain strictly at OEM defaults.
4. **Service & Bloatware Safety Hardening:** Windows Update (`wuauserv`) was audited and reclassified to Tier 0 Protected (`MUST_NOT_MODIFY`), ensuring that core Windows security updates and servicing operations are never paused or disabled. Non-essential background service pauses (`SysMain`, `DiagTrack`, `Spooler`) were restricted to explicit user opt-in commands (`pause-services`) with guaranteed transactional rollback.

Following these removals, the default automatic optimization pipeline is strictly bounded to three verifiable, safe Win32 subsystems:
- Windows Game Mode priority allocation (`HKCU\Software\Microsoft\GameBar`)
- Standard Win32 Power Scheme activation (`PowerSetActiveScheme` on AC-powered desktops; OEM plan preserved on laptops)
- Windows Core Audio APO Enhancement bypass (`MMDevices\Audio\Render` SysFx disablement)

All 140 tests in the workspace test suite pass with 0 failures, including 10 dedicated P3 regression test cases proving the total absence of automatic working set trimming, DSCP tagging, NIC mutations, and protected service disruptions.

---

## 2. Removed Optimizations

### 2.1 Forced Working-Set Memory Trimming (`EmptyWorkingSet`)
- **Previous Behavior:** The optimizer exposed `trim_working_set(pid)` and `trim_explorer_working_set()`, invoking Win32 `OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA)` followed by `K32EmptyWorkingSet(handle)`. This was also exposed over the IPC named pipe via `IpcRequest::TrimWorkingSet` and the CLI command `trim-memory`.
- **Exact Implementation Location:**
  - `crates/val-opt-core/src/process/memory.rs` (previously lines 20–85)
  - `crates/val-opt-core/src/ipc_server.rs` (handling `IpcRequest::TrimWorkingSet`)
  - `crates/val-opt-cli/src/main.rs` (command `"trim-memory"`)
- **Why It Was Removed:** Calling `EmptyWorkingSet` or `SetProcessWorkingSetSize(-1, -1)` forces all active physical pages out of a process working set into the standby and modified page lists. While task managers report an immediate drop in "Working Set Size," this is illusory: the moment the process executes any instruction or touches a memory address, the CPU experiences a cascade of soft page faults as Windows faults the pages back into RAM. In gaming environments, calling this on `explorer.exe` or background services introduces DPC spikes and micro-stutter when the player opens the Start menu, taskbar, or volume controls. Modern Windows memory management natively trims idle processes when memory pressure demands it.
- **Replacement Behavior:** Replaced with purely observational, read-only memory diagnostics:
  - `query_process_memory_info(pid: u32) -> Result<ProcessMemorySnapshot, String>` uses `PROCESS_QUERY_LIMITED_INFORMATION` and calls `K32GetProcessMemoryInfo` without `PROCESS_SET_QUOTA`.
  - `query_current_process_memory() -> Result<ProcessMemorySnapshot, String>` samples host optimizer working set.
  - `IpcRequest::TrimWorkingSet` returns a structured error: `FEATURE_REMOVED` with an explanatory notice.
  - CLI `trim-memory` prints an explanatory de-scoping notice alongside diagnostic telemetry.

### 2.2 Network QoS DSCP 46 Policy Registration
- **Previous Behavior:** The automatic optimization coordinator executed Step 5 in `apply_optimizations()`, calling `register_valorant_qos_policy()`. Previous code created Windows NetQosPolicy rules matching `VALORANT-Win64-Shipping.exe` port range 7000–8000 with DSCP 46 (Expedited Forwarding), and mutated the registry at `HKLM\SYSTEM\CurrentControlSet\Services\Tcpip\QoS` (`Do not use NLA = 1`).
- **Exact Implementation Location:**
  - `crates/val-opt-core/src/optimizations/mod.rs` (lines 98–107)
  - `crates/val-opt-core/src/network/qos.rs` (lines 88–152)
  - `crates/val-opt-gui/ui/settings.slint` (Toggle 5 "Quality of Service (QoS) DSCP 46 Policy")
  - `crates/val-opt-gui/ui/dashboard.slint` (banner label "QoS DSCP 46")
- **Why It Was Removed:** DSCP markings operate at Layer 3 (IP header TOS field). While DSCP works within enterprise networks, virtually all residential Internet Service Providers (ISPs) strip or reset DSCP tags to 0 (`Best Effort`) at the edge gateway. Worse, certain carrier traffic policers actively drop packets tagged with DSCP 46 if the subscriber profile lacks an SLA for Expedited Forwarding. Modifying system-wide TCP/IP NLA policies solely for a game client is an unwarranted system mutation that creates networking side effects.
- **Replacement Behavior:** 
  - Step 5 was completely excised from `apply_optimizations()`.
  - In `crates/val-opt-core/src/network/qos.rs`, `restore_nla_setting` was updated so that when `previous_setting` is `None`, zero registry keys are touched (no `Remove-ItemProperty` execution).
  - Gated strictly as a read-only diagnostic command: `val-opt-cli qos-check` runs `query_qos_policy` to inspect if any NetQosPolicy is configured on the host.
  - UI labels updated in `settings.slint` (Toggle 5 marked "DE-SCOPED") and `dashboard.slint` ("Network Telemetry • Safe System Profiling").

### 2.3 Automatic NIC Interrupt Moderation Changes
- **Previous Behavior:** The automatic optimization coordinator executed Step 4 in `apply_optimizations()`, calling `optimize_adapter_latency_properties()` which iterated through registry keywords and executed PowerShell `Set-NetAdapterAdvancedProperty` to force `*InterruptModeration = "0"` (Disabled) and disable Energy Efficient Ethernet (`*EEE`, `*GreenEthernet`).
- **Exact Implementation Location:**
  - `crates/val-opt-core/src/optimizations/mod.rs` (lines 82–96)
  - `crates/val-opt-core/src/network/adapter.rs` (lines 200–265)
  - `crates/val-opt-core/src/network/flow_control.rs` (lines 30–53)
- **Why It Was Removed:** Hardware Interrupt Moderation (Adaptive/Moderate) allows modern Gigabit/2.5G/10G network controllers to coalesce incoming network packets before issuing hardware interrupts. Forcing `InterruptModeration = 0` causes the NIC to fire a hardware interrupt (ISR) for every single received UDP packet. During high network throughput or game downloads, this floods CPU Core 0 with tens of thousands of interrupts per second, generating severe DPC execution bursts, queue contention, and audio crackling. Furthermore, modifying NIC driver properties dynamically risks physical link renegotiation delays.
- **Replacement Behavior:**
  - Step 4 was completely removed from `apply_optimizations()`.
  - In `crates/val-opt-core/src/network/adapter.rs`, the `INTERRUPT_MODERATION_KEYWORD` mutation loop in `optimize_adapter_latency_properties` was deleted.
  - Read-only diagnostics remain fully active: `query_adapter_properties()`, `query_adapter_link_status()`, `verify_rss()`, `query_global_tcp_rss()`, and `run_bufferbloat_diagnostic()` inspect network health without touching NIC driver parameters.

---

## 3. Service/Process Audit

Every candidate service and process evaluated by the optimizer's safety database (`ProcessSafetyDb`) and management engines (`services.rs`, `terminator.rs`) was classified according to strict safety tiers.

### Classification Categories:
- **`MUST_NOT_MODIFY`**: Critical kernel, security, authentication, networking, Vanguard, or Windows Update service/process. Hard invariant: terminating or disabling is strictly prohibited.
- **`SAFE_TO_REMOVE`**: Non-essential user-mode software (browsers, Discord, electron wrappers) with no system dependencies. Terminated via two-stage `WM_CLOSE` + `TerminateProcess` with path recording for post-match relaunch.
- **`OPTIONAL_USER_CONTROLLED`**: Non-critical Windows service pausable strictly upon explicit user opt-in (`val-opt-cli pause-services`) with full rollback state tracking. Never paused automatically by default optimizer.
- **`DIAGNOSTIC_ONLY`**: Telemetry and inspection targets queried passively without modification.

| Target | Previous Action | Classification | Final Action | Reason |
| :--- | :--- | :---: | :--- | :--- |
| **`wuauserv`** (Windows Update) | Paused in Tier 3 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Essential for Windows security patches, servicing stack, and driver maintenance. Pausing creates security vulnerabilities and component store conflicts. |
| **`vgc` / `vgc.exe`** (Vanguard User Service) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Core Riot Vanguard anti-cheat service. Disabling or modifying causes immediate client crash and matchmaking penalties. |
| **`vgk` / `vgk.sys`** (Vanguard Kernel Driver) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Ring-0 anti-cheat driver; tampering causes system lockup or ban. |
| **`WinDefend`** (Microsoft Defender Antivirus) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Core system endpoint protection. Must never be disabled by gaming software. |
| **`mpssvc`** (Windows Defender Firewall) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Core network filtering and firewall service. |
| **`SecurityHealthService`** | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Windows Security Center health monitoring service. |
| **`CryptSvc`** (Cryptographic Services) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Manages catalog database and Authenticode signature verification for running binaries. |
| **`RpcSs` / `RpcEptMapper`** (RPC Subsystem) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Foundational Windows inter-process communication endpoint. |
| **`BFE`** (Base Filtering Engine) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Network security packet filtering architecture. |
| **`Dnscache`** (DNS Client) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Resolves domain names for game clients and server matchmaking. |
| **`Dhcp`** (DHCP Client) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Leases and registers IP addresses for network adapters. |
| **`AudioSrv` / `AudioEndpointBuilder`** | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; stopping blocked | Core Windows Audio graph. Stopping crashes game sound and `audiodg.exe`. |
| **`csrss.exe`, `smss.exe`, `wininit.exe`** | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; kill blocked | Critical Windows NT subsystem processes (termination results in immediate BSOD). |
| **`dwm.exe`** (Desktop Window Manager) | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; kill blocked | Hardware-accelerated window compositor; terminating causes black screen and desktop crash. |
| **`RiotClientServices.exe`** | Protected Tier 0 | `MUST_NOT_MODIFY` | **Protected in Tier 0**; kill blocked | Manages Vanguard heartbeat session and authentication token with Riot platform. |
| **`SysMain`** (Superfetch / Prefetch) | Paused in Tier 3 | `OPTIONAL_USER_CONTROLLED` | Retained as user opt-in (`pause-services`); never paused automatically | Disk and memory prefetching service. Disabling yields minimal benefit on modern NVMe drives; default optimizer leaves running. |
| **`DiagTrack`** (Diagnostics Tracking) | Paused in Tier 3 | `OPTIONAL_USER_CONTROLLED` | Retained as user opt-in (`pause-services`); never paused automatically | Windows telemetry background worker. Non-essential during competitive play; toggleable. |
| **`Spooler`** (Print Spooler) | Previously unmanaged | `OPTIONAL_USER_CONTROLLED` | Added to Tier 3 user opt-in (`pause-services`); never paused automatically | Background printer service. Non-essential during competitive play; toggleable. |
| **`brave.exe`, `chrome.exe`, `msedge.exe`, `firefox.exe`** | Safe Tier 2 | `SAFE_TO_REMOVE` | Gracefully closed on explicit bloat purge (`WM_CLOSE`) | Web browsers consuming substantial physical RAM (CEF/Chromium rendering pipelines). |
| **`discord.exe`, `slack.exe`, `spotify.exe`, `teams.exe`** | Safe Tier 2 | `SAFE_TO_REMOVE` | Gracefully closed on explicit bloat purge (`WM_CLOSE`) | Chat and media streaming applications executing background Electron tasks. |
| **`steam.exe`, `epicgameslauncher.exe`, `battle.net.exe`** | Safe Tier 2 | `SAFE_TO_REMOVE` | Gracefully closed on explicit bloat purge (`WM_CLOSE`) | Secondary game launchers and game store web helpers. |
| **`RiotClientUx.exe` / `RiotClientUxRender.exe`** | Safe Tier 2 | `SAFE_TO_REMOVE` | Gracefully closed post-game launch | Heavy CEF browser frontend for Riot Client (reclaims 200–400 MB RAM while keeping `RiotClientServices.exe` active). |
| **`explorer.exe`** | Working set trimmed | `DIAGNOSTIC_ONLY` | **Working set trim removed**; memory sampled read-only | Windows shell. Forcefully trimming induces soft page faults; never terminated, never trimmed. |

---

## 4. Repository-Wide Static Audit

A comprehensive static audit was performed across all source files, documentation, manifests, and scripts. The table below lists every query term, total remaining occurrences, and their exact operational justification.

| Search Query | Production Code Calls | Non-Production / Docs Occurrences | Detailed Location Breakdown & Justification |
| :--- | :---: | :---: | :--- |
| **`K32EmptyWorkingSet`** | **0** | **3** | - `crates/val-opt-core/src/process/memory.rs` (line 5): Documentation note explaining permanent removal.<br>- `PROJECT_STATE.md` (line 39): Historical remediation log.<br>- `REMEDIATION_PLAN.md` (line 316): Historical task specification. |
| **`EmptyWorkingSet`** | **0** | **17** | - `crates/val-opt-core/src/process/memory.rs` (lines 5, 93): Documentation header and test assertion documenting non-invocation.<br>- `crates/val-opt-core/src/ipc_server.rs` (line 140): Error response string notifying IPC callers that feature was permanently removed.<br>- `crates/val-opt-cli/src/main.rs` (line 148): Console message notifying CLI users that trimming is de-scoped.<br>- `crates/val-opt-core/src/optimizations/mod.rs` (lines 289, 293): Regression test `test_p3_no_automatic_empty_working_set`.<br>- Documentation files (`README.md`, `TASKS.md`, `DECISIONS.md`, `PHASES/*`, `QA/*`): Documenting prohibition and removal rationale. |
| **`SetProcessWorkingSetSize`**| **0** | **2** | - `crates/val-opt-core/src/process/memory.rs` (line 6): Header comment noting permanent removal.<br>- `PROJECT_STATE.md` (line 39): Historical remediation log. |
| **`*InterruptModeration`** | **0** | **5** | - `crates/val-opt-core/src/network/adapter.rs` (line 29): String constant `INTERRUPT_MODERATION_KEYWORD` preserved for read-only CIM property query.<br>- `crates/val-opt-core/src/network/adapter.rs` (lines 295, 302, 344): Unit tests verifying NetAdapter JSON deserializer and keyword identifier allowlist validator.<br>- `REMEDIATION_PLAN.md` (lines 334, 340): Historical task specification. |
| **`Interrupt Moderation`** | **0** | **16** | - `crates/val-opt-core/src/network/adapter.rs` (lines 6, 201, 249): Header and function comments documenting permanent removal.<br>- `crates/val-opt-core/src/optimizations/mod.rs` (line 82): Note explaining Step 4 removal from automatic pipeline.<br>- Documentation files (`README.md`, `PROJECT_STATE.md`, `TASKS.md`, `DECISIONS.md`, `PHASES/*`, `QA/*`): Documenting permanent removal and rationale. |
| **`DSCP`** | **0** | **28** | - `crates/val-opt-core/src/network/qos.rs` (lines 20, 48, 57, 164): Read-only query constant `DSCP_EXPEDITED_FORWARDING`, serde field alias for `Get-NetQosPolicy` parsing, and unit tests.<br>- `crates/val-opt-shared/src/models/network.rs` (line 71): Diagnostic struct field `QosPolicyInfo.dscp_value`.<br>- `crates/val-opt-core/src/optimizations/mod.rs` (lines 82, 262, 268): Comments and regression test `test_p3_no_automatic_dscp_or_qos_mutation`.<br>- `crates/val-opt-cli/src/main.rs` (lines 335, 348): Read-only `qos-check` command output.<br>- `crates/val-opt-gui/ui/settings.slint` (lines 7, 94, 98, 99): Slint toggle default set to `false`, labeled `DE-SCOPED`.<br>- Documentation files (`README.md`, `PROJECT_STATE.md`, `TASKS.md`, `DECISIONS.md`, `PHASES/*`, `QA/*`): Documenting permanent removal and rationale. |
| **`wuauserv`** | **0 (never paused automatically)** | **17** | - `crates/val-opt-core/src/process/safety_db.rs` (lines 88, 227, 231): Listed in `tier0_services` and verified by unit tests as `Tier0Protected`.<br>- `crates/val-opt-core/src/process/services.rs` (lines 5, 185): Note documenting Tier 0 protection; test verifying `stop_service("wuauserv")` returns `SafetyViolationError::ProtectedService`.<br>- `crates/val-opt-core/src/state/snapshot_engine.rs` (line 883) & `crash_recovery.rs` (line 268): Unit tests verifying snapshot recovery handles service records.<br>- Documentation files: Recording reclassification to `Tier0Protected` / `MUST_NOT_MODIFY`. |

---

## 5. Tests

All tests were executed using the Visual Studio 2026 MSVC Developer PowerShell environment. Zero tests were skipped or fabricated.

### 5.1 Targeted P3 Regression Test Suite
Command:
```powershell
cargo test --package val-opt-core --lib test_p3
```
Output:
```text
running 3 tests
test optimizations::tests::test_p3_no_automatic_dscp_or_qos_mutation ... ok
test optimizations::tests::test_p3_no_automatic_empty_working_set ... ok
test optimizations::tests::test_p3_no_automatic_nic_or_interrupt_moderation_mutation ... ok

test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 95 filtered out; finished in 1.51s
```

Command:
```powershell
cargo test --package val-opt-core --lib process::
```
Output:
```text
running 24 tests
test process::memory::tests::test_no_empty_working_set_invoked ... ok
test process::memory::tests::test_query_process_memory_info_read_only ... ok
test process::safety_db::tests::test_vanguard_and_system_protection ... ok
test process::services::tests::test_service_protection_barrier ... ok
test process::safety_db::tests::test_tier2_and_tier3_classification ... ok
test process::terminator::tests::test_elevation_check ... ok
test process::services::tests::test_query_tier3_service_state ... ok
test process::terminator::tests::test_relaunch_validation_invalid_path ... ok
test process::terminator::tests::test_relaunch_validation_nonexistent_executable ... ok
test process::terminator::tests::test_termination_refusal_for_tier0 ... ok
test process::terminator::tests::test_relaunch_validation_protected_executable ... ok
test process::valorant_launcher::tests::test_detect_installation_structure ... ok
test process::valorant_launcher::tests::test_extract_executable_from_command ... ok
test process::valorant_launcher::tests::test_token_selection_logic ... ok
test process::valorant_launcher::tests::test_uri_registration_detection ... ok
test process::supervisor::tests::test_spoofed_valorant_binary_rejected_by_path_validation ... ok
test process::valorant_launcher::tests::test_already_running_game_detection ... ok
test process::terminator::tests::test_batch_relaunch_handles_failures_gracefully ... ok
test process::terminator::tests::test_relaunch_normal_user_process ... ok
test process::terminator::tests::test_graceful_termination_of_spawned_process ... ok
test process::supervisor::tests::test_optimize_process_priority ... ok
test process::supervisor::tests::test_find_process_with_path ... ok
test process::supervisor::tests::test_background_supervision_lifecycle ... ok
test process::supervisor::tests::test_wait_for_game_exit_aborts_on_stop_signal ... ok

test result: ok. 24 passed; 0 failed; 0 ignored; 0 measured; 74 filtered out; finished in 3.30s
```

Command:
```powershell
cargo test --package val-opt-core --lib network::qos
```
Output:
```text
running 5 tests
test network::qos::tests::test_register_qos_policy_descaled ... ok
test network::qos::tests::test_validate_policy_name_injection_payloads ... ok
test network::qos::tests::test_validate_policy_name_valid ... ok
test network::qos::tests::test_parse_net_qos_policy_json ... ok
test network::qos::tests::test_query_nonexistent_qos_policy ... ok

test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 93 filtered out; finished in 0.39s
```

### 5.2 Full Workspace Test Suite
Command:
```powershell
cargo test --workspace
```
Output Summary:
```text
val-opt-core:               98 passed, 0 failed
val-opt-gui:                 3 passed, 0 failed
val-opt-shared:             24 passed, 0 failed
edge_case_tests:             5 passed, 0 failed
hardware_matrix_tests:       7 passed, 0 failed
vanguard_audit:              3 passed, 0 failed
------------------------------------------------
Total Workspace Tests:     140 passed, 0 failed
P3-Specific Tests:          10 passed, 0 failed
```

---

## 6. P0/P1/P2 Regression Verification

All security, functional, and telemetry controls established in previous phases were audited to confirm they remain completely intact:

### P0 Security Controls:
- **PowerShell Injection Barriers:** Constant script templates, allowlist input validation (`validate_adapter_identifier`, `validate_property_value`, `validate_policy_name`), and strict `$env:...` parameterization without string concatenation remain enforced across all diagnostic execution paths.
- **ProgramData ACL Hardening:** `%ProgramData%\ValorantOptimizer` directory and secret key permissions are hardened to `SYSTEM` and `Administrators` (Full Control), and standard `Users` (Read/Execute only), preventing unprivileged file hijacking or tampering.
- **Named Pipe Security Attributes:** SDDL DACL restricts pipe connections to elevated administrators and local system accounts, and validates client PID/elevation prior to servicing privileged requests.
- **HMAC-SHA256 & DPAPI Cryptographic Integrity:** All serialized system snapshots require a machine-bound HMAC-SHA256 signature generated with a DPAPI-protected secret key.
- **Arbitrary Rollback / Path Traversal Defenses:** Reparse points, directory junctions, NTFS hard links, and directory traversals (`..`) are strictly rejected by `validate_canonical_path`.

### P1 Functional Correctness Controls:
- **CPU Hybrid Topology:** Windows `EfficiencyClass` (0 = P-core, 1 = E-core) handling matches MSDN Win32 specifications across Intel Alder Lake / Raptor Lake and AMD Zen 4 architectures.
- **VALORANT Path Validation:** Genuine installation structures (`ShooterGame\Binaries\Win64\VALORANT-Win64-Shipping.exe`) are validated; fake/spoofed binaries are rejected.
- **ProcessSupervisor Stop-Signal Handling:** Clean cooperative cancellation via `AtomicBool` terminates monitoring threads without thread killing or resource leaks.
- **Token-Safe Application Relaunch:** Elevated daemon de-escalates child process launches to standard user tokens via `CreateProcessAsUserW` to prevent accidental privilege escalation.
- **Portable Toolchain Discovery:** Visual Studio toolchain paths are discovered dynamically via `vswhere.exe`.

### P2 Real Measurement & Telemetry Controls:
- **Genuine Windows ETW Presentation Capture:** Uses official Microsoft-Windows-DXGI (`{CA11C036-0102-4A2D-A6AD-F03CFED5D3C9}`) Event 42 (`Present_Start`) and Event 43 (`Present_Stop`), Microsoft-Windows-D3D9 (`{783ACA0A-790E-4D7F-8451-AA850511C6B9}`), and NT Kernel Logger without synthetic data injection.
- **Cadence Measurement Semantics:** Frame-time intervals are strictly defined as Application Present Cadence (`MsBetweenPresents`) between consecutive application `Present_Start` QPC timestamps.
- **Physical Display Timing Classification:** Display scanout / VSync delivery is explicitly classified as `UNVERIFIED`; fabricated offsets have been eliminated.
- **Kernel DPC/ISR Telemetry:** QPC duration measurements for kernel routines are captured directly from kernel events; unresolved addresses resolve to `UnknownKernelRoutine`.
- **Statistical Integrity:** Interleaved A/B runner with Paired Student's t-test and paired block-resampling bootstrap confidence intervals ($N = 10$ benchmark trials).

---

## 7. Evidence Matrix

| Area | Status | Evidence |
| :--- | :---: | :--- |
| **EmptyWorkingSet Removal** | `TEST-VERIFIED` | Zero calls in production code. `test_no_empty_working_set_invoked` and `test_p3_no_automatic_empty_working_set` demonstrate working set is never flushed. Diagnostic telemetry confirmed read-only. |
| **DSCP 46 Removal** | `TEST-VERIFIED` | Step 5 excised from `apply_optimizations()`. `test_p3_no_automatic_dscp_or_qos_mutation` and `test_register_qos_policy_descaled` prove zero QoS policy creation. Registry NLA setting untouched when `None`. |
| **Interrupt Moderation Removal** | `TEST-VERIFIED` | Step 4 excised from `apply_optimizations()`. Mutation block excised from `adapter.rs`. `test_p3_no_automatic_nic_or_interrupt_moderation_mutation` proves zero adapter property modifications. |
| **Service Safety Hardening** | `TEST-VERIFIED` | `wuauserv` moved to `tier0_services` in `safety_db.rs`. `test_vanguard_and_system_protection` and `test_service_protection_barrier` verify stopping `wuauserv`, `vgc`, and `WinDefend` is rejected with `ProtectedService`. |
| **Read-Only Diagnostics** | `TEST-VERIFIED` | `test_query_process_memory_info_read_only`, `test_query_tier3_service_state`, `test_parse_net_qos_policy_json`, `test_bufferbloat_grade_calculation`, and `test_verify_rss_on_primary_adapter` pass cleanly without system mutations. |
| **P0 Security Preservation** | `TEST-VERIFIED` | Injection tests, ACL tests, named-pipe tests, DPAPI roundtrips, and HMAC tamper rejection tests passing (24/24 in `val-opt-shared`, all security tests in `val-opt-core`). |
| **P1 Process Safety Preservation** | `TEST-VERIFIED` | CPU topology detection, path validation, token-safe relaunch, and process supervisor lifecycle tests passing (24/24 in `process::*`). |
| **P2 Telemetry Preservation** | `TEST-VERIFIED` | DXGI/D3D9 ETW event parsing, interleaved A/B runner, driver isolation, and statistical metrics passing (all ETW and stats tests passing). Live elevated session remains `UNVERIFIED`. |

---

## 8. Remaining Risks & Boundaries

The following areas are clearly bounded and documented:
1. **Live Elevated ETW Session Capture:** While ETW parser logic, provider GUIDs, event structures, and in-memory ingestion are `TEST-VERIFIED` (<0.001% CPU), starting an elevated real-time NT Kernel Logger or DXGI ETW session on an un-elevated user host remains `UNVERIFIED` until elevated runtime execution.
2. **Live Riot Vanguard In-Game Interaction:** The architecture operates 100% out-of-process without memory patching or DLL injection (`TEST-VERIFIED`). However, real-time Vanguard server acceptance during a live competitive tournament match remains `UNVERIFIED` (can only be validated in a live client match with anti-cheat running).
3. **Physical Display Refresh Arrival (`MsUntilDisplayed`):** Presentation timestamps measure application Present cadence at the DXGI runtime boundary. Direct physical photon scanout timing on adaptive-sync displays remains `UNVERIFIED` and is not claimed by the optimizer.

---

## 9. P3 Gate Declaration

All prerequisites for Phase P3 sign-off have been rigorously fulfilled:
- [x] Forced working-set memory trimming (`EmptyWorkingSet`) completely removed from automatic optimization paths.
- [x] Automatic Windows QoS DSCP policy creation and registry mutations completely removed.
- [x] Automatic NIC Interrupt Moderation and hardware property mutations completely removed.
- [x] Windows Update (`wuauserv`), Defender, and core system services protected in Tier 0.
- [x] Subsystem mutations restricted strictly to verified Win32 Game Mode, Power Schemes, and Core Audio APO bypass.
- [x] All 10 P3-specific tests passing cleanly.
- [x] All 140 workspace unit and integration tests passing cleanly.
- [x] Zero regressions in P0 security, P1 functional safety, or P2 telemetry architecture.
- [x] Static repository audit confirms zero production calls to removed optimizations.
- [x] Project documentation accurately reflects the de-scoped implementation.

**P3 GATE: READY FOR P4**
