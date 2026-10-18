//! Windows platform glue. Empty on other platforms so the workspace builds everywhere.

#[cfg(windows)]
pub mod clipboard;
#[cfg(windows)]
mod dpapi;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
pub mod power;

#[cfg(windows)]
pub use dpapi::Dpapi;
