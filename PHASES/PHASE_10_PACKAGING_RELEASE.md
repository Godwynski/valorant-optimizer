# Phase 10: Production Packaging & Security Audit

---

## 1. Phase Objective
Produce the production-ready distribution package. This includes binary hardening (ASLR, DEP, CFG), Microsoft Authenticode digital code-signing to prevent antivirus false positives, and a clean, lightweight one-click installer (WiX Toolset or Inno Setup) with guaranteed complete uninstallation and rollback.

---

## 2. Phase Metadata
- **Phase ID:** `PHASE-10`
- **Status:** `COMPLETE`
- **Dependencies:** Phase 9 complete.
- **Target Deliverable:** Hardened production installer package with real SHA-256 manifests, State B Authenticode architecture, and documented rollback.
- **QA File:** `QA/QA_PHASE_10.md`

---

## 3. Tasks Breakdown

### `TASK-P10-001`: Static Security Analysis & Binary Hardening
- **Objective:** Conduct security audit of compiled native binaries.
- **Files Involved:**
  - Compiler release flags
  - `Cargo.toml`
- **Requirements:** Enable ASLR, DEP/NX, Control Flow Guard (CFG), and stack protection flags; run `cargo audit` to verify zero vulnerable dependencies.
- **Verification Method:** Run `dumpbin /headers` and vulnerability scanner on release binaries.
- **Completion Criteria:** Binaries fully hardened with zero security vulnerabilities.
- **Status:** `COMPLETE`

### `TASK-P10-002`: Digital Code Signing & Antivirus Whitelist Verification
- **Objective:** Apply Microsoft Authenticode digital signature to all binaries and installer.
- **Files Involved:**
  - Release pipeline signing script (`scripts/sign_binaries.ps1`)
- **Requirements:** Sign `val-opt-core.exe`, `val-opt-gui.exe`, and `val-opt-cli.exe` with valid code-signing certificate; provide clean malware submission tooling without fabricated scores.
- **Verification Method:** Verify digital signature via `Get-AuthenticodeSignature` (State B: local test cert; commercial CA marked UNVERIFIED; VirusTotal marked UNVERIFIED).
- **Completion Criteria:** Signing pipeline validated; mock scores eradicated.
- **Status:** `COMPLETE`

### `TASK-P10-003`: Production Installer Packaging (Inno Setup)
- **Objective:** Create lightweight, production-grade installer package.
- **Files Involved:**
  - `installer/setup.iss`, `installer/build_installer.ps1`
- **Requirements:** One-click installation with UAC prompt, installation of core binaries, optional desktop shortcut, clean uninstaller that guarantees full rollback before removal.
- **Verification Method:** Isolated lifecycle simulation test (`tests/installer_lifecycle_test.ps1`); real Clean VM documented as UNVERIFIED.
- **Completion Criteria:** Clean packaging and simulated removal with zero registry or file leftovers.
- **Status:** `COMPLETE`

---

## 4. Phase Completion Gate
1. All 3 tasks marked `COMPLETE` under P4 scope.
2. Binary security audit passes with zero warnings.
3. Clean VM installation procedure documented (live test UNVERIFIED).
4. Checklist in `QA/QA_PHASE_10.md` verified with actual logged output.
5. P4 Release Gate: `P4 GATE: RELEASE CANDIDATE`.
