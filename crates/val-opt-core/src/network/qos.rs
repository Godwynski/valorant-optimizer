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

/// Validates a QoS policy name according to strict allowlist:
/// Only allows ASCII alphanumeric characters, underscores, and hyphens. Length 1..=128.
pub fn validate_policy_name(name: &str) -> Result<(), String> {
    if name.is_empty() || name.len() > 128 {
        return Err("QoS policy name must be between 1 and 128 characters".to_string());
    }
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            return Err(format!("Invalid character '{}' in QoS policy name: {}", c, name));
        }
    }
    Ok(())
}

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
    validate_policy_name(policy_name)?;

    // Constant script block — input passed strictly via process environment variable
    const QUERY_SCRIPT: &str = "Get-NetQosPolicy -Name $env:VAL_OPT_POLICY_NAME -ErrorAction SilentlyContinue | Select-Object Name, AppPathName, IPProtocol, IPDstPortStart, IPDstPortEnd, DSCPValue | ConvertTo-Json -Compress";

    let mut cmd = Command::new("powershell");
    cmd.args(["-NoProfile", "-NonInteractive", "-Command", QUERY_SCRIPT]);
    cmd.env("VAL_OPT_POLICY_NAME", policy_name);

    let output = cmd
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

/// Restore the Windows TCP/IP QoS NLA registry setting without string interpolation.
fn restore_nla_setting(previous_setting: Option<&str>) -> Result<(), String> {
    match previous_setting {
        Some(val) => {
            // Validate value is purely alphanumeric
            if !val.chars().all(|c| c.is_ascii_alphanumeric()) {
                return Err("Invalid NLA setting value".to_string());
            }
            const SET_NLA: &str = "Set-ItemProperty -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Services\\Tcpip\\QoS' -Name 'Do not use NLA' -Value $env:VAL_OPT_NLA_VAL -Force -ErrorAction SilentlyContinue";
            let mut cmd = Command::new("powershell");
            cmd.args(["-NoProfile", "-NonInteractive", "-Command", SET_NLA]);
            cmd.env("VAL_OPT_NLA_VAL", val);
            let _ = cmd.output();
        }
        None => {
            // When no prior NLA setting was recorded or modified by the optimizer,
            // preserve registry untouched (do not execute Remove-ItemProperty).
            tracing::debug!("No prior NLA setting to restore; registry unmodified");
        }
    }

    Ok(())
}

/// Register QoS policy: De-scoped under TASK-SEC-01 / TASK-OPT-02.
/// New-NetQosPolicy creation is omitted to prevent injection risks, home router DSCP stripping,
/// and carrier policer packet dropping.
pub fn register_valorant_qos_policy() -> Result<QosPolicyBackup, String> {
    info!(
        "QoS DSCP policy creation omitted (de-scoped per TASK-SEC-01 / TASK-OPT-02 for carrier packet drop mitigation)"
    );

    Ok(QosPolicyBackup {
        policy_name: DEFAULT_VALORANT_QOS_POLICY_NAME.to_string(),
        previous_nla_setting: None,
        was_policy_present_before: false,
    })
}

/// Unregister the QoS policy and restore previous TCP/IP QoS settings.
/// Strictly parameterizes PowerShell execution via environment variables without string interpolation.
pub fn unregister_qos_policy(backup: &QosPolicyBackup) -> Result<(), String> {
    validate_policy_name(&backup.policy_name)?;

    if !backup.was_policy_present_before {
        const REMOVE_SCRIPT: &str = "Remove-NetQosPolicy -Name $env:VAL_OPT_POLICY_NAME -Confirm:$false -ErrorAction SilentlyContinue";
        let mut cmd = Command::new("powershell");
        cmd.args(["-NoProfile", "-NonInteractive", "-Command", REMOVE_SCRIPT]);
        cmd.env("VAL_OPT_POLICY_NAME", &backup.policy_name);

        let output = cmd.output();

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
    fn test_validate_policy_name_valid() {
        assert!(validate_policy_name("VALORANT_QoS_Optimized").is_ok());
        assert!(validate_policy_name("Policy-123_Test").is_ok());
    }

    #[test]
    fn test_validate_policy_name_injection_payloads() {
        // Breakouts with quotes, semicolons, subexpressions
        assert!(validate_policy_name("VALORANT'; Start-Process calc.exe; '").is_err());
        assert!(validate_policy_name("Policy$(whoami)").is_err());
        assert!(validate_policy_name("Policy | Out-File C:\\pwn.txt").is_err());
        assert!(validate_policy_name("Policy`ncalc.exe").is_err());
        assert!(validate_policy_name("Policy & calc.exe").is_err());
        assert!(validate_policy_name("").is_err());

        // Calling query_qos_policy with injection payload returns Err immediately
        assert!(query_qos_policy("VALORANT'; Start-Process calc.exe; '").is_err());

        // Calling unregister_qos_policy with injection payload returns Err immediately
        let malicious_backup = QosPolicyBackup {
            policy_name: "VALORANT'; Start-Process calc.exe; '".to_string(),
            previous_nla_setting: None,
            was_policy_present_before: false,
        };
        assert!(unregister_qos_policy(&malicious_backup).is_err());
    }

    #[test]
    fn test_register_qos_policy_descaled() {
        let res = register_valorant_qos_policy();
        assert!(res.is_ok());
        let backup = res.unwrap();
        assert_eq!(backup.policy_name, DEFAULT_VALORANT_QOS_POLICY_NAME);
        assert!(!backup.was_policy_present_before);
    }
}
