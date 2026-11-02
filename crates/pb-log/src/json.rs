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
