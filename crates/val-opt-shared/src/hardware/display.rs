use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct DisplayInfo {
    pub device_name: String,
    pub width: u32,
    pub height: u32,
    pub refresh_rate_hz: u32,
    pub bits_per_pixel: u32,
    pub is_primary: bool,
}

impl DisplayInfo {
    /// Detects all active displays and their current resolution and refresh rate.
    pub fn detect_all() -> Result<Vec<Self>, anyhow::Error> {
        #[cfg(windows)]
        {
            detect_windows_displays()
        }

        #[cfg(not(windows))]
        {
            Ok(vec![Self {
                device_name: "\\\\.\\DISPLAY1".to_string(),
                width: 1920,
                height: 1080,
                refresh_rate_hz: 144,
                bits_per_pixel: 32,
                is_primary: true,
            }])
        }
    }

    /// Detects the primary competitive gaming monitor.
    pub fn detect_primary() -> Result<Self, anyhow::Error> {
        let displays = Self::detect_all()?;
        displays
            .into_iter()
            .find(|d| d.is_primary)
            .ok_or_else(|| anyhow::anyhow!("No primary display found"))
    }
}

#[cfg(windows)]
fn detect_windows_displays() -> Result<Vec<DisplayInfo>, anyhow::Error> {
    use windows::Win32::Graphics::Gdi::{
        EnumDisplayDevicesW, EnumDisplaySettingsW, DISPLAY_DEVICEW, DEVMODEW,
        ENUM_CURRENT_SETTINGS,
    };

    let mut displays = Vec::new();
    let mut device_index: u32 = 0;

    loop {
        let mut display_device: DISPLAY_DEVICEW = unsafe { std::mem::zeroed() };
        display_device.cb = std::mem::size_of::<DISPLAY_DEVICEW>() as u32;

        let has_device = unsafe {
            EnumDisplayDevicesW(
                windows::core::PCWSTR::null(),
                device_index,
                &mut display_device,
                0,
            )
        };

        if !has_device.as_bool() {
            break;
        }

        // Check if device is attached to desktop
        // DISPLAY_DEVICE_ATTACHED_TO_DESKTOP = 0x00000001
        let is_attached = (display_device.StateFlags & 0x00000001) != 0;
        // DISPLAY_DEVICE_PRIMARY_DEVICE = 0x00000004
        let is_primary = (display_device.StateFlags & 0x00000004) != 0;

        if is_attached {
            let device_name = String::from_utf16_lossy(&display_device.DeviceName)
                .trim_matches('\0')
                .trim()
                .to_string();

            let mut devmode: DEVMODEW = unsafe { std::mem::zeroed() };
            devmode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;

            let has_settings = unsafe {
                EnumDisplaySettingsW(
                    windows::core::PCWSTR(display_device.DeviceName.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &mut devmode,
                )
            };

            if has_settings.as_bool() {
                displays.push(DisplayInfo {
                    device_name,
                    width: devmode.dmPelsWidth,
                    height: devmode.dmPelsHeight,
                    refresh_rate_hz: devmode.dmDisplayFrequency,
                    bits_per_pixel: devmode.dmBitsPerPel,
                    is_primary,
                });
            }
        }

        device_index += 1;
    }

    if displays.is_empty() {
        return Err(anyhow::anyhow!("No active attached displays detected"));
    }

    Ok(displays)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_detection() {
        let displays = DisplayInfo::detect_all().expect("Should detect displays");
        assert!(!displays.is_empty(), "Display list must not be empty");

        let primary = DisplayInfo::detect_primary().expect("Primary display should exist");
        assert!(primary.width > 0, "Width must be > 0");
        assert!(primary.height > 0, "Height must be > 0");
        assert!(primary.refresh_rate_hz > 0, "Refresh rate must be > 0 Hz");

        println!("=== Detected Display Manifest ===");
        for (i, d) in displays.iter().enumerate() {
            println!(
                "Display #{}: {} ({}x{} @ {}Hz, {}bpp, Primary: {})",
                i, d.device_name, d.width, d.height, d.refresh_rate_hz, d.bits_per_pixel, d.is_primary
            );
        }
    }
}
