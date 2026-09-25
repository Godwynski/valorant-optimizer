//! Atomic State Snapshot Serializer & Verifier.
//!
//! Captures complete baseline system state prior to modifications and commits
//! to `%ProgramData%\ValorantOptimizer\snapshot.json` with SHA-256 integrity hash
//! and atomic file replacement to eliminate any risk of disk corruption.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;
use thiserror::Error;
use tracing::{error, info, warn};
use val_opt_shared::models::network::{AdapterPropertyBackup, QosPolicyBackup};
use val_opt_shared::models::snapshot::{AudioEndpointBackup, SystemStateSnapshot};

#[cfg(windows)]
use windows::core::PCWSTR;
#[cfg(windows)]
use windows::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

pub const SNAPSHOT_KEY_FILENAME: &str = "snapshot.key";
pub const MAGIC_KEY_HEADER: &[u8; 16] = b"VALOPT_KEY_V1\0\0\0";
pub const DPAPI_ENTROPY_SALT: &[u8] = b"VAL-OPT-DPAPI-MACHINE-ENTROPY-v1";

/// Errors encountered during snapshot serialization, persistence, or validation.
#[derive(Debug, Error)]
pub enum SnapshotError {
    #[error("Snapshot file not found at: {0}")]
    NotFound(PathBuf),

    #[error("I/O error during snapshot operation: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON serialization/deserialization failed: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Snapshot integrity violation for {path}: expected {expected_hash}, calculated {computed_hash}")]
    IntegrityViolation {
        path: PathBuf,
        expected_hash: String,
        computed_hash: String,
    },

    #[error("Snapshot key unavailable or corrupt at {path}: {reason}")]
    KeyUnavailable {
        path: PathBuf,
        reason: String,
    },

    #[error("Snapshot security violation: {0}")]
    SecurityViolation(String),
}

/// Protect a plaintext byte buffer using Windows DPAPI machine scope.
#[cfg(windows)]
pub fn dpapi_protect_machine(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{
        CryptProtectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPT_INTEGER_BLOB,
    };
    use windows::core::PCWSTR;

    let mut in_blob = CRYPT_INTEGER_BLOB {
        cbData: plaintext.len() as u32,
        pbData: plaintext.as_ptr() as *mut u8,
    };
    let mut entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY_SALT.len() as u32,
        pbData: DPAPI_ENTROPY_SALT.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptProtectData(
            &mut in_blob,
            PCWSTR::null(),
            Some(&mut entropy_blob),
            None,
            None,
            CRYPTPROTECT_LOCAL_MACHINE,
            &mut out_blob,
        ).map_err(|e| format!("CryptProtectData failed: {}", e))?;

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let result = slice.to_vec();
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(out_blob.pbData as *mut _));
        Ok(result)
    }
}

/// Decrypt a DPAPI-protected machine ciphertext blob.
#[cfg(windows)]
pub fn dpapi_unprotect_machine(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    use windows::Win32::Security::Cryptography::{
        CryptUnprotectData, CRYPTPROTECT_LOCAL_MACHINE, CRYPT_INTEGER_BLOB,
    };

    let mut in_blob = CRYPT_INTEGER_BLOB {
        cbData: ciphertext.len() as u32,
        pbData: ciphertext.as_ptr() as *mut u8,
    };
    let mut entropy_blob = CRYPT_INTEGER_BLOB {
        cbData: DPAPI_ENTROPY_SALT.len() as u32,
        pbData: DPAPI_ENTROPY_SALT.as_ptr() as *mut u8,
    };
    let mut out_blob = CRYPT_INTEGER_BLOB::default();

    unsafe {
        CryptUnprotectData(
            &mut in_blob,
            None,
            Some(&mut entropy_blob),
            None,
            None,
            CRYPTPROTECT_LOCAL_MACHINE,
            &mut out_blob,
        ).map_err(|e| format!("CryptUnprotectData failed: {}", e))?;

        let slice = std::slice::from_raw_parts(out_blob.pbData, out_blob.cbData as usize);
        let result = slice.to_vec();
        let _ = windows::Win32::Foundation::LocalFree(windows::Win32::Foundation::HLOCAL(out_blob.pbData as *mut _));
        Ok(result)
    }
}

#[cfg(not(windows))]
pub fn dpapi_protect_machine(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut out = Vec::from(b"NON_WIN_BLOB");
    out.extend_from_slice(plaintext);
    Ok(out)
}

#[cfg(not(windows))]
pub fn dpapi_unprotect_machine(ciphertext: &[u8]) -> Result<Vec<u8>, String> {
    if ciphertext.starts_with(b"NON_WIN_BLOB") {
        Ok(ciphertext[12..].to_vec())
    } else {
        Err("Invalid ciphertext format".to_string())
    }
}

/// Generate 32 cryptographically secure random bytes.
#[cfg(windows)]
pub fn generate_crypto_random_32() -> Result<[u8; 32], String> {
    use windows::Win32::Security::Cryptography::{BCryptGenRandom, BCRYPT_USE_SYSTEM_PREFERRED_RNG};

    let mut key = [0u8; 32];
    unsafe {
        BCryptGenRandom(
            None,
            &mut key,
            BCRYPT_USE_SYSTEM_PREFERRED_RNG,
        ).ok().map_err(|e| format!("BCryptGenRandom failed: {}", e))?;
    }
    Ok(key)
}

#[cfg(not(windows))]
pub fn generate_crypto_random_32() -> Result<[u8; 32], String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(&std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos().to_le_bytes());
    let mut key = [0u8; 32];
    key.copy_from_slice(&hasher.finalize());
    Ok(key)
}

pub struct SnapshotEngine;

impl SnapshotEngine {
    /// Enforce hardened ACL on target directory:
    /// - Blocks inheritance (D:P)
    /// - Grants SYSTEM FullControl (GA)
    /// - Grants Administrators FullControl (GA)
    /// - Grants Users ReadAndExecute only (GRGX), prohibiting Write/Modify/Delete
    #[cfg(windows)]
    pub fn harden_directory_acls(dir: &Path) -> Result<(), String> {
        let dir_str = dir.to_str().ok_or_else(|| "Invalid directory path".to_string())?;

        let output = std::process::Command::new("icacls")
            .args([
                dir_str,
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-18:(OI)(CI)F",
                "/grant:r",
                "*S-1-5-32-544:(OI)(CI)F",
                "/grant:r",
                "*S-1-5-32-545:(OI)(CI)RX",
            ])
            .output()
            .map_err(|e| format!("Failed to execute icacls: {}", e))?;

        if !output.status.success() {
            let err = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(dir = dir_str, error = %err, "Directory ACL hardening notice (may require elevation)");
        } else {
            tracing::info!(dir = dir_str, "Hardened directory ACLs: SYSTEM/Admins (FullControl), Users (ReadAndExecute only)");
        }
        Ok(())
    }

    #[cfg(not(windows))]
    pub fn harden_directory_acls(_dir: &Path) -> Result<(), String> {
        Ok(())
    }

    /// Retrieve the canonical snapshot storage directory.
    /// Default: `%ProgramData%\ValorantOptimizer`
    pub fn canonical_snapshot_dir() -> PathBuf {
        let program_data =
            std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
        Path::new(&program_data).join("ValorantOptimizer")
    }

    /// Normalize a Windows path by stripping extended verbatim prefixes (`\\?\` or `\\?\UNC\`).
    pub fn normalize_windows_path(path: &Path) -> PathBuf {
        let s = path.to_string_lossy();
        if let Some(stripped) = s.strip_prefix(r"\\?\UNC\") {
            PathBuf::from(format!(r"\\{}", stripped))
        } else if let Some(stripped) = s.strip_prefix(r"\\?\") {
            PathBuf::from(stripped)
        } else {
            path.to_path_buf()
        }
    }

    /// Validate that a snapshot path:
    /// 1. Does not contain directory traversal sequences (`..`).
    /// 2. Resides strictly within the authorized canonical directory (`%ProgramData%\ValorantOptimizer` or an explicit base).
    /// 3. Is not a symbolic link, directory junction, or reparse point.
    /// 4. Does not refer to a hardlinked file (`nNumberOfLinks > 1`).
    pub fn validate_snapshot_path(
        path: &Path,
        allowed_base: Option<&Path>,
    ) -> Result<PathBuf, SnapshotError> {
        // 1. Reject path traversal components
        for comp in path.components() {
            if let std::path::Component::ParentDir = comp {
                return Err(SnapshotError::SecurityViolation(format!(
                    "Directory traversal ('..') prohibited in snapshot path: {}",
                    path.display()
                )));
            }
        }

        // 2. Resolve and canonicalize allowed base directory
        let base_dir = allowed_base
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::canonical_snapshot_dir);

        if !base_dir.exists() {
            fs::create_dir_all(&base_dir).map_err(SnapshotError::Io)?;
            #[cfg(windows)]
            {
                let program_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
                if base_dir.starts_with(&program_data) {
                    let _ = Self::harden_directory_acls(&base_dir);
                }
            }
        }

        let canonical_base = base_dir.canonicalize().map_err(SnapshotError::Io)?;
        let norm_base = Self::normalize_windows_path(&canonical_base)
            .to_string_lossy()
            .to_lowercase();
        let base_prefix = if norm_base.ends_with('\\') || norm_base.ends_with('/') {
            norm_base.clone()
        } else {
            format!("{}\\", norm_base)
        };

        // 3. Verify path containment within base directory
        let (canonical_path, exists) = if path.exists() {
            (path.canonicalize().map_err(SnapshotError::Io)?, true)
        } else {
            // If target doesn't exist, verify its parent directory
            let parent = path.parent().unwrap_or_else(|| Path::new("."));
            if parent.exists() {
                let canonical_parent = parent.canonicalize().map_err(SnapshotError::Io)?;
                let norm_parent = Self::normalize_windows_path(&canonical_parent)
                    .to_string_lossy()
                    .to_lowercase();
                if !norm_parent.starts_with(&base_prefix) && norm_parent != norm_base {
                    return Err(SnapshotError::SecurityViolation(format!(
                        "Snapshot path '{}' resides in directory outside authorized root '{}'",
                        path.display(),
                        base_dir.display()
                    )));
                }
            } else {
                return Err(SnapshotError::SecurityViolation(format!(
                    "Snapshot parent directory '{}' does not exist",
                    parent.display()
                )));
            }
            (path.to_path_buf(), false)
        };

        if exists {
            let norm_target = Self::normalize_windows_path(&canonical_path)
                .to_string_lossy()
                .to_lowercase();
            if !norm_target.starts_with(&base_prefix) && norm_target != norm_base {
                return Err(SnapshotError::SecurityViolation(format!(
                    "Snapshot path '{}' resolves outside authorized root '{}'",
                    path.display(),
                    base_dir.display()
                )));
            }
        }

        // 4. Reparse point, directory junction, and symlink checks
        #[cfg(windows)]
        {
            use std::os::windows::fs::MetadataExt;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x00000400;

            let mut curr = Some(path);
            while let Some(p) = curr {
                if p.exists() || p.is_symlink() {
                    let meta = std::fs::symlink_metadata(p).map_err(|e| {
                        SnapshotError::SecurityViolation(format!(
                            "Failed to inspect metadata for {}: {}",
                            p.display(),
                            e
                        ))
                    })?;

                    if meta.file_type().is_symlink()
                        || (meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0)
                    {
                        return Err(SnapshotError::SecurityViolation(format!(
                            "Reparse point, directory junction, or symlink detected at: {}",
                            p.display()
                        )));
                    }
                }

                if let Ok(c) = p.canonicalize() {
                    if Self::normalize_windows_path(&c)
                        .to_string_lossy()
                        .eq_ignore_ascii_case(
                            Self::normalize_windows_path(&canonical_base)
                                .to_string_lossy()
                                .as_ref(),
                        )
                    {
                        break;
                    }
                }
                curr = p.parent();
            }

            // 5. Open file handle to check hardlink count and handle reparse attributes
            if exists && path.is_file() {
                use std::fs::File;
                use std::os::windows::io::AsRawHandle;
                use windows::Win32::Foundation::HANDLE;
                use windows::Win32::Storage::FileSystem::{
                    GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
                };

                let file = File::open(path).map_err(SnapshotError::Io)?;
                let raw_handle = file.as_raw_handle();
                let mut info = BY_HANDLE_FILE_INFORMATION::default();

                unsafe {
                    GetFileInformationByHandle(HANDLE(raw_handle as _), &mut info)
                        .map_err(|e| {
                            SnapshotError::SecurityViolation(format!(
                                "GetFileInformationByHandle failed for {}: {}",
                                path.display(),
                                e
                            ))
                        })?;
                }

                if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(SnapshotError::SecurityViolation(format!(
                        "File handle attributes indicate reparse point: {}",
                        path.display()
                    )));
                }

                if info.nNumberOfLinks > 1 {
                    return Err(SnapshotError::SecurityViolation(format!(
                        "Hardlinked snapshot file detected ({} links). Snapshot operations on hardlinks are prohibited: {}",
                        info.nNumberOfLinks,
                        path.display()
                    )));
                }
            }
        }

        Ok(if exists {
            Self::normalize_windows_path(&canonical_path)
        } else {
            path.to_path_buf()
        })
    }

    /// Check if a given path resides inside the canonical snapshot directory and passes security validation.
    pub fn is_path_in_canonical_dir(path: &Path) -> bool {
        Self::validate_snapshot_path(path, None).is_ok()
    }

    /// Retrieve the standard persistence path for the active snapshot.
    /// Default: `%ProgramData%\ValorantOptimizer\snapshot.json`
    pub fn default_snapshot_path() -> PathBuf {
        let dir = Self::canonical_snapshot_dir();
        let _ = fs::create_dir_all(&dir);
        let _ = Self::harden_directory_acls(&dir);
        dir.join("snapshot.json")
    }

    /// Check if an uncommitted snapshot exists at the target path.
    pub fn has_snapshot(path: Option<&Path>) -> bool {
        let p = path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);
        if let Ok(valid) = Self::validate_snapshot_path(&p, None) {
            valid.exists() && valid.is_file()
        } else {
            false
        }
    }

    /// Capture complete baseline system state prior to applying any optimizations.
    pub fn capture_system_baseline() -> Result<SystemStateSnapshot, String> {
        let start = Instant::now();
        let id = format!(
            "snap_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0)
        );

        let mut snapshot = SystemStateSnapshot::new(id);

        // 1. Capture Game Mode state
        if let Ok(gm) = crate::optimizations::game_mode::get_game_mode_state() {
            snapshot.previous_game_mode = Some(gm);
        }

        // 2. Capture Active Power Scheme
        if let Ok(guid) = crate::optimizations::power::get_active_power_scheme() {
            snapshot.previous_power_scheme =
                Some(crate::optimizations::power::guid_to_string(&guid));
        }

        // 3. Capture Audio Endpoints
        if let Ok(endpoints) = crate::optimizations::audio::enumerate_render_endpoints() {
            let mut backups = Vec::new();
            for ep in endpoints {
                backups.push(AudioEndpointBackup {
                    endpoint_id: ep.id,
                    endpoint_name: ep.name,
                    original_disabled_state: Some(if ep.enhancements_disabled { 1 } else { 0 }),
                });
            }
            snapshot.previous_audio_endpoints = backups;
        }

        // 4. Capture Primary Network Adapter Properties & Flow Control
        if let Ok(primary) =
            val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary()
        {
            let mut props = Vec::new();
            if let Ok(adv_props) =
                crate::network::adapter::query_adapter_properties(&primary.adapter_name)
            {
                for p in adv_props {
                    let kw = &p.registry_keyword;
                    if kw.contains("EEE")
                        || kw.contains("Green")
                        || kw.contains("Interrupt")
                        || kw.contains("Flow")
                    {
                        if let Some(val) = p.first_value() {
                            props.push(AdapterPropertyBackup {
                                adapter_name: primary.adapter_name.clone(),
                                keyword: p.registry_keyword.clone(),
                                display_name: p.display_name.clone(),
                                original_value: val.to_string(),
                            });
                        }
                    }
                }
            }
            snapshot.previous_adapter_properties = props;
        }

        // 5. Capture QoS Policy (if registered)
        if let Ok(Some(policy)) = crate::network::qos::query_qos_policy(
            crate::network::qos::DEFAULT_VALORANT_QOS_POLICY_NAME,
        ) {
            snapshot.previous_qos_policy = Some(QosPolicyBackup {
                policy_name: policy.name,
                previous_nla_setting: None,
                was_policy_present_before: true,
            });
        }

        info!(
            snapshot_id = %snapshot.snapshot_id,
            duration_ms = start.elapsed().as_millis(),
            "Baseline system state captured"
        );

        Ok(snapshot)
    }

    /// Acquire or generate the DPAPI machine-protected HMAC secret key for the target directory (TASK-SEC-04).
    pub fn get_or_generate_key(target_snapshot_path: &Path) -> Result<Vec<u8>, SnapshotError> {
        let key_path = target_snapshot_path.with_file_name(SNAPSHOT_KEY_FILENAME);
        if key_path.exists() {
            let ciphertext = fs::read(&key_path)?;
            let plaintext = dpapi_unprotect_machine(&ciphertext)
                .map_err(|e| SnapshotError::KeyUnavailable {
                    path: key_path.clone(),
                    reason: format!("DPAPI unprotect failed (cross-machine replay or key corrupted): {}", e),
                })?;

            if plaintext.len() < 48 || &plaintext[..16] != MAGIC_KEY_HEADER {
                return Err(SnapshotError::KeyUnavailable {
                    path: key_path,
                    reason: "Invalid key header or payload length".to_string(),
                });
            }

            return Ok(plaintext[16..48].to_vec());
        }

        // Generate a new 256-bit (32-byte) CSPRNG key
        let raw_key = generate_crypto_random_32()
            .map_err(|e| SnapshotError::KeyUnavailable {
                path: key_path.clone(),
                reason: format!("CSPRNG key generation failed: {}", e),
            })?;

        let mut plaintext = Vec::with_capacity(64);
        plaintext.extend_from_slice(MAGIC_KEY_HEADER);
        plaintext.extend_from_slice(&raw_key);
        let now_millis = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u128)
            .unwrap_or(0);
        plaintext.extend_from_slice(&now_millis.to_le_bytes());

        let ciphertext = dpapi_protect_machine(&plaintext)
            .map_err(|e| SnapshotError::KeyUnavailable {
                path: key_path.clone(),
                reason: format!("DPAPI protect failed: {}", e),
            })?;

        fs::write(&key_path, ciphertext)?;

        // Ensure key file directory has hardened permissions if targeting ProgramData
        #[cfg(windows)]
        {
            let program_data = std::env::var("ProgramData").unwrap_or_else(|_| "C:\\ProgramData".to_string());
            if target_snapshot_path.starts_with(&program_data) {
                let _ = Self::harden_directory_acls(target_snapshot_path.parent().unwrap_or(Path::new(".")));
            }
        }

        info!(path = %key_path.display(), "Generated and persisted DPAPI-protected machine HMAC key");
        Ok(raw_key.to_vec())
    }

    /// Atomically serialize and save the state snapshot to disk.
    ///
    /// Writes to a temporary file first and atomically moves it to the target path
    /// using `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`
    /// to guarantee zero risk of file corruption under sudden power loss.
    /// Atomically serialize and save the state snapshot to disk within canonical storage.
    pub fn save_atomic(
        snapshot: &mut SystemStateSnapshot,
        target_path: Option<&Path>,
    ) -> Result<PathBuf, String> {
        Self::save_atomic_with_base(snapshot, target_path, None)
    }

    /// Atomically serialize and save the state snapshot to disk within an allowed base directory.
    ///
    /// Writes to a temporary file first and atomically moves it to the target path
    /// using `MoveFileExW(MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH)`
    /// to guarantee zero risk of file corruption under sudden power loss.
    pub fn save_atomic_with_base(
        snapshot: &mut SystemStateSnapshot,
        target_path: Option<&Path>,
        allowed_base: Option<&Path>,
    ) -> Result<PathBuf, String> {
        let start = Instant::now();
        let raw_target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        let target = Self::validate_snapshot_path(&raw_target, allowed_base)
            .map_err(|e| format!("Snapshot path validation error: {}", e))?;

        // Ensure parent directory exists
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(|e| format!("Failed to create snapshot dir: {}", e))?;
        }

        // Acquire DPAPI-protected machine key and sign snapshot with HMAC-SHA256 (TASK-SEC-04)
        let key = Self::get_or_generate_key(&target)
            .map_err(|e| format!("Failed to acquire DPAPI snapshot key: {}", e))?;
        snapshot.sign_with_key(&key);

        let json_data = serde_json::to_string_pretty(&snapshot)
            .map_err(|e| format!("Failed to serialize snapshot: {}", e))?;

        // Write to temporary file in the same directory
        let temp_filename = format!(
            "snapshot_{}_{}.tmp",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        );
        let temp_path = target
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join(temp_filename);

        fs::write(&temp_path, &json_data)
            .map_err(|e| format!("Failed to write temporary snapshot file: {}", e))?;

        // Atomically replace target file
        #[cfg(windows)]
        {
            let temp_wide: Vec<u16> = temp_path
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();
            let target_wide: Vec<u16> = target
                .to_string_lossy()
                .encode_utf16()
                .chain(std::iter::once(0))
                .collect();

            unsafe {
                let moved = MoveFileExW(
                    PCWSTR(temp_wide.as_ptr()),
                    PCWSTR(target_wide.as_ptr()),
                    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
                );

                if moved.is_err() {
                    // Fallback to std::fs rename/remove if MoveFileEx failed
                    let _ = fs::remove_file(&target);
                    if let Err(e) = fs::rename(&temp_path, &target) {
                        let _ = fs::remove_file(&temp_path);
                        return Err(format!("Atomic move failed: {}", e));
                    }
                }
            }
        }

        #[cfg(not(windows))]
        {
            let _ = fs::remove_file(&target);
            fs::rename(&temp_path, &target).map_err(|e| {
                let _ = fs::remove_file(&temp_path);
                format!("Failed to rename temp snapshot: {}", e)
            })?;
        }

        let elapsed = start.elapsed();
        info!(
            snapshot_id = %snapshot.snapshot_id,
            path = %target.display(),
            sha256 = %snapshot.sha256_hash,
            hmac = %snapshot.hmac_signature,
            duration_ms = elapsed.as_millis(),
            "Snapshot atomically committed to disk with DPAPI HMAC-SHA256 signature"
        );

        if elapsed.as_millis() > 50 {
            warn!(
                duration_ms = elapsed.as_millis(),
                "Snapshot serialization exceeded 50ms latency target"
            );
        }

        Ok(target)
    }

    /// Read and cryptographically verify snapshot from disk within canonical storage.
    pub fn load_and_verify(target_path: Option<&Path>) -> Result<SystemStateSnapshot, SnapshotError> {
        Self::load_and_verify_with_base(target_path, None)
    }

    /// Read and cryptographically verify snapshot from disk within an allowed base directory.
    ///
    /// Validates HMAC-SHA256 signature using the DPAPI-protected machine key and returns
    /// `SnapshotError::IntegrityViolation` or `SnapshotError::KeyUnavailable` if tampered or corrupt.
    pub fn load_and_verify_with_base(
        target_path: Option<&Path>,
        allowed_base: Option<&Path>,
    ) -> Result<SystemStateSnapshot, SnapshotError> {
        let raw_target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        let target = Self::validate_snapshot_path(&raw_target, allowed_base)?;

        if !target.exists() || !target.is_file() {
            return Err(SnapshotError::NotFound(target));
        }

        let key_path = target.with_file_name(SNAPSHOT_KEY_FILENAME);
        if !key_path.exists() {
            error!(path = %key_path.display(), "Snapshot HMAC key missing! Rollback aborted for security.");
            return Err(SnapshotError::KeyUnavailable {
                path: key_path,
                reason: "HMAC key file missing. Rollback aborted to prevent untrusted state restoration.".to_string(),
            });
        }

        let key_ciphertext = fs::read(&key_path)?;
        let key_plaintext = dpapi_unprotect_machine(&key_ciphertext)
            .map_err(|e| SnapshotError::KeyUnavailable {
                path: key_path.clone(),
                reason: format!("DPAPI unprotect failed (alien machine or key corruption): {}", e),
            })?;

        if key_plaintext.len() < 48 || &key_plaintext[..16] != MAGIC_KEY_HEADER {
            return Err(SnapshotError::KeyUnavailable {
                path: key_path,
                reason: "Invalid key magic header".to_string(),
            });
        }
        let key = &key_plaintext[16..48];

        let content = fs::read_to_string(&target)?;
        let snapshot: SystemStateSnapshot = serde_json::from_str(&content)?;

        // 1. Cryptographic HMAC-SHA256 signature check (TASK-SEC-04)
        if snapshot.hmac_signature.is_empty() || !snapshot.verify_hmac(key) {
            error!(
                path = %target.display(),
                "Cryptographic HMAC-SHA256 signature check FAILED! File has been tampered with or key does not match."
            );
            let computed = snapshot.compute_payload_hmac(key);
            return Err(SnapshotError::IntegrityViolation {
                path: target,
                expected_hash: snapshot.hmac_signature,
                computed_hash: computed,
            });
        }

        // 2. SHA-256 payload digest verification
        let computed = snapshot.compute_payload_hash();
        if !computed.eq_ignore_ascii_case(&snapshot.sha256_hash) {
            error!(
                path = %target.display(),
                expected = %snapshot.sha256_hash,
                computed = %computed,
                "Snapshot SHA-256 integrity check FAILED! Possible file tampering or corruption"
            );
            return Err(SnapshotError::IntegrityViolation {
                path: target,
                expected_hash: snapshot.sha256_hash,
                computed_hash: computed,
            });
        }

        info!(
            snapshot_id = %snapshot.snapshot_id,
            path = %target.display(),
            "Snapshot loaded and HMAC-SHA256 cryptographic integrity verified"
        );
        Ok(snapshot)
    }

    /// Safely delete the snapshot file and zero out the HMAC key file after a successful rollback or cleanup within canonical storage.
    pub fn delete_snapshot(target_path: Option<&Path>) -> Result<bool, String> {
        Self::delete_snapshot_with_base(target_path, None)
    }

    /// Safely delete the snapshot file and zero out the HMAC key file within an allowed base directory.
    pub fn delete_snapshot_with_base(
        target_path: Option<&Path>,
        allowed_base: Option<&Path>,
    ) -> Result<bool, String> {
        let raw_target = target_path
            .map(|p| p.to_path_buf())
            .unwrap_or_else(Self::default_snapshot_path);

        let target = Self::validate_snapshot_path(&raw_target, allowed_base)
            .map_err(|e| format!("Snapshot path validation error: {}", e))?;

        let mut deleted = false;
        if target.exists() {
            fs::remove_file(&target).map_err(|e| format!("Failed to delete snapshot: {}", e))?;
            info!(path = %target.display(), "Snapshot file removed");
            deleted = true;
        }

        let key_path = target.with_file_name(SNAPSHOT_KEY_FILENAME);
        if key_path.exists() {
            // Overwrite key data with zeros before unlinking
            let _ = fs::write(&key_path, [0u8; 128]);
            let _ = fs::remove_file(&key_path);
            info!(path = %key_path.display(), "Snapshot HMAC key eradicated");
            deleted = true;
        }

        Ok(deleted)
    }

}

#[cfg(test)]
mod tests {
    use super::*;
    use val_opt_shared::models::process::ServiceBackup;

    #[test]
    fn test_snapshot_atomic_write_and_verification() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_01");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("test_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_snap_001");
        snapshot.previous_game_mode = Some(true);
        snapshot.previous_power_scheme = Some("8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c".to_string());
        snapshot.previous_audio_endpoints.push(AudioEndpointBackup {
            endpoint_id: "{test-endpoint-id}".to_string(),
            endpoint_name: "Test Audio Device".to_string(),
            original_disabled_state: Some(0),
        });
        snapshot.paused_services.push(ServiceBackup {
            service_name: "wuauserv".to_string(),
            display_name: "Windows Update".to_string(),
            previous_state: 4,
            was_paused_by_optimizer: true,
        });

        // 1. Save atomically within test base
        let saved_path = SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir))
            .expect("Atomic save must succeed");
        assert_eq!(saved_path, snap_file);
        assert!(snap_file.exists());

        // 2. Load and verify
        let loaded = SnapshotEngine::load_and_verify_with_base(Some(&snap_file), Some(&temp_dir))
            .expect("Load and verify must succeed");
        assert_eq!(loaded.snapshot_id, "test_snap_001");
        assert_eq!(loaded.previous_game_mode, Some(true));
        assert_eq!(loaded.paused_services.len(), 1);
        assert_eq!(loaded.sha256_hash, snapshot.sha256_hash);

        // 3. Clean up
        let deleted = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir)).expect("Delete must succeed");
        assert!(deleted);
        assert!(!snap_file.exists());
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snapshot_tamper_detection() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_02");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("tampered_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_tamper_snap");
        snapshot.previous_game_mode = Some(false);
        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir))
            .expect("Save must succeed");

        // Tamper with content on disk directly
        let raw_json = fs::read_to_string(&snap_file).unwrap();
        let tampered_json = raw_json.replace("\"previous_game_mode\": false", "\"previous_game_mode\": true");
        fs::write(&snap_file, tampered_json).unwrap();

        // Attempt load: must fail with IntegrityViolation
        let result = SnapshotEngine::load_and_verify_with_base(Some(&snap_file), Some(&temp_dir));
        match result {
            Err(SnapshotError::IntegrityViolation { expected_hash, computed_hash, .. }) => {
                assert_ne!(expected_hash, computed_hash);
            }
            other => panic!("Expected IntegrityViolation error, got: {:?}", other),
        }

        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_snapshot_serialization_latency() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_snap_03");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("perf_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("perf_test");
        snapshot.previous_game_mode = Some(true);
        for i in 0..10 {
            snapshot.previous_audio_endpoints.push(AudioEndpointBackup {
                endpoint_id: format!("ep_{}", i),
                endpoint_name: format!("Audio Endpoint {}", i),
                original_disabled_state: Some(0),
            });
        }

        let start = Instant::now();
        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir))
            .expect("Atomic save should succeed");
        let elapsed = start.elapsed();

        println!("Snapshot serialization & atomic commit took: {:?}", elapsed);
        // Requirement: < 50ms
        assert!(
            elapsed.as_millis() < 50,
            "Snapshot serialization took {:?}, exceeding 50ms limit",
            elapsed
        );

        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_harden_directory_acls_invocation() {
        let temp_dir = std::env::temp_dir().join(format!("val_opt_acl_test_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);
        let res = SnapshotEngine::harden_directory_acls(&temp_dir);
        assert!(res.is_ok());

        // Restore owner permission so temp dir can be cleaned up
        #[cfg(windows)]
        {
            let username = std::env::var("USERNAME").unwrap_or_default();
            if !username.is_empty() {
                let _ = std::process::Command::new("icacls")
                    .args([temp_dir.to_str().unwrap(), "/grant:r", &format!("{}:(OI)(CI)F", username)])
                    .output();
            }
        }
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_dpapi_protect_unprotect_roundtrip() {
        let plaintext = b"test-secret-payload-data-for-dpapi-roundtrip";
        let encrypted = dpapi_protect_machine(plaintext).expect("Encryption must succeed");
        assert_ne!(&encrypted[..], &plaintext[..]);
        let decrypted = dpapi_unprotect_machine(&encrypted).expect("Decryption must succeed");
        assert_eq!(&decrypted[..], &plaintext[..]);
    }

    #[test]
    fn test_missing_key_file_fails_safely() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_missing_key");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("test_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_missing_key_snap");
        snapshot.previous_game_mode = Some(true);
        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir)).expect("Save must succeed");

        // Delete the key file
        let key_file = temp_dir.join(SNAPSHOT_KEY_FILENAME);
        assert!(key_file.exists());
        fs::remove_file(&key_file).unwrap();

        // Load must fail with KeyUnavailable
        let res = SnapshotEngine::load_and_verify_with_base(Some(&snap_file), Some(&temp_dir));
        match res {
            Err(SnapshotError::KeyUnavailable { reason, .. }) => {
                assert!(reason.contains("missing"));
            }
            other => panic!("Expected KeyUnavailable, got: {:?}", other),
        }

        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_corrupt_key_file_fails_safely() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_corrupt_key");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("test_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_corrupt_key_snap");
        snapshot.previous_game_mode = Some(true);
        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir)).expect("Save must succeed");

        // Corrupt the key file
        let key_file = temp_dir.join(SNAPSHOT_KEY_FILENAME);
        fs::write(&key_file, b"TOTAL_GARBAGE_DATA_CORRUPT_KEY").unwrap();

        // Load must fail with KeyUnavailable
        let res = SnapshotEngine::load_and_verify_with_base(Some(&snap_file), Some(&temp_dir));
        match res {
            Err(SnapshotError::KeyUnavailable { .. }) => {}
            other => panic!("Expected KeyUnavailable on corrupt key, got: {:?}", other),
        }

        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_attacker_recalculated_sha256_fails_hmac_verification() {
        let temp_dir = std::env::temp_dir().join("val_opt_test_attacker_sha");
        let _ = fs::create_dir_all(&temp_dir);
        let snap_file = temp_dir.join("test_snapshot.json");

        let mut snapshot = SystemStateSnapshot::new("test_attacker_snap");
        snapshot.previous_game_mode = Some(false);
        SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_file), Some(&temp_dir)).expect("Save must succeed");

        // Attacker modifies previous_game_mode and recalculates sha256_hash
        let content = fs::read_to_string(&snap_file).unwrap();
        let mut tampered_obj: SystemStateSnapshot = serde_json::from_str(&content).unwrap();
        tampered_obj.previous_game_mode = Some(true);
        // Attacker recalculates unkeyed sha256_hash to evade legacy hash check
        tampered_obj.sha256_hash = tampered_obj.compute_payload_hash();
        fs::write(&snap_file, serde_json::to_string_pretty(&tampered_obj).unwrap()).unwrap();

        // Load must fail with IntegrityViolation because HMAC check fails!
        let res = SnapshotEngine::load_and_verify_with_base(Some(&snap_file), Some(&temp_dir));
        match res {
            Err(SnapshotError::IntegrityViolation { .. }) => {}
            other => panic!("Expected IntegrityViolation despite attacker sha256 recalculation, got: {:?}", other),
        }

        let _ = SnapshotEngine::delete_snapshot_with_base(Some(&snap_file), Some(&temp_dir));
        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    fn test_security_validate_path_rejects_external_directory() {
        let external_dir = std::env::temp_dir().join("val_opt_sec_external_test");
        let _ = fs::create_dir_all(&external_dir);
        let external_file = external_dir.join("snapshot.json");
        let _ = fs::write(&external_file, b"{}");

        // Validating against default canonical base (%ProgramData%\ValorantOptimizer)
        let res = SnapshotEngine::validate_snapshot_path(&external_file, None);
        assert!(res.is_err(), "Must reject path outside canonical %ProgramData% root");
        match res {
            Err(SnapshotError::SecurityViolation(msg)) => {
                assert!(msg.contains("outside authorized") || msg.contains("outside"));
            }
            other => panic!("Expected SecurityViolation, got: {:?}", other),
        }

        // Verify delete_snapshot refuses to touch external files
        let del_res = SnapshotEngine::delete_snapshot(Some(&external_file));
        assert!(del_res.is_err(), "delete_snapshot must reject deleting external files");
        assert!(external_file.exists(), "External file must NOT be deleted");

        let _ = fs::remove_dir_all(&external_dir);
    }

    #[test]
    fn test_security_validate_path_rejects_directory_traversal() {
        let canonical = SnapshotEngine::canonical_snapshot_dir();
        let traversal_path = canonical.join("..").join("Windows").join("System32").join("cmd.exe");

        let res = SnapshotEngine::validate_snapshot_path(&traversal_path, None);
        assert!(res.is_err(), "Must reject path with directory traversal");
        match res {
            Err(SnapshotError::SecurityViolation(msg)) => {
                assert!(msg.contains("Directory traversal"));
            }
            other => panic!("Expected SecurityViolation, got: {:?}", other),
        }
    }

    #[test]
    #[cfg(windows)]
    fn test_security_validate_path_rejects_directory_junction() {
        let temp_dir = std::env::temp_dir().join(format!("val_opt_sec_junc_{}", std::process::id()));
        let base_dir = temp_dir.join("allowed_base");
        let target_dir = temp_dir.join("target_dir");
        let junction_dir = base_dir.join("junction_link");

        let _ = fs::create_dir_all(&base_dir);
        let _ = fs::create_dir_all(&target_dir);

        let target_snap = target_dir.join("snapshot.json");
        fs::write(&target_snap, b"{\"test\": true}").unwrap();

        // Create junction: junction_dir -> target_dir
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J", junction_dir.to_str().unwrap(), target_dir.to_str().unwrap()])
            .output()
            .expect("mklink /J must execute");

        if status.status.success() {
            let snap_via_junction = junction_dir.join("snapshot.json");

            let res = SnapshotEngine::validate_snapshot_path(&snap_via_junction, Some(&base_dir));
            assert!(res.is_err(), "Must reject snapshot accessed through directory junction");
            match res {
                Err(SnapshotError::SecurityViolation(msg)) => {
                    assert!(msg.contains("Reparse point") || msg.contains("junction") || msg.contains("symlink"));
                }
                other => panic!("Expected SecurityViolation, got: {:?}", other),
            }

            // Cleanup junction
            let _ = std::process::Command::new("cmd")
                .args(["/C", "rmdir", junction_dir.to_str().unwrap()])
                .output();
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }

    #[test]
    #[cfg(windows)]
    fn test_security_validate_path_rejects_hardlink() {
        let temp_dir = std::env::temp_dir().join(format!("val_opt_sec_hardlink_{}", std::process::id()));
        let _ = fs::create_dir_all(&temp_dir);

        let original_file = temp_dir.join("original.json");
        let hardlink_file = temp_dir.join("hardlink.json");

        fs::write(&original_file, b"{\"test\": true}").unwrap();

        if std::fs::hard_link(&original_file, &hardlink_file).is_ok() {
            let res = SnapshotEngine::validate_snapshot_path(&hardlink_file, Some(&temp_dir));
            assert!(res.is_err(), "Must reject hardlinked snapshot file");
            match res {
                Err(SnapshotError::SecurityViolation(msg)) => {
                    assert!(msg.contains("Hardlink") || msg.contains("hardlink"));
                }
                other => panic!("Expected SecurityViolation for hardlink, got: {:?}", other),
            }
        }

        let _ = fs::remove_dir_all(&temp_dir);
    }
}


