//! UDP Jitter & Ping Telemetry Collector.
//!
//! Transmits high-precision timestamped UDP datagrams to measure round-trip latency,
//! RFC 3550 packet jitter, and packet loss without relying on ICMP (which Windows QoS often deprioritizes).

use std::net::{SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use tracing::debug;
use val_opt_shared::benchmarking::models::PingProbeReport;
use val_opt_shared::benchmarking::stats::calculate_percentile;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// High-precision QPC timestamp helper.
fn get_qpc_time_us(freq: i64) -> u64 {
    let mut counter: i64 = 0;
    unsafe {
        let _ = QueryPerformanceCounter(&mut counter);
    }
    ((counter as f64 * 1_000_000.0) / (freq as f64)) as u64
}

/// UDP ping probe configuration.
#[derive(Debug, Clone)]
pub struct UdpProbeConfig {
    pub target_addr: SocketAddr,
    pub packet_count: u32,
    pub packet_interval_ms: u64,
    pub timeout_ms: u64,
}

impl Default for UdpProbeConfig {
    fn default() -> Self {
        Self {
            target_addr: "127.0.0.1:27015".parse().unwrap(),
            packet_count: 50,
            packet_interval_ms: 10,
            timeout_ms: 200,
        }
    }
}

/// Headless local UDP echo server for isolated testing of jitter/ping telemetry.
pub struct MockUdpEchoServer {
    is_running: Arc<AtomicBool>,
    bound_addr: SocketAddr,
}

impl MockUdpEchoServer {
    /// Bind an ephemeral UDP socket on localhost to echo received datagrams.
    pub fn spawn() -> Result<Self, String> {
        let socket = UdpSocket::bind("127.0.0.1:0")
            .map_err(|e| format!("Failed to bind mock UDP echo server: {}", e))?;
        let bound_addr = socket.local_addr().map_err(|e| e.to_string())?;
        let is_running = Arc::new(AtomicBool::new(true));
        let running_clone = is_running.clone();

        socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .map_err(|e| e.to_string())?;

        thread::spawn(move || {
            let mut buf = [0u8; 512];
            while running_clone.load(Ordering::Relaxed) {
                if let Ok((amt, src)) = socket.recv_from(&mut buf) {
                    let _ = socket.send_to(&buf[..amt], src);
                }
            }
        });

        Ok(Self {
            is_running,
            bound_addr,
        })
    }

    pub fn addr(&self) -> SocketAddr {
        self.bound_addr
    }
}

impl Drop for MockUdpEchoServer {
    fn drop(&mut self) {
        self.is_running.store(false, Ordering::Relaxed);
    }
}

/// High-precision UDP probe transmitter.
pub struct UdpPingCollector;

impl UdpPingCollector {
    /// Probe the target endpoint and return comprehensive latency & jitter metrics.
    pub fn probe(config: UdpProbeConfig) -> Result<PingProbeReport, String> {
        let mut qpc_freq: i64 = 0;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut qpc_freq);
        }
        if qpc_freq == 0 {
            return Err("High-resolution performance counter unavailable".to_string());
        }

        let socket = UdpSocket::bind("0.0.0.0:0")
            .map_err(|e| format!("Failed to bind local UDP probe socket: {}", e))?;

        socket
            .set_read_timeout(Some(Duration::from_millis(config.timeout_ms)))
            .map_err(|e| e.to_string())?;

        let mut rtt_samples_ms: Vec<f64> = Vec::with_capacity(config.packet_count as usize);
        let mut packets_sent = 0u32;
        let mut packets_received = 0u32;

        let mut recv_buf = [0u8; 128];

        for seq in 1..=config.packet_count {
            packets_sent += 1;
            let send_time_us = get_qpc_time_us(qpc_freq);

            // Construct payload: [u32 sequence][u64 timestamp_us]
            let mut payload = [0u8; 12];
            payload[0..4].copy_from_slice(&seq.to_le_bytes());
            payload[4..12].copy_from_slice(&send_time_us.to_le_bytes());

            if let Err(e) = socket.send_to(&payload, config.target_addr) {
                debug!("Failed to transmit UDP probe seq {}: {}", seq, e);
                thread::sleep(Duration::from_millis(config.packet_interval_ms));
                continue;
            }

            match socket.recv_from(&mut recv_buf) {
                Ok((amt, _src)) if amt >= 12 => {
                    let recv_time_us = get_qpc_time_us(qpc_freq);
                    let recv_seq = u32::from_le_bytes(recv_buf[0..4].try_into().unwrap());
                    if recv_seq == seq {
                        let rtt_us = recv_time_us.saturating_sub(send_time_us);
                        let rtt_ms = (rtt_us as f64) / 1000.0;
                        rtt_samples_ms.push(rtt_ms);
                        packets_received += 1;
                    }
                }
                _ => {
                    // Packet timeout or loss
                    debug!("UDP probe packet seq {} timed out", seq);
                }
            }

            if config.packet_interval_ms > 0 {
                thread::sleep(Duration::from_millis(config.packet_interval_ms));
            }
        }

        let packets_lost = packets_sent - packets_received;
        let packet_loss_percent = if packets_sent > 0 {
            (packets_lost as f64 / packets_sent as f64) * 100.0
        } else {
            0.0
        };

        if rtt_samples_ms.is_empty() {
            return Ok(PingProbeReport {
                target_endpoint: config.target_addr.to_string(),
                packets_sent,
                packets_received: 0,
                packet_loss_percent: 100.0,
                min_rtt_ms: 0.0,
                avg_rtt_ms: 0.0,
                max_rtt_ms: 0.0,
                median_rtt_ms: 0.0,
                jitter_ms: 0.0,
                rtt_std_dev_ms: 0.0,
            });
        }

        let min_rtt_ms = rtt_samples_ms.iter().copied().fold(f64::INFINITY, f64::min);
        let max_rtt_ms = rtt_samples_ms.iter().copied().fold(f64::NEG_INFINITY, f64::max);
        let avg_rtt_ms = rtt_samples_ms.iter().sum::<f64>() / (rtt_samples_ms.len() as f64);

        // Median
        let mut sorted_rtt = rtt_samples_ms.clone();
        sorted_rtt.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_rtt_ms = calculate_percentile(&sorted_rtt, 50.0);

        // Standard Deviation
        let variance = if rtt_samples_ms.len() > 1 {
            rtt_samples_ms
                .iter()
                .map(|&rtt| (rtt - avg_rtt_ms).powi(2))
                .sum::<f64>()
                / ((rtt_samples_ms.len() - 1) as f64)
        } else {
            0.0
        };
        let rtt_std_dev_ms = variance.sqrt();

        // RFC 3550 Interarrival Jitter calculation: J = J + (|D| - J)/16
        let mut jitter_ms = 0.0;
        for i in 1..rtt_samples_ms.len() {
            let d = (rtt_samples_ms[i] - rtt_samples_ms[i - 1]).abs();
            jitter_ms += (d - jitter_ms) / 16.0;
        }

        Ok(PingProbeReport {
            target_endpoint: config.target_addr.to_string(),
            packets_sent,
            packets_received,
            packet_loss_percent,
            min_rtt_ms,
            avg_rtt_ms,
            max_rtt_ms,
            median_rtt_ms,
            jitter_ms,
            rtt_std_dev_ms,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_udp_probe_with_echo_server() {
        let echo = MockUdpEchoServer::spawn().expect("Echo server failed to spawn");
        let config = UdpProbeConfig {
            target_addr: echo.addr(),
            packet_count: 20,
            packet_interval_ms: 2,
            timeout_ms: 100,
        };

        let report = UdpPingCollector::probe(config).expect("Probe failed");
        assert_eq!(report.packets_sent, 20);
        assert_eq!(report.packets_received, 20);
        assert_eq!(report.packet_loss_percent, 0.0);
        assert!(report.min_rtt_ms >= 0.0);
        assert!(report.avg_rtt_ms < 50.0, "Local loopback should be sub-50ms");
        assert!(report.jitter_ms >= 0.0);
    }
}
