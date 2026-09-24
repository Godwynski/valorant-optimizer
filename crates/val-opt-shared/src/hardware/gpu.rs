use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct GpuInfo {
    pub name: String,
    pub vendor: String,
    pub vendor_id: u32,
    pub device_id: u32,
    pub dedicated_vram_mb: u64,
    pub driver_version: Option<String>,
    pub is_primary: bool,
}

impl GpuInfo {
    /// Detects all discrete and integrated GPUs using DXGI.
    pub fn detect_all() -> Result<Vec<Self>, anyhow::Error> {
        #[cfg(windows)]
        {
            detect_dxgi_adapters()
        }

        #[cfg(not(windows))]
        {
            Ok(vec![Self {
                name: "Mock Dedicated GPU".to_string(),
                vendor: "AMD".to_string(),
                vendor_id: 0x1002,
                device_id: 0x67DF,
                dedicated_vram_mb: 8192,
                driver_version: Some("24.8.1".to_string()),
                is_primary: true,
            }])
        }
    }

    /// Detects the primary discrete or high-performance GPU.
    pub fn detect_primary() -> Result<Self, anyhow::Error> {
        let gpus = Self::detect_all()?;
        // Prefer discrete GPU with highest dedicated VRAM
        gpus.into_iter()
            .max_by_key(|g| g.dedicated_vram_mb)
            .ok_or_else(|| anyhow::anyhow!("No active display adapter detected via DXGI"))
    }
}

#[cfg(windows)]
fn detect_dxgi_adapters() -> Result<Vec<GpuInfo>, anyhow::Error> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};

    let factory: IDXGIFactory1 = unsafe { CreateDXGIFactory1() }
        .map_err(|e| anyhow::anyhow!("CreateDXGIFactory1 failed: {}", e))?;

    let mut adapters = Vec::new();
    let mut index: u32 = 0;

    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        if let Ok(desc) = unsafe { adapter.GetDesc1() } {
            // Ignore Microsoft Basic Render Driver (Software rasterizer)
            // DXGI_ADAPTER_FLAG_SOFTWARE = 2
            let is_software = (desc.Flags & 2) != 0;
            if !is_software {
                let name = String::from_utf16_lossy(&desc.Description)
                    .trim_matches('\0')
                    .trim()
                    .to_string();

                let vendor = match desc.VendorId {
                    0x1002 => "AMD",
                    0x10DE => "NVIDIA",
                    0x8086 => "Intel",
                    0x1414 => "Microsoft",
                    _ => "Unknown",
                }
                .to_string();

                let dedicated_vram_mb = (desc.DedicatedVideoMemory as u64) / (1024 * 1024);
                let driver_version = query_driver_version(&name);

                adapters.push(GpuInfo {
                    name,
                    vendor,
                    vendor_id: desc.VendorId,
                    device_id: desc.DeviceId,
                    dedicated_vram_mb,
                    driver_version,
                    is_primary: index == 0,
                });
            }
        }
        index += 1;
    }

    if adapters.is_empty() {
        return Err(anyhow::anyhow!("No hardware graphics adapters found"));
    }

    Ok(adapters)
}

#[cfg(windows)]
fn query_driver_version(gpu_name: &str) -> Option<String> {
    use windows::core::PCWSTR;
    use windows::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_LOCAL_MACHINE,
        KEY_READ, REG_SZ,
    };

    unsafe {
        let subkey = windows::core::w!("SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}");
        let mut h_class_key = HKEY::default();
        if RegOpenKeyExW(HKEY_LOCAL_MACHINE, subkey, 0, KEY_READ, &mut h_class_key).is_err() {
            return None;
        }

        let mut subkey_index = 0;
        let mut key_name_buf = [0u16; 256];
        let mut result_version = None;

        loop {
            let mut key_name_len = key_name_buf.len() as u32;
            if RegEnumKeyExW(
                h_class_key,
                subkey_index,
                windows::core::PWSTR(key_name_buf.as_mut_ptr()),
                &mut key_name_len,
                None,
                windows::core::PWSTR::null(),
                None,
                None,
            )
            .is_err()
            {
                break;
            }

            let sub_name = PCWSTR(key_name_buf.as_ptr());
            let mut h_adapter_key = HKEY::default();
            if RegOpenKeyExW(h_class_key, sub_name, 0, KEY_READ, &mut h_adapter_key).is_ok() {
                // Check DriverDesc or DriverVersion
                let mut data_buf = [0u8; 512];
                let mut data_size = data_buf.len() as u32;
                let mut val_type = REG_SZ;

                let desc_val = windows::core::w!("DriverDesc");
                let ver_val = windows::core::w!("DriverVersion");

                let mut matched = false;
                if RegQueryValueExW(
                    h_adapter_key,
                    desc_val,
                    None,
                    Some(&mut val_type),
                    Some(data_buf.as_mut_ptr()),
                    Some(&mut data_size),
                )
                .is_ok()
                {
                    let slice = std::slice::from_raw_parts(
                        data_buf.as_ptr() as *const u16,
                        (data_size as usize) / 2,
                    );
                    let installed_name = String::from_utf16_lossy(slice)
                        .trim_matches('\0')
                        .to_string();
                    if installed_name.contains(gpu_name) || gpu_name.contains(&installed_name) {
                        matched = true;
                    }
                }

                if matched {
                    data_size = data_buf.len() as u32;
                    if RegQueryValueExW(
                        h_adapter_key,
                        ver_val,
                        None,
                        Some(&mut val_type),
                        Some(data_buf.as_mut_ptr()),
                        Some(&mut data_size),
                    )
                    .is_ok()
                    {
                        let slice = std::slice::from_raw_parts(
                            data_buf.as_ptr() as *const u16,
                            (data_size as usize) / 2,
                        );
                        result_version = Some(
                            String::from_utf16_lossy(slice)
                                .trim_matches('\0')
                                .trim()
                                .to_string(),
                        );
                    }
                }

                let _ = RegCloseKey(h_adapter_key);
                if result_version.is_some() {
                    break;
                }
            }
            subkey_index += 1;
        }

        let _ = RegCloseKey(h_class_key);
        result_version
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpu_detection() {
        let gpus = GpuInfo::detect_all().expect("Should detect at least one GPU");
        assert!(!gpus.is_empty(), "GPU list must not be empty");

        let primary = GpuInfo::detect_primary().expect("Primary GPU must be found");
        assert!(!primary.name.is_empty(), "GPU name should not be empty");

        println!("=== Detected GPU Manifest ===");
        for (i, gpu) in gpus.iter().enumerate() {
            println!(
                "GPU #{}: {} (Vendor: {}, VRAM: {} MB, Driver: {:?})",
                i, gpu.name, gpu.vendor, gpu.dedicated_vram_mb, gpu.driver_version
            );
        }
        println!("Primary Target GPU: {}", primary.name);
    }
}
