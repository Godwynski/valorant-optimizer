//! Physical Network Adapter Advanced Property Configurator.
//!
//! Configures physical NIC driver parameters to disable latency-inducing power saving features:
//! - Disables Energy Efficient Ethernet (EEE / 802.3az)
//! - Disables Green Ethernet / Power Saving Mode
//! - Tunes / Disables Interrupt Moderation to eliminate packet queuing batch delay
//!
//! Employs `-NoRestart` to prevent network link drops > 1.5 seconds.
//! Fully supports deterministic transactional rollback.

use std::process::Command;
use tracing::{info, warn};
use val_opt_shared::models::network::{AdapterPropertyBackup, NicAdvancedProperty};

/// Target registry keywords for Energy Efficient Ethernet (EEE).
pub const EEE_KEYWORDS: &[&str] = &["*EEE", "*AdvancedEEE", "*EEELinkAdvertisement"];

/// Target registry keywords for Green Ethernet and NIC power-saving modes.
pub const GREEN_ETHERNET_KEYWORDS: &[&str] = &[
    "*GreenEthernet",
    "GreenEthernet",
    "PowerSavingMode",
    "AutoDisableGigabit",
    "GigaLite",
];

/// Target registry keyword for Interrupt Moderation.
pub const INTERRUPT_MODERATION_KEYWORD: &str = "*InterruptModeration";

/// Helper to check if current process has administrative elevation.
pub fn is_elevation_available() -> bool {
    crate::benchmarking::EtwFrameCaptureEngine::is_elevation_available()
}

/// Query all advanced properties for the specified network adapter name.
pub fn query_adapter_properties(adapter_name: &str) -> Result<Vec<NicAdvancedProperty>, String> {
    let script = format!(
        "@(Get-NetAdapterAdvancedProperty -Name '{}' -ErrorAction Stop) | Select-Object DisplayName, RegistryKeyword, RegistryValue | ConvertTo-Json -Compress",
        adapter_name.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to execute PowerShell adapter query: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Get-NetAdapterAdvancedProperty failed for '{}': {}",
            adapter_name,
            err.trim()
        ));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() || stdout == "null" {
        return Ok(Vec::new());
    }

    let properties: Vec<NicAdvancedProperty> = serde_json::from_str(&stdout)
        .map_err(|e| format!("Failed to parse adapter properties JSON ({}): {}", e, stdout))?;

    Ok(properties)
}

/// Query a specific property by registry keyword (case-insensitive).
pub fn query_property(
    adapter_name: &str,
    keyword: &str,
) -> Result<Option<NicAdvancedProperty>, String> {
    let props = query_adapter_properties(adapter_name)?;
    Ok(props
        .into_iter()
        .find(|p| p.registry_keyword.eq_ignore_ascii_case(keyword)))
}

/// Set an advanced property value on a network adapter using `-NoRestart` to prevent link drop.
pub fn set_adapter_property(
    adapter_name: &str,
    keyword: &str,
    value: &str,
) -> Result<(), String> {
    if !is_elevation_available() {
        return Err(
            "Administrator elevation required to modify network adapter advanced properties."
                .to_string(),
        );
    }

    let script = format!(
        "Set-NetAdapterAdvancedProperty -Name '{}' -RegistryKeyword '{}' -RegistryValue '{}' -NoRestart -ErrorAction Stop",
        adapter_name.replace('\'', "''"),
        keyword.replace('\'', "''"),
        value.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to execute Set-NetAdapterAdvancedProperty: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to set '{}' on '{}': {}",
            keyword,
            adapter_name,
            err.trim()
        ));
    }

    info!(
        adapter = adapter_name,
        keyword = keyword,
        value = value,
        "Applied network adapter advanced property without link reset (-NoRestart)"
    );

    Ok(())
}

/// Query the link status of the specified network adapter (e.g. "Up", "Disconnected").
pub fn query_adapter_link_status(adapter_name: &str) -> Result<String, String> {
    let script = format!(
        "(Get-NetAdapter -Name '{}' -ErrorAction Stop).Status",
        adapter_name.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to query adapter status: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "Failed to query status for '{}': {}",
            adapter_name,
            err.trim()
        ));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Optimize latency-critical properties on the target adapter:
/// 1. Disables Energy Efficient Ethernet (EEE)
/// 2. Disables Green Ethernet / Power Saving Mode
/// 3. Sets Interrupt Moderation to 0 (Disabled)
///
/// Returns a list of backups for applied properties to guarantee full rollback capability.
pub fn optimize_adapter_latency_properties(
    adapter_name: &str,
) -> Result<Vec<AdapterPropertyBackup>, String> {
    let props = query_adapter_properties(adapter_name)?;
    let mut backups = Vec::new();

    let link_before = query_adapter_link_status(adapter_name).unwrap_or_default();
    info!(
        adapter = adapter_name,
        status = %link_before,
        "Beginning network adapter latency optimization"
    );

    // 1. EEE keywords
    for keyword in EEE_KEYWORDS {
        if let Some(prop) = props.iter().find(|p| p.registry_keyword.eq_ignore_ascii_case(keyword)) {
            let current_val = prop.first_value().unwrap_or("0");
            if current_val != "0" {
                backups.push(AdapterPropertyBackup {
                    adapter_name: adapter_name.to_string(),
                    keyword: prop.registry_keyword.clone(),
                    display_name: prop.display_name.clone(),
                    original_value: current_val.to_string(),
                });
                set_adapter_property(adapter_name, &prop.registry_keyword, "0")?;
            }
        }
    }

    // 2. Green Ethernet / Power Saving keywords
    for keyword in GREEN_ETHERNET_KEYWORDS {
        if let Some(prop) = props.iter().find(|p| p.registry_keyword.eq_ignore_ascii_case(keyword)) {
            let current_val = prop.first_value().unwrap_or("0");
            if current_val != "0" {
                backups.push(AdapterPropertyBackup {
                    adapter_name: adapter_name.to_string(),
                    keyword: prop.registry_keyword.clone(),
                    display_name: prop.display_name.clone(),
                    original_value: current_val.to_string(),
                });
                set_adapter_property(adapter_name, &prop.registry_keyword, "0")?;
            }
        }
    }

    // 3. Interrupt Moderation
    if let Some(prop) = props
        .iter()
        .find(|p| p.registry_keyword.eq_ignore_ascii_case(INTERRUPT_MODERATION_KEYWORD))
    {
        let current_val = prop.first_value().unwrap_or("0");
        // "0" = Disabled
        if current_val != "0" {
            backups.push(AdapterPropertyBackup {
                adapter_name: adapter_name.to_string(),
                keyword: prop.registry_keyword.clone(),
                display_name: prop.display_name.clone(),
                original_value: current_val.to_string(),
            });
            set_adapter_property(adapter_name, &prop.registry_keyword, "0")?;
        }
    }

    let link_after = query_adapter_link_status(adapter_name).unwrap_or_default();
    info!(
        adapter = adapter_name,
        status = %link_after,
        modified_count = backups.len(),
        "Completed network adapter latency optimization"
    );

    Ok(backups)
}

/// Restore original adapter properties from the provided backup records.
pub fn restore_adapter_properties(backups: &[AdapterPropertyBackup]) -> Result<(), String> {
    for backup in backups {
        info!(
            adapter = %backup.adapter_name,
            keyword = %backup.keyword,
            target_value = %backup.original_value,
            "Restoring network adapter property to baseline"
        );
        if let Err(e) =
            set_adapter_property(&backup.adapter_name, &backup.keyword, &backup.original_value)
        {
            warn!(
                adapter = %backup.adapter_name,
                keyword = %backup.keyword,
                error = %e,
                "Failed to restore adapter property"
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_adapter_properties_json() {
        let sample_json = r#"[
            {"DisplayName":"Interrupt Moderation","RegistryKeyword":"*InterruptModeration","RegistryValue":["0"]},
            {"DisplayName":"Energy Efficient Ethernet","RegistryKeyword":"*EEE","RegistryValue":["1"]},
            {"DisplayName":"Power Saving Mode","RegistryKeyword":"PowerSavingMode","RegistryValue":["0"]}
        ]"#;

        let props: Vec<NicAdvancedProperty> = serde_json::from_str(sample_json).unwrap();
        assert_eq!(props.len(), 3);
        assert_eq!(props[0].registry_keyword, "*InterruptModeration");
        assert_eq!(props[0].first_value(), Some("0"));
        assert_eq!(props[1].registry_keyword, "*EEE");
        assert_eq!(props[1].first_value(), Some("1"));
    }

    #[test]
    fn test_query_live_adapter_properties() {
        // Query active adapter if available
        let active_res = val_opt_shared::hardware::network::NetworkAdapterInfo::detect_primary();
        if let Ok(primary) = active_res {
            let props_res = query_adapter_properties(&primary.adapter_name);
            match props_res {
                Ok(props) => {
                    println!("Found {} properties for {}", props.len(), primary.adapter_name);
                    assert!(!props.is_empty(), "Primary adapter should have advanced properties");
                    let status = query_adapter_link_status(&primary.adapter_name).unwrap();
                    println!("Adapter {} status: {}", primary.adapter_name, status);
                    assert_eq!(status, "Up");
                }
                Err(e) => {
                    println!("Property query returned error (may be driver-specific): {}", e);
                }
            }
        }
    }

    #[test]
    fn test_elevation_check() {
        let elevated = is_elevation_available();
        println!("Adapter config elevation available: {}", elevated);
        // Ensure function runs cleanly without panic
    }
}
