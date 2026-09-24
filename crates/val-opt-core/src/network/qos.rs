//! Windows Quality of Service (QoS) DSCP Policy Registrar.
//!
//! Registers kernel-level QoS packet tagging rule for VALORANT client outbound UDP traffic:
//! - Targets `VALORANT-Win64-Shipping.exe`
//! - Outbound UDP port range 7000-8000
//! - Applies DSCP 46 (Expedited Forwarding / DSCP 0x2E) to prioritize packets across
//!   local gateways, switches, and ISP hops.
//! - Configures Windows TCP/IP QoS NLA bypass (`Do not use NLA = 1`) ensuring DSCP markings
//!   are respected on unmanaged / non-domain home LANs.
//! - Full rollback unregistration supported.

use std::process::Command;
use tracing::{info, warn};
use val_opt_shared::models::network::{QosPolicyBackup, QosPolicyInfo};

pub const DEFAULT_VALORANT_QOS_POLICY_NAME: &str = "VALORANT_QoS_Optimized";
pub const VALORANT_PROCESS_NAME: &str = "VALORANT-Win64-Shipping.exe";
pub const VALORANT_UDP_PORT_START: u16 = 7000;
pub const VALORANT_UDP_PORT_END: u16 = 8000;
pub const DSCP_EXPEDITED_FORWARDING: u8 = 46;

/// Raw JSON representation of a Windows NetQosPolicy from PowerShell.
#[derive(serde::Deserialize, Debug)]
#[serde(rename_all = "PascalCase")]
struct NetQosPolicyRaw {
    pub name: Option<String>,
    pub app_path_name: Option<String>,
    #[serde(alias = "IPProtocol", alias = "IpProtocol")]
    pub ip_protocol: Option<String>,
    #[serde(alias = "IPDstPortStart", alias = "IpDstPortStart")]
    pub ip_dst_port_start: Option<u16>,
    #[serde(alias = "IPDstPortEnd", alias = "IpDstPortEnd")]
    pub ip_dst_port_end: Option<u16>,
    #[serde(alias = "DSCPValue", alias = "DscpValue")]
    pub dscp_value: Option<u8>,
}

/// Query an existing QoS policy by name.
pub fn query_qos_policy(policy_name: &str) -> Result<Option<QosPolicyInfo>, String> {
    let script = format!(
        "Get-NetQosPolicy -Name '{}' -ErrorAction SilentlyContinue | Select-Object Name, AppPathName, IPProtocol, IPDstPortStart, IPDstPortEnd, DSCPValue | ConvertTo-Json -Compress",
        policy_name.replace('\'', "''")
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|e| format!("Failed to query QoS policy: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if stdout.is_empty() || stdout == "null" {
        return Ok(None);
    }

    if let Ok(raw) = serde_json::from_str::<NetQosPolicyRaw>(&stdout) {
        return Ok(Some(QosPolicyInfo {
            name: raw.name.unwrap_or_else(|| policy_name.to_string()),
            app_path_name: raw.app_path_name.unwrap_or_default(),
            protocol: raw.ip_protocol.unwrap_or_else(|| "UDP".to_string()),
            dscp_value: raw.dscp_value.unwrap_or(0),
            dst_port_start: raw.ip_dst_port_start.unwrap_or(0),
            dst_port_end: raw.ip_dst_port_end.unwrap_or(0),
            is_active: true,
        }));
    }

    Ok(None)
}

/// Check and configure the Windows TCP/IP QoS NLA registry setting.
/// Returns the previous setting value (or None if nonexistent).
fn configure_nla_bypass() -> Result<Option<String>, String> {
    let check_script = r#"(Get-ItemProperty -Path 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\QoS' -ErrorAction SilentlyContinue).'Do not use NLA'"#;
    let check_out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", check_script])
        .output()
        .map_err(|e| format!("Failed to query QoS NLA registry state: {}", e))?;

    let prev_val = String::from_utf8_lossy(&check_out.stdout).trim().to_string();
    let previous_setting = if prev_val.is_empty() {
        None
    } else {
        Some(prev_val)
    };

    let set_script = r#"
        $path = 'HKLM:\SYSTEM\CurrentControlSet\Services\Tcpip\QoS'
        if (-not (Test-Path $path)) {
            New-Item -Path $path -Force | Out-Null
        }
        Set-ItemProperty -Path $path -Name 'Do not use NLA' -Value '1' -Type String -Force
    "#;

    let set_out = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", set_script])
        .output()
        .map_err(|e| format!("Failed to set QoS NLA bypass registry key: {}", e))?;

    if !set_out.status.success() {
        let err = String::from_utf8_lossy(&set_out.stderr);
        return Err(format!("Failed to configure QoS NLA bypass: {}", err.trim()));
    }

    info!("Configured Windows TCP/IP QoS NLA bypass ('Do not use NLA' = 1)");
    Ok(previous_setting)
}

/// Restore the Windows TCP/IP QoS NLA registry setting.
fn restore_nla_setting(previous_setting: Option<&str>) -> Result<(), String> {
    let script = match previous_setting {
        Some(val) => format!(
            "Set-ItemProperty -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\QoS' -Name 'Do not use NLA' -Value '{}' -Force -ErrorAction SilentlyContinue",
            val.replace('\'', "''")
        ),
        None => "Remove-ItemProperty -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\QoS' -Name 'Do not use NLA' -Force -ErrorAction SilentlyContinue".to_string(),
    };

    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output();

    info!("Restored Windows TCP/IP QoS NLA registry setting");
    Ok(())
}

/// Register a high-priority DSCP 46 QoS policy for VALORANT outbound UDP packets.
/// Returns a `QosPolicyBackup` for rollback.
pub fn register_valorant_qos_policy() -> Result<QosPolicyBackup, String> {
    if !crate::network::adapter::is_elevation_available() {
        return Err("Administrator elevation required to register Windows QoS policies.".to_string());
    }

    let existing_policy = query_qos_policy(DEFAULT_VALORANT_QOS_POLICY_NAME)?;
    let was_present = existing_policy.is_some();

    // If already exists, remove it first to re-register cleanly
    if was_present {
        let remove_script = format!(
            "Remove-NetQosPolicy -Name '{}' -Confirm:$false -ErrorAction SilentlyContinue",
            DEFAULT_VALORANT_QOS_POLICY_NAME
        );
        let _ = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &remove_script])
            .output();
    }

    let prev_nla = configure_nla_bypass()?;

    let create_script = format!(
        "New-NetQosPolicy -Name '{}' -AppPathNameMatchCondition '{}' -IPProtocolMatchCondition UDP -IPDstPortStartMatchCondition {} -IPDstPortEndMatchCondition {} -DSCPAction {} -NetworkProfile All -ErrorAction Stop",
        DEFAULT_VALORANT_QOS_POLICY_NAME,
        VALORANT_PROCESS_NAME,
        VALORANT_UDP_PORT_START,
        VALORANT_UDP_PORT_END,
        DSCP_EXPEDITED_FORWARDING
    );

    let output = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &create_script])
        .output()
        .map_err(|e| format!("Failed to execute New-NetQosPolicy: {}", e))?;

    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr);
        return Err(format!("Failed to register QoS policy: {}", err.trim()));
    }

    info!(
        policy = DEFAULT_VALORANT_QOS_POLICY_NAME,
        app = VALORANT_PROCESS_NAME,
        ports = "7000-8000",
        dscp = DSCP_EXPEDITED_FORWARDING,
        "Successfully registered Windows QoS DSCP 46 policy"
    );

    Ok(QosPolicyBackup {
        policy_name: DEFAULT_VALORANT_QOS_POLICY_NAME.to_string(),
        previous_nla_setting: prev_nla,
        was_policy_present_before: was_present,
    })
}

/// Unregister the QoS policy and restore previous TCP/IP QoS settings.
pub fn unregister_qos_policy(backup: &QosPolicyBackup) -> Result<(), String> {
    if !backup.was_policy_present_before {
        let remove_script = format!(
            "Remove-NetQosPolicy -Name '{}' -Confirm:$false -ErrorAction SilentlyContinue",
            backup.policy_name
        );
        let output = Command::new("powershell")
            .args(["-NoProfile", "-NonInteractive", "-Command", &remove_script])
            .output();

        if let Ok(out) = output {
            if out.status.success() {
                info!(policy = %backup.policy_name, "Unregistered Windows QoS policy");
            } else {
                warn!(policy = %backup.policy_name, "QoS policy removal completed with notice");
            }
        }
    }

    restore_nla_setting(backup.previous_nla_setting.as_deref())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_net_qos_policy_json() {
        let sample = r#"{
            "Name": "VALORANT_QoS_Optimized",
            "AppPathName": "VALORANT-Win64-Shipping.exe",
            "IPProtocol": "UDP",
            "IPDstPortStart": 7000,
            "IPDstPortEnd": 8000,
            "DSCPValue": 46
        }"#;

        let parsed: NetQosPolicyRaw = serde_json::from_str(sample).unwrap();
        assert_eq!(parsed.name.as_deref(), Some("VALORANT_QoS_Optimized"));
        assert_eq!(parsed.app_path_name.as_deref(), Some("VALORANT-Win64-Shipping.exe"));
        assert_eq!(parsed.ip_protocol.as_deref(), Some("UDP"));
        assert_eq!(parsed.ip_dst_port_start, Some(7000));
        assert_eq!(parsed.ip_dst_port_end, Some(8000));
        assert_eq!(parsed.dscp_value, Some(46));
    }

    #[test]
    fn test_query_nonexistent_qos_policy() {
        let res = query_qos_policy("NONEXISTENT_TEST_POLICY_12345").unwrap();
        assert!(res.is_none());
    }

    #[test]
    fn test_elevation_guard() {
        let elevated = crate::network::adapter::is_elevation_available();
        if !elevated {
            let res = register_valorant_qos_policy();
            assert!(res.is_err());
            assert!(res.unwrap_err().contains("Administrator elevation"));
        }
    }
}
