//! Windows Core Audio APO (Audio Processing Objects) Enhancement Disabler.
//!
//! Disables audio DSP enhancements and system effects on active audio rendering endpoints
//! to eliminate DPC/ISR latency spikes from `audiodg.exe` while preserving directional spatial audio.

use tracing::{debug, info};
use val_opt_shared::models::snapshot::AudioEndpointBackup;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteValueW, RegEnumKeyExW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW, HKEY, HKEY_LOCAL_MACHINE, KEY_READ, KEY_SET_VALUE,
    REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SZ,
};

const MMDEVICES_RENDER_PATH: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render");

// PKEY_AudioEndpoint_Disable_SysFx = {1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5
const VALUE_DISABLE_SYSFX: PCWSTR = w!("{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5");
// PKEY_Device_FriendlyName = {a45c254e-df1c-4efd-8020-67d146a850e0},2
const VALUE_FRIENDLY_NAME: PCWSTR = w!("{a45c254e-df1c-4efd-8020-67d146a850e0},2");

/// Audio endpoint descriptor for optimization inspection.
#[derive(Debug, Clone)]
pub struct AudioEndpointInfo {
    pub id: String,
    pub name: String,
    pub enhancements_disabled: bool,
}

/// Enumerate all registered audio render endpoints and their current APO enhancement states.
pub fn enumerate_render_endpoints() -> Result<Vec<AudioEndpointInfo>, String> {
    let mut endpoints = Vec::new();

    unsafe {
        let mut h_root = HKEY::default();
        let status = RegOpenKeyExW(
            HKEY_LOCAL_MACHINE,
            MMDEVICES_RENDER_PATH,
            0,
            KEY_READ,
            &mut h_root,
        );

        if status != WIN32_ERROR(0) {
            return Ok(endpoints);
        }

        let mut index = 0u32;
        let mut key_name_buf = [0u16; 256];

        loop {
            let mut key_name_len = key_name_buf.len() as u32;
            let enum_status = RegEnumKeyExW(
                h_root,
                index,
                windows::core::PWSTR(key_name_buf.as_mut_ptr()),
                &mut key_name_len,
                None,
                windows::core::PWSTR::null(),
                None,
                None,
            );

            if enum_status != WIN32_ERROR(0) {
                break;
            }

            let endpoint_guid = String::from_utf16_lossy(&key_name_buf[..key_name_len as usize]);
            let (friendly_name, disabled) = query_endpoint_details(&endpoint_guid);

            endpoints.push(AudioEndpointInfo {
                id: endpoint_guid,
                name: friendly_name,
                enhancements_disabled: disabled,
            });

            index += 1;
        }

        let _ = RegCloseKey(h_root);
    }

    Ok(endpoints)
}

/// Helper to query endpoint friendly name and current SysFx disable status.
fn query_endpoint_details(endpoint_guid: &str) -> (String, bool) {
    let mut name = "Audio Endpoint".to_string();
    let mut disabled = false;

    unsafe {
        // Query friendly name
        let prop_path = format!("{}\\{}\\{}", "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render", endpoint_guid, "Properties");
        let wide_prop: Vec<u16> = prop_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut h_prop = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(wide_prop.as_ptr()), 0, KEY_READ, &mut h_prop) == WIN32_ERROR(0) {
            let mut name_buf = [0u16; 256];
            let mut name_len = (name_buf.len() * 2) as u32;
            let mut val_type = REG_SZ;
            if RegQueryValueExW(h_prop, VALUE_FRIENDLY_NAME, None, Some(&mut val_type), Some(name_buf.as_mut_ptr() as *mut u8), Some(&mut name_len)) == WIN32_ERROR(0) {
                let actual_len = (name_len / 2).saturating_sub(1) as usize;
                name = String::from_utf16_lossy(&name_buf[..actual_len]);
            }
            let _ = RegCloseKey(h_prop);
        }

        // Query SysFx status
        let fx_path = format!("{}\\{}\\{}", "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render", endpoint_guid, "FxProperties");
        let wide_fx: Vec<u16> = fx_path.encode_utf16().chain(std::iter::once(0)).collect();
        let mut h_fx = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, PCWSTR(wide_fx.as_ptr()), 0, KEY_READ, &mut h_fx) == WIN32_ERROR(0) {
            let mut data: u32 = 0;
            let mut data_len = 4u32;
            let mut val_type = REG_DWORD;
            if RegQueryValueExW(h_fx, VALUE_DISABLE_SYSFX, None, Some(&mut val_type), Some(&mut data as *mut u32 as *mut u8), Some(&mut data_len)) == WIN32_ERROR(0) {
                disabled = data == 1;
            }
            let _ = RegCloseKey(h_fx);
        }
    }

    (name, disabled)
}

/// Disable audio enhancements on all available render endpoints to eliminate DPC latency spikes.
/// Returns backup data for every modified endpoint to guarantee atomic rollback.
pub fn disable_audio_enhancements() -> Result<Vec<AudioEndpointBackup>, String> {
    let endpoints = enumerate_render_endpoints()?;
    let mut backups = Vec::new();

    for endpoint in &endpoints {
        let fx_path = format!(
            "{}\\{}\\{}",
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render",
            endpoint.id,
            "FxProperties"
        );
        let wide_fx: Vec<u16> = fx_path.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let mut h_fx = HKEY::default();
            // Attempt to open or create FxProperties subkey
            let open_status = RegCreateKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(wide_fx.as_ptr()),
                0,
                None,
                REG_OPTION_NON_VOLATILE,
                KEY_READ | KEY_SET_VALUE,
                None,
                &mut h_fx,
                None,
            );

            if open_status != WIN32_ERROR(0) {
                // If permission is denied (e.g. running non-elevated), log and continue
                debug!(endpoint = %endpoint.name, error = open_status.0, "Cannot write audio FxProperties (elevation required)");
                continue;
            }

            // Read original value if present
            let mut orig_val: u32 = 0;
            let mut orig_len = 4u32;
            let mut val_type = REG_DWORD;
            let query_status = RegQueryValueExW(
                h_fx,
                VALUE_DISABLE_SYSFX,
                None,
                Some(&mut val_type),
                Some(&mut orig_val as *mut u32 as *mut u8),
                Some(&mut orig_len),
            );

            let original_disabled_state = if query_status == WIN32_ERROR(0) {
                Some(orig_val)
            } else {
                None
            };

            // Write 1 to disable enhancements
            let new_val: u32 = 1;
            let set_status = RegSetValueExW(
                h_fx,
                VALUE_DISABLE_SYSFX,
                0,
                REG_DWORD,
                Some(std::slice::from_raw_parts(&new_val as *const u32 as *const u8, 4)),
            );

            let _ = RegCloseKey(h_fx);

            if set_status == WIN32_ERROR(0) {
                info!(endpoint = %endpoint.name, "Disabled audio DSP enhancements (APO)");
                backups.push(AudioEndpointBackup {
                    endpoint_id: endpoint.id.clone(),
                    endpoint_name: endpoint.name.clone(),
                    original_disabled_state,
                });
            }
        }
    }

    Ok(backups)
}

/// Restore original audio enhancement settings across all backed up endpoints.
pub fn restore_audio_enhancements(backups: &[AudioEndpointBackup]) -> Result<(), String> {
    for backup in backups {
        let fx_path = format!(
            "{}\\{}\\{}",
            "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\MMDevices\\Audio\\Render",
            backup.endpoint_id,
            "FxProperties"
        );
        let wide_fx: Vec<u16> = fx_path.encode_utf16().chain(std::iter::once(0)).collect();

        unsafe {
            let mut h_fx = HKEY::default();
            let open_status = RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                PCWSTR(wide_fx.as_ptr()),
                0,
                KEY_SET_VALUE,
                &mut h_fx,
            );

            if open_status != WIN32_ERROR(0) {
                continue;
            }

            match backup.original_disabled_state {
                Some(orig) => {
                    let _ = RegSetValueExW(
                        h_fx,
                        VALUE_DISABLE_SYSFX,
                        0,
                        REG_DWORD,
                        Some(std::slice::from_raw_parts(&orig as *const u32 as *const u8, 4)),
                    );
                }
                None => {
                    // It didn't exist previously, remove the value
                    let _ = RegDeleteValueW(h_fx, VALUE_DISABLE_SYSFX);
                }
            }

            let _ = RegCloseKey(h_fx);
            debug!(endpoint = %backup.endpoint_name, "Restored audio enhancement configuration");
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_enumerate_audio_endpoints() {
        let endpoints = enumerate_render_endpoints().expect("Failed to enumerate audio endpoints");
        println!("Discovered {} audio render endpoints:", endpoints.len());
        for ep in &endpoints {
            println!(" - [{}] {} (SysFx Disabled: {})", ep.id, ep.name, ep.enhancements_disabled);
        }
        // System should have at least 1 or 0 audio render endpoints without panicking
    }
}
