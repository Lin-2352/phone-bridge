//! How this PC reaches the phone: the network adapter that routes to the
//! phone's address. A USB-tethering adapter (RNDIS or NCM) means USB, a
//! wireless one Wi-Fi, other Ethernet "Network". The phone's own subnet
//! says nothing reliable (a USB tether can hand out 10.x addresses), so the
//! PC's route decides. `classify` is pure; the lookup is a thin Win32 call.
#![allow(unsafe_code)]

/// The kind of link the phone was reached over.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Path {
    Wifi,
    Usb,
    /// Wired Ethernet (or another adapter that is neither).
    Network,
    #[default]
    Unknown,
}

/// `IF_TYPE_ETHERNET_CSMACD` and `IF_TYPE_IEEE80211` (ipifcons.h).
const IF_TYPE_ETHERNET: u32 = 6;
const IF_TYPE_WIFI: u32 = 71;

/// The path for an adapter of interface type `if_type` whose description
/// and alias are `names`. RNDIS and NCM tether adapters report Ethernet, so
/// their names are checked first.
pub fn classify(if_type: u32, names: &str) -> Path {
    let n = names.to_ascii_lowercase();
    if n.contains("remote ndis") || n.contains("ncm") || n.contains("usb") {
        Path::Usb
    } else if if_type == IF_TYPE_WIFI {
        Path::Wifi
    } else if if_type == IF_TYPE_ETHERNET {
        Path::Network
    } else {
        Path::Unknown
    }
}

/// The path to `addr` (`ip:port`, the address a session used), from the
/// adapter Windows routes it through; `Unknown` if that cannot be told.
#[cfg(windows)]
pub fn path_to(addr: std::net::SocketAddr) -> Path {
    use std::net::SocketAddr;
    use windows::Win32::{
        NetworkManagement::IpHelper::{GetBestInterfaceEx, GetIfEntry2, MIB_IF_ROW2},
        Networking::WinSock::{
            AF_INET, AF_INET6, IN6_ADDR, IN6_ADDR_0, IN_ADDR, IN_ADDR_0, SOCKADDR, SOCKADDR_IN,
            SOCKADDR_IN6, SOCKADDR_IN6_0,
        },
    };
    let mut index = 0u32;
    let r = match addr {
        SocketAddr::V4(a) => {
            let sa = SOCKADDR_IN {
                sin_family: AF_INET,
                sin_port: a.port().to_be(),
                sin_addr: IN_ADDR {
                    S_un: IN_ADDR_0 {
                        S_addr: u32::from_ne_bytes(a.ip().octets()),
                    },
                },
                sin_zero: [0; 8],
            };
            // SAFETY: a valid SOCKADDR_IN that outlives the call.
            unsafe { GetBestInterfaceEx(&sa as *const SOCKADDR_IN as *const SOCKADDR, &mut index) }
        }
        SocketAddr::V6(a) => {
            let sa = SOCKADDR_IN6 {
                sin6_family: AF_INET6,
                sin6_port: a.port().to_be(),
                sin6_flowinfo: 0,
                sin6_addr: IN6_ADDR {
                    u: IN6_ADDR_0 {
                        Byte: a.ip().octets(),
                    },
                },
                Anonymous: SOCKADDR_IN6_0 {
                    sin6_scope_id: a.scope_id(),
                },
            };
            // SAFETY: a valid SOCKADDR_IN6 that outlives the call.
            unsafe { GetBestInterfaceEx(&sa as *const SOCKADDR_IN6 as *const SOCKADDR, &mut index) }
        }
    };
    if r != 0 {
        return Path::Unknown;
    }
    let mut row = MIB_IF_ROW2 {
        InterfaceIndex: index,
        ..Default::default()
    };
    // SAFETY: `row` is a MIB_IF_ROW2 with InterfaceIndex set, as required.
    if unsafe { GetIfEntry2(&mut row) }.is_err() {
        return Path::Unknown;
    }
    let text = |w: &[u16]| {
        let end = w.iter().position(|&c| c == 0).unwrap_or(w.len());
        String::from_utf16_lossy(&w[..end])
    };
    let names = format!("{} {}", text(&row.Description), text(&row.Alias));
    classify(row.Type, &names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usb_tether_adapters_are_usb_whatever_their_type() {
        assert_eq!(
            classify(6, "Remote NDIS based Internet Sharing Device Ethernet 3"),
            Path::Usb
        );
        assert_eq!(classify(6, "UsbNcm Host Device Ethernet 4"), Path::Usb);
        assert_eq!(classify(6, "Android USB Ethernet/RNDIS"), Path::Usb);
    }

    #[test]
    fn wireless_is_wifi_and_other_ethernet_is_network() {
        assert_eq!(
            classify(71, "Intel(R) Wi-Fi 6 AX201 160MHz Wi-Fi"),
            Path::Wifi
        );
        assert_eq!(
            classify(6, "Realtek PCIe GbE Family Controller Ethernet"),
            Path::Network
        );
        assert_eq!(classify(24, "Software Loopback Interface 1"), Path::Unknown);
        assert_eq!(classify(131, "Some VPN tunnel"), Path::Unknown);
    }
}
