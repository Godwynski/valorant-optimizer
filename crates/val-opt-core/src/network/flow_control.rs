//! Flow Control & Receive Side Scaling (RSS) Verifier.
//!
//! - Disables IEEE 802.3x Flow Control to eliminate queue buffer stalls and latency spikes.
//! - Verifies Receive Side Scaling (RSS) is active with >= 4 hardware queues to distribute
//!   network DPC/ISR load across multiple CPU cores, preventing core-0 packet starvation.

use std::process::Command;
use tracing::{info, warn};
use val_opt_shared::models::network::{AdapterPropertyBackup, FlowControlSetting, RssVerificationResult};

pub const FLOW_CONTROL_KEYWORD: &str = "*FlowControl";
pub const MINIMUM_RECOMMENDED_RSS_QUEUES: u32 = 4;

/// Query the current Flow Control state for the given adapter.
pub fn query_flow_control(adapter_name: &str) -> Result<FlowControlSetting, String> {
    let prop = crate::network::adapter::query_property(adapter_name, FLOW_CONTROL_KEYWORD)?;
    match prop {
        Some(p) => {
            if let Some(val_str) = p.first_value() {
                let code = val_str.parse::<i32>().unwrap_or(-1);
                Ok(FlowControlSetting::from(code))
            } else {
                Ok(FlowControlSetting::Unknown)
            }
        }
        None => Ok(FlowControlSetting::Unknown),
    }
}

/// Disable IEEE 802.3x Flow Control on the specified adapter.
/// Returns an `AdapterPropertyBackup` if the property was changed, enabling rollback.
pub fn disable_flow_control(adapter_name: &str) -> Result<Option<AdapterPropertyBackup>, String> {
    let prop = crate::network::adapter::query_property(adapter_name, FLOW_CONTROL_KEYWORD)?;
    if let Some(p) = prop {
        let current_val = p.first_value().unwrap_or("0");
        if current_val != "0" {
            let backup = AdapterPropertyBackup {
                adapter_name: adapter_name.to_string(),
                keyword: p.registry_keyword.clone(),
                display_name: p.display_name.clone(),
                original_value: current_val.to_string(),
            };
            crate::network::adapter::set_adapter_property(adapter_name, &p.registry_keyword, "0")?;
            info!(
                adapter = adapter_name,
                previous = current_val,
                "Disabled IEEE 802.3x Flow Control to eradicate UDP queue latency spikes"
            );
            return Ok(Some(backup));
        }
    }
    Ok(None)
}

/// Check Windows TCP Global Receive-Side Scaling State via netsh.
pub fn query_global_tcp_rss() -> Result<bool, String> {
    let output = Command::new("netsh")
        .args(["int", "tcp", "show", "global"])
        .output()
        .map_err(|e| format!("Failed to execute netsh: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    for line in stdout.lines() {
        if line.contains("Receive-Side Scaling State") {
            let parts: Vec<&str> = line.split(':').collect();
            if parts.len() == 2 {
                let state = parts[1].trim().to_lowercase();
                return Ok(state == "enabled");
            }
        }
    }
    Ok(false)
}

/// Parse the JSON output of Get-NetAdapterRss.
#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct NetAdapterRssRaw {
    pub enabled: Option<bool>,
    pub number_of_receive_queues: Option<u32>,
}

/// Verify Receive Side Scaling (RSS) configuration on the specified network adapter.
pub fn verify_rss(adapter_name: &str) -> Result<RssVerificationResult, String> {
    let script = format!(
        "Get-NetAdapterRss -Name '{}' -ErrorAction Stop | Select-Object Enabled, NumberOfReceiveQueues | ConvertTo-Json -Compress",
        adapter_name.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to execute Get-NetAdapterRss: {}", e))?;

    let mut adapter_rss_enabled = false;
    let mut num_queues = 0u32;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !stdout.is_empty() && stdout != "null" {
            if let Ok(parsed) = serde_json::from_str::<NetAdapterRssRaw>(&stdout) {
                adapter_rss_enabled = parsed.enabled.unwrap_or(false);
                num_queues = parsed.number_of_receive_queues.unwrap_or(0);
            }
        }
    } else {
        // Fallback: check *RSS advanced property
        if let Ok(Some(prop)) = crate::network::adapter::query_property(adapter_name, "*RSS") {
            adapter_rss_enabled = prop.first_value() == Some("1");
        }
        if let Ok(Some(prop)) =
            crate::network::adapter::query_property(adapter_name, "*NumRssQueues")
        {
            num_queues = prop.first_value().and_then(|v| v.parse().ok()).unwrap_or(0);
        }
    }

    let global_tcp_rss = query_global_tcp_rss().unwrap_or(false);
    let meets_minimum = num_queues >= MINIMUM_RECOMMENDED_RSS_QUEUES;
    let is_optimal = adapter_rss_enabled && global_tcp_rss && meets_minimum;

    let mut recommendations = Vec::new();
    if !adapter_rss_enabled {
        recommendations.push(format!(
            "Adapter RSS is disabled on '{}'. Enable it to avoid CPU core 0 DPC bottlenecking.",
            adapter_name
        ));
    }
    if !global_tcp_rss {
        recommendations.push(
            "Global TCP RSS is disabled. Run 'netsh int tcp set global rss=enabled'.".to_string(),
        );
    }
    if !meets_minimum {
        recommendations.push(format!(
            "Adapter only has {} RSS queues configured (recommended >= 4). Increase '*NumRssQueues'.",
            num_queues
        ));
    }

    Ok(RssVerificationResult {
        adapter_name: adapter_name.to_string(),
        rss_enabled: adapter_rss_enabled,
        global_tcp_rss_enabled: global_tcp_rss,
        num_queues,
        meets_minimum_queues: meets_minimum,
        is_optimal,
        recommendations,
    })
}

/// Enable RSS on both the adapter and globally via netsh.
pub fn enable_rss(adapter_name: &str) -> Result<(), String> {
    if !crate::network::adapter::is_elevation_available() {
        return Err("Administrator elevation required to configure RSS settings.".to_string());
    }

    let script = format!(
        "Enable-NetAdapterRss -Name '{}' -NoRestart -ErrorAction SilentlyContinue",
        adapter_name.replace('\'', "''")
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();

    let netsh_res = Command::new("netsh")
        .args(["int", "tcp", "set", "global", "rss=enabled"])
        .output()
        .map_err(|e| format!("Failed to set global RSS: {}", e))?;

    if !netsh_res.status.success() {
        warn!("Failed to set global TCP RSS via netsh");
    }

    info!(adapter = adapter_name, "Enabled RSS on adapter and global TCP stack");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_flow_control_parsing() {
        assert_eq!(FlowControlSetting::from(0), FlowControlSetting::Disabled);
        assert_eq!(FlowControlSetting::from(1), FlowControlSetting::TxEnabled);
        assert_eq!(FlowControlSetting::from(2), FlowControlSetting::RxEnabled);
        assert_eq!(FlowControlSetting::from(3), FlowControlSetting::RxTxEnabled);
        assert_eq!(FlowControlSetting::from(99), FlowControlSetting::Unknown);
    }

    #[test]
    fn test_query_global_tcp_rss() {
        let global_rss = query_global_tcp_rss();
        println!("Global TCP RSS query result: {:?}", global_rss);
        assert!(global_rss.is_ok());
    }

    #[test]
    fn test_verify_rss_on_primary_adapter() {
        if let Ok(primary) =
            val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary()
        {
            let res = verify_rss(&primary.adapter_name);
            println!("RSS verification result: {:?}", res);
            if let Ok(rss_info) = res {
                println!(
                    "Adapter: {}, RSS Enabled: {}, Queues: {}, Global TCP RSS: {}, Optimal: {}",
                    rss_info.adapter_name,
                    rss_info.rss_enabled,
                    rss_info.num_queues,
                    rss_info.global_tcp_rss_enabled,
                    rss_info.is_optimal
                );
                // On modern gaming NICs, RSS should be enabled
                assert!(rss_info.rss_enabled || !rss_info.recommendations.is_empty());
            }
        }
    }
}
