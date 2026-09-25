# DPAPI / HMAC Snapshot Security & Key Lifecycle Model

**Specification for TASK-SEC-04**  
**Classification:** Core Security Architecture  
**Applies to:** `val-opt-core`, `val-opt-cli`, `val-opt-shared`

---

## 1. Threat Model & Security Objectives

### Threats Mitigated:
1. **Unprivileged Modification (LPE Vector):** A standard unprivileged local user or malicious process modifying `%ProgramData%\ValorantOptimizer\snapshot.json` to inject arbitrary executable paths, services, or registry values that would then be executed/restored by an elevated daemon (`SYSTEM`) or administrator running rollback.
2. **Offline / Tampered State Injection:** Replacing or altering the state snapshot while the optimizer daemon is offline.
3. **Cross-Machine Replay:** Exporting a snapshot from one computer and replaying it on another machine to alter system settings.

### Cryptographic Foundation:
- **Algorithm:** HMAC-SHA256 (RFC 2104 / FIPS 198) over canonical UTF-8 JSON payload.
- **Key Generation:** 256-bit cryptographically secure pseudorandom key (CSPRNG via Win32 `BCryptGenRandom` or `ProcessPrng`).
- **At-Rest Protection:** Windows Data Protection API (DPAPI) via `CryptProtectData` with `CRYPTPROTECT_LOCAL_MACHINE` flag and application-specific salt entropy.

---

## 2. Key Lifecycle & Operational Model

### 2.1 Key Storage Location
- **Filesystem Path:** `%ProgramData%\ValorantOptimizer\snapshot.key`
- **Data Format:** Encrypted binary DPAPI blob (`DATA_BLOB`) containing:
  - 16-byte magic header: `VALOPT_KEY_V1\0\0\0`
  - 32-byte (256-bit) raw HMAC-SHA256 secret key
  - 16-byte key generation timestamp (UNIX epoch milliseconds)
- **Entropy Salt:** Fixed internal application entropy: `VAL-OPT-DPAPI-MACHINE-ENTROPY-v1`.

### 2.2 Windows Security Identity & Access Control
- **Execution Contexts:**
  - `val-opt-core` daemon: Runs as `NT AUTHORITY\SYSTEM` or elevated Administrator.
  - `val-opt-cli rollback`: Runs as elevated Administrator (`BUILTIN\Administrators`).
- **DPAPI Scope:**
  - Must use `CRYPTPROTECT_LOCAL_MACHINE` (flag `0x4`).
  - *Rationale:* User-scoped DPAPI encrypted by `SYSTEM` cannot be decrypted by an elevated Administrator user account, and vice-versa. Machine-scoped DPAPI allows any process with administrative privileges (`SYSTEM` or elevated `Administrators`) on the *same physical machine* to decrypt the key, while completely denying access to standard users and external computers.
- **Filesystem DACL (`snapshot.key` and `%ProgramData%\ValorantOptimizer`):**
  - Inherited permissions explicitly blocked (`D:P`).
  - `NT AUTHORITY\SYSTEM`: Full Control (`GA`).
  - `BUILTIN\Administrators`: Full Control (`GA`).
  - `BUILTIN\Users`: **NO ACCESS** (no read, no write, no traverse).
  - `Authenticated Users`: **NO ACCESS**.

### 2.3 Reinstallation Behavior
- **Clean Install (No active snapshot exists):**
  - Installer creates fresh `%ProgramData%\ValorantOptimizer` directory with hardened DACL.
  - Existing stale `snapshot.key` (if any without a matching snapshot) is eradicated.
- **Reinstall over an uncommitted snapshot (System mid-optimization):**
  - If `snapshot.json` and `snapshot.key` exist prior to installer running:
    1. Installer invokes `val-opt-cli rollback` using the existing key to cleanly restore system baseline.
    2. After clean rollback, both `snapshot.json` and `snapshot.key` are deleted.
    3. New installation proceeds.

### 2.4 Machine Migration Behavior
- System snapshots contain hardware-specific UUIDs, NDIS adapter GUIDs, and display identifiers.
- **Snapshots must NEVER be migrated across machines.**
- Because the key is protected with `CRYPTPROTECT_LOCAL_MACHINE`, copying `snapshot.json` and `snapshot.key` to a different machine causes `CryptUnprotectData` to fail with `NTE_BAD_KEY_STATE` (0x8009000B) or `SEC_E_DECRYPT_FAILURE`.
- When cross-machine decryption fails, the recovery service:
  1. Immediately halts processing.
  2. Flags the snapshot as alien/untrusted.
  3. Quarantines the file (renaming to `snapshot.alien.<timestamp>.bak`).
  4. Refuses to apply any changes.

### 2.5 Key Rotation Policy
- **Ephemeral Session Keying:**
  - A new random 256-bit HMAC key is generated each time `SnapshotEngine::capture_system_baseline()` creates a new optimization snapshot.
  - The key is written to `snapshot.key` and protected via DPAPI.
- **Key Destruction:**
  - When the optimization session ends normally (game exit $\to$ restoration) OR when an emergency rollback finishes:
    1. Memory holding the raw key is overwritten with zeros (`zeroize`).
    2. `snapshot.json` is deleted.
    3. `snapshot.key` is securely deleted from disk (`SetFileInformationByHandle` / zero-fill before deletion).

### 2.6 Crash-Recovery / Boot-Recovery Behavior
1. Upon system boot or daemon startup, `CrashRecoveryService::check_and_recover` checks for `%ProgramData%\ValorantOptimizer\snapshot.json`.
2. If `snapshot.json` is present:
   - Check for `%ProgramData%\ValorantOptimizer\snapshot.key`.
   - Read and decrypt the key via DPAPI machine scope.
   - Read `snapshot.json` and verify its `hmac_signature` matches `HMAC-SHA256(key, canonical_json_payload)`.
   - Use **constant-time byte comparison** to prevent timing attacks.
3. If HMAC verification succeeds:
   - Execute full transactional rollback.
   - Delete `snapshot.json` and `snapshot.key`.
   - Log successful recovery event to `recovery.log`.

### 2.7 Behavior When Key is Unavailable or Corrupted
If `snapshot.json` exists but any of the following occur:
- `snapshot.key` is missing.
- `snapshot.key` DPAPI decryption fails (`CryptUnprotectData` error).
- `snapshot.key` header is invalid.
- `snapshot.json` HMAC verification fails (computed HMAC $\ne$ recorded HMAC).

**Failure Protocol:**
1. **FAIL SECURE (NO AUTOMATIC EXECUTION):**
   - The recovery service **MUST NOT** execute any commands, restore arbitrary registry keys, or relaunch executables from an unverified snapshot.
2. **Security Alert Logging:**
   - Log `CRITICAL_SECURITY_ALERT` in `recovery.log` detailing the integrity violation (expected vs. actual, timestamp, caller PID).
3. **Quarantine:**
   - Move the untrusted snapshot to `snapshot.tampered.<timestamp>.corrupt`.
   - Remove any invalid key file.
4. **User Notification:**
   - Emit an error report via IPC to GUI and console in CLI:
     `"CRITICAL: System snapshot integrity check failed. The snapshot was altered or key was lost. Automatic rollback aborted for security. Manual verification required."`
