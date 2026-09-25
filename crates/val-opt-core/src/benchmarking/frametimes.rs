//! Thread-safe in-memory frame telemetry collector.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use val_opt_shared::benchmarking::models::{BenchmarkMetrics, FrameSample};
use val_opt_shared::benchmarking::stats::compute_metrics;

/// Thread-safe accumulator of individual frame-time presentation records.
#[derive(Debug, Clone)]
pub struct FrameCollector {
    samples: Arc<Mutex<Vec<FrameSample>>>,
    is_recording: Arc<AtomicBool>,
    total_frames_captured: Arc<AtomicU64>,
}

impl Default for FrameCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameCollector {
    /// Create a new empty frame collector.
    pub fn new() -> Self {
        Self {
            samples: Arc::new(Mutex::new(Vec::with_capacity(16384))),
            is_recording: Arc::new(AtomicBool::new(false)),
            total_frames_captured: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Begin active frame telemetry collection.
    pub fn start(&self) {
        self.is_recording.store(true, Ordering::SeqCst);
    }

    /// Stop active frame telemetry collection.
    pub fn stop(&self) {
        self.is_recording.store(false, Ordering::SeqCst);
    }

    /// Check if collector is actively accepting samples.
    pub fn is_active(&self) -> bool {
        self.is_recording.load(Ordering::Relaxed)
    }

    /// Push an individual frame sample into the stream if active.
    pub fn record_sample(&self, sample: FrameSample) {
        if !self.is_recording.load(Ordering::Relaxed) {
            return;
        }

        if let Ok(mut lock) = self.samples.lock() {
            lock.push(sample);
            self.total_frames_captured.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Retrieve the total number of frames captured.
    pub fn count(&self) -> usize {
        self.total_frames_captured.load(Ordering::Relaxed) as usize
    }

    /// Retrieve a cloned snapshot of all recorded frame samples.
    pub fn snapshot(&self) -> Vec<FrameSample> {
        self.samples.lock().map(|s| s.clone()).unwrap_or_default()
    }

    /// Clear all recorded samples and reset counters.
    pub fn clear(&self) {
        if let Ok(mut lock) = self.samples.lock() {
            lock.clear();
        }
        self.total_frames_captured.store(0, Ordering::SeqCst);
    }

    /// Calculate aggregate benchmark metrics over the recorded frames,
    /// discarding the specified number of warmup frames.
    pub fn calculate_metrics(&self, warmup_frames: usize) -> Result<BenchmarkMetrics, String> {
        let snapshot = self.snapshot();
        compute_metrics(&snapshot, warmup_frames)
    }

    /// Measure the total elapsed time between the first and last recorded frame.
    pub fn elapsed_duration(&self) -> Duration {
        let lock = match self.samples.lock() {
            Ok(l) => l,
            Err(_) => return Duration::ZERO,
        };

        if lock.len() < 2 {
            return Duration::ZERO;
        }

        let first_ts = lock.first().unwrap().timestamp_us;
        let last_ts = lock.last().unwrap().timestamp_us;

        if last_ts > first_ts {
            Duration::from_micros(last_ts - first_ts)
        } else {
            Duration::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collector_recording_lifecycle() {
        let collector = FrameCollector::new();
        assert_eq!(collector.count(), 0);
        assert!(!collector.is_active());

        // Sample pushed while inactive should be ignored
        collector.record_sample(FrameSample {
            frame_index: 1,
            timestamp_us: 1000,
            ms_between_presents: 4.16,
            ms_until_displayed: None,
            frame_time_ms: 4.16,
        });
        assert_eq!(collector.count(), 0);

        collector.start();
        assert!(collector.is_active());

        for i in 1..=100 {
            collector.record_sample(FrameSample {
                frame_index: i,
                timestamp_us: i * 4166,
                ms_between_presents: 4.166,
                ms_until_displayed: None,
                frame_time_ms: 4.166,
            });
        }

        assert_eq!(collector.count(), 100);
        collector.stop();
        assert!(!collector.is_active());

        let metrics = collector.calculate_metrics(10).expect("Metrics calculation failed");
        assert_eq!(metrics.total_frames, 90);
        assert!(metrics.avg_fps > 235.0 && metrics.avg_fps < 245.0);

        collector.clear();
        assert_eq!(collector.count(), 0);
    }

    /// Benchmarks the in-process synthetic user-mode sample ingestion overhead of the FrameCollector.
    ///
    /// IMPORTANT TELEMETRY OVERHEAD CLASSIFICATION:
    /// - This test measures ONLY user-mode memory ingestion into the FrameCollector ring buffer.
    /// - It DOES NOT measure kernel ETW buffer management, context switches for ProcessTrace,
    ///   or dxgi.dll provider emission overhead.
    /// - Actual production ETW collection overhead is classified as:
    ///   `UNVERIFIED — production ETW collection overhead not measured` (requires live elevated gaming benchmark capture).
    #[test]
    fn test_synthetic_collector_ingestion_cpu_overhead() {
        use windows::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};
        use windows::Win32::Foundation::FILETIME;

        let collector = FrameCollector::new();
        collector.start();

        fn filetime_to_u64(ft: &FILETIME) -> u64 {
            ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
        }

        let mut ct = FILETIME::default();
        let mut et = FILETIME::default();
        let mut kt_start = FILETIME::default();
        let mut ut_start = FILETIME::default();

        unsafe {
            let _ = GetProcessTimes(GetCurrentProcess(), &mut ct, &mut et, &mut kt_start, &mut ut_start);
        }

        let wall_start = std::time::Instant::now();

        // Ingest 50,000 synthetic frame telemetry samples into user-mode collector
        for i in 1..=50_000 {
            collector.record_sample(FrameSample {
                frame_index: i as u64,
                timestamp_us: (i * 4166) as u64,
                ms_between_presents: 4.166,
                ms_until_displayed: None,
                frame_time_ms: 4.166,
            });
        }

        let wall_elapsed = wall_start.elapsed();

        let mut kt_end = FILETIME::default();
        let mut ut_end = FILETIME::default();
        unsafe {
            let _ = GetProcessTimes(GetCurrentProcess(), &mut ct, &mut et, &mut kt_end, &mut ut_end);
        }

        let cpu_ticks = (filetime_to_u64(&kt_end) - filetime_to_u64(&kt_start))
            + (filetime_to_u64(&ut_end) - filetime_to_u64(&ut_start));
        // 1 FILETIME tick = 100 nanoseconds = 0.1 microseconds
        let cpu_time_ms = (cpu_ticks as f64) * 0.0001;

        // In real gameplay at 240 FPS, 50,000 frames spans 208.3 seconds (over 3.4 minutes of match play)
        let simulated_gameplay_duration_ms = (50_000.0 / 240.0) * 1000.0;
        let cpu_utilization_percent = (cpu_time_ms / simulated_gameplay_duration_ms) * 100.0;

        println!(
            "Synthetic Collector Ingestion: 50,000 frames ingested in {:?}, consumed {:.2}ms CPU time. In-process overhead across 3.4 mins of 240Hz play = {:.4}%. (NOTE: Production kernel ETW collection overhead is UNVERIFIED).",
            wall_elapsed, cpu_time_ms, cpu_utilization_percent
        );

        // Verification requirement: In-process synthetic ingestion overhead must be < 0.2%
        assert!(
            cpu_utilization_percent < 0.2,
            "Synthetic frame ingestion overhead must be < 0.2%, measured {:.4}%",
            cpu_utilization_percent
        );
    }
}
