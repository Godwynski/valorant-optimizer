//! Windows Game Mode programmatic enforcer.
//!
//! Enforces Windows Game Mode via user registry keys (`HKCU\Software\Microsoft\GameBar`)
//! to prioritize foreground gaming threads and suppress background contention.

use tracing::{debug, info};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_DWORD, REG_OPTION_NON_VOLATILE,
};

const GAME_BAR_SUBKEY: PCWSTR = w!("Software\\Microsoft\\GameBar");
const VALUE_ALLOW_AUTO_GAME_MODE: PCWSTR = w!("AllowAutoGameMode");
const VALUE_AUTO_GAME_MODE_ENABLED: PCWSTR = w!("AutoGameModeEnabled");

/// Query the current Windows Game Mode state.
/// Returns `true` if Game Mode is active/enabled.
pub fn get_game_mode_state() -> Result<bool, String> {
    unsafe {
        let mut hkey = HKEY::default();
        let open_status = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            GAME_BAR_SUBKEY,
            0,
            KEY_READ,
            &mut hkey,
        );

        if open_status != WIN32_ERROR(0) {
            // Default on modern Windows 11 is enabled
            return Ok(true);
        }

        let mut data: u32 = 0;
        let mut data_len: u32 = std::mem::size_of::<u32>() as u32;
        let mut val_type = REG_DWORD;

        let query_status = RegQueryValueExW(
            hkey,
            VALUE_ALLOW_AUTO_GAME_MODE,
            None,
            Some(&mut val_type),
            Some(&mut data as *mut u32 as *mut u8),
            Some(&mut data_len),
        );

        let _ = RegCloseKey(hkey);

        if query_status == WIN32_ERROR(0) {
            Ok(data != 0)
        } else {
            // If the key is not explicitly set, Game Mode defaults to enabled on Windows 11
            Ok(true)
        }
    }
}

/// Enforce Windows Game Mode state.
/// Returns the `previous_state: bool` so it can be restored exactly.
pub fn set_game_mode(enable: bool) -> Result<bool, String> {
    let previous_state = get_game_mode_state().unwrap_or(true);

    unsafe {
        let mut hkey = HKEY::default();
        let create_status = RegCreateKeyExW(
            HKEY_CURRENT_USER,
            GAME_BAR_SUBKEY,
            0,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut hkey,
            None,
        );

        if create_status != WIN32_ERROR(0) {
            return Err(format!(
                "Failed to open/create HKCU\\Software\\Microsoft\\GameBar (Win32 error: {})",
                create_status.0
            ));
        }

        let val: u32 = if enable { 1 } else { 0 };

        let set_status_1 = RegSetValueExW(
            hkey,
            VALUE_ALLOW_AUTO_GAME_MODE,
            0,
            REG_DWORD,
            Some(std::slice::from_raw_parts(&val as *const u32 as *const u8, 4)),
        );

        let set_status_2 = RegSetValueExW(
            hkey,
            VALUE_AUTO_GAME_MODE_ENABLED,
            0,
            REG_DWORD,
            Some(std::slice::from_raw_parts(&val as *const u32 as *const u8, 4)),
        );

        let _ = RegCloseKey(hkey);

        if set_status_1 != WIN32_ERROR(0) && set_status_2 != WIN32_ERROR(0) {
            return Err("Failed to write Game Mode registry values".to_string());
        }

        info!(
            previous = previous_state,
            current = enable,
            "Windows Game Mode updated successfully"
        );

        Ok(previous_state)
    }
}

/// Restore Windows Game Mode to its previous state.
pub fn restore_game_mode(previous_state: bool) -> Result<(), String> {
    let _ = set_game_mode(previous_state)?;
    debug!(restored_to = previous_state, "Windows Game Mode restored to baseline");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_game_mode_query_and_toggle() {
        let _lock = crate::optimizations::SYSTEM_STATE_MUTEX.lock().unwrap();
        let initial_state = get_game_mode_state().expect("Failed to query initial Game Mode state");

        // Toggle to true
        let prev = set_game_mode(true).expect("Failed to set Game Mode to true");
        assert_eq!(prev, initial_state);
        assert!(get_game_mode_state().expect("Failed to query"));

        // Toggle to false
        let _ = set_game_mode(false).expect("Failed to set Game Mode to false");
        assert!(!get_game_mode_state().expect("Failed to query"));

        // Restore to original
        restore_game_mode(initial_state).expect("Failed to restore Game Mode");
        assert_eq!(get_game_mode_state().expect("Query after restore"), initial_state);
    }
}
