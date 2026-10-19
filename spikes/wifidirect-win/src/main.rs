//! Spike: start a Wi-Fi Direct group on this PC in legacy mode, so a phone
//! can join it like an ordinary Wi-Fi network while the PC keeps its normal
//! Wi-Fi connection. Used to measure what a direct phone <-> PC link gives on
//! real hardware before building the feature. Runs until Enter (or exit);
//! the group disappears with the process.
//!
//!   wifidirect-win [--ssid DIRECT-PB-test] [--pass phonebridge]
//!
//! Without options the name and passphrase are random. Fixed ones are for
//! testing only (easy to type on the phone).

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}

#[cfg(windows)]
fn main() -> windows::core::Result<()> {
    use std::{
        collections::hash_map::RandomState,
        hash::{BuildHasher, Hasher},
    };

    use windows::{
        core::HSTRING,
        Devices::WiFiDirect::{
            WiFiDirectAdvertisementListenStateDiscoverability, WiFiDirectAdvertisementPublisher,
            WiFiDirectAdvertisementPublisherStatusChangedEventArgs, WiFiDirectConnectionListener,
            WiFiDirectConnectionRequestedEventArgs, WiFiDirectDevice,
        },
        Foundation::TypedEventHandler,
        Security::Credentials::PasswordCredential,
    };

    // Spike-grade randomness (std's per-process hash keys).
    let rand = || RandomState::new().build_hasher().finish();
    let alphabet = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let word = |n: usize| -> String {
        (0..n)
            .map(|_| alphabet[(rand() % alphabet.len() as u64) as usize] as char)
            .collect()
    };
    let args: Vec<String> = std::env::args().collect();
    let opt = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1).cloned())
    };
    let ssid = opt("--ssid").unwrap_or_else(|| format!("DIRECT-PB-{}", word(4)));
    let pass = opt("--pass").unwrap_or_else(|| word(12));
    if pass.len() < 8 {
        eprintln!("--pass needs at least 8 characters (WPA2)");
        std::process::exit(2);
    }

    let publisher = WiFiDirectAdvertisementPublisher::new()?;
    let adv = publisher.Advertisement()?;
    adv.SetListenStateDiscoverability(WiFiDirectAdvertisementListenStateDiscoverability::Normal)?;
    let legacy = adv.LegacySettings()?;
    legacy.SetIsEnabled(true)?;
    legacy.SetSsid(&HSTRING::from(ssid.as_str()))?;
    let cred = PasswordCredential::new()?;
    cred.SetPassword(&HSTRING::from(pass.as_str()))?;
    legacy.SetPassphrase(&cred)?;

    publisher.StatusChanged(&TypedEventHandler::<
        WiFiDirectAdvertisementPublisher,
        WiFiDirectAdvertisementPublisherStatusChangedEventArgs,
    >::new(|_, args| {
        if let Some(a) = args.as_ref() {
            println!(
                "STATUS {:?} error={:?}",
                a.Status().map(|s| s.0),
                a.Error().map(|e| e.0)
            );
        }
        Ok(())
    }))?;

    // Like Microsoft's legacy-AP sample: a connection listener registered
    // before Start, accepting what asks (Wi-Fi Direct peers; legacy clients
    // join with the passphrase). Accepted devices are kept alive here.
    let accepted: std::sync::Arc<std::sync::Mutex<Vec<WiFiDirectDevice>>> = Default::default();
    let listener = WiFiDirectConnectionListener::new()?;
    let keep = accepted.clone();
    listener.ConnectionRequested(&TypedEventHandler::<
        WiFiDirectConnectionListener,
        WiFiDirectConnectionRequestedEventArgs,
    >::new(move |_, args| {
        if let Some(a) = args.as_ref() {
            let req = a.GetConnectionRequest()?;
            let id = req.DeviceInformation()?.Id()?;
            println!("REQUEST from {id}");
            match WiFiDirectDevice::FromIdAsync(&id).and_then(|op| op.join()) {
                Ok(dev) => {
                    println!("ACCEPTED {id}");
                    if let Ok(mut k) = keep.lock() {
                        k.push(dev);
                    }
                }
                Err(e) => println!("ACCEPT failed: {e}"),
            }
        }
        Ok(())
    }))?;

    publisher.Start()?;
    println!("SSID {ssid}");
    println!("PASSWORD {pass}");
    println!("status codes: 0 created, 1 started, 2 stopped, 3 aborted; error 0 ok, 1 radio unavailable, 2 resource in use");
    println!("Press Enter to stop.");
    let mut line = String::new();
    if matches!(std::io::stdin().read_line(&mut line), Ok(0) | Err(_)) {
        // No console (started in the background): run until killed.
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    publisher.Stop()?;
    drop(listener);
    println!("stopped");
    Ok(())
}
