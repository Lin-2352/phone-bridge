# Wi-Fi-layer techniques for a small real-time audio stream on a crowded 2.4 GHz AP (ordinary Android app + ordinary Windows app)

Research status: about 20 tool calls. Many results were search-engine summaries of primary pages, not the pages themselves. Items I fetched in full are marked (fetched). Everything marked INFERRED is my reasoning, not a source. Several key questions ended with gaps; they are listed honestly. No project files were touched.

## 1. WMM / DSCP: does it help, how does an ordinary app get into the voice queue, who honours it

### Takeaway
The mechanism is real (DSCP in the IP header is mapped by the Wi-Fi stack to an 802.11 User Priority and then an EDCA access category), but I found no quantified, citable benefit for one small flow on a contended public AP. The values and the AP/OS behaviour are the risk: EF (46) maps by default to AC_VI, not AC_VO, unless the RFC 8325 exception is applied. Windows qWAVE needs admin to set an arbitrary DSCP. Whether Android lets an ordinary app's setTrafficClass reach the air is unconfirmed (there is a report of it not working).

### Cited Findings
- Default DSCP-to-UP mapping shifts the 3 high-order DSCP bits into the 802.11 UP, so EF (46) lands on UP 5 = AC_VI (video), not UP 6 = AC_VO (voice). RFC 8325 section 4 corrects this: EF (46) and VOICE-ADMIT (44) to UP 6; CS6 (48) to UP 7 in the Linux cfg80211 change and in OpenWrt's hostapd map. A Zebra vendor table lists CS6/CS7 at UP 6 instead, so sources disagree on CS6/CS7. UP 6 and 7 are both AC_VO. — [Linux commit applying RFC 8325 exceptions](https://mirrors.lzu.edu.cn/git/linux-net/-/commit/6fdb8b8781d59796324efa25909f3e2112833f01); [mrncciew RFC 8325 summary](https://mrncciew.com/2021/09/14/rfc-8325-wifi-qos-mappings/); [Zebra table](https://docs.zebra.com/us/en/mobile-computers/software/best-practices-for-voice-deployment-with-cisco-wlan-infrastructure/c-voice-deployment-device-settings/r-voice-device-wi-fi-qos-tagging-and-mapping.html). I did not read the RFC text itself.
- Classic PJSIP DSCP table (not RFC 8325): VIDEO 0x28 (CS5) to VI, VOICE 0x30 (CS6) to VO, CONTROL 0x38 (CS7) to VO, BEST_EFFORT 0x00, BACKGROUND 0x08. PJSIP warns WMM is not user-settable on some platforms and needs both adapter and driver support. — [PJSIP QoS docs](https://docs.pjsip.org/en/latest/specific-guides/network_nat/qos.html)
- Java/Android `DatagramSocket.setTrafficClass` is a hint; the OS may ignore it and the getter may return something else; the low bit is ignored. A Stack Overflow question reports `setTrafficClass(128)` on an Android UDP socket with no change in Wireshark; no Android version given, no confirmed fix. — [API docs mirror](https://learn.microsoft.com/en-us/dotnet/api/java.net.socket.trafficclass?view=net-android-35.0); [SO question mirror](https://kiwix.lema.org/stackoverflow.com_en_all_2023-11/questions/44042460/unable-to-set-dscp-value-in-android-app)
- The AP may map DSCP to queues differently from the router (Ruckus community post, user report, not authoritative). — [Ruckus post](https://community.ruckuswireless.com/t5/Unleashed/ARC-dscp-and-WMM-TOS-dscp-Inconsistency/m-p/36524)
- Windows qWAVE: `QOSAddSocketToFlow(handle, socket, destAddr, trafficType, QOS_NON_ADAPTIVE_FLOW, &flowId)` works for any socket, Vista+, desktop apps only. Microsoft states "Network traffic associated with this flow is not affected by making this call alone" and `QOSSetFlow` need not be called unless shaping is desired. IPv4/v6 mixed addresses not supported. (fetched) — [QOSAddSocketToFlow](https://learn.microsoft.com/en-us/windows/win32/api/qos2/nf-qos2-qosaddsockettoflow)
- Setting an explicit outgoing DSCP (`QOSSetOutgoingDSCPValue`, Win7+) requires membership in Administrators or Network Configuration Operators; otherwise ERROR_ACCESS_DENIED. Seen in the localized Microsoft pages and an ABI diff, not in the English page I could fetch. — [QOS_SET_FLOW (pt-br)](https://learn.microsoft.com/pt-br/windows/win32/api/qos2/ne-qos2-qos_set_flow); [qwave ABI diff](https://abi-laboratory.pro/compatibility/Windows_6.0_to_Windows_7.0/x86_64/headers_diff/qwave.dll/diff.html)
- Studies (only abstracts or snippets seen, no numbers extracted): hardware 802.11e testbed showed EDCA voice can be protected from competing greedy data; AP throttling limits voice capacity in infrastructure mode; WMM gives little visible benefit when uncongested but reduces harm to delay-sensitive traffic when congested; TXOP helps small-packet streams. — [Dangerfield/Malone/Leith 2006](https://www.maths.tcd.ie/~dwmalone/p/winmee06.pdf); [2007 follow-up](https://www.maths.tcd.ie/~dwmalone/p/winmee07.pdf); [Ucar et al. 2009](https://web.itu.edu.tr/akingok/gabrovo09/ObtainingTheEffectsofWMMinCongstNetworks.pdf); [Porto thesis](https://repositorio-aberto.up.pt/handle/10216/101640)

### Inferences
- INFERRED: the AC_VO/AC_VI queues give shorter contention windows and AIFS only against other stations' best-effort traffic; they do not remove co-channel interference from other APs or non-Wi-Fi sources, which is likely most of the 2.4 GHz problem here. Expect modest benefit, mainly on the phone-to-AP uplink hop and the AP-to-PC downlink if the AP honours the tag (downlink marking is done by the AP, which may map from the DSCP it receives, or ignore it).
- INFERRED: public APs commonly trust or re-map DSCP; some strip it to 0 at the edge. Only a capture can tell.
- INFERRED: if setting DSCP, prefer a value that lands in AC_VO under both the legacy table and RFC 8325. CS6 (48, 0xC0 TOS) lands in VO under both PJSIP and RFC 8325 for the kernel mapping; EF (46) lands in VI under the legacy shift mapping. This is my deduction from the cited maps, not a tested result.
- INFERRED verification: capture on the PC with Wireshark/pktmon and look at the IP DSCP field of arriving datagrams (survives only if the AP does not strip it); for the 802.11 layer you need a monitor-mode capture (QoS Control TID field), which normal Windows adapters cannot do. On the phone, `adb shell` `tcpdump` needs root; otherwise a PC-side DSCP check is the only no-root evidence. On the PC, Windows exposes `wlan_intf_opcode_qos_info` (see section 3) to read WFA QoS capability and state.

### Gaps
- No quantified result for "one 400 B/20 ms flow in AC_VO vs AC_BE on a congested 2.4 GHz AP" was found. The studies' result tables were not read.
- Whether stock Android (Android 13, MediaTek, realme) honours a Rust/JNI UDP socket's IP_TOS and maps it to a WMM queue: not found. Linux mac80211 normally derives UP from DSCP, but I did not confirm Android's netd/Wi-Fi HAL path or MediaTek driver behaviour. Note a connected-UDP socket in Rust (`setsockopt IP_TOS`) is plausible but untested here. QUIC library (noq/iroh) may not expose it; would need socket-level access.
- No MediaTek/Qualcomm driver documentation on DSCP mapping found.

## 2. Android 13+ QoS APIs, low-latency lock, power save

### Takeaway
WIFI_MODE_FULL_LOW_LATENCY works only with Wi-Fi connected, app in foreground, and screen on; it disables 802.11 power save. In the background or with the screen off it does not take effect. `addQosPolicies` (SCS) is gated by device overlay, vendor HAL and AP support, and I could not confirm an ordinary app may call it.

### Cited Findings
- Low-latency conditions: Wi-Fi enabled with internet access, app holds a Wi-Fi lock and runs in the foreground, screen on. Power save (802.11 doze) "is explicitly disabled by the framework". Driver/firmware scan and roam parameters "can be tuned to reduce latency", but changes are implementation-specific and not listed. OEMs must support HAL `setLatencyMode` and driver `NL80211_CMD_SET_POWER_SAVE`; OEMs can disable it. No latency figures are given. (fetched) — [AOSP wifi-low-latency](https://source.android.com/docs/core/connect/wifi-low-latency)
- Javadoc history: older wording said the lock stays active with screen off/background; a later commit replaced it with the foreground + screen-on limit. If both locks held, low latency only applies in foreground with screen on. — [e/OS commit 1](https://gitlab.e.foundation/e/os/android_frameworks_base/-/commit/8fb283da77f7ea8433be4f2cf9379efb108b1a48); [commit 2](https://gitlab.e.foundation/e/os/android_frameworks_base/-/commit/0dfff0de844bbcfdd6164f882621ec791a4ac380)
- `WIFI_MODE_FULL_HIGH_PERF` deprecated in API 34; low-latency "provides much of the same desired functionality with less impact on power". (fetched) — [WifiManager reference](https://developer.android.com/reference/android/net/wifi/WifiManager)
- `ROAMING_MODE_*` constants (NONE/NORMAL/AGGRESSIVE) exist from API 35 in the WifiManager reference; permission and use not read. — same page
- QoS (SCS) framework: the `addQosPolicies` / `removeQosPolicies` / `removeAllQosPolicies` CTS test exits early unless an overlay is true, a device-config flag is true, and the supplicant implements AIDL V2; supplicant AIDL has `addQosPolicyRequestForScs`. — [CTS change](https://android.googlesource.com/platform/cts/+/08408d3d1a2); [HAL commit](https://android.googlesource.com/platform/hardware/interfaces/+/34081b3607)
- Listen interval / DTIM: a patent states a station with beacon interval 100 ms and listen interval 5 may leave buffered frames up to 500 ms; skipping DTIM beacons trades power for latency. General mechanics, not a measurement on Android. — [patent US10638424](https://patents.google.com/patent/US10638424); [patent 9210656](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/9210656)

### Inferences
- INFERRED: Relay's existing locks are right, but only help while the app is foreground and screen on. A share with the screen off or app backgrounded likely loses the power-save disable (the observed 2.4 GHz test conditions are unknown; check whether they were foreground).
- INFERRED: power-save wake latency affects the phone's receive path and the PC's downlink to phone only; the phone-to-PC audio flow is uplink from the phone, so DTIM mainly matters for ACK/control and QUIC acks coming back. It cannot explain 480-1995 ms gaps by itself when screen is on and low-latency is active.
- INFERRED: `addQosPolicies` is probably unavailable to ordinary apps (I recall it is a system/privileged API, but I did not confirm the annotation). Treat as not available.

### Gaps
- Permission annotation and "who may call" for `addQosPolicies` and `QosPolicyParams`: the developer.android.com QosPolicyParams page returned 404 and the WifiManager page was too long to read the method section.
- Which realme/MediaTek devices implement latency mode: not found. Verify via `adb shell dumpsys wifi` (read it on a device, not done here).
- No Android-side measurements of low-latency lock benefit.

## 3. Roaming, scanning, Windows media streaming mode, hotspot

### Takeaway
I found no figures for off-channel scan time on Android. Android firmware has lazy and alert roaming, and a mailing-list post says lazy scans are throttled when voice/video-class traffic is present, which is another reason WMM tagging could matter. Windows has a documented media-streaming-mode opcode and a background-scan-disable opcode; I could not confirm admin requirements or measured benefit.

### Cited Findings
- Windows: `wlan_intf_opcode_media_streaming_mode` sets or queries the driver's media streaming mode; can only be set while connected; enabled if at least one client enables it; auto-disabled on disconnect. `wlan_intf_opcode_background_scan_enabled`: can only be disabled while connected; disabled if any client disables it; auto-restored on disconnect. (fetched) — [WLAN_INTF_OPCODE](https://learn.microsoft.com/en-us/windows/win32/api/wlanapi/ne-wlanapi-wlan_intf_opcode)
- Windows also has `wlan_intf_opcode_realtime_connection_quality` (no location permission required) and `wlan_intf_opcode_qos_info` (reads WFA QoS Management capability/state; Microsoft says its primary purpose is telemetry/UI, not behaviour). (fetched) — same page
- Android firmware roaming: "alert roaming" when RSSI low, "lazy roaming" hunts for a better BSSID when RSSI is good; lazy scanning is throttled/disabled by the framework when real-time VO/VI traffic exists (code comment quoted in post). — [android-platform thread](https://groups.google.com/g/android-platform/c/UwjS-HR6Wfo)
- Generic supplicant example `bgscan="simple:15:-75:60"` shows scan interval of 15 s on weak signal below -75 dBm and 60 s otherwise (seconds). — [Laird FAQ](https://www.lairdconnect.com/support/faqs/how-do-i-control-background-scanning-roaming-wi-fi-devices)
- Android STA/AP concurrency docs treat channel mismatch between STA and hotspot as a planning issue (move AP to same channel or other band). — [AOSP STA/AP concurrency](https://source.android.com/devices/tech/connect/wifi-sta-ap-concurrency)

### Inferences
- INFERRED: gaps of 0.5-2 s on 2.4 GHz are bigger than a typical single channel scan, and more consistent with retries/rate-down plus bufferbloat in the AP than with scans. A scan on a dual-band phone visits many channels; whether it explains the spikes needs `adb shell dumpsys wifi` scan logs or pktmon timestamps (not run).
- INFERRED: the media streaming mode likely is an NDIS hint to the driver to avoid background/roam scanning; effect depends on the Intel/Realtek/MediaTek driver. The PC end may not be the bottleneck (the PC pings were lost, though). Apply, then measure with before/after gap logs.
- INFERRED: if the Windows Mobile Hotspot (virtual adapter) is active on the PC, it forces the radio to the STA's channel (concurrency), which can reduce throughput; not verified.

### Gaps
- Admin requirement for `WlanSetInterface` with media streaming mode: not stated on the fetched page; unconfirmed.
- Any measured benefit of media streaming mode: not found.
- Off-channel dwell times / ms impact of Android scans: not found.
- Windows Mobile Hotspot channel-hopping behaviour: no documentation found.

## 4. Airtime, aggregation, retries, rate adaptation, low signal

### Takeaway
Small-packet airtime is dominated by overhead, so cost scales with packet count (transmit opportunities) more than bytes; halving the packet rate is the main lever in the VoIP literature, at a cost of +20 ms delay. I found no data specific to the 8 Mbit/s collapse.

### Cited Findings
- A small VoIP packet's payload airtime is tiny compared with overhead: NSDI 2009 paper quotes 35 us for the whole IP packet, i.e. 680% overhead in its example, and 802.11g best case 240% (above 400% in protected mode with legacy 802.11b stations). PHY not clearly stated. — [Verkaik et al., NSDI 2009](https://static.usenix.org/event/nsdi09/tech/full_papers/verkaik/verkaik.pdf)
- A patent states MAC/PHY overhead caps packets per second; 20 ms packetization is common; smaller packets do not significantly improve capacity; a longer interval cuts the packet rate. Claimed capacity of about 10 calls (realistic 3) per 802.11b AP at 20 ms: a patent claim, unverified. — [US7983243](https://image-ppubs.uspto.gov/dirsearch-public/print/downloadPdf/7983243)
- Other patent: switching 20 ms to 40 ms packets adds 20 ms end-to-end delay and improves capacity. — [US8422489](https://patents.google.com/patent/US8422489B2/en)

### Inferences
- INFERRED: with 400 B at 160 kbit/s the flow is about 0.2 Mbit/s, so byte volume is irrelevant; what matters is 50 pps x 2 hops plus ACKs. Moving to 40 ms (25 pps) halves contention events at +20 ms; for a one-way stream with playout buffering the added delay is probably acceptable if the app already buffers 100+ ms. Not measured.
- INFERRED: a link rate falling to 8 Mbit/s suggests rate adaptation after retries (low SNR at -64 to -71 dBm should not by itself force 8 Mbit/s; interference/collisions from hidden stations or co-channel APs are the usual cause). Also 802.11b-rate management frames or protection mode when a legacy station is on the AP would raise overhead. I did not find a source for this.
- INFERRED: A-MPDU aggregation does not help periodic 20 ms packets because nothing queues to aggregate; it matters only if packets bunch up after a stall.

### Gaps
- No measured retry/rate figures for -70 dBm; no study mapping 8 Mbit/s collapse to a cause; Hwang-Cho and Venkatesha PDFs were suggested by search but not read.

## 5. Direct links that skip the AP: TDLS, Wi-Fi Direct, hotspots

### Takeaway
TDLS exists in the WifiManager API (`setTdlsEnabled`, `isTdlsSupported`, `isTdlsOperationCurrentlyAvailable`) but I found no permission, AP-prohibition or Windows-support evidence; it is unlikely to be practical. Wi-Fi Direct and hotspot options were not researched successfully.

### Cited Findings
- `setTdlsEnabled(InetAddress, boolean)` "Enable/Disable TDLS on a specific local route"; `setTdlsEnabledWithMacAddress` variant; `isTdlsOperationCurrentlyAvailable` checks STA connected state, device TDLS support and available sessions. The executor+callback overload was added in API 34; the Xamarin binding says the InetAddress overload is only supported on Android >= 34. — [WifiManager reference](https://developer.android.com/reference/android/net/wifi/WifiManager); [Xamarin binding](https://developer.xamarin.com/api/member/Android.Net.Wifi.WifiManager.SetTdlsEnabledWithMacAddress/p/System.String/System.Boolean/)
- Disabling TDLS tears down the session and falls back to going through the AP. — [e/OS commit](https://gitlab.e.foundation/e/os/android_frameworks_base/-/commit/b676a4a70db36bdd6ce5060d16f453f797378098)
- Windows 11 "Wi-Fi Dual Station" is a client feature for two simultaneous connections, not a hotspot feature; the 4x latency claim is Qualcomm's own testing. — [Notebookcheck](https://www.notebookcheck.net/Qualcomm-Dual-Station-technology-reduces-Wi-Fi-6-latency-and-jitter-for-games-played-on-Windows-11.569790.0.html)

### Inferences
- INFERRED: TDLS needs both ends' drivers and an AP that permits it; a Windows PC has no public ordinary-app TDLS API that I found; so treat as impractical for a PC peer.

### Gaps
- Wi-Fi Direct permissions (NEARBY_WIFI_DEVICES / location), effect on the main connection and channel selection, LocalOnlyHotspot behaviour, PC hotspot pairing, battery cost: not researched with sources.
- Whether APs prohibit TDLS: not found.

## 6. Preferring 5 GHz / detecting band

### Takeaway
Not covered by sourced findings. What I did see: Android 13 apps calling Wi-Fi APIs need NEARBY_WIFI_DEVICES. — [Android Wi-Fi permissions docs (search result)](https://developer.android.com/develop/connectivity/wifi/wifi-permissions)

### Inferences
- INFERRED and likely true from general knowledge, not sourced here: `WifiInfo.getFrequency()` exposes the connected channel without extra permission beyond what the app already has (the phone's Relay measurement used 2412 vs 5580 MHz, so it is readable); a warning to the user is feasible. Forcing a band or AP needs WifiNetworkSpecifier (user-approval dialog) or suggestions, and I did not verify whether either helps on a public AP.

### Gaps
- WifiNetworkSpecifier / suggestion behaviour for band preference: not researched.

## 7. Bluetooth coexistence

### Takeaway
No source isolated idle Bluetooth. Active-profile coexistence cost on 2.4 GHz is large in chipset tests.

### Cited Findings
- NXP-chipset thesis: A2DP sink coexistence degraded 2.4 GHz Wi-Fi throughput 38% versus 2% on 5 GHz; multi-profile scenarios averaged 60% degradation versus 25% single-profile. — [Nirma Univ. thesis](https://repository.nirmauni.ac.in/jspui/handle/123456789/11913)
- Nordic nRF7002 with BLE traffic: Wi-Fi UDP TX throughput fell from 10.2 Mbit/s without BLE to 6.2 Mbit/s with coexistence in one 2.4 GHz configuration, in an RF shield box. — [Nordic sample](https://developer.nordicsemi.com/nRF_Connect_SDK/doc/latest/nrf/samples/wifi/ble_coex/README.html)
- One Alibaba "lifetips" page gave a 52% figure with unverifiable citations; I discarded it.

### Inferences
- INFERRED: a phone streaming A2DP to earbuds at the same time would be a real 2.4 GHz penalty; idle Bluetooth probably small but unmeasured. Test by toggling Bluetooth off and comparing gap logs.

### Gaps
- Idle Bluetooth effect: no source.

## Priority list of things an ordinary app can try (INFERRED ranking, each needs on-device verification)
1. Detect band (frequency) and warn the user on 2.4 GHz; confirm foreground + screen-on while sharing so low-latency lock applies.
2. Try DSCP marking on the phone's UDP socket (CS6/48 or EF/46 tested both) and on the PC via qWAVE flow (no admin); measure with a PC capture of the DSCP field and compare gap logs with and without. Admin needed to force a specific DSCP value on Windows.
3. Try 40 ms frames (25 pps), measure gap histogram vs 20 ms.
4. Windows `media_streaming_mode` and background-scan-disable, A/B measured.
5. Turn Bluetooth off in A/B tests.
Not available without root/admin/AP control: `addQosPolicies` (unconfirmed), changing AP settings, monitor-mode captures.
