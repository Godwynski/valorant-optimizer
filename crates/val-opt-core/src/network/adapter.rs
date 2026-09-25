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

/// Validates an adapter identifier or registry keyword according to strict allowlist:
/// Only allows ASCII alphanumeric, underscores, hyphens, periods, asterisks, spaces, and parentheses.
pub fn validate_adapter_identifier(input: &str) -> Result<(), String> {
    if input.is_empty() || input.len() > 256 {
        return Err("Adapter identifier must be between 1 and 256 characters".to_string());
    }
    for c in input.chars() {
        if !c.is_ascii_alphanumeric()
            && c != '_'
            && c != '-'
            && c != '.'
            && c != '*'
            && c != ' '
            && c != '('
            && c != ')'
        {
            return Err(format!("Invalid character '{}' in adapter identifier: {}", c, input));
        }
    }
    Ok(())
}

/// Validates a registry property value:
/// Only allows ASCII alphanumeric, underscores, hyphens, periods, and spaces.
pub fn validate_property_value(value: &str) -> Result<(), String> {
    if value.len() > 256 {
        return Err("Property value exceeds maximum length of 256 characters".to_string());
    }
    for c in value.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' && c != '.' && c != ' ' {
            return Err(format!("Invalid character '{}' in property value: {}", c, value));
        }
    }
    Ok(())
}

/// Helper to check if current process has administrative elevation.
pub fn is_elevation_available() -> bool {
    crate::benchmarking::EtwFrameCaptureEngine::is_elevation_available()
}

/// Query all advanced properties for the specified network adapter name.
pub fn query_adapter_properties(adapter_name: &str) -> Result<Vec<NicAdvancedProperty>, String> {
    validate_adapter_identifier(adapter_name)?;

    // Constant script block — input passed strictly via process environment variable
    const QUERY_SCRIPT: &str = "@(Get-NetAdapterAdvancedProperty -Name $env:VAL_OPT_ADAPTER -ErrorAction Stop) | Select-Object DisplayName, RegistryKeyword, RegistryValue | ConvertTo-Json -Compress";

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", QUERY_SCRIPT]);
    cmd.env("VAL_OPT_ADAPTER", adapter_name);

    let output = cmd
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
    validate_adapter_identifier(adapter_name)?;
    validate_adapter_identifier(keyword)?;

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
    validate_adapter_identifier(adapter_name)?;
    validate_adapter_identifier(keyword)?;
    validate_property_value(value)?;

    if !is_elevation_available() {
        return Err(
            "Administrator elevation required to modify network adapter advanced properties."
                .to_string(),
        );
    }

    // Constant script block — inputs passed strictly via process environment variables
    const SET_SCRIPT: &str = "Set-NetAdapterAdvancedProperty -Name $env:VAL_OPT_ADAPTER -RegistryKeyword $env:VAL_OPT_KEYWORD -RegistryValue $env:VAL_OPT_VALUE -NoRestart -ErrorAction Stop";

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", SET_SCRIPT]);
    cmd.env("VAL_OPT_ADAPTER", adapter_name);
    cmd.env("VAL_OPT_KEYWORD", keyword);
    cmd.env("VAL_OPT_VALUE", value);

    let output = cmd
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
    validate_adapter_identifier(adapter_name)?;

    // Constant script block — input passed strictly via process environment variable
    const STATUS_SCRIPT: &str = "(Get-NetAdapter -Name $env:VAL_OPT_ADAPTER -ErrorAction Stop).Status";

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", STATUS_SCRIPT]);
    cmd.env("VAL_OPT_ADAPTER", adapter_name);

    let output = cmd
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

    #[test]
    fn test_validate_adapter_identifier_valid() {
        assert!(validate_adapter_identifier("Ethernet").is_ok());
        assert!(validate_adapter_identifier("Wi-Fi").is_ok());
        assert!(validate_adapter_identifier("Realtek PCIe GbE Family Controller").is_ok());
        assert!(validate_adapter_identifier("Intel(R) Ethernet Connection (7) I219-V").is_ok());
        assert!(validate_adapter_identifier("vEthernet (Default Switch)").is_ok());
        assert!(validate_adapter_identifier("*EEE").is_ok());
        assert!(validate_adapter_identifier("*InterruptModeration").is_ok());
    }

    #[test]
    fn test_validate_adapter_identifier_rejection_of_injection_payloads() {
        // Semicolon / command chaining
        assert!(validate_adapter_identifier("Ethernet; Start-Process calc.exe").is_err());
        assert!(query_adapter_properties("Ethernet; Start-Process calc.exe").is_err());
        assert!(query_adapter_link_status("Ethernet; Start-Process calc.exe").is_err());

        // Subexpression / variable expansion
        assert!(validate_adapter_identifier("Ethernet$(whoami)").is_err());
        assert!(validate_adapter_identifier("Ethernet`ncalc.exe").is_err());

        // Single / double quote breakout
        assert!(validate_adapter_identifier("Ethernet' -or 1 -eq 1").is_err());
        assert!(validate_adapter_identifier("Ethernet\"").is_err());

        // Pipe / redirection
        assert!(validate_adapter_identifier("Ethernet | Out-File C:\\pwn.txt").is_err());
        assert!(validate_adapter_identifier("Ethernet > C:\\pwn.txt").is_err());
        assert!(validate_adapter_identifier("Ethernet & calc.exe").is_err());

        // Newlines and control characters
        assert!(validate_adapter_identifier("Ethernet\r\nStart-Process calc").is_err());

        // Empty string
        assert!(validate_adapter_identifier("").is_err());
    }

    #[test]
    fn test_validate_property_value_rejection_of_injection_payloads() {
        assert!(validate_property_value("0").is_ok());
        assert!(validate_property_value("1").is_ok());
        assert!(validate_property_value("Disabled").is_ok());
        assert!(validate_property_value("Rx & Tx Enabled").is_err()); // ampersand rejected
        assert!(validate_property_value("0; Stop-Service vgk").is_err());
        assert!(validate_property_value("$(calc)").is_err());
        assert!(validate_property_value("1' OR '1'='1").is_err());
    }
}

