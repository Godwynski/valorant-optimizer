//! Statistical computation engine for frame-time distribution, percentiles, and A/B Student's t-tests.

use super::models::{ABComparisonReport, BenchmarkMetrics, EnvironmentalTelemetry, FrameSample, TelemetryProvenance};

/// Compute complete benchmark metrics from a slice of frame telemetry samples.
/// Optionally trims initial warmup frames to eliminate shader caching / level load hitching.
pub fn compute_metrics(
    samples: &[FrameSample],
    warmup_frames: usize,
) -> Result<BenchmarkMetrics, String> {
    compute_metrics_with_provenance(samples, warmup_frames, TelemetryProvenance::default(), None)
}

/// Compute complete benchmark metrics including provenance metadata and environmental conditions.
pub fn compute_metrics_with_provenance(
    samples: &[FrameSample],
    warmup_frames: usize,
    provenance: TelemetryProvenance,
    environment: Option<EnvironmentalTelemetry>,
) -> Result<BenchmarkMetrics, String> {
    if samples.len() <= warmup_frames {
        return Err(format!(
            "Insufficient samples: total {} frames, but warmup required {}",
            samples.len(),
            warmup_frames
        ));
    }

    let active_samples = &samples[warmup_frames..];
    let total_frames = active_samples.len();

    let mut frame_times: Vec<f64> = active_samples.iter().map(|s| s.frame_time_ms).collect();
    if frame_times.is_empty() {
        return Err("No active frame times available for analysis".to_string());
    }

    let total_duration_ms: f64 = frame_times.iter().sum();
    let duration_seconds = total_duration_ms / 1000.0;
    let avg_fps = (total_frames as f64) / duration_seconds;

    let frame_time_mean_ms = total_duration_ms / (total_frames as f64);

    // Variance & Standard Deviation
    let variance: f64 = if total_frames > 1 {
        frame_times
            .iter()
            .map(|&t| (t - frame_time_mean_ms).powi(2))
            .sum::<f64>()
            / ((total_frames - 1) as f64)
    } else {
        0.0
    };
    let frame_time_std_dev_ms = variance.sqrt();

    // Sort frame times in ascending order for percentile calculations
    frame_times.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let min_ft = frame_times[0];
    let max_ft = frame_times[total_frames - 1];

    let max_fps = if min_ft > 0.0 { 1000.0 / min_ft } else { 0.0 };
    let min_fps = if max_ft > 0.0 { 1000.0 / max_ft } else { 0.0 };

    let p50_frame_time_ms = calculate_percentile(&frame_times, 50.0);
    let p90_frame_time_ms = calculate_percentile(&frame_times, 90.0);
    let p95_frame_time_ms = calculate_percentile(&frame_times, 95.0);
    let p99_frame_time_ms = calculate_percentile(&frame_times, 99.0);
    let p99_9_frame_time_ms = calculate_percentile(&frame_times, 99.9);

    // 1% Low is derived from 99th percentile frame-time (CapFrameX / PresentMon convention)
    let one_percent_low_fps = if p99_frame_time_ms > 0.0 {
        1000.0 / p99_frame_time_ms
    } else {
        0.0
    };

    // 0.1% Low is derived from 99.9th percentile frame-time
    let zero_point_one_percent_low_fps = if p99_9_frame_time_ms > 0.0 {
        1000.0 / p99_9_frame_time_ms
    } else {
        0.0
    };

    // Count hitches / stutters
    let threshold_1_5x = frame_time_mean_ms * 1.5;
    let threshold_2_0x = frame_time_mean_ms * 2.0;

    let stutter_count_1_5x = frame_times.iter().filter(|&&t| t >= threshold_1_5x).count();
    let stutter_count_2_0x = frame_times.iter().filter(|&&t| t >= threshold_2_0x).count();

    Ok(BenchmarkMetrics {
        provenance,
        environment,
        total_frames,
        duration_seconds,
        avg_fps,
        one_percent_low_fps,
        zero_point_one_percent_low_fps,
        min_fps,
        max_fps,
        frame_time_mean_ms,
        frame_time_std_dev_ms,
        p50_frame_time_ms,
        p90_frame_time_ms,
        p95_frame_time_ms,
        p99_frame_time_ms,
        p99_9_frame_time_ms,
        stutter_count_1_5x,
        stutter_count_2_0x,
    })
}

/// Linear interpolation percentile on a pre-sorted ascending slice.
pub fn calculate_percentile(sorted_data: &[f64], percentile: f64) -> f64 {
    let n = sorted_data.len();
    if n == 0 {
        return 0.0;
    }
    if n == 1 {
        return sorted_data[0];
    }

    let p = percentile.clamp(0.0, 100.0);
    // Index position using standard rank: (N - 1) * p / 100
    let rank = (p / 100.0) * ((n - 1) as f64);
    let low_idx = rank.floor() as usize;
    let high_idx = (low_idx + 1).min(n - 1);
    let frac = rank - (low_idx as f64);

    sorted_data[low_idx] + frac * (sorted_data[high_idx] - sorted_data[low_idx])
}

/// Compute two-sample Welch's t-test (unequal variances) for two sample populations.
/// Returns (t_statistic, degrees_of_freedom, two_tailed_p_value).
pub fn welch_t_test(sample_a: &[f64], sample_b: &[f64]) -> Result<(f64, f64, f64), String> {
    let n1 = sample_a.len() as f64;
    let n2 = sample_b.len() as f64;

    if n1 < 2.0 || n2 < 2.0 {
        return Err(format!(
            "Welch's t-test requires at least 2 samples per group, got {} and {}",
            n1, n2
        ));
    }

    let mean1: f64 = sample_a.iter().sum::<f64>() / n1;
    let mean2: f64 = sample_b.iter().sum::<f64>() / n2;

    let var1: f64 = sample_a.iter().map(|&x| (x - mean1).powi(2)).sum::<f64>() / (n1 - 1.0);
    let var2: f64 = sample_b.iter().map(|&x| (x - mean2).powi(2)).sum::<f64>() / (n2 - 1.0);

    let se1 = var1 / n1;
    let se2 = var2 / n2;
    let se_diff = (se1 + se2).sqrt();

    if se_diff <= 1e-12 {
        // If there is zero variance and identical means
        if (mean1 - mean2).abs() < 1e-12 {
            return Ok((0.0, (n1 + n2 - 2.0), 1.0));
        }
        return Ok((f64::INFINITY, (n1 + n2 - 2.0), 0.0));
    }

    let t_stat = (mean2 - mean1) / se_diff;

    // Welch-Satterthwaite equation for degrees of freedom
    let df = (se1 + se2).powi(2) / ((se1.powi(2) / (n1 - 1.0)) + (se2.powi(2) / (n2 - 1.0)));

    let p_value = compute_t_distribution_two_tailed_p(t_stat.abs(), df);

    Ok((t_stat, df, p_value))
}

/// Compute two-tailed p-value from t-statistic and degrees of freedom
/// using the regularized incomplete beta function: p = I_{df / (df + t^2)}(df/2, 1/2)
pub fn compute_t_distribution_two_tailed_p(t: f64, df: f64) -> f64 {
    if df <= 0.0 {
        return 1.0;
    }
    let x = df / (df + t.powi(2));
    regularized_incomplete_beta(df / 2.0, 0.5, x)
}

/// Regularized incomplete beta function I_x(a, b) via continued fractions.
pub fn regularized_incomplete_beta(a: f64, b: f64, x: f64) -> f64 {
    if x <= 0.0 {
        return 0.0;
    }
    if x >= 1.0 {
        return 1.0;
    }

    // Symmetry transform if needed
    if x > (a + 1.0) / (a + b + 2.0) {
        return 1.0 - regularized_incomplete_beta(b, a, 1.0 - x);
    }

    // Factor before continued fraction: exp(ln_gamma(a+b) - ln_gamma(a) - ln_gamma(b) + a*ln(x) + b*ln(1-x))
    let lbeta = ln_gamma(a + b) - ln_gamma(a) - ln_gamma(b);
    let front = (lbeta + a * x.ln() + b * (1.0 - x).ln()).exp() / a;

    // Continued fraction calculation (Lentz's method)
    let max_iter = 200;
    let eps = 1e-15;
    let mut c = 1.0;
    let mut d = 1.0 - (a + b) * x / (a + 1.0);
    if d.abs() < eps {
        d = eps;
    }
    d = 1.0 / d;
    let mut h = d;

    for m in 1..=max_iter {
        let mf = m as f64;
        // Even step: 2m
        let num_even = -(a + mf) * (a + b + mf) * x / ((a + 2.0 * mf) * (a + 2.0 * mf + 1.0));
        d = 1.0 + num_even * d;
        if d.abs() < eps {
            d = eps;
        }
        c = 1.0 + num_even / c;
        if c.abs() < eps {
            c = eps;
        }
        d = 1.0 / d;
        h *= d * c;

        // Odd step: 2m + 1
        let num_odd = mf * (b - mf) * x / ((a + 2.0 * mf - 1.0) * (a + 2.0 * mf));
        d = 1.0 + num_odd * d;
        if d.abs() < eps {
            d = eps;
        }
        c = 1.0 + num_odd / c;
        if c.abs() < eps {
            c = eps;
        }
        d = 1.0 / d;
        let delta = d * c;
        h *= delta;

        if (delta - 1.0).abs() < eps {
            break;
        }
    }

    front * h
}

/// Natural log of the Gamma function via Lanczos approximation (g=7, n=9).
pub fn ln_gamma(z: f64) -> f64 {
    if z <= 0.0 {
        return 0.0;
    }
    const P: [f64; 9] = [
        0.99999999999980993,
        676.5203681218851,
        -1259.1392167224028,
        771.32342877765313,
        -176.61502916214059,
        12.507343278686905,
        -0.13857109526572012,
        9.9843695780195716e-6,
        1.5056327351493116e-7,
    ];

    let mut x = P[0];
    let t = z + 7.0 - 0.5;
    for i in 1..9 {
        x += P[i] / (z + (i as f64) - 1.0);
    }
    0.5 * (2.0 * std::f64::consts::PI).ln() + (z - 0.5) * t.ln() - t + x.ln()
}

/// Evaluate multi-trial benchmark distributions (Baseline vs Optimized) and produce
/// an A/B Comparison Report with statistical significance verification.
pub fn compare_ab_trials(
    baseline_runs: &[BenchmarkMetrics],
    optimized_runs: &[BenchmarkMetrics],
) -> Result<ABComparisonReport, String> {
    if baseline_runs.is_empty() || optimized_runs.is_empty() {
        return Err("Cannot compare empty benchmark trial sets".to_string());
    }

    let n = baseline_runs.len().min(optimized_runs.len());
    let base_fps: Vec<f64> = baseline_runs.iter().map(|m| m.avg_fps).collect();
    let opt_fps: Vec<f64> = optimized_runs.iter().map(|m| m.avg_fps).collect();

    let base_1p: Vec<f64> = baseline_runs.iter().map(|m| m.one_percent_low_fps).collect();
    let opt_1p: Vec<f64> = optimized_runs.iter().map(|m| m.one_percent_low_fps).collect();

    let base_01p: Vec<f64> = baseline_runs.iter().map(|m| m.zero_point_one_percent_low_fps).collect();
    let opt_01p: Vec<f64> = optimized_runs.iter().map(|m| m.zero_point_one_percent_low_fps).collect();

    let base_pacing: Vec<f64> = baseline_runs.iter().map(|m| m.frame_time_std_dev_ms).collect();
    let opt_pacing: Vec<f64> = optimized_runs.iter().map(|m| m.frame_time_std_dev_ms).collect();

    let baseline_avg_fps = base_fps.iter().sum::<f64>() / (base_fps.len() as f64);
    let optimized_avg_fps = opt_fps.iter().sum::<f64>() / (opt_fps.len() as f64);
    let delta_avg_fps_percent = ((optimized_avg_fps - baseline_avg_fps) / baseline_avg_fps) * 100.0;

    let baseline_one_percent_low = base_1p.iter().sum::<f64>() / (base_1p.len() as f64);
    let optimized_one_percent_low = opt_1p.iter().sum::<f64>() / (opt_1p.len() as f64);
    let delta_one_percent_low_percent = ((optimized_one_percent_low - baseline_one_percent_low) / baseline_one_percent_low) * 100.0;

    let baseline_zero_one_percent_low = base_01p.iter().sum::<f64>() / (base_01p.len() as f64);
    let optimized_zero_one_percent_low = opt_01p.iter().sum::<f64>() / (opt_01p.len() as f64);
    let delta_zero_one_percent_low_percent = ((optimized_zero_one_percent_low - baseline_zero_one_percent_low) / baseline_zero_one_percent_low) * 100.0;

    let baseline_pacing_std_dev = base_pacing.iter().sum::<f64>() / (base_pacing.len() as f64);
    let optimized_pacing_std_dev = opt_pacing.iter().sum::<f64>() / (opt_pacing.len() as f64);
    let delta_pacing_std_dev_percent = ((optimized_pacing_std_dev - baseline_pacing_std_dev) / baseline_pacing_std_dev) * 100.0;

    let (t_statistic, degrees_of_freedom, p_value) = welch_t_test(&base_1p, &opt_1p)?;
    let (mann_whitney_u_stat, mann_whitney_p_value) = mann_whitney_u_test(&base_1p, &opt_1p).unwrap_or((0.0, 1.0));
    let bootstrap_one_percent_ci = bootstrap_percentile_delta_ci(&base_1p, &opt_1p, 2000, 0.95).unwrap_or((0.0, 0.0));
    let (levene_f_stat, levene_p_value) = levene_variance_test(&base_pacing, &opt_pacing).unwrap_or((0.0, 1.0));
    let is_variance_significantly_reduced = levene_p_value < 0.05 && optimized_pacing_std_dev < baseline_pacing_std_dev;

    // Both parametric and non-parametric tests considered for robust confirmation
    let is_statistically_significant = p_value < 0.01 && mann_whitney_p_value < 0.05;
    let meets_threshold = is_statistically_significant && (delta_one_percent_low_percent >= 3.0 || delta_avg_fps_percent >= 3.0);

    let recommendation = if meets_threshold {
        "ACCEPTED — Statistically significant performance improvement confirmed (p < 0.01, Delta >= 3.0%)".to_string()
    } else if is_statistically_significant && delta_one_percent_low_percent > 0.0 {
        "BORDERLINE — Statistically significant but under 3.0% effect threshold (Minor gain)".to_string()
    } else if delta_one_percent_low_percent < -1.0 {
        "REJECTED — Optimization caused performance regression".to_string()
    } else {
        "REJECTED (Placebo / Statistically Insignificant) — Failed significance test".to_string()
    };

    let provenance = baseline_runs.first().map(|r| r.provenance.clone()).unwrap_or_default();
    let environment = optimized_runs.last().and_then(|r| r.environment.clone()).or_else(|| baseline_runs.last().and_then(|r| r.environment.clone()));

    Ok(ABComparisonReport {
        baseline_label: "Stock Windows Baseline".to_string(),
        optimized_label: "Optimized Profile".to_string(),
        trials_count: n,
        provenance,
        environment,
        baseline_avg_fps,
        optimized_avg_fps,
        delta_avg_fps_percent,
        baseline_one_percent_low,
        optimized_one_percent_low,
        delta_one_percent_low_percent,
        baseline_zero_one_percent_low,
        optimized_zero_one_percent_low,
        delta_zero_one_percent_low_percent,
        baseline_pacing_std_dev,
        optimized_pacing_std_dev,
        delta_pacing_std_dev_percent,
        t_statistic,
        degrees_of_freedom,
        p_value,
        mann_whitney_u_stat,
        mann_whitney_p_value,
        bootstrap_one_percent_ci,
        levene_f_stat,
        levene_p_value,
        is_variance_significantly_reduced,
        is_statistically_significant,
        meets_threshold,
        recommendation,
    })
}

/// Standard normal cumulative distribution function Φ(x).
pub fn standard_normal_cdf(x: f64) -> f64 {
    // Abramowitz and Stegun formula 7.1.26 approximation for erf(x)
    let sign = if x < 0.0 { -1.0 } else { 1.0 };
    let z = x.abs() / std::f64::consts::SQRT_2;
    let p = 0.3275911;
    let t = 1.0 / (1.0 + p * z);
    let a1 = 0.254829592;
    let a2 = -0.284496736;
    let a3 = 1.421413741;
    let a4 = -1.453152027;
    let a5 = 1.061405429;
    let poly = (((a5 * t + a4) * t + a3) * t + a2) * t + a1;
    let erf_val = sign * (1.0 - poly * (-z * z).exp());
    0.5 * (1.0 + erf_val)
}

/// Non-parametric Mann-Whitney U test (Wilcoxon rank-sum) for comparing two continuous
/// non-normal distributions with tie handling and continuity correction.
/// Returns (U_statistic, two_tailed_p_value).
pub fn mann_whitney_u_test(sample_a: &[f64], sample_b: &[f64]) -> Result<(f64, f64), String> {
    let n1 = sample_a.len();
    let n2 = sample_b.len();

    if n1 < 2 || n2 < 2 {
        return Err(format!("Mann-Whitney U requires >= 2 samples per group, got {} and {}", n1, n2));
    }

    let n1_f = n1 as f64;
    let n2_f = n2 as f64;
    let n_total = n1 + n2;

    // Combine samples with group labels: 0 for A, 1 for B
    let mut combined: Vec<(f64, u8)> = Vec::with_capacity(n_total);
    for &x in sample_a {
        combined.push((x, 0));
    }
    for &x in sample_b {
        combined.push((x, 1));
    }

    combined.sort_by(|(v1, _), (v2, _)| v1.partial_cmp(v2).unwrap_or(std::cmp::Ordering::Equal));

    // Assign mid-ranks to ties
    let mut ranks = vec![0.0f64; n_total];
    let mut tie_sum = 0.0f64;
    let mut i = 0;

    while i < n_total {
        let mut j = i;
        while j < n_total && (combined[j].0 - combined[i].0).abs() < 1e-12 {
            j += 1;
        }
        let tie_count = (j - i) as f64;
        let avg_rank = ((i + 1 + j) as f64) / 2.0;
        for k in i..j {
            ranks[k] = avg_rank;
        }
        if tie_count > 1.0 {
            tie_sum += (tie_count.powi(3) - tie_count) / (n_total as f64 * (n_total as f64 - 1.0));
        }
        i = j;
    }

    let mut rank_sum_a = 0.0f64;
    for (k, (_, group)) in combined.iter().enumerate() {
        if *group == 0 {
            rank_sum_a += ranks[k];
        }
    }

    let u1 = n1_f * n2_f + (n1_f * (n1_f + 1.0)) / 2.0 - rank_sum_a;
    let u2 = n1_f * n2_f - u1;
    let u = u1.min(u2);

    let mu_u = (n1_f * n2_f) / 2.0;
    let var_u = (n1_f * n2_f / 12.0) * ((n_total as f64 + 1.0) - tie_sum);

    if var_u <= 1e-12 {
        let p = if (u - mu_u).abs() < 1e-12 { 1.0 } else { 0.0 };
        return Ok((u, p));
    }

    let sigma_u = var_u.sqrt();
    let z = if u < mu_u {
        (u - mu_u + 0.5) / sigma_u
    } else if u > mu_u {
        (u - mu_u - 0.5) / sigma_u
    } else {
        0.0
    };

    let p_value = (2.0 * (1.0 - standard_normal_cdf(z.abs()))).clamp(0.0, 1.0);
    Ok((u, p_value))
}

/// Compute 95% bootstrap confidence interval of percentage delta between sample populations.
/// Uses deterministic pseudo-random sampling with replacement.
pub fn bootstrap_percentile_delta_ci(
    sample_a: &[f64],
    sample_b: &[f64],
    resamples: usize,
    confidence_level: f64,
) -> Result<(f64, f64), String> {
    if sample_a.is_empty() || sample_b.is_empty() {
        return Err("Cannot bootstrap with empty sample sets".to_string());
    }

    let mut deltas = Vec::with_capacity(resamples);
    let mut rng_state: u64 = 0x9E3779B97F4A7C15;

    // Xorshift64 PRNG for fast deterministic bootstrapping
    let mut next_rand = || {
        rng_state ^= rng_state << 13;
        rng_state ^= rng_state >> 7;
        rng_state ^= rng_state << 17;
        rng_state
    };

    let n1 = sample_a.len();
    let n2 = sample_b.len();

    for _ in 0..resamples {
        let mut sum_a = 0.0;
        for _ in 0..n1 {
            let idx = (next_rand() as usize) % n1;
            sum_a += sample_a[idx];
        }
        let mean_a = sum_a / (n1 as f64);

        let mut sum_b = 0.0;
        for _ in 0..n2 {
            let idx = (next_rand() as usize) % n2;
            sum_b += sample_b[idx];
        }
        let mean_b = sum_b / (n2 as f64);

        if mean_a > 0.0 {
            let delta_pct = ((mean_b - mean_a) / mean_a) * 100.0;
            deltas.push(delta_pct);
        }
    }

    if deltas.is_empty() {
        return Ok((0.0, 0.0));
    }

    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let alpha = 1.0 - confidence_level;
    let lower_pct = (alpha / 2.0) * 100.0;
    let upper_pct = (1.0 - alpha / 2.0) * 100.0;

    let lower = calculate_percentile(&deltas, lower_pct);
    let upper = calculate_percentile(&deltas, upper_pct);

    Ok((lower, upper))
}

/// Brown-Forsythe variant of Levene's test for equality of variances across two groups.
/// Robust against non-normal, skewed frame-time distributions.
/// Returns (F_statistic, p_value).
pub fn levene_variance_test(sample_a: &[f64], sample_b: &[f64]) -> Result<(f64, f64), String> {
    let n1 = sample_a.len();
    let n2 = sample_b.len();

    if n1 < 2 || n2 < 2 {
        return Err(format!("Levene test requires >= 2 samples per group, got {} and {}", n1, n2));
    }

    let mut sorted_a = sample_a.to_vec();
    let mut sorted_b = sample_b.to_vec();
    sorted_a.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    sorted_b.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

    let med_a = calculate_percentile(&sorted_a, 50.0);
    let med_b = calculate_percentile(&sorted_b, 50.0);

    let z1: Vec<f64> = sample_a.iter().map(|&x| (x - med_a).abs()).collect();
    let z2: Vec<f64> = sample_b.iter().map(|&x| (x - med_b).abs()).collect();

    let mean_z1 = z1.iter().sum::<f64>() / (n1 as f64);
    let mean_z2 = z2.iter().sum::<f64>() / (n2 as f64);
    let overall_mean_z = (z1.iter().sum::<f64>() + z2.iter().sum::<f64>()) / ((n1 + n2) as f64);

    let ss_between = (n1 as f64) * (mean_z1 - overall_mean_z).powi(2)
        + (n2 as f64) * (mean_z2 - overall_mean_z).powi(2);

    let ss_within: f64 = z1.iter().map(|&z| (z - mean_z1).powi(2)).sum::<f64>()
        + z2.iter().map(|&z| (z - mean_z2).powi(2)).sum::<f64>();

    let df_between = 1.0;
    let df_within = (n1 + n2 - 2) as f64;

    if ss_within <= 1e-12 {
        let p = if ss_between < 1e-12 { 1.0 } else { 0.0 };
        return Ok((0.0, p));
    }

    let f_stat = (ss_between / df_between) / (ss_within / df_within);
    // For df1 = 1, F = t^2 with df2 degrees of freedom. p = I_{df2 / (df2 + F)}(df2/2, 1/2)
    let p_value = compute_t_distribution_two_tailed_p(f_stat.sqrt(), df_within);

    Ok((f_stat, p_value))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_percentile_calculation() {
        let data = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let p50 = calculate_percentile(&data, 50.0);
        assert!((p50 - 5.5).abs() < 1e-4, "Expected p50 = 5.5, got {}", p50);

        let p0 = calculate_percentile(&data, 0.0);
        assert_eq!(p0, 1.0);

        let p100 = calculate_percentile(&data, 100.0);
        assert_eq!(p100, 10.0);
    }

    #[test]
    fn test_compute_metrics_synthetic() {
        // Generate 1000 frames with steady ~4.166ms (240 FPS) and deliberate stutters
        let mut samples = Vec::new();
        let mut cur_time = 0u64;

        for i in 0..1000 {
            // Introduce occasional stutters at frames 200, 400, 600, 800
            let ft = if i % 200 == 0 && i > 0 { 16.66 } else { 4.166 };
            cur_time += (ft * 1000.0) as u64;

            samples.push(FrameSample {
                frame_index: (i + 1) as u64,
                timestamp_us: cur_time,
                ms_between_presents: ft,
                ms_until_displayed: ft + 1.0,
                frame_time_ms: ft,
            });
        }

        // Test with 50 warmup frames discarded
        let metrics = compute_metrics(&samples, 50).expect("Metrics computation failed");
        assert_eq!(metrics.total_frames, 950);
        assert!(metrics.avg_fps > 220.0 && metrics.avg_fps < 245.0);
        assert!(metrics.one_percent_low_fps > 50.0);
        assert!(metrics.stutter_count_1_5x > 0);
        assert!(metrics.frame_time_std_dev_ms > 0.0);
    }

    #[test]
    fn test_welch_t_test_accuracy() {
        // Group A: High baseline variance
        let a = vec![230.0, 232.0, 229.0, 231.0, 230.5, 228.0, 233.0, 231.5, 230.0, 229.5];
        // Group B: Distinctly higher optimized performance
        let b = vec![255.0, 258.0, 256.0, 257.5, 259.0, 254.5, 258.0, 257.0, 256.5, 258.5];

        let (t_stat, df, p_val) = welch_t_test(&a, &b).expect("t-test should succeed");
        assert!(t_stat > 10.0, "Expected large t-statistic, got {}", t_stat);
        assert!(df > 10.0, "Expected valid degrees of freedom, got {}", df);
        assert!(p_val < 0.0001, "Expected highly significant p-value, got {}", p_val);
    }

    #[test]
    fn test_mann_whitney_u_test_accuracy() {
        let a = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let b = vec![11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0, 18.0, 19.0, 20.0];

        let (u_stat, p_val) = mann_whitney_u_test(&a, &b).expect("U-test must succeed");
        assert_eq!(u_stat, 0.0, "Complete separation must yield U = 0");
        assert!(p_val < 0.001, "Expected highly significant p-value for separated groups, got {}", p_val);

        // Identical groups
        let (u_ident, p_ident) = mann_whitney_u_test(&a, &a).expect("U-test on identical groups");
        assert_eq!(u_ident, 50.0);
        assert!((p_ident - 1.0).abs() < 1e-4, "Expected p-value ~ 1.0 for identical groups, got {}", p_ident);
    }

    #[test]
    fn test_bootstrap_confidence_interval() {
        let a = vec![100.0, 102.0, 101.0, 99.0, 100.5, 101.5];
        let b = vec![110.0, 112.0, 111.0, 109.0, 110.5, 111.5];

        let (ci_low, ci_high) = bootstrap_percentile_delta_ci(&a, &b, 1000, 0.95).expect("Bootstrap must succeed");
        // Mean A ~ 100.67, Mean B ~ 110.67, Delta ~ 9.93%
        assert!(ci_low > 7.0, "Lower CI must show positive gain, got {}", ci_low);
        assert!(ci_high < 13.0, "Upper CI must be bounded, got {}", ci_high);
        assert!(ci_low < ci_high, "CI bounds must be ordered: {} < {}", ci_low, ci_high);
    }

    #[test]
    fn test_levene_variance_test_accuracy() {
        // Group A: High variance
        let a = vec![10.0, 20.0, 5.0, 30.0, 2.0, 40.0, 8.0, 25.0, 15.0, 35.0];
        // Group B: Low variance (consistent pacing)
        let b = vec![19.5, 20.5, 20.0, 19.8, 20.2, 20.1, 19.9, 20.3, 19.7, 20.0];

        let (f_stat, p_val) = levene_variance_test(&a, &b).expect("Levene test must succeed");
        assert!(f_stat > 10.0, "Expected large F-statistic for variance difference, got {}", f_stat);
        assert!(p_val < 0.01, "Expected significant p-value for variance reduction, got {}", p_val);
    }
}
