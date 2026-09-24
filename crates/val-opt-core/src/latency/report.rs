//! Latency Diagnostic Report Generator.
//!
//! Synthesizes DPC and ISR execution telemetry, kernel driver isolation rankings,
//! and threshold violations into comprehensive markdown and structured diagnostic reports.
//!
//! Evaluates hardware and driver suitability for competitive gaming:
//! - DPC latency < 500µs: Optimal for competitive VALORANT (0 frame drops from DPCs).
//! - DPC latency 500µs - 1000µs: Acceptable but at risk of micro-stutter during intense action.
//! - DPC latency >= 1000µs: Unsuitable; drivers causing audible crackling or visual hitches.

use val_opt_shared::models::latency::LatencyReport;
use super::etw_session::KernelLatencySessionManager;

/// Evaluates latency session metrics and generates a structured LatencyReport.
pub fn generate_report(session: &KernelLatencySessionManager, duration_secs: f64) -> LatencyReport {
    let isolator = session.isolator();
    let offending_drivers = isolator.get_offending_drivers();
    let top_drivers = isolator.get_top_drivers(10);

    let max_dpc = session.max_dpc_us();
    let max_isr = session.max_isr_us();

    let highest_dpc_driver = top_drivers.first().map(|d| d.driver_name.clone());
    let highest_isr_driver = top_drivers.get(1).map(|d| d.driver_name.clone()).or_else(|| highest_dpc_driver.clone());

    let suitable = max_dpc < 1000;

    let mut recommendations = Vec::new();
    if max_dpc >= 1000 {
        recommendations.push(
            "CRITICAL: System DPC latency exceeds 1000µs. Severe micro-stutter and frame drop hazard during gameplay."
                .to_string(),
        );
    } else if max_dpc >= 500 {
        recommendations.push(
            "WARNING: System DPC latency exceeds 500µs. Input processing may experience occasional micro-delays."
                .to_string(),
        );
    } else {
        recommendations.push(
            "EXCELLENT: Maximum DPC latency is well under 500µs. System hardware and drivers are optimal for competitive gaming."
                .to_string(),
        );
    }

    for off in &offending_drivers {
        let name_lower = off.driver_name.to_lowercase();
        if name_lower.contains("ndis") || name_lower.contains("rt640") || name_lower.contains("e1d") {
            recommendations.push(format!(
                "Network driver '{}' spiked to {}µs: Ensure EEE and Flow Control are disabled via val-opt network module.",
                off.driver_name, off.max_execution_us
            ));
        } else if name_lower.contains("nvlddmkm") || name_lower.contains("amdkmdag") {
            recommendations.push(format!(
                "GPU driver '{}' spiked to {}µs: Enable MSI (Message Signaled Interrupts) and set Power Management Mode to 'Prefer Maximum Performance'.",
                off.driver_name, off.max_execution_us
            ));
        } else if name_lower.contains("audio") || name_lower.contains("hdaudbus") {
            recommendations.push(format!(
                "Audio driver '{}' spiked to {}µs: Disable audio DSP/APO enhancements using val-opt Phase 3 optimizer.",
                off.driver_name, off.max_execution_us
            ));
        } else if name_lower.contains("acpi") || name_lower.contains("wdf01000") {
            recommendations.push(format!(
                "Power management driver '{}' spiked to {}µs: Review BIOS CPU C-States and USB power savings.",
                off.driver_name, off.max_execution_us
            ));
        }
    }

    LatencyReport {
        total_dpcs_captured: session.total_dpcs(),
        total_isrs_captured: session.total_isrs(),
        highest_dpc_us: max_dpc,
        highest_isr_us: max_isr,
        highest_dpc_driver,
        highest_isr_driver,
        offending_drivers,
        top_drivers,
        system_suitable_for_competitive: suitable,
        recommendations,
        test_duration_secs: duration_secs,
    }
}

/// Format the latency report as GitHub markdown.
pub fn format_markdown_report(report: &LatencyReport) -> String {
    let mut md = String::new();

    md.push_str("# Hardware & Driver Latency Health Diagnostic Report\n\n");
    md.push_str(&format!("- **Test Duration:** {:.2} seconds\n", report.test_duration_secs));
    md.push_str(&format!("- **Total DPCs Captured:** {}\n", report.total_dpcs_captured));
    md.push_str(&format!("- **Total ISRs Captured:** {}\n", report.total_isrs_captured));
    md.push_str(&format!(
        "- **Highest DPC Execution:** {}µs ({})\n",
        report.highest_dpc_us,
        report.highest_dpc_driver.as_deref().unwrap_or("None")
    ));
    md.push_str(&format!(
        "- **Highest ISR Execution:** {}µs ({})\n",
        report.highest_isr_us,
        report.highest_isr_driver.as_deref().unwrap_or("None")
    ));
    md.push_str(&format!(
        "- **Competitive Suitability:** {}\n\n",
        if report.system_suitable_for_competitive {
            "PASS (Suitable for Low-Latency Gaming)"
        } else {
            "FAIL (Hazardous DPC Spikes Detected)"
        }
    ));

    md.push_str("## Top Kernel Drivers by Highest DPC/ISR Execution Time\n\n");
    md.push_str("| Driver Name | Max Execution (µs) | Total Time (µs) | Execution Count | Avg (µs) | Status |\n");
    md.push_str("| :--- | :--- | :--- | :--- | :--- | :--- |\n");

    for drv in &report.top_drivers {
        let status = if drv.exceeds_500us_threshold {
            "**OFFENDING (>500µs)**"
        } else {
            "Optimal"
        };
        md.push_str(&format!(
            "| `{}` | {} | {} | {} | {:.1} | {} |\n",
            drv.driver_name,
            drv.max_execution_us,
            drv.total_execution_us,
            drv.execution_count,
            drv.avg_execution_us,
            status
        ));
    }

    md.push_str("\n## Diagnostic Recommendations\n\n");
    for rec in &report.recommendations {
        md.push_str(&format!("- {}\n", rec));
    }

    md
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_report_generation_and_markdown_formatting() {
        let mut session = KernelLatencySessionManager::new();
        session.run_synthetic_session(Duration::from_millis(150), true);

        let report = generate_report(&session, 0.15);
        assert!(report.total_dpcs_captured > 0);
        assert!(report.highest_dpc_us >= 1000);
        assert!(!report.offending_drivers.is_empty());

        let markdown = format_markdown_report(&report);
        println!("{}", markdown);
        assert!(markdown.contains("Hardware & Driver Latency Health Diagnostic Report"));
        assert!(markdown.contains("Top Kernel Drivers by Highest DPC/ISR Execution Time"));
    }
}
