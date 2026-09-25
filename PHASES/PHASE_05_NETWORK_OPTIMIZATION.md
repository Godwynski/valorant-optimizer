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

### `TASK-P05-001`: NIC Property Configurator & Diagnostics
- **Objective:** Programmatically query physical network adapter properties and validate hardware configuration.
- **Files Involved:**
  - `crates/val-opt-core/src/network/adapter.rs`
- **Requirements:** Query active adapter via CIM/WMI/NetAdapter; inspect property values; support rollback for any legacy applied settings.
- **Phase P3 De-Scoping Note:** Automatic Interrupt Moderation mutation was PERMANENTLY REMOVED in Phase P3 (`TASK-OPT-03`). Disabling interrupt moderation induces high packet interrupt rates and severe DPC latency spikes on Core 0 under throughput, destabilizing frame pacing. All automatic NIC driver mutations have been removed from the default optimization path; adapter settings remain at OEM driver defaults.
- **Verification Method:** Check adapter advanced properties in Windows Device Manager before and after inspection.
- **Completion Criteria:** Properties queried without network link drop lasting > 1.5 seconds; automatic driver mutations eliminated.
- **Status:** `COMPLETE (REMEDIATED IN P3)`

### `TASK-P05-002`: Flow Control & RSS Verifier
- **Objective:** Verify Receive Side Scaling (RSS) is active and inspect Flow Control on the primary network interface.
- **Files Involved:**
  - `crates/val-opt-core/src/network/flow_control.rs`
- **Requirements:** Ensure Flow Control query functions reliably and RSS has $\ge 4$ queues allocated.
- **Verification Method:** Verify settings via PowerShell `Get-NetAdapterAdvancedProperty` and netsh.
- **Completion Criteria:** Successfully reads Flow Control and validates RSS queue distribution.
- **Status:** `COMPLETE`

### `TASK-P05-003`: Windows QoS DSCP Policy Diagnostic (Read-Only)
- **Objective:** Inspect registered Windows QoS policies without mutating system network policies.
- **Files Involved:**
  - `crates/val-opt-core/src/network/qos.rs`
- **Requirements:** Query active NetQosPolicy rules via PowerShell `Get-NetQosPolicy`.
- **Phase P3 De-Scoping Note:** Automatic creation of DSCP 46 QoS policies was PERMANENTLY REMOVED in Phase P3 (`TASK-OPT-02`). Residential ISPs and consumer routers routinely strip or reset DSCP tags upon ingress, and commercial carrier traffic policers drop unauthorized Expedited Forwarding packets. The module is retained strictly as a passive, read-only diagnostic tool (`query_qos_policy`).
- **Verification Method:** Verify policy entry using PowerShell `Get-NetQosPolicy` and check IP header DSCP field via Wireshark.
- **Completion Criteria:** Read-only inspection operates without registry modification or packet tagging.
- **Status:** `COMPLETE (REMEDIATED IN P3)`

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
