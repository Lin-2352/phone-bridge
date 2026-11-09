//! A single HTTPS GET through WinHTTP (the system proxy and certificate store). Used only by the
//! update check, only when the button is pressed.

#![allow(unsafe_code)]
use std::ffi::c_void;
use windows::core::{w, PCWSTR};
use windows::Win32::Networking::WinHttp::*;

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle this module opened and has not closed.
            let _ = unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

/// `https://host/path` only. Stops reading once `cap` bytes are in (the caller treats a body that
/// long as too large).
pub fn get(url: &str, cap: usize) -> Result<Vec<u8>, &'static str> {
    let rest = url.strip_prefix("https://").ok_or("not an https address")?;
    let (host, path) = rest.split_once('/').ok_or("bad address")?;
    let (host_w, path_w) = (wide(host), wide(&format!("/{path}")));
    // SAFETY: plain WinHTTP calls on handles owned by `Handle`; every buffer outlives its call.
    unsafe {
        let session = Handle(WinHttpOpen(
            w!("Relay"),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("Windows networking is not available");
        }
        WinHttpSetTimeouts(session.0, 10_000, 10_000, 10_000, 10_000)
            .map_err(|_| "no connection")?;
        let conn = Handle(WinHttpConnect(
            session.0,
            PCWSTR(host_w.as_ptr()),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if conn.0.is_null() {
            return Err("no connection");
        }
        let req = Handle(WinHttpOpenRequest(
            conn.0,
            w!("GET"),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if req.0.is_null() {
            return Err("no connection");
        }
        let headers: Vec<u16> = "Accept: application/vnd.github+json\r\n"
            .encode_utf16()
            .collect();
        WinHttpSendRequest(req.0, Some(&headers), None, 0, 0, 0).map_err(|_| "no connection")?;
        WinHttpReceiveResponse(req.0, std::ptr::null_mut()).map_err(|_| "no answer")?;
        let mut status = 0u32;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            req.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut c_void),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|_| "no answer")?;
        if status == 403 || status == 429 {
            return Err("GitHub is limiting requests, try later");
        }
        if status != 200 {
            return Err("GitHub did not return the release list");
        }
        let mut out = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            let mut got = 0u32;
            WinHttpReadData(
                req.0,
                buf.as_mut_ptr() as *mut c_void,
                buf.len() as u32,
                &mut got,
            )
            .map_err(|_| "the connection dropped")?;
            if got == 0 {
                return Ok(out);
            }
            out.extend_from_slice(&buf[..got as usize]);
            if out.len() >= cap {
                return Ok(out);
            }
        }
    }
}
