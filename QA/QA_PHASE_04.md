# QA Checklist & Results: Phase 4 — Process & Service Management Engine

---

## 1. Phase Gate Status
- **Status:** `COMPLETE`
- **Execution Date:** 2026-09-25
- **Sign-Off:** `PASS` (Lead Performance Engineer & Security Compliance)

---

## 2. Pre-Verification Checklist
- [x] Phase 3 is verified and signed off.
- [x] Whitelist enforcement blocks attempts to kill Tier 0 processes (Vanguard, DWM, System, Windows Update).
- [x] Services (`SysMain`, `DiagTrack`, `Spooler`) pause cleanly and resume upon session end (`wuauserv` protected in Tier 0).
- [x] Game supervisor applies priority `High` and terminates `RiotClientUx.exe`.

---

## 3. Concrete Verification Items & Test Results

| Test ID | Test Target | Verification Command / Procedure | Expected Result | Actual Result | Pass/Fail |
| :--- | :--- | :--- | :--- | :--- | :--- |
| `TC-P04-01` | Whitelist Guard | `test_vanguard_and_system_protection` & `test_termination_refusal_for_tier0` | Hard error; termination blocked | Invariant check returned `Err(SafetyViolationError)`; `vgc.exe`, `dwm.exe`, `csrss.exe` protected | `PASS` |
| `TC-P04-02` | Process Purge | `test_graceful_termination_of_spawned_process` | Graceful exit with no lingering process handles | `WM_CLOSE` + 1000ms wait terminated child process cleanly; status verified `STILL_ACTIVE` cleared | `PASS` |
| `TC-P04-03` | Service Pause & Resume | `test_service_protection_barrier` & `val-opt-cli pause-services` / `resume-services` | Tier 0 services protected; Tier 3 services stopped and resumed | Tier 0 services (`RpcSs`, `vgc`) protected with error; SCM pause/resume verified | `PASS` |
| `TC-P04-04` | Riot Client CEF | `ProcessSupervisor::terminate_riot_client_cef_ui` | CEF UI terminated; `RiotClientServices.exe` remains alive | `RiotClientUx.exe` targeted while `RiotClientServices.exe` explicitly guarded in Tier 0 whitelist | `PASS` |
| `TC-P04-05` | Game Priority | `test_optimize_process_priority` | Priority Class = `HIGH_PRIORITY_CLASS` (0x00000080) | Verified `GetPriorityClass(handle) == HIGH_PRIORITY_CLASS.0` on active target process | `PASS` |

---

## 4. Observed Defects & Remediation Log
1. **Parallel Test Preemption:** Busy-waiting in synthetic DirectX app caused CPU scheduling noise during multi-threaded test runs.
   - *Resolution:* Added `with_virtual_time(true)` for instantaneous and deterministic synthetic execution without busy-wait loops.
2. **Missing CLI Resume Command:** Tier 3 services could be paused but lacked dedicated CLI resume subcommand.
   - *Resolution:* Implemented `val-opt-cli resume-services` using `restore_tier3_services`.

---

## 5. Phase Sign-Off Criteria
- [x] All 5 test cases marked `PASS`.
- [x] Zero impact on Windows shell or Vanguard integrity.
- [x] All Phase 4 tasks marked `COMPLETE` in `TASKS.md` and `PROJECT_STATE.md`.
