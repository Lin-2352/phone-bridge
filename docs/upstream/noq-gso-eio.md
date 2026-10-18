# Draft upstream report: first GSO send fails with EIO and the connection never recovers

Not filed yet; the project owner decides whether and where to file (iroh / noq).

**Versions:** iroh 1.3.0, noq-udp 1.3.0, noq-proto 1.3.0. Android 13 (realme RMX3561, MediaTek, arm64), interface `rndis0` (USB tethering gadget). Peer: Windows 11.

**What happens:** on a freshly started process, the first large outgoing transmit uses GSO
(`len=12780, segment_size=1420`). `sendmsg` returns `EIO`. noq-udp logs
"halting segmentation offload" and sets `max_gso_segments = 1`, but the connection stalls:
no further data reaches the peer and it closes with "connection lost: timed out" after the 30 s idle timeout.
Later connections in the same process work (GSO is already off). Reproduced 11/11.

**Trace (phone side, RUST_LOG=noq_udp=trace,noq=debug,iroh=debug):**
```
INFO  poll_send{... len=12780}: noq_udp::imp: `libc::sendmsg` failed with I/O error (os error 5); halting segmentation offload
DEBUG noq_udp: sendmsg error: Os { code: 5 ... }, Transmit { len: 12780, segment_size: Some(1420), ecn: Some(Ect0) ... }
DEBUG noq_proto::connection: path added validated=false path_id=1 network_path=(local: 192.0.0.4, remote: [::ffff:10.107.168.101]:60530)
DEBUG ... Abandoned { id: PathId(1), reason: RemoteAbandoned { error_code: 62 } } ... path not in path_id_map id=1
(no further activity until idle timeout)
```

**Expected:** after GSO is halted, the lost packets are retransmitted unsegmented and the transfer continues.

**Workaround:** `QuicTransportConfig::builder().enable_segmentation_offload(false)`; 3/3 fresh runs then pass.

**Repro:** `spikes/iroh-spike` in this repo: run `serve` on the phone over USB tethering, then from the PC `connect --id <id> --addr <phone-ip>:<port> --only download`.
