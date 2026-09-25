# Phase P4: Final Cleanup & P5 Freeze Audit

**Project:** VALORANT Performance Optimizer  
**Audit Phase:** P4 Final Cleanup & P5 Freeze  
**Repository:** [Godwynski/valorant-optimizer](https://github.com/Godwynski/valorant-optimizer)  
**Accepted Baselines:** P0 ACCEPTED, P1 ACCEPTED, P2 ACCEPTED, P3 ACCEPTED  
**Audit Date:** 2026-09-26  

---

## 1. Project Phase & Gate Status

```text
P0 ACCEPTED
P1 ACCEPTED
P2 ACCEPTED
P3 ACCEPTED
P4 GATE: RELEASE CANDIDATE
P5 STATUS: NOT STARTED
```

### Final Gate Statement:
The project remains firmly classified as **`P4 GATE: RELEASE CANDIDATE`**. It is **not** declared `RELEASE READY` because live external validations (commercial CA Authenticode signing, clean bare-metal VM execution, and live VirusTotal API submission) remain unperformed in this automated offline build environment. 

---

## 2. P5 Freeze & Repository Audit

A comprehensive search of the repository and its complete Git history was conducted for:
- `P5`, `PHASE_11`, `P5_`, `p5`, `phase 11`
- Any future/hypothetical P5 source files, tasks, or branches

### Findings:
1. **Source Code:** Zero P5 source files, branches, or modules exist. The only substring match in the codebase is `p50_frame_time_ms` (the 50th percentile / median frame time metric in [`crates/val-opt-shared/src/benchmarking/stats.rs`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/crates/val-opt-shared/src/benchmarking/stats.rs)).
2. **Task & Phase Specifications:** Zero P5 tasks or specifications exist in [`TASKS.md`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/TASKS.md) or [`PROJECT_STATE.md`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/PROJECT_STATE.md).
3. **Commit History:** Zero commits have touched, referenced, or implemented any P5 items.
4. **Status Determination:**
   ```text
   P5 STATUS: NOT STARTED
   ```
   Zero unauthorized P5 work was created, continued, or planned. The project remediation stops strictly at Phase P4.

---

## 3. VC++ Runtime Dependency: Investigation & Resolution

### 3.1 Current Dependency Audit
PE imports inspected via `dumpbin /imports` revealed:
- `ucrtbase.dll`: Universal C Runtime (inbox on all Windows 10/11 installations since build 10240).
- `VCRUNTIME140.dll`: Dynamic MSVC C Runtime (Microsoft Visual C++ 2015–2022 x64 Redistributable).

### 3.2 Why the Dependency Exists
The Rust compiler targeting `x86_64-pc-windows-msvc` dynamically links against `VCRUNTIME140.dll` by default to provide standard MSVC ABI compatibility, structured exception handling (`__CxxFrameHandler3`), and runtime support required by Slint UI and native Windows subsystems.

### 3.3 Evaluation of Packaging Solutions
- **Option A (Bundle Redistributable Installer):** Bundling Microsoft's `vc_redist.x64.exe` (approx. 25 MB) would quadruple the installer package size (from 6.5 MB to over 31 MB) and requires specific Microsoft redistribution licensing compliance.
- **Option B (Proactive Detection & Guided Installation — SELECTED):** Detect `VCRUNTIME140.dll` during installer initialization. If absent, warn the user and provide the official Microsoft download URL.
- **Option C (Static CRT Linkage via `+crt-static`):** Evaluated. Attempting static CRT linking (`/MT`) failed due to absence of static Universal CRT libraries (`libucrt.lib`) in the build environment, and introduces known boundary issues across dynamic OpenGL/DirectX driver interfaces.

### 3.4 Selected Implementation Details
Implemented in [`installer/setup.iss`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/installer/setup.iss):
```pascal
function IsVCRuntimeInstalled(): Boolean;
begin
  Result := FileExists(ExpandConstant('{sys}\vcruntime140.dll'));
end;

function InitializeSetup(): Boolean;
var
  Msg: String;
begin
  Result := True;
  if not IsVCRuntimeInstalled() then
  begin
    if not WizardSilent() then
    begin
      Msg := 'Notice: Microsoft Visual C++ 2015-2022 x64 Redistributable was not detected on this system.' + #13#10 + #13#10 +
             'VALORANT Performance Optimizer requires VCRUNTIME140.dll to run.' + #13#10 + #13#10 +
             'You can download the official runtime from Microsoft at:' + #13#10 +
             'https://aka.ms/vs/17/release/vc_redist.x64.exe' + #13#10 + #13#10 +
             'Do you wish to continue with the installation anyway?';
      if MsgBox(Msg, mbConfirmation, MB_YESNO) = IDNO then
      begin
        Result := False;
      end;
    end;
  end;
end;
```

### 3.5 Failure Behavior on Systems Lacking the Runtime
On a bare Windows installation lacking `VCRUNTIME140.dll`:
- The installer alerts the user proactively during setup initialization before placing files.
- If bypassed, the Windows process loader fails on launch at `LdrpProcessInitialization` with `STATUS_DLL_NOT_FOUND` (`0xC0000135`).

---

## 4. Rigorous Uninstaller Safety Classification

In accordance with release audit instructions, uninstallation safety is divided into three distinct verification tiers:

1. **Uninstall mechanism / static audit = VERIFIED**
   - [`installer/setup.iss`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/installer/setup.iss) enforces:
     - `[UninstallRun]` executes `val-opt-cli.exe rollback` before file removal.
     - Process termination uses explicit system path `"{sys}\taskkill.exe" /F /IM val-opt-core.exe /IM val-opt-gui.exe /T` (preventing PATH search-order hijacking).
     - `[UninstallDelete]` removes only `{commonappdata}\ValorantOptimizer` and empty `{app}`.
     - Zero wildcard deletions (`*.*`) or arbitrary parent directory deletions.
     - Canonical path restrictions and junction/reparse point rejection enforced by `val-opt-cli` rollback engine.
2. **Local lifecycle simulation = TEST-VERIFIED**
   - Simulated deployment, CLI rollback execution, Authenticode signature retention, and clean folder deletion passed 100% via [`tests/installer_lifecycle_test.ps1`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/tests/installer_lifecycle_test.ps1).
3. **Real clean Windows VM uninstall = UNVERIFIED**
   - Because no isolated clean Windows VM is available on this build machine, end-to-end Windows uninstallation on a bare clean OS is formally **`UNVERIFIED`**.

---

## 5. Clean Windows VM Verification Status

No hypervisor (Hyper-V, VirtualBox, VMware) or clean VM is available on the local host. In adherence to strict integrity guidelines, clean VM verification is **not simulated**:
- Clean VM installation = **`UNVERIFIED`**
- Clean VM launch = **`UNVERIFIED`**
- Clean VM uninstall = **`UNVERIFIED`**
- Clean VM upgrade = **`UNVERIFIED`**

---

## 6. Code Signing & Authenticode Status

Inspected via `Get-AuthenticodeSignature`:
- `val-opt-gui.exe`: Authenticode signed (Local Self-Signed Certificate `1ADAD71BCDB535754F853156A4045486106EB813`)
- `val-opt-core.exe`: Authenticode signed (Local Self-Signed Certificate `1ADAD71BCDB535754F853156A4045486106EB813`)
- `val-opt-cli.exe`: Authenticode signed (Local Self-Signed Certificate `1ADAD71BCDB535754F853156A4045486106EB813`)
- `ValorantOptimizer_Setup_0.1.0.exe`: Authenticode signed (Local Self-Signed Certificate `1ADAD71BCDB535754F853156A4045486106EB813`)
- **Commercial Authenticode Status:** **`UNVERIFIED` (State B)**
- **RFC 3161 Timestamping:** Configured and tested against `http://timestamp.digicert.com`.

No fake CA or SmartScreen trust is claimed.

---

## 7. VirusTotal & External Scanning Status

- All occurrences of fabricated historical strings (e.g., `"0/70"`, `"clean"`, `"0 detections"`, fake submission IDs) were eradicated from the codebase.
- **VirusTotal Status:** **`UNVERIFIED`** (No automated external API submission was performed).

---

## 8. P0–P3 Regression Test Results

Full workspace regression test executed:
```powershell
cargo test --workspace
```
**Result:** **140 PASSED, 0 FAILED, 0 IGNORED**

### Verification of Prohibited Mutations:
Confirmed that previously removed harmful mutations remain completely absent from the automatic optimization pipeline:
- `EmptyWorkingSet` / working-set trimming: **ABSENT** (Tested: `test_p3_no_automatic_empty_working_set`)
- Network DSCP / QoS mutation: **ABSENT** (Tested: `test_p3_no_automatic_dscp_or_qos_mutation`)
- NIC Interrupt Moderation mutation: **ABSENT** (Tested: `test_p3_no_automatic_nic_or_interrupt_moderation_mutation`)
- Automatic pausing of `wuauserv` (Windows Update): **ABSENT** (Tested: `test_termination_refusal_for_tier0`)

---

## 9. Final Release Artifact Manifest

Hashes recomputed from live on-disk files match [`SHA256SUMS.txt`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/SHA256SUMS.txt):

| File | Size (Bytes) | SHA-256 Digest | Status |
|---|---|---|---|
| [`val-opt-gui.exe`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/target/release/val-opt-gui.exe) | 11,457,168 | `8ce3351a6360207789c8b162f268c9dfe401d0014148db4a3232e2be29c6c729` | **VERIFIED** |
| [`val-opt-core.exe`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/target/release/val-opt-core.exe) | 1,417,360 | `8922148ac43ccd1fbac9dc969f2c7ea1d50b955ab90ff7caea008179bf02518c` | **VERIFIED** |
| [`val-opt-cli.exe`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/target/release/val-opt-cli.exe) | 886,928 | `58f21b0a4cec1c3b0869a2ceb84a59e863c8a631dc04f5f23c42ea9cf608a967` | **VERIFIED** |
| [`ValorantOptimizer_Setup_0.1.0.exe`](file:///c:/Users/Godwyn/Documents/Projects/valorant%20optimize/installer/output/ValorantOptimizer_Setup_0.1.0.exe) | 6,531,568 | `7e19a731cda6a5d5defbbbfc387b338c985c6556efdea34f4fdc9779aed12c8e` | **VERIFIED** |

---

## 10. Remaining Unresolved P4 Release Limitations

The following items are the exact, exhaustive list of unresolved items preventing an upgrade from `RELEASE CANDIDATE` to `RELEASE READY`:

1. **Commercial Authenticode Certificate:** Requires an EV/OV code-signing certificate from a public commercial CA to eliminate Windows Defender SmartScreen untrusted publisher warnings.
2. **Clean Windows VM Execution:** Requires testing on an independent, bare-metal or hypervisor-hosted clean Windows 11 installation without developer tools.
3. **External VirusTotal Scan:** Requires submitting release binary hashes to VirusTotal API or web portal.
4. **Live Vanguard Match Validation:** Requires live match validation on production Riot Games servers.
5. **Real-world Multi-Version Upgrade:** Requires executing version N to version N+1 upgrade on a clean consumer Windows OS.
6. **Live VC++ Runtime Absence Validation:** Requires validating user prompt and fallback behavior on a clean machine physically lacking `VCRUNTIME140.dll`.

---

## 11. Final Gate Decision

```
P4 GATE: RELEASE CANDIDATE
```

All internal engineering, safety boundaries, and packaging mechanisms are verified. The freeze on P5 is in full effect. No P5 implementation will be performed.
