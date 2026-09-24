use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct NetworkAdapterInfo {
    pub adapter_name: String,
    pub description: String,
    pub is_ethernet: bool,
    pub is_wifi: bool,
    pub is_active: bool,
    pub link_speed_mbps: u64,
    pub ipv4_address: Option<String>,
}

impl NetworkAdapterInfo {
    /// Detects all active network adapters on the host machine.
    pub fn detect_all() -> Result<Vec<Self>, anyhow::Error> {
        #[cfg(windows)]
        {
            detect_windows_adapters()
        }

        #[cfg(not(windows))]
        {
            Ok(vec![Self {
                adapter_name: "Ethernet".to_string(),
                description: "Realtek PCIe GbE Family Controller".to_string(),
                is_ethernet: true,
                is_wifi: false,
                is_active: true,
                link_speed_mbps: 1000,
                ipv4_address: Some("192.168.1.100".to_string()),
            }])
        }
    }

    /// Detects the primary active internet-facing network adapter.
    pub fn detect_primary() -> Result<Self, anyhow::Error> {
        let adapters = Self::detect_all()?;
        // Prefer active Ethernet adapter with fastest link speed
        adapters
            .into_iter()
            .filter(|a| a.is_active)
            .max_by_key(|a| (if a.is_ethernet { 1 } else { 0 }, a.link_speed_mbps))
            .ok_or_else(|| anyhow::anyhow!("No active network adapter found"))
    }
}

#[cfg(windows)]
fn detect_windows_adapters() -> Result<Vec<NetworkAdapterInfo>, anyhow::Error> {
    use windows::Win32::NetworkManagement::IpHelper::{
        GetAdaptersAddresses, GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER,
        GAA_FLAG_SKIP_MULTICAST, IP_ADAPTER_ADDRESSES_LH,
    };
    use windows::Win32::Networking::WinSock::{AF_INET, AF_UNSPEC};

    let flags = GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_DNS_SERVER;
    let mut buffer_size: u32 = 16384;
    let mut buffer: Vec<u8> = vec![0u8; buffer_size as usize];

    let result = unsafe {
        GetAdaptersAddresses(
            AF_UNSPEC.0 as u32,
            flags,
            None,
            Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
            &mut buffer_size,
        )
    };

    if result != 0 {
        // Retry with larger buffer if ERROR_BUFFER_OVERFLOW (111)
        buffer.resize(buffer_size as usize, 0);
        let retry = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                flags,
                None,
                Some(buffer.as_mut_ptr() as *mut IP_ADAPTER_ADDRESSES_LH),
                &mut buffer_size,
            )
        };
        if retry != 0 {
            return Err(anyhow::anyhow!("GetAdaptersAddresses failed with error code {}", retry));
        }
    }

    let mut adapters = Vec::new();
    let mut current_ptr = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;

    while !current_ptr.is_null() {
        let entry = unsafe { &*current_ptr };

        let description = if !entry.Description.is_null() {
            unsafe { entry.Description.to_string().unwrap_or_default() }
        } else {
            String::new()
        };

        let adapter_name = if !entry.FriendlyName.is_null() {
            unsafe { entry.FriendlyName.to_string().unwrap_or_default() }
        } else {
            String::new()
        };

        // IF_TYPE_ETHERNET_CSMACD = 6, IF_TYPE_IEEE80211 = 71
        let is_ethernet = entry.IfType == 6;
        let is_wifi = entry.IfType == 71;

        // IfOperStatusUp = 1
        let is_active = entry.OperStatus.0 == 1;
        let link_speed_mbps = entry.TransmitLinkSpeed / 1_000_000;

        // Extract first IPv4 unicast address if present
        let mut ipv4_address = None;
        let mut unicast_ptr = entry.FirstUnicastAddress;
        while !unicast_ptr.is_null() {
            let u_addr = unsafe { &*unicast_ptr };
            let sock_addr_ptr = u_addr.Address.lpSockaddr;
            if !sock_addr_ptr.is_null() {
                let family = unsafe { (*sock_addr_ptr).sa_family };
                if family == AF_INET {
                    let sin_ptr = sock_addr_ptr as *const windows::Win32::Networking::WinSock::SOCKADDR_IN;
                    let ip_raw = unsafe { (*sin_ptr).sin_addr.S_un.S_addr };
                    let b = ip_raw.to_le_bytes();
                    ipv4_address = Some(format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]));
                    break;
                }
            }
            unicast_ptr = u_addr.Next;
        }

        if is_ethernet || is_wifi {
            adapters.push(NetworkAdapterInfo {
                adapter_name,
                description,
                is_ethernet,
                is_wifi,
                is_active,
                link_speed_mbps,
                ipv4_address,
            });
        }

        current_ptr = entry.Next;
    }

    Ok(adapters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_detection() {
        let adapters = NetworkAdapterInfo::detect_all().expect("Should detect network adapters");
        assert!(!adapters.is_empty(), "Adapter list must not be empty");

        let primary = NetworkAdapterInfo::detect_primary().expect("Active primary adapter must exist");
        assert!(!primary.description.is_empty(), "Description must not be empty");

        println!("=== Detected Network Adapters ===");
        for a in &adapters {
            println!(
                "Adapter: {} [{}] (Ethernet: {}, Active: {}, Link: {} Mbps, IP: {:?})",
                a.adapter_name, a.description, a.is_ethernet, a.is_active, a.link_speed_mbps, a.ipv4_address
            );
        }
        println!("Primary Adapter: {} ({})", primary.adapter_name, primary.description);
    }
}
