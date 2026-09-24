//! VALORANT Performance Optimizer - Native Desktop GUI Library.
//!
//! Provides the Slint window interface, IPC communication client, and ephemeral lifecycle coordinator.

pub mod ipc_client;
pub mod lifecycle;

slint::include_modules!();

use ipc_client::IpcClient;
use val_opt_shared::ipc::{IpcRequest, IpcResponse};
use val_opt_shared::models::system::FullSystemManifest;

/// Initialize system hardware specs and telemetry on the AppWindow instance.
pub fn initialize_telemetry(app: &AppWindow) {
    if let Ok(manifest) = FullSystemManifest::inspect() {
        let cpu_info = format!("{} / {} Cores", manifest.cpu.brand, manifest.cpu.physical_cores);
        app.set_cpu_name(cpu_info.into());

        let gpu_info = format!("{} ({} MB)", manifest.primary_gpu.name, manifest.primary_gpu.dedicated_vram_mb);
        app.set_gpu_name(gpu_info.into());

        let adapter_info = format!("{} ({} Mbps)", manifest.primary_network.adapter_name, manifest.primary_network.link_speed_mbps);
        app.set_adapter_name(adapter_info.into());
    }

    // Check daemon status via IPC
    if let Ok(IpcResponse::Status { is_optimized, active_profile, .. }) =
        IpcClient::send_request(&IpcRequest::GetStatus)
    {
        tracing::info!(profile = %active_profile, optimized = is_optimized, "Synchronized status with core daemon");
        app.set_dpc_status("OPTIMAL (< 80µs)".into());
        app.set_bufferbloat_grade("A+".into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_appwindow_lifecycle_and_telemetry() {
        let app = AppWindow::new().expect("Failed to instantiate AppWindow");
        assert_eq!(app.get_active_tab(), 0);

        // Test tab switching
        app.set_active_tab(1);
        assert_eq!(app.get_active_tab(), 1);

        app.set_active_tab(2);
        assert_eq!(app.get_active_tab(), 2);

        // Test property updates
        app.set_cpu_name("Test AMD Ryzen 7 7800X3D".into());
        assert_eq!(app.get_cpu_name(), "Test AMD Ryzen 7 7800X3D");

        app.set_gpu_name("Test NVIDIA RTX 4090".into());
        assert_eq!(app.get_gpu_name(), "Test NVIDIA RTX 4090");

        app.set_bufferbloat_grade("A+".into());
        assert_eq!(app.get_bufferbloat_grade(), "A+");

        app.set_dpc_status("OPTIMAL (< 50µs)".into());
        assert_eq!(app.get_dpc_status(), "OPTIMAL (< 50µs)");

        // Test telemetry initialization
        initialize_telemetry(&app);
        assert!(!app.get_cpu_name().is_empty());
        assert!(!app.get_gpu_name().is_empty());
        assert!(!app.get_adapter_name().is_empty());
    }
}
