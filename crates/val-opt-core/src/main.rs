//! val-opt-core: Headless Windows Service / Native Daemon for VALORANT Performance Optimizer.

use val_opt_core::state::CrashRecoveryService;

fn main() {
    tracing_subscriber::fmt::init();
    tracing::info!("val-opt-core daemon initializing...");

    // Boot orphan crash recovery check
    match CrashRecoveryService::check_and_recover(None) {
        Ok(report) => {
            if report.uncommitted_snapshot_found {
                tracing::warn!(
                    snapshot = ?report.snapshot_id,
                    restored = report.services_restored,
                    "Crash auto-recovery executed on daemon startup"
                );
            } else {
                tracing::info!("No orphaned state snapshots found. System is clean.");
            }
        }
        Err(e) => {
            tracing::error!("Crash recovery check error: {}", e);
        }
    }

    let mut server = val_opt_core::ipc_server::IpcServer::new();
    if let Err(e) = server.start() {
        tracing::error!("Failed to start IPC server: {}", e);
    } else {
        tracing::info!("val-opt-core daemon listening on Named Pipe.");
    }
}
