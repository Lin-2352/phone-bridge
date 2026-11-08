//! Windows platform glue. Empty on other platforms so the workspace builds everywhere.

#[cfg(windows)]
pub mod audio_capture;
pub mod audio_conv;
#[cfg(windows)]
pub mod audio_out;
#[cfg(windows)]
pub mod clipboard;
pub mod dib;
#[cfg(windows)]
mod dpapi;
pub mod hello;
#[cfg(windows)]
pub mod input;
pub mod opus;
#[cfg(windows)]
pub mod power;
#[cfg(windows)]
mod thread_priority;

#[cfg(windows)]
pub use dpapi::{Dpapi, DpapiClipHistory};
