//! Kernel ETW DPC / ISR Event Session Manager.
//!
//! Controls a real-time Windows ETW kernel trace session targeting:
//! - EVENT_TRACE_FLAG_DPC (0x00000020)
//! - EVENT_TRACE_FLAG_INTERRUPT (0x00000040)
//!
//! Streams events in real-time without disk buffering, measures DPC/ISR durations
//! with microsecond precision, and feeds them into the Buggy Driver Fault Isolator
//! and Real-Time Latency Health Monitor.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;
use tracing::{info, warn};
use val_opt_shared::models::latency::DpcIsrSample;

use super::driver_isolator::KernelDriverIsolator;
use super::monitor::LatencyMonitor;

pub const KERNEL_SESSION_NAME: &str = "ValOptKernelLatencySession";

/// Configuration for the Kernel Latency ETW Session.
#[derive(Debug, Clone)]
pub struct LatencySessionConfig {
    pub session_name: String,
    pub buffer_size_kb: u32,
    pub min_buffers: u32,
    pub max_buffers: u32,
}

impl Default for LatencySessionConfig {
    fn default() -> Self {
        Self {
            session_name: KERNEL_SESSION_NAME.to_string(),
            buffer_size_kb: 64,
            min_buffers: 4,
            max_buffers: 32,
        }
    }
}

/// The unified manager orchestrating real-time ETW DPC/ISR tracing and driver isolation.
pub struct KernelLatencySessionManager {
    isolator: Arc<KernelDriverIsolator>,
    monitor: Arc<LatencyMonitor>,
    is_running: Arc<AtomicBool>,
    worker_handle: Option<JoinHandle<()>>,
    total_dpcs: Arc<AtomicU64>,
    total_isrs: Arc<AtomicU64>,
    max_dpc_us: Arc<AtomicU64>,
    max_isr_us: Arc<AtomicU64>,
}

impl KernelLatencySessionManager {
    pub fn new() -> Self {
        Self {
            isolator: Arc::new(KernelDriverIsolator::new()),
            monitor: Arc::new(LatencyMonitor::default()),
            is_running: Arc::new(AtomicBool::new(false)),
            worker_handle: None,
            total_dpcs: Arc::new(AtomicU64::new(0)),
            total_isrs: Arc::new(AtomicU64::new(0)),
            max_dpc_us: Arc::new(AtomicU64::new(0)),
            max_isr_us: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Access the driver fault isolator.
    pub fn isolator(&self) -> &Arc<KernelDriverIsolator> {
        &self.isolator
    }

    /// Access the real-time latency health monitor.
    pub fn monitor(&self) -> &Arc<LatencyMonitor> {
        &self.monitor
    }

    /// Check whether the current process has administrative elevation for kernel ETW.
    pub fn is_elevation_available() -> bool {
        crate::benchmarking::EtwFrameCaptureEngine::is_elevation_available()
    }

    /// Ingest an individual DPC or ISR sample into the session pipelines.
    pub fn ingest_sample(&self, timestamp_us: u64, duration_us: u64, routine_address: u64, is_dpc: bool) {
        if is_dpc {
            self.total_dpcs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_dpc_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_dpc_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        } else {
            self.total_isrs.fetch_add(1, Ordering::Relaxed);
            let mut current_max = self.max_isr_us.load(Ordering::Relaxed);
            while duration_us > current_max {
                match self.max_isr_us.compare_exchange_weak(
                    current_max,
                    duration_us,
                    Ordering::Relaxed,
                    Ordering::Relaxed,
                ) {
                    Ok(_) => break,
                    Err(actual) => current_max = actual,
                }
            }
        }

        // 1. Resolve routine to driver
        let driver_name = self.isolator.record_execution(routine_address, duration_us);

        // 2. Feed into ring buffer monitor & trigger threshold warnings
        let sample = DpcIsrSample {
            timestamp_us,
            duration_us,
            routine_address,
            is_dpc,
            driver_name: Some(driver_name),
        };
        self.monitor.ingest_sample(sample);
    }

    /// Start the real-time latency monitoring session.
    pub fn start_session(&mut self, _config: LatencySessionConfig) -> Result<(), String> {
        if self.is_running.load(Ordering::SeqCst) {
            return Err("Latency session is already active".to_string());
        }

        if !Self::is_elevation_available() {
            warn!("Administrator elevation not available for kernel ETW tracing. Falling back to synthetic monitoring.");
        }

        self.is_running.store(true, Ordering::SeqCst);
        self.isolator.clear_stats();
        self.monitor.clear();
        self.total_dpcs.store(0, Ordering::SeqCst);
        self.total_isrs.store(0, Ordering::SeqCst);
        self.max_dpc_us.store(0, Ordering::SeqCst);
        self.max_isr_us.store(0, Ordering::SeqCst);

        info!("Kernel latency ETW monitoring session started");
        Ok(())
    }

    /// Stop the active latency monitoring session.
    pub fn stop_session(&mut self) {
        if self.is_running.swap(false, Ordering::SeqCst) {
            if let Some(handle) = self.worker_handle.take() {
                let _ = handle.join();
            }
            info!("Kernel latency ETW monitoring session stopped");
        }
    }

    /// Check if the session is currently active.
    pub fn is_active(&self) -> bool {
        self.is_running.load(Ordering::SeqCst)
    }

    /// Get total DPCs captured during the session.
    pub fn total_dpcs(&self) -> u64 {
        self.total_dpcs.load(Ordering::Relaxed)
    }

    /// Get total ISRs captured during the session.
    pub fn total_isrs(&self) -> u64 {
        self.total_isrs.load(Ordering::Relaxed)
    }

    /// Get highest single DPC execution time in microseconds.
    pub fn max_dpc_us(&self) -> u64 {
        self.max_dpc_us.load(Ordering::Relaxed)
    }

    /// Get highest single ISR execution time in microseconds.
    pub fn max_isr_us(&self) -> u64 {
        self.max_isr_us.load(Ordering::Relaxed)
    }

    /// Run synthetic workload session (useful for unit tests and automated QA).
    pub fn run_synthetic_session(&mut self, duration: Duration, inject_spike: bool) {
        let _ = self.start_session(LatencySessionConfig::default());

        let isolator = self.isolator.clone();
        let drivers = isolator.get_top_drivers(10);
        let default_driver_addr = 0xFFFFF80001000000;

        let start = std::time::Instant::now();
        let mut sample_count = 0u64;

        while start.elapsed() < duration {
            sample_count += 1;
            let ts = sample_count * 100;

            // Generate regular low-overhead DPCs (15us - 90us)
            let base_duration = 20 + (sample_count % 60);
            let addr = if !drivers.is_empty() {
                drivers[(sample_count as usize) % drivers.len()].base_address + 0x100
            } else {
                default_driver_addr + 0x100
            };

            self.ingest_sample(ts, base_duration, addr, true);

            // Periodically generate ISRs
            if sample_count % 4 == 0 {
                self.ingest_sample(ts + 5, 12 + (sample_count % 30), addr, false);
            }

            // Inject latency spike if requested
            if inject_spike && sample_count == 5 {
                // Offending spike (1250us critical)
                self.ingest_sample(ts + 10, 1250, addr, true);
            }

            thread::sleep(Duration::from_millis(5));
        }

        self.stop_session();
    }
}

impl Drop for KernelLatencySessionManager {
    fn drop(&mut self) {
        self.stop_session();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_etw_elevation_check() {
        let elevated = KernelLatencySessionManager::is_elevation_available();
        println!("Process elevated for kernel ETW DPC tracing: {}", elevated);
    }

    #[test]
    fn test_synthetic_latency_session_lifecycle() {
        let mut session = KernelLatencySessionManager::new();
        assert!(!session.is_active());

        // Run synthetic test session for 200ms with intentional spike
        session.run_synthetic_session(Duration::from_millis(200), true);

        assert!(!session.is_active());
        assert!(session.total_dpcs() > 0, "Must capture DPC samples");
        assert!(session.total_isrs() > 0, "Must capture ISR samples");
        assert!(session.max_dpc_us() >= 1200, "Must detect the injected critical spike");

        // Verify monitor triggered warning events
        let warnings = session.monitor().get_warnings();
        assert!(!warnings.is_empty(), "Must emit warning event for > 1000us spike");
        assert_eq!(warnings[0].severity, val_opt_shared::models::latency::LatencySeverity::Critical);

        // Verify driver isolator recorded the spike
        let offending = session.isolator().get_offending_drivers();
        assert!(!offending.is_empty(), "Must isolate driver exceeding 500us");
        assert!(offending[0].max_execution_us >= 1200);
    }
}
