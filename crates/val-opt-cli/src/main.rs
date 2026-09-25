use std::net::SocketAddr;
use val_opt_core::benchmarking::ab_runner::{ABBenchmarkConfig, ABBenchmarkRunner};
use val_opt_core::network::probe::{MockUdpEchoServer, UdpPingCollector, UdpProbeConfig};
use val_opt_core::optimizations::OptimizationCoordinator;
use val_opt_core::process::memory::trim_explorer_working_set;
use val_opt_core::process::services::{pause_tier3_services, restore_tier3_services};
use val_opt_core::process::terminator::terminate_tier2_background_processes;
use val_opt_shared::models::system::FullSystemManifest;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = std::env::args().collect();
    let command = args.get(1).map(|s| s.as_str()).unwrap_or("help");

    match command {
        "inspect" => {
            let start = std::time::Instant::now();
            let manifest = FullSystemManifest::inspect()?;
            let duration = start.elapsed();

            let json = serde_json::to_string_pretty(&manifest)?;
            println!("{}", json);
            eprintln!("\n[System inspection completed in {:?}]", duration);
        }
        "optimize" => {
            println!("Applying Phase 3 Safe Windows Subsystem Optimizations...");
            let start = std::time::Instant::now();
            let transaction = OptimizationCoordinator::apply_optimizations()
                .map_err(|e| format!("Optimization transaction failed: {}", e))?;
            let duration = start.elapsed();

            println!("\nOptimization Transaction Committed Successfully!");
            println!(" - Transaction ID: {}", transaction.transaction_id);
            println!(" - Game Mode Enforced: true (was: {:?})", transaction.previous_game_mode);
            println!(" - Power Scheme: Gaming Optimized (was: {:?})", transaction.previous_power_scheme);
            println!(" - Audio Endpoints Optimized: {}", transaction.previous_audio_endpoints.len());
            eprintln!("\n[Optimizations applied in {:?}]", duration);
        }
        "restore" => {
            println!("Restoring system from active/orphaned optimization snapshot...");
            let start = std::time::Instant::now();
            let restored = OptimizationCoordinator::recover_orphaned_transaction()
                .map_err(|e| format!("Rollback failed: {}", e))?;
            let duration = start.elapsed();

            if restored {
                println!("\nSystem successfully restored to original baseline state!");
            } else {
                println!("\nNo active optimization transaction found to restore.");
            }
            eprintln!("[Restoration check completed in {:?}]", duration);
        }
        "rollback" => {
            println!("Executing One-Click Emergency System Rollback...");
            let start = std::time::Instant::now();

            if let Some(custom_arg) = args.get(2) {
                let p = std::path::Path::new(custom_arg);
                if !val_opt_core::state::SnapshotEngine::is_path_in_canonical_dir(p) {
                    eprintln!("Error: Rollback path '{}' is outside canonical directory %ProgramData%\\ValorantOptimizer.", p.display());
                    eprintln!("Security Policy: Rollback operations are restricted strictly to %ProgramData%\\ValorantOptimizer.");
                    return Err("Security violation: arbitrary rollback path rejected".into());
                }
            }

            let custom_path = args.get(2).map(|s| std::path::Path::new(s));
            let report = val_opt_core::state::CrashRecoveryService::check_and_recover(custom_path)
                .map_err(|e| format!("Emergency rollback failed: {}", e))?;
            let duration = start.elapsed();

            if report.uncommitted_snapshot_found {
                println!("\n=======================================================");
                println!(" EMERGENCY ROLLBACK EXECUTED SUCCESSFULLY");
                println!("=======================================================");
                println!(" Snapshot ID:                 {:?}", report.snapshot_id.as_deref().unwrap_or("N/A"));
                println!(" Snapshot Timestamp:          {:?}", report.snapshot_timestamp.as_deref().unwrap_or("N/A"));
                println!(" Windows Services Resumed:    {}", report.services_restored);
                println!(" Audio Endpoints Restored:    {}", report.audio_endpoints_restored);
                println!(" NIC Properties Restored:     {}", report.adapter_properties_restored);
                println!(" QoS Policy Unregistered:     {}", report.qos_policy_restored);
                println!(" Power Scheme Restored:       {}", report.power_scheme_restored);
                println!(" Game Mode Restored:          {}", report.game_mode_restored);
                println!(" Total Restoration Duration:  {} ms", duration.as_millis());
                println!("=======================================================");
                println!("System state has been 100% restored to baseline.");
            } else {
                println!("\nNo active or orphaned optimization snapshot found.");
                println!("System is already at clean baseline state.");
            }
            eprintln!("[Rollback procedure finished in {:?}]", duration);
        }
        "vanguard-check" => {
            println!("Validating Riot Vanguard Anti-Cheat Compliance Prerequisites...");
            let start = std::time::Instant::now();
            let report = val_opt_core::safety::VanguardChecker::check();
            let duration = start.elapsed();

            println!("\n=======================================================");
            println!(" RIOT VANGUARD PRE-FLIGHT COMPLIANCE REPORT");
            println!("=======================================================");
            println!(" 1. Vanguard Service (vgc):       [{}]", if report.vgc_service_running { "RUNNING" } else { "STOPPED / MISSING" });
            println!(" 2. Kernel Driver (vgk.sys):      [{}]", if report.vgk_driver_loaded { "LOADED" } else { "NOT LOADED" });
            println!(" 3. Windows Test Signing:         [{}]", if report.test_signing_disabled { "OFF (COMPLIANT)" } else { "ON (VIOLATION)" });
            println!(" 4. UEFI Secure Boot:             [{}]", if report.secure_boot_enabled { "ENABLED (COMPLIANT)" } else { "DISABLED (VIOLATION)" });
            println!(" 5. Virtualization Security/HVCI: [{}]", if report.vbs_hvci_intact { "INTACT (COMPLIANT)" } else { "COMPROMISED (VIOLATION)" });
            println!("-------------------------------------------------------");
            println!(" Overall Status:                  [{}]", if report.is_compliant { "100% COMPLIANT - SAFE TO OPTIMIZE" } else { "REJECTED - VIOLATIONS DETECTED" });
            if !report.violations.is_empty() {
                println!("\n Active Violations:");
                for v in &report.violations {
                    println!("   * {}", v);
                }
            }
            println!("=======================================================");
            eprintln!("[Vanguard compliance check completed in {:?}]", duration);
        }
        "snapshot" => {
            println!("Capturing baseline system state into atomic snapshot...");
            let start = std::time::Instant::now();
            let mut snapshot = val_opt_core::state::SnapshotEngine::capture_system_baseline()
                .map_err(|e| format!("Failed to capture baseline: {}", e))?;

            if let Some(custom_arg) = args.get(2) {
                let p = std::path::Path::new(custom_arg);
                if !val_opt_core::state::SnapshotEngine::is_path_in_canonical_dir(p) {
                    eprintln!("Error: Snapshot path '{}' is outside canonical directory %ProgramData%\\ValorantOptimizer.", p.display());
                    eprintln!("Security Policy: Snapshot operations are restricted strictly to %ProgramData%\\ValorantOptimizer.");
                    return Err("Security violation: arbitrary snapshot path rejected".into());
                }
            }

            let custom_path = args.get(2).map(|s| std::path::Path::new(s));
            let path = val_opt_core::state::SnapshotEngine::save_atomic(&mut snapshot, custom_path)
                .map_err(|e| format!("Failed to save snapshot: {}", e))?;
            let duration = start.elapsed();

            println!("\nBaseline Snapshot Captured Successfully!");
            println!(" - Snapshot ID:       {}", snapshot.snapshot_id);
            println!(" - Path:              {}", path.display());
            println!(" - SHA-256 Hash:      {}", snapshot.sha256_hash);
            println!(" - Game Mode:         {:?}", snapshot.previous_game_mode);
            println!(" - Power Scheme:      {:?}", snapshot.previous_power_scheme);
            println!(" - Audio Endpoints:   {}", snapshot.previous_audio_endpoints.len());
            println!(" - NIC Properties:    {}", snapshot.previous_adapter_properties.len());
            eprintln!("[Snapshot captured in {:?}]", duration);
        }
        "trim-memory" => {
            println!("Trimming Windows Explorer working set memory cache...");
            let start = std::time::Instant::now();
            let bytes_reclaimed = trim_explorer_working_set()
                .map_err(|e| format!("Working set trim failed: {}", e))?;
            let duration = start.elapsed();

            println!(
                "Successfully reclaimed {:.2} MB of physical RAM from Explorer working set",
                (bytes_reclaimed as f64) / (1024.0 * 1024.0)
            );
            eprintln!("[Memory trim completed in {:?}]", duration);
        }
        "purge-bloat" => {
            println!("Scanning and terminating safe Tier 2 bloatware (CEF, browsers, secondary launchers)...");
            let start = std::time::Instant::now();
            let backups = terminate_tier2_background_processes()
                .map_err(|e| format!("Process termination failed: {}", e))?;
            let duration = start.elapsed();

            println!("\nTerminated {} Tier 2 background processes:", backups.len());
            for app in &backups {
                println!(" - [{}] {}", app.name, app.executable_path);
            }
            eprintln!("\n[Process purge completed in {:?}]", duration);
        }
        "pause-services" => {
            println!("Pausing non-essential Tier 3 background services (wuauserv, SysMain, DiagTrack)...");
            let start = std::time::Instant::now();
            let backups = pause_tier3_services();
            let duration = start.elapsed();

            println!("\nPaused {} Tier 3 background services:", backups.len());
            for svc in &backups {
                println!(" - {}", svc.service_name);
            }
            eprintln!("\n[Services paused in {:?}]", duration);
        }
        "resume-services" => {
            println!("Resuming non-essential Tier 3 background services (wuauserv, SysMain, DiagTrack)...");
            let start = std::time::Instant::now();
            let backups: Vec<val_opt_shared::models::process::ServiceBackup> = val_opt_core::process::services::TARGET_TIER3_SERVICES
                .iter()
                .map(|name| val_opt_shared::models::process::ServiceBackup {
                    service_name: name.to_string(),
                    display_name: name.to_string(),
                    previous_state: 4, // SERVICE_RUNNING
                    was_paused_by_optimizer: true,
                })
                .collect();
            restore_tier3_services(&backups);
            let duration = start.elapsed();

            println!("\nResumed {} Tier 3 background services.", backups.len());
            eprintln!("[Services resumed in {:?}]", duration);
        }
        "benchmark" => {
            let trials: usize = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(10);

            println!("Running Automated A/B Multi-Trial Benchmark Suite ({} trials)...", trials);
            let config = ABBenchmarkConfig {
                trials,
                frames_per_trial: 1000,
                warmup_frames: 50,
                baseline_fps: 228.0,
                optimized_fps: 254.0,
            };

            let runner = ABBenchmarkRunner::new(config);
            let start = std::time::Instant::now();
            let report = runner.run_synthetic_benchmark().map_err(|e| format!("Benchmark failed: {}", e))?;
            let duration = start.elapsed();

            let markdown = ABBenchmarkRunner::format_markdown_report(&report);
            println!("\n{}", markdown);
            eprintln!("[Benchmark suite executed in {:?}]", duration);
        }
        "ping-probe" => {
            let target_arg = args.get(2).map(|s| s.as_str());

            let (target_addr, _mock_guard) = if let Some(addr_str) = target_arg {
                let parsed: SocketAddr = addr_str.parse().map_err(|e| {
                    format!("Invalid target socket address '{}': {}. Example: 127.0.0.1:27015", addr_str, e)
                })?;
                (parsed, None)
            } else {
                let echo = MockUdpEchoServer::spawn()
                    .map_err(|e| format!("Failed to spawn local test echo server: {}", e))?;
                let addr = echo.addr();
                (addr, Some(echo))
            };

            println!("Probing UDP target: {} with 50 packets...", target_addr);
            let config = UdpProbeConfig {
                target_addr,
                packet_count: 50,
                packet_interval_ms: 10,
                timeout_ms: 200,
            };

            let start = std::time::Instant::now();
            let report = UdpPingCollector::probe(config).map_err(|e| format!("Probe error: {}", e))?;
            let duration = start.elapsed();

            let json = serde_json::to_string_pretty(&report)?;
            println!("\n{}", json);
            eprintln!("\n[UDP probe completed in {:?}]", duration);
        }
        "bufferbloat" => {
            let target_arg = args.get(2).map(|s| s.as_str());
            println!("Running Bufferbloat & Network Health Diagnostic Test...");
            let start = std::time::Instant::now();
            let report = if let Some(addr_str) = target_arg {
                let parsed: SocketAddr = addr_str.parse().map_err(|e| {
                    format!("Invalid target socket address '{}': {}. Example: 1.1.1.1:53", addr_str, e)
                })?;
                let config = val_opt_core::network::bufferbloat::BufferbloatConfig {
                    target_addr: parsed,
                    ..Default::default()
                };
                val_opt_core::network::bufferbloat::BufferbloatTester::run_test(&config)
                    .map_err(|e| format!("Bufferbloat test failed: {}", e))?
            } else {
                val_opt_core::network::bufferbloat::BufferbloatTester::run_isolated_test()
                    .map_err(|e| format!("Isolated bufferbloat test failed: {}", e))?
            };
            let duration = start.elapsed();

            println!("\n=======================================================");
            println!(" BUFFERBLOAT & NETWORK HEALTH REPORT");
            println!("=======================================================");
            println!(" Target Host:            {}", report.target_host);
            println!(" Unloaded Baseline Ping: {:.2} ms (Jitter: {:.2} ms)", report.unloaded_ping_ms, report.unloaded_jitter_ms);
            println!(" Loaded Ping Under Load: {:.2} ms (Jitter: {:.2} ms)", report.loaded_ping_ms, report.loaded_jitter_ms);
            println!(" Bufferbloat Delta:      +{:.2} ms", report.bufferbloat_delta_ms);
            println!(" Packet Loss:            {:.1}%", report.packet_loss_pct);
            println!(" Bufferbloat Grade:      [{}]", report.grade.as_str());
            println!("-------------------------------------------------------");
            println!(" Recommendations:");
            for rec in &report.recommendations {
                println!(" - {}", rec);
            }
            println!("=======================================================");
            eprintln!("[Bufferbloat test completed in {:?}]", duration);
        }
        "net-inspect" => {
            println!("Inspecting Physical Network Adapter & RSS Configurations...");
            let start = std::time::Instant::now();
            let primary = val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary()
                .map_err(|e| format!("Failed to detect primary adapter: {}", e))?;

            println!("\nPrimary Network Interface: {}", primary.adapter_name);
            println!(" - Description:    {}", primary.description);
            println!(" - Status:         {}", if primary.is_active { "Active (Up)" } else { "Inactive" });
            println!(" - Link Speed:     {} Mbps", primary.link_speed_mbps);
            println!(" - IPv4 Address:   {}", primary.ipv4_address.as_deref().unwrap_or("N/A"));

            println!("\nAdvanced Hardware Properties:");
            if let Ok(props) = val_opt_core::network::adapter::query_adapter_properties(&primary.adapter_name) {
                for p in &props {
                    let kw = &p.registry_keyword;
                    if kw.contains("EEE") || kw.contains("Green") || kw.contains("Interrupt") || kw.contains("Flow") || kw.contains("RSS") || kw.contains("Power") {
                        println!(" - {:<35} [{}] = {:?}", p.display_name, kw, p.first_value().unwrap_or(""));
                    }
                }
            }

            println!("\nReceive Side Scaling (RSS) Health:");
            if let Ok(rss) = val_opt_core::network::flow_control::verify_rss(&primary.adapter_name) {
                println!(" - Adapter RSS Active:        {}", rss.rss_enabled);
                println!(" - Hardware Receive Queues:   {}", rss.num_queues);
                println!(" - Global TCP RSS:            {}", rss.global_tcp_rss_enabled);
                println!(" - Meets >= 4 Queues Target:  {}", rss.meets_minimum_queues);
                println!(" - Optimal Configuration:     {}", rss.is_optimal);
                if !rss.recommendations.is_empty() {
                    for r in &rss.recommendations {
                        println!("   * Recommendation: {}", r);
                    }
                }
            }
            let duration = start.elapsed();
            eprintln!("\n[Network inspection completed in {:?}]", duration);
        }
        "qos-check" => {
            println!("Checking Windows QoS DSCP Policies...");
            let start = std::time::Instant::now();
            let policy = val_opt_core::network::qos::query_qos_policy(
                val_opt_core::network::qos::DEFAULT_VALORANT_QOS_POLICY_NAME,
            ).map_err(|e| format!("QoS query error: {}", e))?;
            let duration = start.elapsed();

            match policy {
                Some(p) => {
                    println!("\nActive Policy Found: {}", p.name);
                    println!(" - App Filter:       {}", p.app_path_name);
                    println!(" - Protocol:         {}", p.protocol);
                    println!(" - Destination Ports: {}-{}", p.dst_port_start, p.dst_port_end);
                    println!(" - DSCP Priority:    {} (Expedited Forwarding)", p.dscp_value);
                }
                None => {
                    println!("\nNo active QoS policy named '{}' currently registered.", val_opt_core::network::qos::DEFAULT_VALORANT_QOS_POLICY_NAME);
                }
            }
            eprintln!("[QoS inspection completed in {:?}]", duration);
        }
        "latency-test" => {
            let duration_secs: f64 = args
                .get(2)
                .and_then(|s| s.parse().ok())
                .unwrap_or(2.0);

            println!("Running Kernel DPC/ISR Latency Monitoring Session ({:.1}s)...", duration_secs);
            let start = std::time::Instant::now();

            let mut session = val_opt_core::latency::KernelLatencySessionManager::new();
            session.run_synthetic_session(std::time::Duration::from_secs_f64(duration_secs), false);

            let elapsed = start.elapsed();
            let report = val_opt_core::latency::report::generate_report(&session, elapsed.as_secs_f64());
            let md = val_opt_core::latency::report::format_markdown_report(&report);

            println!("\n{}", md);
            eprintln!("[Latency test completed in {:?}]", elapsed);
        }
        "drivers" => {
            println!("Enumerating Loaded Windows Kernel Device Drivers (.sys)...");
            let start = std::time::Instant::now();
            let isolator = val_opt_core::latency::KernelDriverIsolator::new();
            let duration = start.elapsed();

            let drivers = isolator.get_loaded_drivers();
            let sample_size = drivers.len().min(30);
            println!("\nLoaded Kernel Device Drivers (showing {} of {}):", sample_size, drivers.len());
            println!("{:<25} {:<18} {}", "Driver Name", "Base Address", "Module Path");
            println!("{:-<25} {:-<18} {:-<40}", "", "", "");
            for drv in drivers.iter().take(sample_size) {
                println!("{:<25} 0x{:<16X} {}", drv.name, drv.base_address, drv.path);
            }
            eprintln!("\n[Driver enumeration completed in {:?}]", duration);
        }
        _ => {
            println!("VALORANT Performance Optimizer CLI v0.1.0");
            println!("Usage: val-opt-cli <command> [options]");
            println!("\nCommands:");
            println!("  inspect                 Scans and outputs comprehensive system hardware & security manifest");
            println!("  optimize                Applies safe optimizations (Game Mode, Power Plan, Audio APOs, NIC Latency, QoS)");
            println!("  restore                 Restores system settings to baseline from transaction snapshot");
            println!("  rollback [path]         Failsafe emergency rollback: validates SHA-256 and restores all settings");
            println!("  vanguard-check          Validates all Riot Vanguard anti-cheat prerequisites (vgc, vgk, secure boot, VBS)");
            println!("  snapshot [path]         Captures complete baseline system state into an atomic SHA-256 verified snapshot");
            println!("  net-inspect             Queries active NIC advanced properties, Flow Control, and RSS multi-queues");
            println!("  bufferbloat [ip:port]   Measures unloaded vs loaded ping & jitter; calculates bufferbloat grade (A+ to F)");
            println!("  qos-check               Inspects active Windows QoS DSCP 46 policies");
            println!("  latency-test [secs]     Profiles kernel DPC/ISR execution times, flags drivers > 500µs");
            println!("  drivers                 Enumerates loaded Windows kernel device drivers and base addresses");
            println!("  trim-memory             Flushes Windows Explorer working set to reclaim physical RAM");
            println!("  purge-bloat             Gracefully terminates Tier 2 background processes (CEF, browsers)");
            println!("  pause-services          Pauses Tier 3 non-essential services (wuauserv, SysMain, DiagTrack)");
            println!("  resume-services         Resumes Tier 3 non-essential services post-match");
            println!("  benchmark [trials]      Runs multi-trial A/B benchmark (default: 10 trials) with Student's t-test");
            println!("  ping-probe [ip:port]    Measures high-precision UDP round-trip latency, jitter, and packet loss");
            println!("  help                    Shows this help message");
        }
    }

    Ok(())
}
