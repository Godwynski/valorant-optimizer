//! Power Scheme & Processor Boost Manager.
//!
//! Manages Windows power plans with desktop vs. laptop auto-detection.
//! Activates High / Ultimate Performance on AC-powered desktops and preserves
//! balanced thermal profiles on laptops to prevent thermal throttling.

use tracing::{debug, info};
use windows::core::GUID;
use windows::Win32::Foundation::{LocalFree, HLOCAL, WIN32_ERROR};
use windows::Win32::System::Power::{
    GetSystemPowerStatus, PowerGetActiveScheme, PowerSetActiveScheme, SYSTEM_POWER_STATUS,
};

/// High Performance Plan GUID: 8c5e7fda-e8bf-4a96-9a85-a6e23a8c635c
pub const HIGH_PERFORMANCE_GUID: GUID = GUID::from_u128(0x8c5e7fda_e8bf_4a96_9a85_a6e23a8c635c);

/// Ultimate Performance Plan GUID: e9a42b02-d5df-448d-aa00-03f14749eb61
pub const ULTIMATE_PERFORMANCE_GUID: GUID = GUID::from_u128(0xe9a42b02_d5df_448d_aa00_03f14749eb61);

/// Balanced Plan GUID: 381b4222-f694-41f0-9685-ff5bb260df2e
pub const BALANCED_PLAN_GUID: GUID = GUID::from_u128(0x381b4222_f694_41f0_9685_ff5bb260df2e);

/// Power Saver Plan GUID: a1841308-3541-4fab-bc81-f71556f20b4a
pub const POWER_SAVER_GUID: GUID = GUID::from_u128(0xa1841308_3541_4fab_bc81_f71556f20b4a);

/// Query whether the host system is running as a battery-dependent laptop.
pub fn is_laptop() -> bool {
    let mut status = SYSTEM_POWER_STATUS::default();
    unsafe {
        if GetSystemPowerStatus(&mut status).is_ok() {
            // BatteryFlag 128 indicates "No system battery" (Desktop PC)
            // If BatteryFlag has bits 1, 2, 4, or is NOT 128, a battery is present
            return status.BatteryFlag != 128 && status.BatteryFlag != 255;
        }
    }
    false
}

/// Retrieve the currently active Windows power scheme GUID.
pub fn get_active_power_scheme() -> Result<GUID, String> {
    unsafe {
        let mut p_guid: *mut GUID = std::ptr::null_mut();
        let status = PowerGetActiveScheme(None, &mut p_guid);

        if status != WIN32_ERROR(0) || p_guid.is_null() {
            return Err(format!("Failed to retrieve active power scheme (Win32 error: {})", status.0));
        }

        let active_guid = *p_guid;
        let _ = LocalFree(HLOCAL(p_guid as *mut _));

        Ok(active_guid)
    }
}

/// Set the active Windows power scheme by GUID.
pub fn set_active_power_scheme(target_guid: GUID) -> Result<(), String> {
    unsafe {
        let status = PowerSetActiveScheme(None, Some(&target_guid));
        if status != WIN32_ERROR(0) {
            return Err(format!(
                "Failed to activate power scheme {:?} (Win32 error: {})",
                target_guid, status.0
            ));
        }
        Ok(())
    }
}

/// Apply optimal gaming power scheme based on system chassis (desktop vs laptop).
/// Returns the `previous_scheme: GUID` for atomic rollback.
pub fn apply_gaming_power_scheme() -> Result<GUID, String> {
    let original_guid = get_active_power_scheme()?;
    let laptop = is_laptop();

    if laptop {
        info!("Laptop chassis detected; preserving balanced thermal profile to avoid thermal throttling");
        // On laptops, preserve OEM plan
        return Ok(original_guid);
    }

    info!("Desktop PC detected on AC power; switching to High/Ultimate Performance");

    // Try Ultimate Performance first; if unavailable, apply High Performance
    if set_active_power_scheme(ULTIMATE_PERFORMANCE_GUID).is_err() {
        set_active_power_scheme(HIGH_PERFORMANCE_GUID)?;
    }

    Ok(original_guid)
}

/// Restore original Windows power scheme.
pub fn restore_power_scheme(original_guid: GUID) -> Result<(), String> {
    set_active_power_scheme(original_guid)?;
    debug!(restored_to = ?original_guid, "Power scheme restored to baseline");
    Ok(())
}

/// Convert GUID to string representation (standard hyphenated format).
pub fn guid_to_string(guid: &GUID) -> String {
    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7],
    )
}

/// Parse GUID from standard hyphenated string.
pub fn string_to_guid(s: &str) -> Result<GUID, String> {
    let clean = s.trim().trim_matches('{').trim_matches('}');
    let parts: Vec<&str> = clean.split('-').collect();
    if parts.len() != 5 {
        return Err(format!("Invalid GUID string: {}", s));
    }

    let d1 = u32::from_str_radix(parts[0], 16).map_err(|e| e.to_string())?;
    let d2 = u16::from_str_radix(parts[1], 16).map_err(|e| e.to_string())?;
    let d3 = u16::from_str_radix(parts[2], 16).map_err(|e| e.to_string())?;

    let p3 = u16::from_str_radix(parts[3], 16).map_err(|e| e.to_string())?;
    let p4 = u64::from_str_radix(parts[4], 16).map_err(|e| e.to_string())?;

    let mut d4 = [0u8; 8];
    d4[0] = (p3 >> 8) as u8;
    d4[1] = (p3 & 0xFF) as u8;
    for i in 0..6 {
        d4[2 + i] = ((p4 >> (8 * (5 - i))) & 0xFF) as u8;
    }

    Ok(GUID {
        data1: d1,
        data2: d2,
        data3: d3,
        data4: d4,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_power_scheme_query_and_restore() {
        let _lock = crate::optimizations::SYSTEM_STATE_MUTEX.lock().unwrap();
        let active = get_active_power_scheme().expect("Failed to retrieve active power scheme");
        let active_str = guid_to_string(&active);
        println!("Active power scheme: {}", active_str);

        let parsed = string_to_guid(&active_str).expect("Failed to parse back to GUID");
        assert_eq!(active, parsed);

        let laptop = is_laptop();
        println!("Is laptop chassis: {}", laptop);

        // Apply and immediately restore
        let prev = apply_gaming_power_scheme().expect("Failed to apply gaming power scheme");
        restore_power_scheme(prev).expect("Failed to restore power scheme");

        let current = get_active_power_scheme().expect("Failed to query after restore");
        assert_eq!(active, current, "Power scheme must match baseline after restore");
    }
}
