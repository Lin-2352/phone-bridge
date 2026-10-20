//! The camera's shared pieces: the frames ring (tray writes, the media
//! source reads), the NV12 row copy, the 16:9 fit and the 30 fps heartbeat.
//! Pure and platform-free so both sides and the tests use the same code.

pub mod copy;
pub mod fit;
pub mod heartbeat;
pub mod layout;

pub use copy::copy_nv12;
pub use fit::{fit, Rect};
pub use heartbeat::{Deliver, Heartbeat};
pub use layout::{Read, Ring, DATA_OFFSET, FILE_BYTES, MAGIC, SLOT_BYTES, VERSION};
