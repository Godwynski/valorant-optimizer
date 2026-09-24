# Phase 5: Network Optimization & Diagnostics

---

## 1. Phase Objective
Build the host-side network optimization and diagnostics suite. This disables Energy Efficient Ethernet (EEE) and Green Ethernet on the physical NIC, optimizes Interrupt Moderation, verifies Receive Side Scaling (RSS), disables Flow Control, registers a DSCP 46 QoS policy for VALORANT UDP packets, and provides an objective bufferbloat test tool.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-05`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 4 complete.
- **Target Deliverable:** Network adapter configurator and UDP diagnostics suite with zero network dropouts during match initialization.
- **QA File:** `QA/QA_PHASE_05.md`

---

## 3. Tasks Breakdown

### `TASK-P05-001`: NIC Property Configurator (EEE & Interrupt Moderation)
- **Objective:** Programmatically configure physical network adapter to disable Energy Efficient Ethernet (EEE) and tune Interrupt Moderation.
- **Files Involved:**
  - `crates/val-opt-core/src/network/adapter.rs`
- **Requirements:** Query active adapter via CIM/WMI/NetAdapter, set `*EEE` = 0 (Disabled), `*GreenEthernet` = 0 (Disabled), and `*InterruptModeration` = Low or Disabled; record original settings.
- **Verification Method:** Check adapter advanced properties in Windows Device Manager before and after application.
- **Completion Criteria:** Properties applied without network link drop lasting > 1.5 seconds; full rollback supported.
- **Status:** `COMPLETE`

### `TASK-P05-002`: Flow Control & RSS Verifier
- **Objective:** Verify Receive Side Scaling (RSS) is active and disable 802.3x Flow Control on the primary network interface.
- **Files Involved:**
  - `crates/val-opt-core/src/network/flow_control.rs`
- **Requirements:** Ensure Flow Control is disabled (to avoid UDP packet stalling) and RSS has $\ge 4$ queues allocated.
- **Verification Method:** Verify settings via PowerShell `Get-NetAdapterAdvancedProperty` and netsh.
- **Completion Criteria:** Successfully disables Flow Control and validates RSS queue distribution.
- **Status:** `COMPLETE`

### `TASK-P05-003`: Windows QoS DSCP Policy Registrar
- **Objective:** Register a local QoS policy tagging VALORANT UDP outbound packets with DSCP 46 (Expedited Forwarding).
- **Files Involved:**
  - `crates/val-opt-core/src/network/qos.rs`
- **Requirements:** Create Group Policy / Windows QoS policy rule matching `VALORANT-Win64-Shipping.exe` port range 7000-8000 with DSCP 46; ensure rollback removes rule.
- **Verification Method:** Verify policy entry using PowerShell `Get-NetQosPolicy` and check IP header DSCP field via Wireshark.
- **Completion Criteria:** QoS policy successfully registered and unregisters cleanly on rollback.
- **Status:** `COMPLETE`

### `TASK-P05-004`: Bufferbloat & Network Health Diagnostic Runner
- **Objective:** Implement standalone network quality test measuring loaded vs unloaded ping and jitter.
- **Files Involved:**
  - `crates/val-opt-core/src/network/bufferbloat.rs`
- **Requirements:** Measure baseline RTT to Riot edge servers, initiate asynchronous saturating throughput burst, measure induced latency spike, and compute bufferbloat grade (A+ to F).
- **Verification Method:** Run diagnostic against test servers; verify score consistency across multiple runs.
- **Completion Criteria:** Generates diagnostic report displaying unloaded ping, loaded ping, jitter, and bufferbloat grade.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 4 tasks marked `COMPLETE`.
2. Unit tests and integration tests pass with 100% success rate.
3. Checklist in `QA/QA_PHASE_05.md` verified with actual logged output.
4. STOP and await user instruction to proceed to Phase 6.
