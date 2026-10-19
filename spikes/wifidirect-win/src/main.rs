//! Spike: start a Wi-Fi Direct group on this PC in legacy mode, so a phone
//! can join it like an ordinary Wi-Fi network while the PC keeps its normal
//! Wi-Fi connection. Used to measure what a direct phone <-> PC link gives on
//! real hardware before building the feature. Runs until Enter (or exit);
//! the group disappears with the process.

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
            WiFiDirectAdvertisementPublisherStatusChangedEventArgs,
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
    let ssid = format!("DIRECT-PB-{}", word(4));
    let pass = word(12);

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
    println!("stopped");
    Ok(())
}
