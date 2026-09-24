//! Bufferbloat & Network Health Diagnostic Runner.
//!
//! Measures network queue delay under load vs idle baseline to grade local router
//! and ISP bufferbloat characteristics (Bufferbloat.net / Flent standard A+ to F scale).
//!
//! Measures:
//! - Baseline unloaded RTT and RFC 3550 interarrival jitter.
//! - Latency and jitter under concurrent saturating throughput load.
//! - Induced latency delta (bufferbloat spike) and packet loss percentage.
//! - Emits structured diagnostics and tuning recommendations for router SQM (CAKE / fq_codel).

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::{debug, info};
use val_opt_shared::models::network::{BufferbloatGrade, BufferbloatReport};

use crate::network::probe::{MockUdpEchoServer, UdpPingCollector, UdpProbeConfig};

/// Configuration for the Bufferbloat Diagnostic Runner.
#[derive(Debug, Clone)]
pub struct BufferbloatConfig {
    pub target_addr: SocketAddr,
    pub probe_count_unloaded: u32,
    pub probe_count_loaded: u32,
    pub probe_interval_ms: u64,
    pub timeout_ms: u64,
    pub load_concurrency: usize,
    pub load_target_addr: Option<SocketAddr>,
    pub load_duration_secs: u64,
}

impl Default for BufferbloatConfig {
    fn default() -> Self {
        Self {
            target_addr: "1.1.1.1:53".parse().unwrap(),
            probe_count_unloaded: 25,
            probe_count_loaded: 25,
            probe_interval_ms: 20,
            timeout_ms: 200,
            load_concurrency: 4,
            load_target_addr: None,
            load_duration_secs: 2,
        }
    }
}

/// Standalone diagnostic runner evaluating bufferbloat and network pacing.
pub struct BufferbloatTester;

impl BufferbloatTester {
    /// Execute a full bufferbloat diagnostic test:
    /// 1. Unloaded baseline phase: Measures idle latency and jitter.
    /// 2. Asynchronous saturating throughput phase: Spawns load workers.
    /// 3. Loaded phase: Measures latency and jitter while under saturating traffic.
    /// 4. Aggregates delta and computes objective grade.
    pub fn run_test(config: &BufferbloatConfig) -> Result<BufferbloatReport, String> {
        info!(
            target = %config.target_addr,
            unloaded_probes = config.probe_count_unloaded,
            loaded_probes = config.probe_count_loaded,
            "Starting Bufferbloat & Network Health Diagnostic Test"
        );

        let start_time = std::time::Instant::now();

        // Phase 1: Unloaded Baseline
        let unloaded_config = UdpProbeConfig {
            target_addr: config.target_addr,
            packet_count: config.probe_count_unloaded,
            packet_interval_ms: config.probe_interval_ms,
            timeout_ms: config.timeout_ms,
        };

        let unloaded_report = UdpPingCollector::probe(unloaded_config)
            .map_err(|e| format!("Failed to collect unloaded baseline ping: {}", e))?;

        let unloaded_ping = unloaded_report.median_rtt_ms;
        let unloaded_jitter = unloaded_report.jitter_ms;

        debug!(
            unloaded_ping_ms = unloaded_ping,
            unloaded_jitter_ms = unloaded_jitter,
            "Unloaded network baseline captured"
        );

        // Phase 2: Start Saturating Load Workers
        let is_loading = Arc::new(AtomicBool::new(true));
        let load_target = config.load_target_addr.unwrap_or(config.target_addr);
        let mut load_handles = Vec::new();

        for worker_id in 0..config.load_concurrency {
            let loading_flag = is_loading.clone();
            let handle = thread::spawn(move || {
                let socket = match UdpSocket::bind("0.0.0.0:0") {
                    Ok(s) => s,
                    Err(_) => return,
                };
                let dummy_payload = [0x5A; 1024]; // 1 KB payload bursts
                while loading_flag.load(Ordering::Relaxed) {
                    let _ = socket.send_to(&dummy_payload, load_target);
                    // Minimal pause to avoid locking local CPU thread while saturating UDP stack
                    thread::sleep(Duration::from_micros(250));
                }
                debug!(worker_id, "Load worker completed");
            });
            load_handles.push(handle);
        }

        // Allow load to ramp up and saturate local buffers
        thread::sleep(Duration::from_millis(150));

        // Phase 3: Loaded Latency Probing
        let loaded_config = UdpProbeConfig {
            target_addr: config.target_addr,
            packet_count: config.probe_count_loaded,
            packet_interval_ms: config.probe_interval_ms,
            timeout_ms: config.timeout_ms,
        };

        let loaded_report_res = UdpPingCollector::probe(loaded_config);

        // Terminate load generation immediately after probing completes
        is_loading.store(false, Ordering::Relaxed);
        for handle in load_handles {
            let _ = handle.join();
        }

        let loaded_report =
            loaded_report_res.map_err(|e| format!("Failed to collect loaded ping: {}", e))?;

        let loaded_ping = loaded_report.median_rtt_ms;
        let loaded_jitter = loaded_report.jitter_ms;
        let packet_loss = loaded_report.packet_loss_percent;

        // Phase 4: Compute Bufferbloat Delta and Grade
        let delta_ms = (loaded_ping - unloaded_ping).max(0.0);
        let grade = BufferbloatGrade::from_latency_delta(delta_ms, packet_loss);

        let total_duration = start_time.elapsed().as_secs_f64();

        let mut recommendations = Vec::new();
        match grade {
            BufferbloatGrade::APlus => {
                recommendations.push(
                    "Flawless queue management: Zero bufferbloat detected (< 5ms delta). Network pacing is optimal for VALORANT."
                        .to_string(),
                );
            }
            BufferbloatGrade::A => {
                recommendations.push(
                    "Excellent network pacing: Induced latency delta is under 15ms. In-game gunplay registration will remain consistent."
                        .to_string(),
                );
            }
            BufferbloatGrade::B => {
                recommendations.push(
                    "Good performance: Moderate queue delay under load (15-30ms spike). Consider enabling router QoS or Smart Queue Management (SQM)."
                        .to_string(),
                );
            }
            BufferbloatGrade::C => {
                recommendations.push(
                    "Noticeable bufferbloat: Latency spiked 30-60ms under traffic. Background downloads/streams will create noticeable peekers advantage penalty."
                        .to_string(),
                );
            }
            BufferbloatGrade::D => {
                recommendations.push(
                    "High bufferbloat: Router queues buffer packets heavily under load (60-120ms spike). Configure CAKE or fq_codel SQM on your gateway."
                        .to_string(),
                );
            }
            BufferbloatGrade::F => {
                recommendations.push(
                    "Critical bufferbloat: Severe latency spikes (>= 120ms) or packet loss (> 5%). VALORANT will experience severe rubberbanding during concurrent network usage."
                        .to_string(),
                );
            }
        }

        if loaded_jitter > 5.0 {
            recommendations.push(format!(
                "High loaded packet jitter detected ({:.2}ms). Prioritize 5GHz/6GHz Wi-Fi channels or switch to a direct Cat6 Ethernet connection.",
                loaded_jitter
            ));
        }

        info!(
            unloaded_ping_ms = unloaded_ping,
            loaded_ping_ms = loaded_ping,
            delta_ms = delta_ms,
            grade = grade.as_str(),
            loss_pct = packet_loss,
            "Completed Bufferbloat Diagnostic Test"
        );

        Ok(BufferbloatReport {
            unloaded_ping_ms: unloaded_ping,
            unloaded_jitter_ms: unloaded_jitter,
            loaded_ping_ms: loaded_ping,
            loaded_jitter_ms: loaded_jitter,
            bufferbloat_delta_ms: delta_ms,
            grade,
            packet_loss_pct: packet_loss,
            target_host: config.target_addr.to_string(),
            test_duration_secs: total_duration,
            recommendations,
        })
    }

    /// Convenience runner utilizing the built-in isolated mock UDP echo server,
    /// enabling 100% offline, deterministic automated testing.
    pub fn run_isolated_test() -> Result<BufferbloatReport, String> {
        let echo_server = MockUdpEchoServer::spawn()?;
        let target = echo_server.addr();

        let config = BufferbloatConfig {
            target_addr: target,
            probe_count_unloaded: 20,
            probe_count_loaded: 20,
            probe_interval_ms: 5,
            timeout_ms: 100,
            load_concurrency: 2,
            load_target_addr: Some(target),
            load_duration_secs: 1,
        };

        Self::run_test(&config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bufferbloat_grade_calculation() {
        assert_eq!(BufferbloatGrade::from_latency_delta(2.0, 0.0), BufferbloatGrade::APlus);
        assert_eq!(BufferbloatGrade::from_latency_delta(10.0, 0.0), BufferbloatGrade::A);
        assert_eq!(BufferbloatGrade::from_latency_delta(25.0, 0.0), BufferbloatGrade::B);
        assert_eq!(BufferbloatGrade::from_latency_delta(45.0, 0.0), BufferbloatGrade::C);
        assert_eq!(BufferbloatGrade::from_latency_delta(80.0, 0.0), BufferbloatGrade::D);
        assert_eq!(BufferbloatGrade::from_latency_delta(150.0, 0.0), BufferbloatGrade::F);
        // High packet loss always results in F
        assert_eq!(BufferbloatGrade::from_latency_delta(2.0, 6.0), BufferbloatGrade::F);
    }

    #[test]
    fn test_isolated_bufferbloat_runner() {
        let report = BufferbloatTester::run_isolated_test()
            .expect("Isolated bufferbloat diagnostic runner should succeed cleanly");

        println!("Bufferbloat report: {:?}", report);
        assert!(report.unloaded_ping_ms >= 0.0);
        assert!(report.loaded_ping_ms >= 0.0);
        assert!(report.bufferbloat_delta_ms >= 0.0);
        assert_eq!(report.packet_loss_pct, 0.0);
        assert!(!report.recommendations.is_empty());
        // On loopback mock server, bufferbloat delta should be minimal (A+ or A)
        assert!(
            report.grade == BufferbloatGrade::APlus || report.grade == BufferbloatGrade::A,
            "Local loopback bufferbloat grade should be A or A+, got: {:?}",
            report.grade
        );
    }
}
