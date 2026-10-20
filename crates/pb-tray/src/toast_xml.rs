//! Toast XML and toast button arguments. Pure (no WinRT), so it is tested on
//! every platform. All notification text is untrusted and escaped here.

use std::path::PathBuf;

/// One toast. Each button is (label, arguments).
#[derive(Clone, Debug, Default)]
pub struct ToastSpec {
    pub tag: String,
    pub group: String,
    pub attribution: String,
    pub title: String,
    pub lines: Vec<String>,
    pub logo: Option<PathBuf>,
    pub picture: Option<PathBuf>,
    pub when_ms: i64,
    pub launch: String,
    pub buttons: Vec<(String, String)>,
    /// (placeholder, arguments of the Send button)
    pub reply: Option<(String, String)>,
    pub menu: Vec<(String, String)>,
    pub popup: bool,
    pub silent: bool,
    pub call: bool,
}

/// Windows shows at most this many buttons, including Send.
const MAX_BUTTONS: usize = 5;

pub fn toast_xml(s: &ToastSpec) -> String {
    let mut x = format!(
        "<toast launch=\"{}\" displayTimestamp=\"{}\"",
        esc(&s.launch),
        iso8601(s.when_ms)
    );
    if s.call {
        x += " scenario=\"incomingCall\"";
    }
    x += "><visual><binding template=\"ToastGeneric\">";
    x += &format!("<text>{}</text>", esc(&s.title));
    if !s.lines.is_empty() {
        x += &format!("<text>{}</text>", esc(&s.lines.join("\n")));
    }
    if !s.attribution.is_empty() {
        x += &format!(
            "<text placement=\"attribution\">{}</text>",
            esc(&s.attribution)
        );
    }
    if let Some(logo) = &s.logo {
        x += &format!(
            "<image placement=\"appLogoOverride\" hint-crop=\"circle\" src=\"{}\"/>",
            esc(&file_uri(logo))
        );
    }
    if let Some(pic) = &s.picture {
        x += &format!("<image src=\"{}\"/>", esc(&file_uri(pic)));
    }
    x += "</binding></visual>";
    let mut actions = String::new();
    let mut room = MAX_BUTTONS;
    if let Some((placeholder, send)) = &s.reply {
        actions += &format!(
            "<input id=\"reply\" type=\"text\" placeHolderContent=\"{}\"/>",
            esc(placeholder)
        );
        actions += &format!(
            "<action content=\"Send\" arguments=\"{}\" hint-inputId=\"reply\"/>",
            esc(send)
        );
        room -= 1;
    }
    for (label, a) in s.buttons.iter().take(room) {
        actions += &format!(
            "<action content=\"{}\" arguments=\"{}\"/>",
            esc(label),
            esc(a)
        );
    }
    for (label, a) in &s.menu {
        actions += &format!(
            "<action content=\"{}\" arguments=\"{}\" placement=\"contextMenu\"/>",
            esc(label),
            esc(a)
        );
    }
    if !actions.is_empty() {
        x += &format!("<actions>{actions}</actions>");
    }
    if s.silent {
        x += "<audio silent=\"true\"/>";
    }
    x += "</toast>";
    x
}

/// Escape text for an XML attribute or element.
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out += "&amp;",
            '<' => out += "&lt;",
            '>' => out += "&gt;",
            '"' => out += "&quot;",
            '\'' => out += "&apos;",
            // Characters XML 1.0 cannot carry at all.
            c if (c as u32) < 0x20 && !matches!(c, '\n' | '\r' | '\t') => {}
            c => out.push(c),
        }
    }
    out
}

/// `C:\a b\c.png` -> `file:///C:/a%20b/c.png`
fn file_uri(p: &std::path::Path) -> String {
    let path = p.to_string_lossy().replace('\\', "/");
    let mut out = String::from("file:///");
    for b in path.trim_start_matches('/').bytes() {
        if b.is_ascii_alphanumeric() || b"-._~/:".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

/// Unix milliseconds as `YYYY-MM-DDTHH:MM:SSZ`.
fn iso8601(ms: i64) -> String {
    let secs = ms.div_euclid(1000);
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ToastArgs {
    Open { tag: String },
    Button { tag: String, index: u32 },
    Reply { tag: String },
    CopyOtp { tag: String },
    Mute { tag: String },
    Silence { tag: String },
}

/// `a=<open|btn|reply|otp|mute|silence>&t=<tag>[&i=<n>]`, tag percent-encoded.
pub fn args(a: &ToastArgs) -> String {
    let (kind, tag, index) = match a {
        ToastArgs::Open { tag } => ("open", tag, None),
        ToastArgs::Button { tag, index } => ("btn", tag, Some(*index)),
        ToastArgs::Reply { tag } => ("reply", tag, None),
        ToastArgs::CopyOtp { tag } => ("otp", tag, None),
        ToastArgs::Mute { tag } => ("mute", tag, None),
        ToastArgs::Silence { tag } => ("silence", tag, None),
    };
    let mut out = format!("a={kind}&t={}", pct(tag));
    if let Some(i) = index {
        out += &format!("&i={i}");
    }
    out
}

pub fn parse_args(s: &str) -> Option<ToastArgs> {
    let (mut kind, mut tag, mut index) = (None, None, None);
    for part in s.split('&') {
        let (k, v) = part.split_once('=')?;
        match k {
            "a" => kind = Some(v),
            "t" => tag = Some(unpct(v)?),
            "i" => index = Some(v.parse::<u32>().ok()?),
            _ => return None,
        }
    }
    let tag = tag.filter(|t| !t.is_empty())?;
    Some(match (kind?, index) {
        ("open", None) => ToastArgs::Open { tag },
        ("btn", Some(index)) => ToastArgs::Button { tag, index },
        ("reply", None) => ToastArgs::Reply { tag },
        ("otp", None) => ToastArgs::CopyOtp { tag },
        ("mute", None) => ToastArgs::Mute { tag },
        ("silence", None) => ToastArgs::Silence { tag },
        _ => return None,
    })
}

fn pct(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn unpct(s: &str) -> Option<String> {
    let mut out = Vec::new();
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' {
            let hex = std::str::from_utf8(b.get(i + 1..i + 3)?).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(b[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec() -> ToastSpec {
        ToastSpec {
            tag: "0123456789abcdef".into(),
            group: "fedcba9876543210".into(),
            attribution: "WhatsApp".into(),
            title: "Mom".into(),
            lines: vec!["Dinner at 8?".into()],
            when_ms: 1_792_000_000_000,
            launch: args(&ToastArgs::Open {
                tag: "0123456789abcdef".into(),
            }),
            popup: true,
            ..Default::default()
        }
    }

    fn count(hay: &str, needle: &str) -> usize {
        hay.matches(needle).count()
    }

    #[test]
    fn toast_xml_escapes_untrusted_text() {
        let s = ToastSpec {
            title: "<b>&\"Mom\"</b>".into(),
            lines: vec!["]]><action content='x'/>".into()],
            buttons: vec![("Mark as read".into(), "a=btn&t=x&i=0".into())],
            ..spec()
        };
        let x = toast_xml(&s);
        assert!(x.contains("&lt;b&gt;&amp;&quot;Mom&quot;&lt;/b&gt;"), "{x}");
        assert!(!x.contains("<action content='x'"), "{x}");
        assert!(!x.contains("<b>"), "{x}");
        assert_eq!(count(&x, "<action "), 1, "{x}");
        assert!(x.contains("arguments=\"a=btn&amp;t=x&amp;i=0\""), "{x}");
    }

    #[test]
    fn basic_toast_has_title_lines_attribution_and_timestamp() {
        let x = toast_xml(&spec());
        assert!(x.starts_with("<toast "));
        assert!(x.contains("<text>Mom</text>"));
        assert!(x.contains("<text>Dinner at 8?</text>"));
        assert!(x.contains("<text placement=\"attribution\">WhatsApp</text>"));
        assert!(x.contains("displayTimestamp=\"2026-10-14T"), "{x}");
        assert!(!x.contains("<audio"));
    }

    #[test]
    fn images_use_file_uris() {
        let s = ToastSpec {
            logo: Some(PathBuf::from(r"C:\Users\A B\toast-images\icon-ab.png")),
            picture: Some(PathBuf::from(r"C:\data\toast-images\t.jpg")),
            ..spec()
        };
        let x = toast_xml(&s);
        assert!(x.contains("placement=\"appLogoOverride\""));
        assert!(
            x.contains("src=\"file:///C:/Users/A%20B/toast-images/icon-ab.png\""),
            "{x}"
        );
        assert!(
            x.contains("src=\"file:///C:/data/toast-images/t.jpg\""),
            "{x}"
        );
    }

    #[test]
    fn call_toast_has_scenario_and_three_buttons() {
        let s = ToastSpec {
            call: true,
            buttons: vec![
                ("Answer".into(), "a=btn&t=x&i=0".into()),
                ("Decline".into(), "a=btn&t=x&i=1".into()),
                ("Silence".into(), "a=silence&t=x".into()),
            ],
            ..spec()
        };
        let x = toast_xml(&s);
        assert!(x.contains("scenario=\"incomingCall\""));
        assert_eq!(count(&x, "<action "), 3);
    }

    #[test]
    fn quiet_toast_is_silent() {
        let s = ToastSpec {
            silent: true,
            popup: false,
            ..spec()
        };
        assert!(toast_xml(&s).contains("<audio silent=\"true\"/>"));
    }

    #[test]
    fn reply_box_has_input_and_send() {
        let s = ToastSpec {
            reply: Some(("Reply to Mom".into(), "a=reply&t=x".into())),
            ..spec()
        };
        let x = toast_xml(&s);
        assert!(
            x.contains("<input id=\"reply\" type=\"text\" placeHolderContent=\"Reply to Mom\"/>"),
            "{x}"
        );
        assert!(x.contains("hint-inputId=\"reply\""), "{x}");
    }

    #[test]
    fn buttons_are_capped_at_five_including_send() {
        let s = ToastSpec {
            reply: Some(("Reply".into(), "a=reply&t=x".into())),
            buttons: (0..6)
                .map(|i| (format!("B{i}"), format!("a=btn&t=x&i={i}")))
                .collect(),
            menu: vec![("Stop showing from Chat".into(), "a=mute&t=x".into())],
            ..spec()
        };
        let x = toast_xml(&s);
        let buttons = x
            .split("<action ")
            .skip(1)
            .filter(|a| !a.contains("placement=\"contextMenu\""))
            .count();
        assert_eq!(buttons, 5, "{x}");
        assert_eq!(count(&x, "placement=\"contextMenu\""), 1);
    }

    #[test]
    fn args_round_trip() {
        let tag = "t&=x y".to_owned();
        for a in [
            ToastArgs::Open { tag: tag.clone() },
            ToastArgs::Button {
                tag: tag.clone(),
                index: 4,
            },
            ToastArgs::Reply { tag: tag.clone() },
            ToastArgs::CopyOtp { tag: tag.clone() },
            ToastArgs::Mute { tag: tag.clone() },
            ToastArgs::Silence { tag: tag.clone() },
        ] {
            assert_eq!(parse_args(&args(&a)), Some(a.clone()), "{}", args(&a));
        }
    }

    #[test]
    fn parse_args_rejects_garbage() {
        for s in [
            "",
            "a=btn&t=x&i=-1",
            "a=nope&t=x",
            "a=btn&t=x",
            "t=x",
            "a=open",
        ] {
            assert_eq!(parse_args(s), None, "{s}");
        }
    }
}
