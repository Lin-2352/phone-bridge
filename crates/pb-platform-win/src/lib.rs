//! Windows platform glue. Empty on other platforms so the workspace builds everywhere.

#[cfg(windows)]
mod dpapi;

#[cfg(windows)]
pub use dpapi::Dpapi;
