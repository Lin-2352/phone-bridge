//! Phone notifications on the PC: helpers shared by both sides.

use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

/// Words that mark a message as carrying a one-time code.
const OTP_WORDS: [&str; 8] = [
    "otp",
    "code",
    "passcode",
    "verification",
    "verify",
    "one-time",
    "pin",
    "password",
];

/// The one-time code in a notification, if any: a 4–8 character token of
/// digits (or capitals and digits, at least 4 digits) in a message that
/// mentions a code. Parts of prices, dates, times, decimals and phone
/// numbers are not codes.
pub fn find_otp(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let mentions_code = OTP_WORDS.iter().any(|w| {
        lower.match_indices(w).any(|(i, _)| {
            !lower[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_alphabetic)
        })
    });
    if !mentions_code {
        return None;
    }
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if !chars[i].is_ascii_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < chars.len() && chars[i].is_ascii_alphanumeric() {
            i += 1;
        }
        let token = &chars[start..i];
        if is_code(token) && !joined_to_number(&chars, start, i) {
            return Some(token.iter().collect());
        }
    }
    None
}

fn is_code(token: &[char]) -> bool {
    let digits = token.iter().filter(|c| c.is_ascii_digit()).count();
    (4..=8).contains(&token.len()) && digits >= 4 && !token.iter().any(char::is_ascii_lowercase)
}

/// The token at `start..end` continues a number (`1,299`, `12/10/2026`,
/// `10:30`, `45231.50`) or follows a currency sign.
fn joined_to_number(chars: &[char], start: usize, end: usize) -> bool {
    const JOINERS: [char; 4] = [',', '.', ':', '/'];
    const CURRENCY: [char; 4] = ['₹', '$', '€', '£'];
    let before = start.checked_sub(1).map(|i| chars[i]);
    let before2 = start.checked_sub(2).map(|i| chars[i]);
    if before.is_some_and(|c| CURRENCY.contains(&c))
        || (before == Some(' ') && before2.is_some_and(|c| CURRENCY.contains(&c)))
    {
        return true;
    }
    if before.is_some_and(|c| JOINERS.contains(&c)) && before2.is_some_and(|c| c.is_ascii_digit()) {
        return true;
    }
    let after = chars.get(end).copied();
    let after2 = chars.get(end + 1).copied();
    after.is_some_and(|c| JOINERS.contains(&c)) && after2.is_some_and(|c| c.is_ascii_digit())
}

/// The longest prefix of `s` that fits in `max_bytes` without splitting a
/// character.
pub fn truncate_utf8(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// A short stable id for a notification key: safe for logs (says nothing
/// about the content) and short enough for a Windows toast tag.
pub fn key_hash(key: &str) -> String {
    Sha256::digest(key.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Counts events in one-second windows; `hit` is false once more than
/// `limit` events fell inside the current window.
pub struct RateWindow {
    limit: u32,
    count: u32,
    since: Instant,
}

impl RateWindow {
    pub fn new(limit: u32) -> Self {
        Self {
            limit,
            count: 0,
            since: Instant::now(),
        }
    }

    pub fn hit(&mut self) -> bool {
        if self.since.elapsed() >= Duration::from_secs(1) {
            self.since = Instant::now();
            self.count = 0;
        }
        self.count += 1;
        self.count <= self.limit
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_codes_next_to_keywords() {
        for (t, c) in [
            ("Your OTP is 482913", "482913"),
            ("G-582044 is your Google verification code", "582044"),
            ("Use code 7731 to sign in", "7731"),
            ("Passcode: 09AB12", "09AB12"),
            ("123456 is your WhatsApp code", "123456"),
            ("Your login code is 904417. Do not share it.", "904417"),
        ] {
            assert_eq!(find_otp(t).as_deref(), Some(c), "{t}");
        }
    }

    #[test]
    fn ignores_numbers_that_are_not_codes() {
        for t in [
            "Your order of ₹1,299 ships on 12/10/2026",
            "Call me on 9876543210",
            "Meeting at 10:30",
            "Balance 45231.50",
            "See you in 2026",
            "Room 4021 is ready",
        ] {
            assert_eq!(find_otp(t), None, "{t}");
        }
    }

    #[test]
    fn truncation_keeps_char_boundaries() {
        let s = "a😀b";
        assert_eq!(truncate_utf8(s, 2), "a");
        assert_eq!(truncate_utf8(s, 5), "a😀");
        assert_eq!(truncate_utf8("שלום", 3), "ש");
        assert_eq!(truncate_utf8("abc", 10), "abc");
    }

    #[test]
    fn rate_window_allows_limit_then_refuses() {
        let mut w = RateWindow::new(50);
        assert!((0..50).all(|_| w.hit()));
        assert!(!w.hit());
    }

    #[test]
    fn key_hash_is_16_hex() {
        let h = key_hash("0|com.whatsapp|1|null|10123");
        assert_eq!(h.len(), 16);
        assert!(h.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(h, key_hash("x"));
    }
}
