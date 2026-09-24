//! Automated Multi-Trial A/B Testing Harness.
//!
//! Executes repeated benchmark trials (e.g. 10 Baseline vs 10 Optimized), calculates
//! Student's t-test statistical significance, and generates structured comparative reports.

use tracing::info;
use val_opt_shared::benchmarking::models::ABComparisonReport;
use val_opt_shared::benchmarking::stats::compare_ab_trials;

use super::frametimes::FrameCollector;
use super::synthetic::MockDirectXApp;

/// Configuration options for the automated A/B benchmark harness.
#[derive(Debug, Clone)]
pub struct ABBenchmarkConfig {
    /// Number of trials to run per state (recommended: 10).
    pub trials: usize,
    /// Number of frames to capture per trial (e.g. 1000 frames @ 240Hz = ~4.16s).
    pub frames_per_trial: usize,
    /// Warmup frames to discard from the beginning of each trial (default: 50).
    pub warmup_frames: usize,
    /// Target baseline FPS for simulation / validation (default: 230.0).
    pub baseline_fps: f64,
    /// Target optimized FPS for simulation / validation (default: 255.0).
    pub optimized_fps: f64,
}

impl Default for ABBenchmarkConfig {
    fn default() -> Self {
        Self {
            trials: 10,
            frames_per_trial: 1000,
            warmup_frames: 50,
            baseline_fps: 230.0,
            optimized_fps: 255.0,
        }
    }
}

/// The Multi-Trial A/B Benchmark Runner.
pub struct ABBenchmarkRunner {
    config: ABBenchmarkConfig,
}

impl ABBenchmarkRunner {
    pub fn new(config: ABBenchmarkConfig) -> Self {
        Self { config }
    }

    /// Run synthetic multi-trial benchmark comparing baseline vs optimized states.
    pub fn run_synthetic_benchmark(&self) -> Result<ABComparisonReport, String> {
        info!(
            trials = self.config.trials,
            frames_per_trial = self.config.frames_per_trial,
            "Executing Multi-Trial A/B Benchmark Suite..."
        );

        let mut baseline_runs = Vec::with_capacity(self.config.trials);
        let mut optimized_runs = Vec::with_capacity(self.config.trials);

        // Run Baseline Trials (simulating stock Windows pacing with periodic hitches)
        for _i in 1..=self.config.trials {
            let app = MockDirectXApp::new(self.config.baseline_fps)
                .with_stutters(150, 16.66)
                .with_virtual_time(true); // Fast deterministic trial execution
            let collector = FrameCollector::new();
            collector.start();

            app.run_frames(self.config.frames_per_trial, |sample| {
                collector.record_sample(sample);
            });

            collector.stop();
            let metrics = collector.calculate_metrics(self.config.warmup_frames)?;
            baseline_runs.push(metrics);
        }

        // Run Optimized Trials (simulating flattened frame-times and cleaner pacing)
        for _i in 1..=self.config.trials {
            let app = MockDirectXApp::new(self.config.optimized_fps)
                .with_stutters(400, 8.33)
                .with_virtual_time(true); // Fast deterministic trial execution
            let collector = FrameCollector::new();
            collector.start();

            app.run_frames(self.config.frames_per_trial, |sample| {
                collector.record_sample(sample);
            });

            collector.stop();
            let metrics = collector.calculate_metrics(self.config.warmup_frames)?;
            optimized_runs.push(metrics);
        }

        compare_ab_trials(&baseline_runs, &optimized_runs)
    }

    /// Render an A/B Comparison Report into a readable Markdown document.
    pub fn format_markdown_report(report: &ABComparisonReport) -> String {
        format!(
            r#"# Empirical A/B Benchmark Report: {} vs {}

---

## 1. Executive Summary & Verdict
- **Trials per State:** {} repeated runs
- **p-value (Student's t-test):** {:.6} (Threshold: p < 0.01)
- **Statistically Significant:** **{}**
- **Meets 3.0% Minimum Effect:** **{}**
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

## 3. Inferential Statistics Details
- **Test Type:** Two-Tailed Welch's t-test (unequal variance)
- **t-statistic:** {:.4}
- **Degrees of Freedom ($\nu$):** {:.2}
- **Significance Level ($\alpha$):** 0.01
"#,
            report.baseline_label,
            report.optimized_label,
            report.trials_count,
            report.p_value,
            if report.is_statistically_significant { "YES (p < 0.01)" } else { "NO (p >= 0.01)" },
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
            report.t_statistic,
            report.degrees_of_freedom,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ab_runner_10_trials_synthetic() {
        let config = ABBenchmarkConfig {
            trials: 10,
            frames_per_trial: 300,
            warmup_frames: 20,
            baseline_fps: 220.0,
            optimized_fps: 245.0,
        };

        let runner = ABBenchmarkRunner::new(config);
        let report = runner.run_synthetic_benchmark().expect("Benchmark execution failed");

        assert_eq!(report.trials_count, 10);
        assert!(report.delta_avg_fps_percent > 5.0, "Delta FPS should show clear improvement");
        assert!(report.is_statistically_significant, "10 trials with distinct distribution must be p < 0.01");
        assert!(report.meets_threshold, "Must meet threshold");
        assert!(report.recommendation.contains("ACCEPTED"));

        let md = ABBenchmarkRunner::format_markdown_report(&report);
        assert!(md.contains("Empirical A/B Benchmark Report"));
        assert!(md.contains("Average FPS"));
    }
}
