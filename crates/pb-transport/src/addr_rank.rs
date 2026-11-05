//! Orders this device's addresses so the real LAN comes first. The pairing code and the saved
//! addresses keep only the first few, and virtual adapters (Hyper-V, WSL, Docker, VPNs) used to
//! push the real one out.

use std::net::{IpAddr, SocketAddr};

/// The kind of network adapter an address sits on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfKind {
    Wired,
    WiFi,
    Other,
}

/// One local address and what the OS says about its adapter.
#[derive(Clone, Debug)]
pub struct IfaceInfo {
    pub ip: IpAddr,
    pub kind: IfKind,
    /// The adapter's name, friendly name and description, joined.
    pub names: String,
}

/// Adapter names of virtual switches, containers and VPNs (matched case-insensitively).
const VIRTUAL: &[&str] = &[
    "hyper-v",
    "vethernet",
    "wsl",
    "docker",
    "virtualbox",
    "vbox",
    "vmware",
    "vmnet",
    "tap-",
    "wireguard",
    "vpn",
    "tailscale",
    "zerotier",
    "virbr",
    "veth",
    "utun",
    "tun0",
    "ppp",
];

fn is_virtual(names: &str) -> bool {
    let n = names.to_ascii_lowercase();
    VIRTUAL.iter().any(|v| n.contains(v))
}

fn link_local(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_link_local(),
        IpAddr::V6(v) => (v.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// 0 is best: a private IPv4 address on Wi-Fi or Ethernet. Then other private IPv4, then CGNAT,
/// public and IPv6 addresses, then anything on a virtual adapter, then link-local addresses.
pub fn addr_rank(ip: IpAddr, iface: Option<&IfaceInfo>) -> u8 {
    if link_local(ip) {
        return 4;
    }
    if iface.is_some_and(|i| is_virtual(&i.names)) {
        return 3;
    }
    match ip {
        IpAddr::V4(v) if v.is_private() => {
            if iface.is_some_and(|i| matches!(i.kind, IfKind::Wired | IfKind::WiFi)) {
                0
            } else {
                1
            }
        }
        _ => 2,
    }
}

/// [addrs] best first (see [addr_rank]); equal ranks keep their order.
pub fn rank_addrs(mut addrs: Vec<SocketAddr>, ifaces: &[IfaceInfo]) -> Vec<SocketAddr> {
    addrs.sort_by_key(|a| addr_rank(a.ip(), ifaces.iter().find(|i| i.ip == a.ip())));
    addrs
}

/// Every up, non-loopback adapter's addresses, as [IfaceInfo]. Empty when the OS gives no answer.
pub fn local_ifaces() -> Vec<IfaceInfo> {
    netdev::get_interfaces()
        .into_iter()
        .filter(|i| i.is_up() && !i.is_loopback())
        .flat_map(|i| {
            let kind = match i.if_type {
                netdev::interface::types::InterfaceType::Wireless80211 => IfKind::WiFi,
                netdev::interface::types::InterfaceType::Ethernet
                | netdev::interface::types::InterfaceType::GigabitEthernet
                | netdev::interface::types::InterfaceType::FastEthernetT
                | netdev::interface::types::InterfaceType::FastEthernetFx => IfKind::Wired,
                _ => IfKind::Other,
            };
            let names = format!(
                "{} {} {}",
                i.name,
                i.friendly_name.clone().unwrap_or_default(),
                i.description.clone().unwrap_or_default()
            );
            let ips: Vec<IpAddr> = i
                .ipv4
                .iter()
                .map(|n| IpAddr::V4(n.addr()))
                .chain(i.ipv6.iter().map(|n| IpAddr::V6(n.addr())))
                .collect();
            ips.into_iter().map(move |ip| IfaceInfo {
                ip,
                kind,
                names: names.clone(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(ip: &str, kind: IfKind, names: &str) -> IfaceInfo {
        IfaceInfo {
            ip: ip.parse().unwrap(),
            kind,
            names: names.into(),
        }
    }

    fn sa(s: &str) -> SocketAddr {
        s.parse().unwrap()
    }

    fn ifaces() -> Vec<IfaceInfo> {
        vec![
            iface("172.17.0.1", IfKind::Wired, "docker0"),
            iface(
                "172.29.16.1",
                IfKind::Wired,
                "vEthernet (WSL (Hyper-V firewall))",
            ),
            iface(
                "192.168.1.20",
                IfKind::WiFi,
                "Wi-Fi Intel(R) Wi-Fi 6E AX211",
            ),
            iface("100.101.1.2", IfKind::Other, "Tailscale"),
            iface("10.0.0.5", IfKind::Other, "rmnet_data0"),
        ]
    }

    #[test]
    fn wifi_lan_beats_docker_and_vethernet() {
        let r = rank_addrs(
            vec![
                sa("172.17.0.1:47200"),
                sa("172.29.16.1:47200"),
                sa("192.168.1.20:47200"),
            ],
            &ifaces(),
        );
        assert_eq!(r[0], sa("192.168.1.20:47200"));
    }

    #[test]
    fn link_local_goes_last() {
        let r = rank_addrs(
            vec![
                sa("[fe80::1]:47200"),
                sa("169.254.3.4:47200"),
                sa("192.168.1.20:47200"),
            ],
            &ifaces(),
        );
        assert_eq!(r[0], sa("192.168.1.20:47200"));
        assert!(
            addr_rank(sa("[fe80::1]:1").ip(), None)
                > addr_rank(sa("172.17.0.1:1").ip(), ifaces().first())
        );
    }

    #[test]
    fn cgnat_after_lan() {
        let r = rank_addrs(
            vec![
                sa("100.101.1.2:47200"),
                sa("10.0.0.5:47200"),
                sa("192.168.1.20:47200"),
            ],
            &ifaces(),
        );
        assert_eq!(
            r,
            vec![
                sa("192.168.1.20:47200"),
                sa("10.0.0.5:47200"),
                sa("100.101.1.2:47200")
            ]
        );
    }

    #[test]
    fn order_is_stable_within_a_rank() {
        let r = rank_addrs(vec![sa("10.1.1.1:1"), sa("10.2.2.2:1")], &[]);
        assert_eq!(r, vec![sa("10.1.1.1:1"), sa("10.2.2.2:1")]);
    }

    #[test]
    fn real_lan_survives_the_six_address_limit() {
        let mut addrs: Vec<SocketAddr> = (1..=7)
            .map(|i| sa(&format!("172.17.0.{i}:47200")))
            .collect();
        addrs.push(sa("192.168.1.20:47200"));
        let docker: Vec<IfaceInfo> = (1..=7)
            .map(|i| {
                iface(
                    &format!("172.17.0.{i}"),
                    IfKind::Wired,
                    "Docker Desktop vEthernet",
                )
            })
            .chain([iface("192.168.1.20", IfKind::WiFi, "wlan0")])
            .collect();
        assert!(rank_addrs(addrs, &docker)[..6].contains(&sa("192.168.1.20:47200")));
    }
}
