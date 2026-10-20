//! Phone Bridge camera source: a COM in-proc server that Windows' Frame
//! Server loads for the "Phone Bridge Camera" virtual camera. It reads
//! frames the tray writes into the shared frames file and delivers 30 fps
//! NV12 samples. A pure software source: it never opens another device.

pub mod activate;
pub mod com;
pub mod file;
pub mod pump;
