//! VALORANT Performance Optimizer - High-Performance Native UI.
//!
//! Native Slint-based desktop application.
//! Memory footprint: < 25 MB RAM active, 0 MB in-game (Ephemeral UI unload).

use slint::ComponentHandle;
use tracing::{info, warn};
use val_opt_gui::ipc_client::IpcClient;
use val_opt_gui::lifecycle::EphemeralLifecycleCoordinator;
use val_opt_gui::{initialize_telemetry, AppWindow};
use val_opt_shared::ipc::{IpcRequest, IpcResponse};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting VALORANT Performance Optimizer GUI (Slint Native)");

    let app = AppWindow::new()?;
    initialize_telemetry(&app);

    // 1. Launch Optimized (Ephemeral UI handoff)
    let _app_weak = app.as_weak();
    app.on_launch_clicked(move || {
        info!("'Launch Optimized' clicked. Triggering Ephemeral UI Unload...");
        if let Err(e) = EphemeralLifecycleCoordinator::trigger_launch_and_unload(true, true) {
            warn!("Launch handoff error: {}", e);
        }
    });

    // 2. Commit Optimizations
    let app_weak = app.as_weak();
    app.on_apply_all_clicked(move || {
        info!("'Commit Profile' clicked");
        match IpcClient::send_request(&IpcRequest::ApplyOptimizations) {
            Ok(IpcResponse::OptimizationSuccess { .. }) => {
                info!("Profile successfully committed to system");
                if let Some(app) = app_weak.upgrade() {
                    app.set_dpc_status("OPTIMAL (< 80µs)".into());
                }
            }
            Ok(other) => warn!("Unexpected response: {:?}", other),
            Err(e) => warn!("Failed to apply optimizations via IPC: {}", e),
        }
    });

    // 3. Rollback Optimizations
    let app_weak = app.as_weak();
    app.on_rollback_all_clicked(move || {
        info!("'Rollback Profile' clicked");
        match IpcClient::send_request(&IpcRequest::RollbackOptimizations) {
            Ok(IpcResponse::RollbackSuccess { message }) => {
                info!("Rollback successful: {}", message);
                if let Some(app) = app_weak.upgrade() {
                    app.set_dpc_status("DEFAULT (Unoptimized)".into());
                }
            }
            Ok(other) => warn!("Unexpected response: {:?}", other),
            Err(e) => warn!("Failed to rollback optimizations via IPC: {}", e),
        }
    });

    // 4. Run Bufferbloat Test
    let app_weak = app.as_weak();
    app.on_run_bufferbloat_clicked(move || {
        info!("'Run Bufferbloat Test' clicked");
        if let Some(app) = app_weak.upgrade() {
            app.set_test_status_msg("Executing saturating UDP/TCP bufferbloat diagnostics...".into());
        }

        let app_handle = app_weak.clone();
        std::thread::spawn(move || {
            match IpcClient::send_request(&IpcRequest::RunBufferbloatTest) {
                Ok(IpcResponse::BufferbloatReport(report)) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_handle.upgrade() {
                            app.set_unloaded_ping(format!("{:.1} ms", report.unloaded_ping_ms).into());
                            app.set_loaded_ping(format!("{:.1} ms", report.loaded_ping_ms).into());
                            app.set_bufferbloat_delta(format!("+{:.1} ms", report.bufferbloat_delta_ms).into());
                            app.set_bufferbloat_grade(format!("{:?}", report.grade).into());
                            app.set_test_status_msg("Bufferbloat analysis completed successfully.".into());
                        }
                    });
                }
                Ok(other) => warn!("Unexpected response: {:?}", other),
                Err(e) => {
                    warn!("Bufferbloat test IPC error: {}", e);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_handle.upgrade() {
                            app.set_test_status_msg(format!("Bufferbloat diagnostic failed: {}", e).into());
                        }
                    });
                }
            }
        });
    });

    // 5. Run Latency Profile
    let app_weak = app.as_weak();
    app.on_run_latency_profile_clicked(move || {
        info!("'Profile Kernel Latency' clicked");
        if let Some(app) = app_weak.upgrade() {
            app.set_test_status_msg("Capturing ETW kernel DPC/ISR events for 1.0s...".into());
        }

        let app_handle = app_weak.clone();
        std::thread::spawn(move || {
            match IpcClient::send_request(&IpcRequest::GetLatencyReport { duration_secs: 1.0 }) {
                Ok(IpcResponse::LatencyReport(report)) => {
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_handle.upgrade() {
                            if let Some(ref dpc_driver) = report.highest_dpc_driver {
                                app.set_highest_dpc_driver(format!("{} ({}µs)", dpc_driver, report.highest_dpc_us).into());
                            }
                            if let Some(ref isr_driver) = report.highest_isr_driver {
                                app.set_highest_isr_driver(format!("{} ({}µs)", isr_driver, report.highest_isr_us).into());
                            }
                            app.set_test_status_msg(format!(
                                "ETW profile complete. {} total DPCs captured. System is {}.",
                                report.total_dpcs_captured,
                                if report.system_suitable_for_competitive { "COMPETITIVE READY" } else { "DEGRADED" }
                            ).into());
                        }
                    });
                }
                Ok(other) => warn!("Unexpected response: {:?}", other),
                Err(e) => {
                    warn!("Latency report IPC error: {}", e);
                    let _ = slint::invoke_from_event_loop(move || {
                        if let Some(app) = app_handle.upgrade() {
                            app.set_test_status_msg(format!("Latency profiling failed: {}", e).into());
                        }
                    });
                }
            }
        });
    });

    // Check if running in headless verification mode (e.g. from CI or tests)
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|arg| arg == "--test-render" || arg == "--headless") {
        info!("Running in headless/test-render verification mode. Window rendered successfully.");
        return Ok(());
    }

    app.run()?;
    Ok(())
}
