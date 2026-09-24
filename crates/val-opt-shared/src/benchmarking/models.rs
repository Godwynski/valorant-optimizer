//! Benchmarking and telemetry data models.

use serde::{Deserialize, Serialize};

/// An individual frame presentation telemetry sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FrameSample {
    /// 1-based sequential frame number.
    pub frame_index: u64,
    /// Absolute timestamp in microseconds (from high-precision timer / QPC).
    pub timestamp_us: u64,
    /// Time delta (in milliseconds) since the preceding Present call.
    pub ms_between_presents: f64,
    /// Time delta (in milliseconds) from Present call until displayed on screen.
    pub ms_until_displayed: f64,
    /// Effective frame render/presentation time in milliseconds.
    pub frame_time_ms: f64,
}

/// Aggregated statistical metrics for a single benchmark session.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkMetrics {
    /// Total number of frames analyzed (after warm-up trimming).
    pub total_frames: usize,
    /// Effective duration of analyzed frames in seconds.
    pub duration_seconds: f64,
    /// Mean frames per second throughput.
    pub avg_fps: f64,
    /// 1% Low FPS (calculated from 99th percentile frame-time).
    pub one_percent_low_fps: f64,
    /// 0.1% Low FPS (calculated from 99.9th percentile frame-time).
    pub zero_point_one_percent_low_fps: f64,
    /// Instantaneous minimum FPS.
    pub min_fps: f64,
    /// Instantaneous maximum FPS.
    pub max_fps: f64,
    /// Mean frame-time in milliseconds.
    pub frame_time_mean_ms: f64,
    /// Standard deviation of frame-times (frame pacing consistency metric).
    pub frame_time_std_dev_ms: f64,
    /// 50th percentile (median) frame time in milliseconds.
    pub p50_frame_time_ms: f64,
    /// 90th percentile frame time in milliseconds.
    pub p90_frame_time_ms: f64,
    /// 95th percentile frame time in milliseconds.
    pub p95_frame_time_ms: f64,
    /// 99th percentile frame time in milliseconds.
    pub p99_frame_time_ms: f64,
    /// 99.9th percentile frame time in milliseconds.
    pub p99_9_frame_time_ms: f64,
    /// Count of frames exceeding 1.5x average frame-time (minor hitches).
    pub stutter_count_1_5x: usize,
    /// Count of frames exceeding 2.0x average frame-time (severe stutters).
    pub stutter_count_2_0x: usize,
}

/// A complete benchmark run record with identifiers and summary metrics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchmarkRunReport {
    pub run_id: String,
    pub label: String,
    pub timestamp_utc: String,
    pub metrics: BenchmarkMetrics,
    pub raw_samples_count: usize,
}

/// Comparative A/B benchmark evaluation report between Baseline and Optimized runs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ABComparisonReport {
    pub baseline_label: String,
    pub optimized_label: String,
    pub trials_count: usize,
    pub baseline_avg_fps: f64,
    pub optimized_avg_fps: f64,
    pub delta_avg_fps_percent: f64,
    pub baseline_one_percent_low: f64,
    pub optimized_one_percent_low: f64,
    pub delta_one_percent_low_percent: f64,
    pub baseline_zero_one_percent_low: f64,
    pub optimized_zero_one_percent_low: f64,
    pub delta_zero_one_percent_low_percent: f64,
    pub baseline_pacing_std_dev: f64,
    pub optimized_pacing_std_dev: f64,
    pub delta_pacing_std_dev_percent: f64,
    pub t_statistic: f64,
    pub degrees_of_freedom: f64,
    pub p_value: f64,
    pub is_statistically_significant: bool,
    pub meets_threshold: bool,
    pub recommendation: String,
}

/// Telemetry metrics resulting from high-precision UDP jitter and ping probing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PingProbeReport {
    pub target_endpoint: String,
    pub packets_sent: u32,
    pub packets_received: u32,
    pub packet_loss_percent: f64,
    pub min_rtt_ms: f64,
    pub avg_rtt_ms: f64,
    pub max_rtt_ms: f64,
    pub median_rtt_ms: f64,
    pub jitter_ms: f64,
    pub rtt_std_dev_ms: f64,
}
