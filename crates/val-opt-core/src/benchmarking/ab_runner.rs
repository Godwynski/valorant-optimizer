//! Automated Multi-Trial Interleaved A/B Benchmarking Harness.
//!
//! Executes repeated benchmark trials in strictly interleaved order:
//! A_1 -> B_1 -> A_2 -> B_2 -> ... -> A_n -> B_n
//!
//! Interleaving minimizes time-dependent confounding factors (such as gradual thermal build-up,
//! background OS tasks, or memory fragmentation). Records real-time environmental telemetry
//! (CPU utilization, RAM load, AC power status, process count) across each trial.
//!
//! Computes non-parametric statistics (Mann-Whitney U, Bootstrap 95% CI) and robust variance
//! tests (Brown-Forsythe / Levene test) to avoid invalid normality assumptions on frame-time percentiles.

use std::time::Duration;
use tracing::info;
use val_opt_shared::benchmarking::models::{ABComparisonReport, EnvironmentalTelemetry, TelemetryProvenance, TelemetrySource};
use val_opt_shared::benchmarking::stats::{compare_ab_trials, compute_metrics_with_provenance};

use super::etw_capture::{EtwFrameCaptureEngine, FrameCaptureConfig};
use super::frametimes::FrameCollector;
use super::synthetic::MockDirectXApp;

/// Samples the host environment state (CPU, memory, power, processes).
pub fn sample_environmental_telemetry() -> EnvironmentalTelemetry {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::FILETIME;
        use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
        use windows::Win32::System::Threading::GetSystemTimes;

        // 1. Memory load percentage
        let mut mem_status = MEMORYSTATUSEX::default();
        mem_status.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        let mem_load = unsafe {
            if GlobalMemoryStatusEx(&mut mem_status).is_ok() {
                mem_status.dwMemoryLoad
            } else {
                0
            }
        };

        // 2. AC utility power status
        let mut power_status = SYSTEM_POWER_STATUS::default();
        let ac_online = unsafe {
            if GetSystemPowerStatus(&mut power_status).is_ok() {
                Some(power_status.ACLineStatus == 1)
            } else {
                None
            }
        };

        // 3. CPU utilization over a brief delta (30ms)
        let filetime_to_u64 = |ft: FILETIME| -> u64 {
            ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64)
        };

        let mut idle1 = FILETIME::default();
        let mut kernel1 = FILETIME::default();
        let mut user1 = FILETIME::default();
        let cpu_util = unsafe {
            if GetSystemTimes(Some(&mut idle1), Some(&mut kernel1), Some(&mut user1)).is_ok() {
                std::thread::sleep(Duration::from_millis(30));
                let mut idle2 = FILETIME::default();
                let mut kernel2 = FILETIME::default();
                let mut user2 = FILETIME::default();
                if GetSystemTimes(Some(&mut idle2), Some(&mut kernel2), Some(&mut user2)).is_ok() {
                    let idle = filetime_to_u64(idle2).saturating_sub(filetime_to_u64(idle1));
                    let kernel = filetime_to_u64(kernel2).saturating_sub(filetime_to_u64(kernel1));
                    let user = filetime_to_u64(user2).saturating_sub(filetime_to_u64(user1));
                    let total = kernel + user;
                    if total > 0 && total >= idle {
                        let active = total - idle;
                        ((active as f64) / (total as f64)) * 100.0
                    } else {
                        0.0
                    }
                } else {
                    0.0
                }
            } else {
                0.0
            }
        };

        // 4. Active processes count
        let mut process_count = 0u32;
        unsafe {
            use windows::Win32::System::Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                TH32CS_SNAPPROCESS,
            };
            if let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) {
                let mut entry = PROCESSENTRY32W::default();
                entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
                if Process32FirstW(snapshot, &mut entry).is_ok() {
                    loop {
                        process_count += 1;
                        if Process32NextW(snapshot, &mut entry).is_err() {
                            break;
                        }
                    }
                }
                let _ = windows::Win32::Foundation::CloseHandle(snapshot);
            }
        }

        EnvironmentalTelemetry {
            cpu_utilization_percent: (cpu_util * 10.0).round() / 10.0,
            memory_load_percent: mem_load,
            thermal_status: "WMI / ACPI thermal zones unqueried (hardware-specific kernel driver required)".to_string(),
            ac_power_online: ac_online,
            active_processes_count: process_count,
        }
    }

    #[cfg(not(windows))]
    {
        EnvironmentalTelemetry::default()
    }
}

/// Configuration options for the interleaved A/B benchmark harness.
#[derive(Debug, Clone)]
pub struct ABBenchmarkConfig {
    /// Number of paired trials to run (recommended: 10, yields 10 A and 10 B trials).
    pub trials: usize,
    /// Number of frames to capture per trial (for frame-count based benchmarks).
    pub frames_per_trial: usize,
    /// Duration of each capture trial (for time-based ETW benchmarks, e.g. 5.0s).
    pub trial_duration: Duration,
    /// Warmup frames to discard from the beginning of each trial (default: 50).
    pub warmup_frames: usize,
    /// Target baseline FPS for synthetic validation (default: 230.0).
    pub baseline_fps: f64,
    /// Target optimized FPS for synthetic validation (default: 255.0).
    pub optimized_fps: f64,
    /// Optional PID of game process to target for real ETW capture.
    pub target_pid: Option<u32>,
}

impl Default for ABBenchmarkConfig {
    fn default() -> Self {
        Self {
            trials: 10,
            frames_per_trial: 1000,
            trial_duration: Duration::from_secs(5),
            warmup_frames: 50,
            baseline_fps: 230.0,
            optimized_fps: 255.0,
            target_pid: None,
        }
    }
}

/// The Multi-Trial Interleaved A/B Benchmark Runner.
pub struct ABBenchmarkRunner {
    config: ABBenchmarkConfig,
}

impl ABBenchmarkRunner {
    pub fn new(config: ABBenchmarkConfig) -> Self {
        Self { config }
    }

    /// Access runner configuration.
    pub fn config(&self) -> &ABBenchmarkConfig {
        &self.config
    }

    /// Execute a live, genuine Windows ETW interleaved A/B benchmark suite against a target game process.
    ///
    /// Interleaves: A_1 -> B_1 -> A_2 -> B_2 -> ... -> A_n -> B_n
    ///
    /// Accepts a mutable state switch callback `apply_state(is_optimized: bool)` which switches
    /// the system optimization profile between trials.
    pub fn run_live_etw_benchmark<F>(
        &self,
        mut apply_state: F,
    ) -> Result<ABComparisonReport, String>
    where
        F: FnMut(bool) -> Result<(), String>,
    {
        if !EtwFrameCaptureEngine::is_elevation_available() {
            return Err("Administrator elevation required for live Windows ETW benchmark capture.".to_string());
        }

        info!(
            trials = self.config.trials,
            duration = ?self.config.trial_duration,
            pid = ?self.config.target_pid,
            "Executing Live Interleaved ETW A/B Benchmark Suite (A -> B -> A -> B)..."
        );

        let mut baseline_runs = Vec::with_capacity(self.config.trials);
        let mut optimized_runs = Vec::with_capacity(self.config.trials);

        for trial_idx in 1..=self.config.trials {
            info!(trial = trial_idx, "Starting interleaved pair trial...");

            // 1. Run Baseline Trial (A_i)
            apply_state(false)?;
            std::thread::sleep(Duration::from_millis(500)); // Settle period
            let env_a = sample_environmental_telemetry();

            let engine_a = EtwFrameCaptureEngine::new();
            let config_a = FrameCaptureConfig {
                session_name: format!("ValOptAB_A_{}", trial_idx),
                target_pid: self.config.target_pid,
                max_frames: None,
                duration_limit: Some(self.config.trial_duration),
            };

            let metrics_a = engine_a.capture_duration_blocking(
                config_a,
                self.config.trial_duration,
                self.config.warmup_frames,
                Some(env_a),
            )?;
            baseline_runs.push(metrics_a);

            // 2. Run Optimized Trial (B_i)
            apply_state(true)?;
            std::thread::sleep(Duration::from_millis(500)); // Settle period
            let env_b = sample_environmental_telemetry();

            let engine_b = EtwFrameCaptureEngine::new();
            let config_b = FrameCaptureConfig {
                session_name: format!("ValOptAB_B_{}", trial_idx),
                target_pid: self.config.target_pid,
                max_frames: None,
                duration_limit: Some(self.config.trial_duration),
            };

            let metrics_b = engine_b.capture_duration_blocking(
                config_b,
                self.config.trial_duration,
                self.config.warmup_frames,
                Some(env_b),
            )?;
            optimized_runs.push(metrics_b);
        }

        compare_ab_trials(&baseline_runs, &optimized_runs)
    }

    /// Execute synthetic multi-trial benchmark in interleaved sequence (A -> B -> A -> B).
    /// Strictly marked for offline unit testing, CI regression testing, and mathematical validation.
    pub fn run_synthetic_benchmark(&self) -> Result<ABComparisonReport, String> {
        info!(
            trials = self.config.trials,
            frames_per_trial = self.config.frames_per_trial,
            "Executing Synthetic Interleaved A/B Benchmark Suite (A -> B -> A -> B)..."
        );

        let mut baseline_runs = Vec::with_capacity(self.config.trials);
        let mut optimized_runs = Vec::with_capacity(self.config.trials);

        for _trial_idx in 1..=self.config.trials {
            // 1. Interleaved Baseline Trial (A_i)
            let app_a = MockDirectXApp::new(self.config.baseline_fps)
                .with_stutters(150, 16.66)
                .with_virtual_time(true);
            let collector_a = FrameCollector::new();
            collector_a.start();

            app_a.run_frames(self.config.frames_per_trial, |sample| {
                collector_a.record_sample(sample);
            });
            collector_a.stop();

            let snapshot_a = collector_a.snapshot();
            let prov_a = TelemetryProvenance {
                source: TelemetrySource::SyntheticFixture,
                collection_mechanism: "MockDirectXApp (Synthetic Frame Generator)".to_string(),
                timestamp_source: "Virtual QPC Simulation".to_string(),
                unit: "Milliseconds".to_string(),
                is_directly_measured: false,
                sample_count: snapshot_a.len(),
                dropped_events: 0,
                known_limitations: vec!["Synthetic test fixture; not real graphics presentation telemetry".to_string()],
            };
            let env_a = sample_environmental_telemetry();
            let metrics_a = compute_metrics_with_provenance(&snapshot_a, self.config.warmup_frames, prov_a, Some(env_a))?;
            baseline_runs.push(metrics_a);

            // 2. Interleaved Optimized Trial (B_i)
            let app_b = MockDirectXApp::new(self.config.optimized_fps)
                .with_stutters(400, 8.33)
                .with_virtual_time(true);
            let collector_b = FrameCollector::new();
            collector_b.start();

            app_b.run_frames(self.config.frames_per_trial, |sample| {
                collector_b.record_sample(sample);
            });
            collector_b.stop();

            let snapshot_b = collector_b.snapshot();
            let prov_b = TelemetryProvenance {
                source: TelemetrySource::SyntheticFixture,
                collection_mechanism: "MockDirectXApp (Synthetic Frame Generator)".to_string(),
                timestamp_source: "Virtual QPC Simulation".to_string(),
                unit: "Milliseconds".to_string(),
                is_directly_measured: false,
                sample_count: snapshot_b.len(),
                dropped_events: 0,
                known_limitations: vec!["Synthetic test fixture; not real graphics presentation telemetry".to_string()],
            };
            let env_b = sample_environmental_telemetry();
            let metrics_b = compute_metrics_with_provenance(&snapshot_b, self.config.warmup_frames, prov_b, Some(env_b))?;
            optimized_runs.push(metrics_b);
        }

        compare_ab_trials(&baseline_runs, &optimized_runs)
    }

    /// Render an A/B Comparison Report into a comprehensive GitHub markdown report.
    pub fn format_markdown_report(report: &ABComparisonReport) -> String {
        let env_summary = if let Some(ref env) = report.environment {
            format!(
                "- **CPU Utilization:** {:.1}%\n- **Physical Memory Load:** {}%\n- **AC Power Connected:** {}\n- **Active Host Processes:** {}\n- **Thermal Telemetry:** {}\n",
                env.cpu_utilization_percent,
                env.memory_load_percent,
                env.ac_power_online.map(|ac| if ac { "YES (AC Utility)" } else { "NO (Battery)" }).unwrap_or("Unknown"),
                env.active_processes_count,
                env.thermal_status
            )
        } else {
            "- **Environmental Telemetry:** None recorded\n".to_string()
        };

        format!(
            r#"# Empirical Interleaved A/B Benchmark Report: {} vs {}

---

## 1. Executive Summary & Verdict
- **Telemetry Source:** `{:?}`
- **Experimental Design:** Interleaved Paired Trials ($A_1 \to B_1 \to A_2 \to B_2 \dots$)
- **Trials per State:** {} repeated runs
- **Non-Parametric Mann-Whitney U Test:** p = {:.6} (U = {:.1})
- **Bootstrap 95% Confidence Interval (1% Low Delta):** [{:+.2} FPS, {:+.2} FPS]
- **Levene Pacing Variance Test:** p = {:.6} (F = {:.4})
- **Pacing Variance Significantly Reduced:** **{}**
- **Student's t-test p-value:** {:.6} ($\alpha = 0.01$)
- **Statistically Significant:** **{}**
- **Meets 3.0% Minimum Effect Threshold:** **{}**
- **Final Recommendation:** **{}**

---

## 2. Statistical Metric Comparisons

| Metric | Stock Baseline | Optimized Profile | Absolute Delta | Percentage Delta |
| :--- | :--- | :--- | :--- | :--- |
| **Average FPS** | {:.2} FPS | {:.2} FPS | {:+.2} FPS | {:+.2}% |
| **1% Low FPS (99th %ile)** | {:.2} FPS | {:.2} FPS | {:+.2} FPS | {:+.2}% |
| **0.1% Low FPS (99.9th %ile)** | {:.2} FPS | {:.2} FPS | {:+.2} FPS | {:+.2}% |
| **Frame-time Std Dev ($\sigma$)** | {:.3} ms | {:.3} ms | {:+.3} ms | {:+.2}% |

---

## 3. Host Environmental State & Control Telemetry
{}
---

## 4. Telemetry Provenance & Integrity
- **Collection Mechanism:** {}
- **Timestamp Source:** {}
- **Units:** {}
- **Directly Measured:** {}
- **Known Limitations:** {}
"#,
            report.baseline_label,
            report.optimized_label,
            report.provenance.source,
            report.trials_count,
            report.mann_whitney_p_value,
            report.mann_whitney_u_stat,
            report.bootstrap_one_percent_ci.0,
            report.bootstrap_one_percent_ci.1,
            report.levene_p_value,
            report.levene_f_stat,
            if report.is_variance_significantly_reduced { "YES (Pacing hitching reduced)" } else { "NO / Indeterminate" },
            report.p_value,
            if report.is_statistically_significant { "YES (Robust confirmation)" } else { "NO (Not significant)" },
            if report.meets_threshold { "YES (Delta >= 3.0%)" } else { "NO" },
            report.recommendation,
            report.baseline_avg_fps,
            report.optimized_avg_fps,
            report.optimized_avg_fps - report.baseline_avg_fps,
            report.delta_avg_fps_percent,
            report.baseline_one_percent_low,
            report.optimized_one_percent_low,
            report.optimized_one_percent_low - report.baseline_one_percent_low,
            report.delta_one_percent_low_percent,
            report.baseline_zero_one_percent_low,
            report.optimized_zero_one_percent_low,
            report.optimized_zero_one_percent_low - report.baseline_zero_one_percent_low,
            report.delta_zero_one_percent_low_percent,
            report.baseline_pacing_std_dev,
            report.optimized_pacing_std_dev,
            report.optimized_pacing_std_dev - report.baseline_pacing_std_dev,
            report.delta_pacing_std_dev_percent,
            env_summary,
            report.provenance.collection_mechanism,
            report.provenance.timestamp_source,
            report.provenance.unit,
            report.provenance.is_directly_measured,
            if report.provenance.known_limitations.is_empty() {
                "None".to_string()
            } else {
                report.provenance.known_limitations.join("; ")
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sample_environmental_telemetry() {
        let env = sample_environmental_telemetry();
        println!("Sampled environment: {:?}", env);
        #[cfg(windows)]
        {
            assert!(env.active_processes_count > 0, "Must detect running processes");
            assert!(env.memory_load_percent > 0, "Memory load must be > 0");
        }
    }

    #[test]
    fn test_interleaved_ab_runner_synthetic() {
        let config = ABBenchmarkConfig {
            trials: 10,
            frames_per_trial: 300,
            trial_duration: Duration::from_millis(50),
            warmup_frames: 20,
            baseline_fps: 220.0,
            optimized_fps: 245.0,
            target_pid: None,
        };

        let runner = ABBenchmarkRunner::new(config);
        let report = runner.run_synthetic_benchmark().expect("Benchmark execution failed");

        assert_eq!(report.trials_count, 10);
        assert_eq!(report.provenance.source, TelemetrySource::SyntheticFixture);
        assert!(report.delta_avg_fps_percent > 5.0, "Delta FPS should show clear improvement");
        assert!(report.is_statistically_significant, "Must pass significance testing");
        assert!(report.meets_threshold, "Must meet threshold");
        assert!(report.recommendation.contains("ACCEPTED"));

        let md = ABBenchmarkRunner::format_markdown_report(&report);
        assert!(md.contains("Empirical Interleaved A/B Benchmark Report"));
        assert!(md.contains("Mann-Whitney U"));
        assert!(md.contains("Bootstrap 95% Confidence Interval"));
        assert!(md.contains("Host Environmental State"));
        println!("{}", md);
    }
}
