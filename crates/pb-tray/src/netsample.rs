//! Periodic `network` and `perf` samples for the structured log.
//!
//! `netsh` output is parsed by pure functions (tested on canned text); the
//! sampler thread only runs the commands and writes the lines. The Wi-Fi
//! name is never recorded and the access point only as the first 8 hex
//! digits of a SHA-256 of its BSSID.

use pb_log::{Logger, Stream, Value};
use sha2::{Digest, Sha256};

#[derive(Debug, Default, PartialEq)]
pub struct Wlan {
    pub band: String,
    pub channel: i64,
    pub signal_pct: i64,
    pub rssi_dbm: i64,
    pub rx_mbps: f64,
    pub tx_mbps: f64,
    pub bssid_hash: String,
}

/// First 8 hex digits of SHA-256 of the lowercased BSSID.
pub fn hash_bssid(bssid: &str) -> String {
    let d = Sha256::digest(bssid.trim().to_ascii_lowercase().as_bytes());
    d.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

fn band_of(channel: i64) -> &'static str {
    match channel {
        1..=14 => "2.4 GHz",
        32..=177 => "5 GHz",
        _ => "",
    }
}

/// The connected interface of `netsh wlan show interfaces` (English
/// output), or `None` when Wi-Fi is not connected.
pub fn parse_wlan(text: &str) -> Option<Wlan> {
    let mut w = Wlan::default();
    let mut connected = false;
    let mut have_band = false;
    for line in text.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match k.as_str() {
            "state" => connected = v.eq_ignore_ascii_case("connected"),
            "bssid" => w.bssid_hash = hash_bssid(v),
            "band" => {
                w.band = v.to_owned();
                have_band = true;
            }
            "channel" => w.channel = v.parse().unwrap_or(0),
            "receive rate (mbps)" => w.rx_mbps = v.parse().unwrap_or(0.0),
            "transmit rate (mbps)" => w.tx_mbps = v.parse().unwrap_or(0.0),
            "signal" => {
                w.signal_pct = v.trim_end_matches('%').trim().parse().unwrap_or(0);
                // The usual Windows mapping of quality to dBm.
                w.rssi_dbm = w.signal_pct / 2 - 100;
            }
            _ => {}
        }
    }
    if !connected {
        return None;
    }
    if !have_band {
        w.band = band_of(w.channel).to_owned();
    }
    Some(w)
}

#[derive(Debug, Default, PartialEq)]
pub struct Ifaces {
    pub wifi: bool,
    pub ethernet: bool,
    pub usb: bool,
    pub vpn: bool,
    pub connected: i64,
}

/// What kind of adapter a name is, by the words Windows uses for them.
pub fn iface_kind(name: &str) -> &'static str {
    let n = name.to_ascii_lowercase();
    const VPN: [&str; 10] = [
        "vpn",
        "wireguard",
        "tun",
        "tap",
        "nord",
        "proton",
        "tailscale",
        "zerotier",
        "mullvad",
        "wintun",
    ];
    if VPN.iter().any(|w| n.contains(w)) {
        "vpn"
    } else if n.contains("remote ndis") || n.contains("ncm") || n.contains("usb") {
        "usb"
    } else if n.contains("wi-fi")
        || n.contains("wifi")
        || n.contains("wireless")
        || n.contains("wlan")
    {
        "wifi"
    } else if n.contains("ethernet") || n.contains("local area") {
        "ethernet"
    } else {
        "other"
    }
}

/// `netsh interface show interface`: rows of admin state, state, type and
/// name; only connected rows count.
pub fn parse_interfaces(text: &str) -> Ifaces {
    let mut r = Ifaces::default();
    for line in text.lines() {
        let cols: Vec<&str> = line.split_whitespace().collect();
        // "Enabled  Connected  Dedicated  Wi-Fi"
        if cols.len() < 4 || !cols[1].eq_ignore_ascii_case("connected") {
            continue;
        }
        r.connected += 1;
        match iface_kind(&cols[3..].join(" ")) {
            "vpn" => r.vpn = true,
            "usb" => r.usb = true,
            "wifi" => r.wifi = true,
            "ethernet" => r.ethernet = true,
            _ => {}
        }
    }
    r
}

pub fn network_fields(w: Option<&Wlan>, i: &Ifaces) -> Vec<(&'static str, Value)> {
    let mut f: Vec<(&'static str, Value)> = vec![
        ("wifi", i.wifi.into()),
        ("ethernet", i.ethernet.into()),
        ("usb", i.usb.into()),
        ("vpn", i.vpn.into()),
        ("ifaces", i.connected.into()),
    ];
    if let Some(w) = w {
        f.push(("band", w.band.clone().into()));
        f.push(("channel", w.channel.into()));
        f.push(("rssi_dbm", w.rssi_dbm.into()));
        f.push(("rx_mbps", w.rx_mbps.into()));
        f.push(("tx_mbps", w.tx_mbps.into()));
        f.push(("bssid", w.bssid_hash.clone().into()));
    }
    f
}

#[cfg(windows)]
fn run(args: &[&str]) -> String {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    std::process::Command::new("netsh")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

/// Start the sampler: a `perf` line every 10 s and a `network` line every
/// 60 s, both on a background thread that ends with the process.
#[cfg(windows)]
pub fn start(lg: Logger) {
    let _ = std::thread::Builder::new()
        .name("pb-sample".into())
        .spawn(move || {
            let mut n = 0u32;
            loop {
                if lg.enabled() {
                    if n.is_multiple_of(6) {
                        lg.set_tz_offset_secs(crate::applog_sys::tz_offset_secs());
                        let w = parse_wlan(&run(&["wlan", "show", "interfaces"]));
                        let i = parse_interfaces(&run(&["interface", "show", "interface"]));
                        lg.event(Stream::Network, "net", &network_fields(w.as_ref(), &i));
                    }
                    if lg.sample(Stream::Perf, "tray", 10) {
                        let mb = crate::applog_sys::working_set_bytes() as f64 / 1_048_576.0;
                        lg.event(
                            Stream::Perf,
                            "tray",
                            &[
                                ("ws_mb", mb.into()),
                                ("sessions", crate::applog::open_sessions().into()),
                            ],
                        );
                    }
                }
                n = n.wrapping_add(1);
                std::thread::sleep(std::time::Duration::from_secs(10));
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    const WLAN: &str = "\nThere is 1 interface on the system:\n\n    Name                   : Wi-Fi\n    Description            : Intel(R) Wi-Fi 6 AX201\n    State                  : connected\n    SSID                   : HomeNet-secret\n    BSSID                  : AA:BB:CC:DD:EE:FF\n    Network type           : Infrastructure\n    Radio type             : 802.11ax\n    Band                   : 5 GHz\n    Channel                : 36\n    Receive rate (Mbps)    : 866.7\n    Transmit rate (Mbps)   : 573.5\n    Signal                 : 92%\n";

    #[test]
    fn parses_a_connected_wifi_interface() {
        let w = parse_wlan(WLAN).unwrap();
        assert_eq!(w.band, "5 GHz");
        assert_eq!(w.channel, 36);
        assert_eq!(w.signal_pct, 92);
        assert_eq!(w.rssi_dbm, -54);
        assert_eq!(w.rx_mbps, 866.7);
        assert_eq!(w.tx_mbps, 573.5);
        assert_eq!(w.bssid_hash.len(), 8);
        assert_eq!(
            w.bssid_hash,
            hash_bssid("aa:bb:cc:dd:ee:ff"),
            "case-insensitive"
        );
        let fields = format!("{:?}", network_fields(Some(&w), &Ifaces::default()));
        assert!(
            !fields.contains("HomeNet") && !fields.contains("AA:BB"),
            "{fields}"
        );
    }

    #[test]
    fn disconnected_or_missing_wifi_is_none_and_band_falls_back_to_channel() {
        assert_eq!(parse_wlan("    State : disconnected\n    SSID : x"), None);
        assert_eq!(
            parse_wlan("The Wireless AutoConfig Service (wlansvc) is not running."),
            None
        );
        let w = parse_wlan("State : connected\nChannel : 6\nSignal : 40%").unwrap();
        assert_eq!(w.band, "2.4 GHz");
        assert_eq!(w.rssi_dbm, -80);
    }

    #[test]
    fn bssid_hash_is_the_first_8_hex_of_sha256() {
        // sha256("aa:bb:cc:dd:ee:ff") is checked against the sha2 crate's own
        // output, not hand-typed, so this pins the length/format and case.
        let h = hash_bssid("AA:BB:CC:DD:EE:FF");
        assert!(h
            .chars()
            .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_eq!(h.len(), 8);
        assert_ne!(h, hash_bssid("aa:bb:cc:dd:ee:00"));
    }

    #[test]
    fn interface_list_flags_vpn_usb_wifi() {
        let t = "\nAdmin State    State          Type             Interface Name\n-------------------------------------------------------------------------\nEnabled        Connected      Dedicated        Wi-Fi\nEnabled        Connected      Dedicated        Ethernet 3\nEnabled        Disconnected   Dedicated        Ethernet\nEnabled        Connected      Dedicated        Remote NDIS based Internet Sharing Device\nEnabled        Connected      Dedicated        Mullvad VPN\n";
        let i = parse_interfaces(t);
        assert_eq!(
            i,
            Ifaces {
                wifi: true,
                ethernet: true,
                usb: true,
                vpn: true,
                connected: 4
            }
        );
        assert_eq!(parse_interfaces("garbage"), Ifaces::default());
        assert_eq!(iface_kind("Local Area Connection"), "ethernet");
        assert_eq!(iface_kind("vEthernet (WSL)"), "ethernet");
    }
}
