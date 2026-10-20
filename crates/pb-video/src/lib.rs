//! The phone→PC video module shared by "phone screen on the PC" and
//! Continuity Camera: how encoded video travels (records), how fast
//! (pacer), at what size, and with which codec.

pub mod codec;
pub mod pacer;
pub mod record;
pub mod size;

pub use codec::Codec;
pub use pacer::{Decision, Pacer};
pub use record::{Config, Record};
