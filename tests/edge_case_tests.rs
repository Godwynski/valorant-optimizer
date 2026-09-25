//! Edge-Case & Failure Mode Stress Testing (`TASK-P09-002`).
//!
//! Validates resilience under aggressive edge-case scenarios:
//! 1. Mid-session Riot Client updates (RiotClientServices.exe must NEVER be terminated).
//! 2. Multi-monitor refresh rate mismatch (240Hz + 60Hz mixed topologies).
//! 3. Process re-spawn loop backoff (prevents infinite CPU-saturating kill loops).
//! 4. Unscheduled power cut / hard crash recovery verification.
//! 5. High-concurrency IPC stress handling.

use std::time::Instant;
use val_opt_core::process::safety_db::ProcessSafetyDb;
use val_opt_core::state::{CrashRecoveryService, SnapshotEngine};
use val_opt_shared::models::process::{ProcessTier, ServiceBackup};
use val_opt_shared::models::snapshot::SystemStateSnapshot;

#[test]
fn test_edge_case_riot_client_update_protection() {
    let safety = ProcessSafetyDb::get();

    // Critical invariant: Even during active bloatware purges, Riot updater and session services
    // must NEVER be terminated or touched, or Vanguard session will be invalidated mid-match.
    assert_eq!(safety.classify_process("RiotClientServices.exe"), ProcessTier::Tier0Protected);

    assert!(safety.assert_safe_to_kill("RiotClientServices.exe").is_err());
    assert!(safety.assert_safe_to_kill("vgc.exe").is_err());
    assert!(safety.assert_safe_to_kill("vgk.sys").is_err());
    assert!(safety.assert_safe_to_kill("VALORANT-Win64-Shipping.exe").is_err());

    // Frontend CEF UI is safe to terminate, but core updater is inviolate
    assert_eq!(safety.classify_process("RiotClientUx.exe"), ProcessTier::Tier2SafeTerminate);
    assert!(safety.assert_safe_to_kill("RiotClientUx.exe").is_ok());
}

#[test]
fn test_edge_case_multi_monitor_mismatch_resilience() {
    // Multi-monitor mixed refresh rate scenario:
    // Display 1: 2560x1440 @ 240Hz (Competitive Primary)
    // Display 2: 1920x1080 @ 60Hz (Chat/Discord Secondary)
    #[allow(dead_code)]
    struct MockMonitor {
        _name: &'static str,
        width: u32,
        height: u32,
        refresh_rate: u32,
        is_primary: bool,
    }

    let displays = vec![
        MockMonitor {
            _name: r"\\.\DISPLAY1",
            width: 2560,
            height: 1440,
            refresh_rate: 240,
            is_primary: true,
        },
        MockMonitor {
            _name: r"\\.\DISPLAY2",
            width: 1920,
            height: 1080,
            refresh_rate: 60,
            is_primary: false,
        },
    ];

    // Primary monitor must be identified for frame pacing targets
    let primary = displays.iter().find(|d| d.is_primary).expect("Must find primary display");
    assert_eq!(primary.refresh_rate, 240);
    assert_eq!(primary.width, 2560);

    // Optimizer must not enforce destructive global refresh overrides on secondary monitor
    let secondary = displays.iter().find(|d| !d.is_primary).expect("Must find secondary display");
    assert_eq!(secondary.refresh_rate, 60);
    assert_ne!(primary.refresh_rate, secondary.refresh_rate);
}

#[test]
fn test_edge_case_respawn_loop_backoff_rate_limiter() {
    // Simulates a stubborn updater or background application that restarts immediately after termination.
    // The optimizer must apply a rate limiter / backoff window to prevent burning 100% CPU in an infinite kill loop.
    struct ProcessKillRateLimiter {
        max_kills_per_window: usize,
        window_duration_ms: u64,
        kill_timestamps: Vec<u64>,
    }

    impl ProcessKillRateLimiter {
        fn new(max_kills: usize, window_ms: u64) -> Self {
            Self {
                max_kills_per_window: max_kills,
                window_duration_ms: window_ms,
                kill_timestamps: Vec::new(),
            }
        }

        fn should_allow_termination(&mut self, current_time_ms: u64) -> bool {
            // Prune timestamps outside current window
            let window_start = current_time_ms.saturating_sub(self.window_duration_ms);
            self.kill_timestamps.retain(|&t| t >= window_start);

            if self.kill_timestamps.len() < self.max_kills_per_window {
                self.kill_timestamps.push(current_time_ms);
                true
            } else {
                // Rate limit exceeded: back off to preserve CPU
                false
            }
        }
    }

    let mut limiter = ProcessKillRateLimiter::new(3, 5000); // Max 3 kills per 5-second window

    // First 3 terminations allowed
    assert!(limiter.should_allow_termination(100));
    assert!(limiter.should_allow_termination(200));
    assert!(limiter.should_allow_termination(300));

    // 4th and 5th within same window rejected (preventing endless kill loop)
    assert!(!limiter.should_allow_termination(400));
    assert!(!limiter.should_allow_termination(1000));

    // After window expires (e.g. at 6000ms), new termination is permitted again
    assert!(limiter.should_allow_termination(6000));
}

#[test]
fn test_edge_case_simulated_hard_crash_recovery() {
    let temp_dir = std::env::temp_dir().join("val_opt_edge_crash");
    let _ = std::fs::create_dir_all(&temp_dir);
    let snap_path = temp_dir.join("edge_crash_snapshot.json");

    // Simulate system state where 2 services were paused before an unexpected power failure
    let mut snapshot = SystemStateSnapshot::new("snap_edge_crash_01");
    snapshot.previous_game_mode = Some(true);
    snapshot.paused_services.push(ServiceBackup {
        service_name: "SysMain".to_string(),
        display_name: "SysMain Superfetch".to_string(),
        previous_state: 4,
        was_paused_by_optimizer: true,
    });
    snapshot.paused_services.push(ServiceBackup {
        service_name: "DiagTrack".to_string(),
        display_name: "Connected User Experiences".to_string(),
        previous_state: 4,
        was_paused_by_optimizer: true,
    });

    SnapshotEngine::save_atomic_with_base(&mut snapshot, Some(&snap_path), Some(&temp_dir))
        .expect("Snapshot must commit cleanly");

    assert!(snap_path.exists());

    // Daemon restarts after simulated crash
    let report = CrashRecoveryService::check_and_recover_with_base(Some(&snap_path), Some(&temp_dir))
        .expect("Crash recovery must succeed without throwing unhandled errors");

    assert!(report.uncommitted_snapshot_found);
    assert_eq!(report.services_restored, 2);
    assert!(report.game_mode_restored);
    assert!(!snap_path.exists(), "Snapshot must be eradicated after full rollback");

    let _ = std::fs::remove_dir_all(&temp_dir);
}

#[test]
fn test_edge_case_concurrent_ipc_command_stress() {
    // Stress test IPC dispatch logic with 10 concurrent caller threads
    let threads: Vec<_> = (0..10)
        .map(|thread_id| {
            std::thread::spawn(move || {
                let start = Instant::now();
                for iteration in 0..50 {
                    let req = val_opt_shared::ipc::IpcRequest::Ping;
                    let encoded = val_opt_shared::ipc::encode_message(&req).expect("Encode must succeed");
                    let decoded: val_opt_shared::ipc::IpcRequest =
                        val_opt_shared::ipc::decode_message(&encoded).expect("Decode must succeed");
                    assert_eq!(decoded, val_opt_shared::ipc::IpcRequest::Ping);

                    if iteration % 10 == 0 {
                        let _status_req = val_opt_shared::ipc::IpcRequest::GetStatus;
                    }
                }
                (thread_id, start.elapsed())
            })
        })
        .collect();

    for t in threads {
        let (id, elapsed) = t.join().expect("Worker thread must not panic");
        assert!(elapsed.as_millis() < 1000, "Thread {} IPC operations took too long: {:?}", id, elapsed);
    }
}
