use serde::{Deserialize, Serialize};
use crate::hardware::{
    cpu::CpuInfo,
    display::DisplayInfo,
    gpu::GpuInfo,
    memory::MemoryInfo,
    network::NetworkAdapterInfo,
    security::SecurityInfo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FullSystemManifest {
    pub timestamp_utc: String,
    pub os_name: String,
    pub cpu: CpuInfo,
    pub primary_gpu: GpuInfo,
    pub all_gpus: Vec<GpuInfo>,
    pub primary_display: DisplayInfo,
    pub all_displays: Vec<DisplayInfo>,
    pub memory: MemoryInfo,
    pub primary_network: NetworkAdapterInfo,
    pub all_networks: Vec<NetworkAdapterInfo>,
    pub security: SecurityInfo,
}

impl FullSystemManifest {
    /// Inspects and aggregates all system hardware and security states into a unified manifest.
    pub fn inspect() -> Result<Self, anyhow::Error> {
        let cpu = CpuInfo::detect()?;
        let all_gpus = GpuInfo::detect_all()?;
        let primary_gpu = GpuInfo::detect_primary()?;
        let all_displays = DisplayInfo::detect_all()?;
        let primary_display = DisplayInfo::detect_primary()?;
        let memory = MemoryInfo::detect()?;
        let all_networks = NetworkAdapterInfo::detect_all()?;
        let primary_network = NetworkAdapterInfo::detect_primary()?;
        let security = SecurityInfo::detect()?;

        Ok(Self {
            timestamp_utc: chrono_now_iso(),
            os_name: "Windows 11".to_string(),
            cpu,
            primary_gpu,
            all_gpus,
            primary_display,
            all_displays,
            memory,
            primary_network,
            all_networks,
            security,
        })
    }
}

fn chrono_now_iso() -> String {
    // Generate ISO8601 timestamp using Win32 GetSystemTime without external heavy dependencies
    #[cfg(windows)]
    unsafe {
        let st = windows::Win32::System::SystemInformation::GetSystemTime();
        format!(
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
        )
    }

    #[cfg(not(windows))]
    {
        "2026-09-25T00:00:00.000Z".to_string()
    }
}
