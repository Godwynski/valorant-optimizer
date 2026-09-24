use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;
use std::time::Instant;
use val_opt_shared::benchmarking::models::FrameSample;
use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

pub struct MockDirectXApp {
    target_fps: f64,
    stutter_interval: usize,
    stutter_duration_ms: f64,
    virtual_time: bool,
    #[allow(dead_code)]
    is_running: Arc<AtomicBool>,
}

impl MockDirectXApp {
    /// Create a mock DirectX app targeting a specified refresh/frame rate (e.g. 240.0 FPS).
    pub fn new(target_fps: f64) -> Self {
        Self {
            target_fps,
            stutter_interval: 0,
            stutter_duration_ms: 0.0,
            virtual_time: false,
            is_running: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Enable virtual time for instant, deterministic simulation without busy-waiting.
    pub fn with_virtual_time(mut self, virtual_time: bool) -> Self {
        self.virtual_time = virtual_time;
        self
    }

    /// Configure periodic synthetic micro-stutters for testing pacing detection.
    pub fn with_stutters(mut self, interval_frames: usize, duration_ms: f64) -> Self {
        self.stutter_interval = interval_frames;
        self.stutter_duration_ms = duration_ms;
        self
    }

    /// Emit a stream of synthetic frame presentation events to a callback.
    /// Operates using Windows QueryPerformanceCounter for sub-microsecond precision.
    pub fn run_frames<F>(&self, total_frames: usize, mut on_frame: F)
    where
        F: FnMut(FrameSample),
    {
        let target_frame_time_ms = 1000.0 / self.target_fps;

        if self.virtual_time {
            let mut cumulative_us: u64 = 0;
            for frame_idx in 1..=total_frames {
                let mut current_frame_time_ms = target_frame_time_ms;
                if self.stutter_interval > 0 && frame_idx % self.stutter_interval == 0 {
                    current_frame_time_ms = self.stutter_duration_ms;
                }
                cumulative_us += (current_frame_time_ms * 1000.0) as u64;
                let sample = FrameSample {
                    frame_index: frame_idx as u64,
                    timestamp_us: cumulative_us,
                    ms_between_presents: current_frame_time_ms,
                    ms_until_displayed: current_frame_time_ms + 0.8,
                    frame_time_ms: current_frame_time_ms,
                };
                on_frame(sample);
            }
            return;
        }

        let mut qpc_freq: i64 = 0;
        let mut qpc_start: i64 = 0;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut qpc_freq);
            let _ = QueryPerformanceCounter(&mut qpc_start);
        }

        let mut last_qpc = qpc_start;

        for frame_idx in 1..=total_frames {
            let mut current_frame_time_ms = target_frame_time_ms;

            if self.stutter_interval > 0 && frame_idx % self.stutter_interval == 0 {
                current_frame_time_ms = self.stutter_duration_ms;
            }

            // High-resolution simulated render & present sleep
            let sleep_us = (current_frame_time_ms * 1000.0) as u64;
            let frame_start = Instant::now();
            while frame_start.elapsed().as_micros() < (sleep_us as u128) {
                std::hint::spin_loop();
            }

            let mut now_qpc: i64 = 0;
            unsafe {
                let _ = QueryPerformanceCounter(&mut now_qpc);
            }

            let elapsed_qpc_ticks = now_qpc - last_qpc;
            let measured_ms = (elapsed_qpc_ticks as f64 * 1000.0) / (qpc_freq as f64);
            let timestamp_us = ((now_qpc - qpc_start) as f64 * 1_000_000.0 / (qpc_freq as f64)) as u64;

            last_qpc = now_qpc;

            let sample = FrameSample {
                frame_index: frame_idx as u64,
                timestamp_us,
                ms_between_presents: measured_ms,
                ms_until_displayed: measured_ms + 0.8, // Typical DWM display latency offset
                frame_time_ms: measured_ms,
            };

            on_frame(sample);
        }
    }

    /// Run frame simulation asynchronously in a background thread for testing concurrent ingestion.
    pub fn spawn_async<F>(self, total_frames: usize, mut on_frame: F) -> thread::JoinHandle<()>
    where
        F: FnMut(FrameSample) + Send + 'static,
    {
        thread::spawn(move || {
            self.run_frames(total_frames, move |sample| {
                on_frame(sample);
            });
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn test_mock_directx_app_precision() {
        let app = MockDirectXApp::new(240.0);
        let frame_count = Arc::new(AtomicU64::new(0));
        let frame_count_clone = frame_count.clone();

        let mut samples = Vec::new();
        app.run_frames(50, |sample| {
            frame_count_clone.fetch_add(1, Ordering::Relaxed);
            samples.push(sample);
        });

        assert_eq!(frame_count.load(Ordering::Relaxed), 50);
        assert_eq!(samples.len(), 50);

        // Verify timestamps are strictly monotonically increasing
        for i in 1..samples.len() {
            assert!(
                samples[i].timestamp_us > samples[i - 1].timestamp_us,
                "Timestamps must strictly increase: frame {} ({}) <= frame {} ({})",
                i,
                samples[i].timestamp_us,
                i - 1,
                samples[i - 1].timestamp_us
            );
        }

        // Verify frame times cluster around ~4.166ms
        let avg_measured: f64 = samples.iter().skip(1).map(|s| s.frame_time_ms).sum::<f64>() / 49.0;
        assert!(
            (avg_measured - 4.166).abs() < 2.5,
            "Target ~4.166ms per frame, measured average = {}ms",
            avg_measured
        );
    }
}
