//! Hand-written JSON line building (flat objects only).

/// Longest string value kept in a line.
pub const MAX_STR: usize = 256;

/// A field value. Deliberately tiny: no nesting, no byte blobs.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Str(String),
    Int(i64),
    Float(f64),
    Bool(bool),
}

impl Value {
    pub fn s(v: &str) -> Self {
        Value::Str(v.to_owned())
    }
}

impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Value::s(v)
    }
}
impl From<String> for Value {
    fn from(v: String) -> Self {
        Value::Str(v)
    }
}
impl From<i64> for Value {
    fn from(v: i64) -> Self {
        Value::Int(v)
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Value::Int(i64::try_from(v).unwrap_or(i64::MAX))
    }
}
impl From<usize> for Value {
    fn from(v: usize) -> Self {
        Value::from(v as u64)
    }
}
impl From<f64> for Value {
    fn from(v: f64) -> Self {
        Value::Float(v)
    }
}
impl From<bool> for Value {
    fn from(v: bool) -> Self {
        Value::Bool(v)
    }
}

pub fn push_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars().take(MAX_STR) {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn push_value(out: &mut String, v: &Value) {
    match v {
        Value::Str(s) => push_str(out, s),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) if f.is_finite() => out.push_str(&format!("{f:.3}")),
        Value::Float(_) => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
    }
}

/// `"k":"kind","a":1` (no braces, no timestamp).
pub fn body(kind: &str, fields: &[(&str, Value)]) -> String {
    let mut o = String::from("\"k\":");
    push_str(&mut o, kind);
    for (name, v) in fields {
        if matches!(*name, "t" | "k") {
            continue;
        }
        o.push(',');
        push_str(&mut o, name);
        o.push(':');
        push_value(&mut o, v);
    }
    o
}

/// Why a field object was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldsError {
    Syntax,
    Nested,
    TooLong,
    TooMany,
}

const MAX_FIELDS: usize = 32;
const MAX_KEY: usize = 64;

struct P<'a> {
    b: &'a [u8],
    s: &'a str,
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.b.get(self.i).is_some_and(|c| c.is_ascii_whitespace()) {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> bool {
        self.ws();
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }
    fn string(&mut self, max: usize) -> Result<String, FieldsError> {
        if !self.eat(b'"') {
            return Err(FieldsError::Syntax);
        }
        let mut out = String::new();
        loop {
            let c = self.s[self.i..].chars().next().ok_or(FieldsError::Syntax)?;
            self.i += c.len_utf8();
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.s[self.i..].chars().next().ok_or(FieldsError::Syntax)?;
                    self.i += e.len_utf8();
                    match e {
                        '"' | '\\' | '/' => out.push(e),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'u' => {
                            let hex = self.s.get(self.i..self.i + 4).ok_or(FieldsError::Syntax)?;
                            let n =
                                u32::from_str_radix(hex, 16).map_err(|_| FieldsError::Syntax)?;
                            self.i += 4;
                            out.push(char::from_u32(n).unwrap_or('\u{fffd}'));
                        }
                        _ => return Err(FieldsError::Syntax),
                    }
                }
                c => out.push(c),
            }
            if out.chars().count() > max {
                return Err(FieldsError::TooLong);
            }
        }
    }
    fn word(&mut self) -> &str {
        let a = self.i;
        while self
            .b
            .get(self.i)
            .is_some_and(|c| !matches!(c, b',' | b'}' | b' ' | b'\t' | b'\r' | b'\n'))
        {
            self.i += 1;
        }
        &self.s[a..self.i]
    }
}

/// Parse a flat JSON object (`{"a":"x","n":1,"ok":true}`) into fields.
/// Nested objects or arrays are refused, as are strings over 256
/// characters, keys over 64 and more than 32 fields; `null` values are
/// skipped.
pub fn parse_flat(s: &str) -> Result<Vec<(String, Value)>, FieldsError> {
    let mut p = P {
        b: s.as_bytes(),
        s,
        i: 0,
    };
    let mut out = Vec::new();
    if !p.eat(b'{') {
        return Err(FieldsError::Syntax);
    }
    if p.eat(b'}') {
        return p.finish(out);
    }
    loop {
        let k = p.string(MAX_KEY)?;
        if !p.eat(b':') {
            return Err(FieldsError::Syntax);
        }
        p.ws();
        let v = match p.b.get(p.i) {
            Some(b'{') | Some(b'[') => return Err(FieldsError::Nested),
            Some(b'"') => Some(Value::Str(p.string(MAX_STR)?)),
            Some(_) => {
                let w = p.word();
                match w {
                    "true" => Some(Value::Bool(true)),
                    "false" => Some(Value::Bool(false)),
                    "null" => None,
                    n => Some(
                        n.parse::<i64>()
                            .map(Value::Int)
                            .or_else(|_| n.parse::<f64>().map(Value::Float))
                            .map_err(|_| FieldsError::Syntax)?,
                    ),
                }
            }
            None => return Err(FieldsError::Syntax),
        };
        if let Some(v) = v {
            if out.len() >= MAX_FIELDS {
                return Err(FieldsError::TooMany);
            }
            out.push((k, v));
        }
        if p.eat(b',') {
            continue;
        }
        if p.eat(b'}') {
            return p.finish(out);
        }
        return Err(FieldsError::Syntax);
    }
}

impl P<'_> {
    fn finish(&mut self, out: Vec<(String, Value)>) -> Result<Vec<(String, Value)>, FieldsError> {
        self.ws();
        if self.i == self.b.len() {
            Ok(out)
        } else {
            Err(FieldsError::Syntax)
        }
    }
}

/// An error text safe to keep: quoted parts (names) and path-like words are
/// replaced, and it is cut to 120 characters.
pub fn scrub(e: &str) -> String {
    let mut out = String::new();
    let mut in_q = false;
    for c in e.chars() {
        if c == '"' {
            if !in_q {
                out.push_str("<q>");
            }
            in_q = !in_q;
        } else if !in_q {
            out.push(c);
        }
    }
    let words: Vec<&str> = out
        .split_whitespace()
        .map(|w| if w.contains(['\\', '/']) { "<path>" } else { w })
        .collect();
    words.join(" ").chars().take(120).collect()
}
