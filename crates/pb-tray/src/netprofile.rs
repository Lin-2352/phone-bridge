//! Which network type Windows gives the networks this PC is connected to (Public, Private or
//! Domain), so Relay can say why the phone cannot reach it. Windows' own firewall treats a
//! *Public* network as hostile and blocks the phone there; Relay is allowed on Private networks
//! only (the installer's rule and the README say so). Relay never changes the network type: that
//! is the person's decision, and it relaxes other Windows rules too.
//!
//! `guidance` is pure; `connected` is a thin read of the Network List Manager (no administrator).
#![allow(unsafe_code)]
// Its only caller is the Windows tray window; elsewhere the module is built for its tests only.
#![cfg_attr(not(windows), allow(dead_code))]

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Category {
    Public,
    Private,
    Domain,
}

/// The one line to show, or None when there is nothing to say. Said only when every connected
/// network is Public: with a Private or Domain network also connected the phone may be on that
/// one, and a wrong warning is worse than none.
pub fn guidance(connected: &[Category]) -> Option<String> {
    if connected.is_empty() || connected.iter().any(|c| *c != Category::Public) {
        return None;
    }
    Some(
        "Windows calls this network Public, so it blocks your phone. \
         Set the network to Private in Windows settings, or use USB tethering."
            .to_owned(),
    )
}

/// The categories of the networks that are connected now; empty when Windows does not say.
#[cfg(windows)]
pub fn connected() -> Vec<Category> {
    use windows::Win32::Networking::NetworkListManager::{
        INetworkListManager, NetworkListManager, NLM_ENUM_NETWORK_CONNECTED,
        NLM_NETWORK_CATEGORY_PRIVATE, NLM_NETWORK_CATEGORY_PUBLIC,
    };
    use windows::Win32::System::Com::{
        CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED,
    };
    // A short-lived thread of its own: COM is initialised and released around the call.
    std::thread::spawn(|| {
        // SAFETY: COM is initialised on this thread before use and released before it ends; every
        // interface is dropped (released) before CoUninitialize runs.
        unsafe {
            let inited = CoInitializeEx(None, COINIT_MULTITHREADED).is_ok();
            let out = (|| -> windows::core::Result<Vec<Category>> {
                let mgr: INetworkListManager =
                    CoCreateInstance(&NetworkListManager, None, CLSCTX_ALL)?;
                let nets = mgr.GetNetworks(NLM_ENUM_NETWORK_CONNECTED)?;
                let mut out = Vec::new();
                loop {
                    let mut one = [None];
                    let mut got = 0u32;
                    nets.Next(&mut one, Some(&mut got))?;
                    let Some(net) = one[0].take().filter(|_| got == 1) else {
                        break;
                    };
                    out.push(match net.GetCategory()? {
                        c if c == NLM_NETWORK_CATEGORY_PUBLIC => Category::Public,
                        c if c == NLM_NETWORK_CATEGORY_PRIVATE => Category::Private,
                        _ => Category::Domain,
                    });
                }
                Ok(out)
            })()
            .unwrap_or_default();
            if inited {
                CoUninitialize();
            }
            out
        }
    })
    .join()
    .unwrap_or_default()
}

#[cfg(not(windows))]
pub fn connected() -> Vec<Category> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use Category::*;

    #[test]
    fn only_public_networks_get_the_guidance() {
        let g = guidance(&[Public]).unwrap();
        assert!(g.contains("Public") && g.contains("Private") && g.contains("USB tethering"));
        assert!(guidance(&[Public, Public]).is_some());
    }

    #[test]
    fn a_private_or_domain_network_means_no_warning() {
        assert_eq!(guidance(&[Private]), None);
        assert_eq!(guidance(&[Domain]), None);
        // The phone may be on the Private one: a wrong warning is worse than none.
        assert_eq!(guidance(&[Public, Private]), None);
    }

    #[cfg(windows)]
    #[test]
    fn reading_the_real_network_types_works_and_does_not_crash() {
        // Whatever this machine's networks are, the call returns (possibly nothing, on a runner without
        // a network) and COM is released.
        let seen = connected();
        eprintln!("network categories seen: {seen:?}");
        assert!(seen.len() < 64);
    }

    #[test]
    fn no_information_means_no_warning() {
        assert_eq!(guidance(&[]), None);
    }
}
