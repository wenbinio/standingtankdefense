//! A minimal, dependency-free JSON value / writer / parser.
//!
//! Used ONLY by the `content.json` + parity-vector exporters and their tests
//! (`roblox/CONTRACTS.md` C1/C4). It is deliberately *not* reachable from any
//! simulation path: nothing in `step()` touches it, and no `serde` (or any other
//! crate) enters the `determinism`/`sim` dependency graph because of it.
//!
//! Only the subset the exporters need is supported: null, bool, **integer**
//! numbers (never floats — see the C1 "no floats anywhere" rule), strings,
//! arrays and objects with insertion-ordered keys.

use core::fmt::Write as _;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The only numeric form. There is no float variant, by design.
    Int(i64),
    Str(String),
    Arr(Vec<Value>),
    /// Insertion-ordered key/value pairs (order is part of the canonical form).
    Obj(Vec<(String, Value)>),
}

/// Build an object from `(key, value)` pairs, preserving order.
pub fn obj(pairs: Vec<(&str, Value)>) -> Value {
    Value::Obj(pairs.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}
/// Shorthand for `Value::Int`.
pub fn int(v: i64) -> Value {
    Value::Int(v)
}
/// Shorthand for `Value::Str`.
pub fn s(v: &str) -> Value {
    Value::Str(v.to_string())
}
/// Shorthand for an array of ints.
pub fn ints(v: impl IntoIterator<Item = i64>) -> Value {
    Value::Arr(v.into_iter().map(Value::Int).collect())
}

impl Value {
    // ---------------- accessors (used by the round-trip / identity tests) ----

    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(kv) => kv.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }
    /// Panicking accessor — tests want a loud failure, not an `Option` dance.
    pub fn at(&self, key: &str) -> &Value {
        self.get(key).unwrap_or_else(|| panic!("missing JSON key {key:?}"))
    }
    pub fn as_i64(&self) -> i64 {
        match self {
            Value::Int(v) => *v,
            _ => panic!("expected int, got {self:?}"),
        }
    }
    pub fn as_str(&self) -> &str {
        match self {
            Value::Str(v) => v,
            _ => panic!("expected string, got {self:?}"),
        }
    }
    pub fn as_bool(&self) -> bool {
        match self {
            Value::Bool(v) => *v,
            _ => panic!("expected bool, got {self:?}"),
        }
    }
    pub fn as_arr(&self) -> &[Value] {
        match self {
            Value::Arr(v) => v,
            _ => panic!("expected array, got {self:?}"),
        }
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }

    // ---------------- writing ----------------------------------------------

    /// Compact, whitespace-free serialization. This is the **canonical form**
    /// that `content_hash` is computed over — it is a pure function of the
    /// value, independent of the pretty-printer's layout choices.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        self.write_compact(&mut out);
        out
    }

    /// Human-diffable layout: containers are expanded for the first
    /// `expand_depth` levels (so one catalog entry occupies exactly one line),
    /// and printed compactly below that. Deterministic — no width heuristics.
    pub fn pretty(&self, expand_depth: usize) -> String {
        let mut out = String::new();
        self.write_pretty(&mut out, 0, expand_depth);
        out.push('\n');
        out
    }

    fn write_compact(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(true) => out.push_str("true"),
            Value::Bool(false) => out.push_str("false"),
            Value::Int(v) => {
                let _ = write!(out, "{v}");
            }
            Value::Str(v) => write_string(out, v),
            Value::Arr(items) => {
                out.push('[');
                for (i, it) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    it.write_compact(out);
                }
                out.push(']');
            }
            Value::Obj(kv) => {
                out.push('{');
                for (i, (k, v)) in kv.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    write_string(out, k);
                    out.push(':');
                    v.write_compact(out);
                }
                out.push('}');
            }
        }
    }

    fn write_pretty(&self, out: &mut String, depth: usize, expand_depth: usize) {
        let expand = depth < expand_depth;
        match self {
            Value::Arr(items) if expand && !items.is_empty() => {
                out.push_str("[\n");
                for (i, it) in items.iter().enumerate() {
                    indent(out, depth + 1);
                    it.write_pretty(out, depth + 1, expand_depth);
                    if i + 1 < items.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push(']');
            }
            Value::Obj(kv) if expand && !kv.is_empty() => {
                out.push_str("{\n");
                for (i, (k, v)) in kv.iter().enumerate() {
                    indent(out, depth + 1);
                    write_string(out, k);
                    out.push_str(": ");
                    v.write_pretty(out, depth + 1, expand_depth);
                    if i + 1 < kv.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                indent(out, depth);
                out.push('}');
            }
            other => other.write_compact(out),
        }
    }

    // ---------------- parsing ----------------------------------------------

    /// Parse a JSON document. Integers only; a `.`/`e` in a number is an error
    /// (the exporters must never emit one).
    pub fn parse(src: &str) -> Result<Value, String> {
        let b = src.as_bytes();
        let mut p = Parser { b, i: 0 };
        p.ws();
        let v = p.value()?;
        p.ws();
        if p.i != b.len() {
            return Err(format!("trailing input at byte {}", p.i));
        }
        Ok(v)
    }
}

fn indent(out: &mut String, depth: usize) {
    for _ in 0..depth {
        out.push_str("  ");
    }
}

fn write_string(out: &mut String, v: &str) {
    out.push('"');
    for c in v.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }
    fn eat(&mut self, c: u8) -> Result<(), String> {
        if self.i < self.b.len() && self.b[self.i] == c {
            self.i += 1;
            Ok(())
        } else {
            Err(format!("expected {:?} at byte {}", c as char, self.i))
        }
    }
    fn lit(&mut self, word: &str) -> Result<(), String> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(())
        } else {
            Err(format!("expected {word:?} at byte {}", self.i))
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        match self.b.get(self.i) {
            None => Err("unexpected end of input".into()),
            Some(b'n') => {
                self.lit("null")?;
                Ok(Value::Null)
            }
            Some(b't') => {
                self.lit("true")?;
                Ok(Value::Bool(true))
            }
            Some(b'f') => {
                self.lit("false")?;
                Ok(Value::Bool(false))
            }
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => {
                self.i += 1;
                let mut items = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b']') {
                    self.i += 1;
                    return Ok(Value::Arr(items));
                }
                loop {
                    self.ws();
                    items.push(self.value()?);
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b']') => {
                            self.i += 1;
                            return Ok(Value::Arr(items));
                        }
                        _ => return Err(format!("bad array at byte {}", self.i)),
                    }
                }
            }
            Some(b'{') => {
                self.i += 1;
                let mut kv = Vec::new();
                self.ws();
                if self.b.get(self.i) == Some(&b'}') {
                    self.i += 1;
                    return Ok(Value::Obj(kv));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    self.eat(b':')?;
                    self.ws();
                    kv.push((k, self.value()?));
                    self.ws();
                    match self.b.get(self.i) {
                        Some(b',') => self.i += 1,
                        Some(b'}') => {
                            self.i += 1;
                            return Ok(Value::Obj(kv));
                        }
                        _ => return Err(format!("bad object at byte {}", self.i)),
                    }
                }
            }
            Some(_) => self.number(),
        }
    }
    fn string(&mut self) -> Result<String, String> {
        self.eat(b'"')?;
        let mut out = String::new();
        loop {
            let c = *self.b.get(self.i).ok_or("unterminated string")?;
            self.i += 1;
            match c {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = *self.b.get(self.i).ok_or("unterminated escape")?;
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let hex = core::str::from_utf8(&self.b[self.i..self.i + 4])
                                .map_err(|_| "bad \\u escape")?;
                            let cp = u32::from_str_radix(hex, 16).map_err(|_| "bad \\u escape")?;
                            self.i += 4;
                            out.push(char::from_u32(cp).ok_or("bad code point")?);
                        }
                        other => return Err(format!("bad escape \\{}", other as char)),
                    }
                }
                // Multi-byte UTF-8 passes through verbatim.
                c => {
                    let len = utf8_len(c);
                    let bytes = &self.b[self.i - 1..self.i - 1 + len];
                    out.push_str(core::str::from_utf8(bytes).map_err(|_| "bad utf-8")?);
                    self.i += len - 1;
                }
            }
        }
    }
    fn number(&mut self) -> Result<Value, String> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while matches!(self.b.get(self.i), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        if matches!(self.b.get(self.i), Some(b'.') | Some(b'e') | Some(b'E')) {
            return Err(format!("non-integer number at byte {start} — floats are forbidden"));
        }
        let text = core::str::from_utf8(&self.b[start..self.i]).map_err(|_| "bad utf-8")?;
        text.parse::<i64>().map(Value::Int).map_err(|e| format!("bad number {text:?}: {e}"))
    }
}

fn utf8_len(first: u8) -> usize {
    match first {
        0x00..=0x7F => 1,
        0xC0..=0xDF => 2,
        0xE0..=0xEF => 3,
        _ => 4,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_through_the_parser() {
        let v = obj(vec![
            ("a", int(-5)),
            ("b", Value::Bool(true)),
            ("c", Value::Null),
            ("d", s("he said \"hi\"\n")),
            ("e", ints([1, 2, 3])),
            ("f", Value::Arr(vec![obj(vec![("x", int(i64::MIN))])])),
        ]);
        assert_eq!(Value::parse(&v.canonical()).unwrap(), v);
        assert_eq!(Value::parse(&v.pretty(2)).unwrap(), v);
        assert_eq!(Value::parse(&v.pretty(4)).unwrap(), v);
    }

    #[test]
    fn floats_are_rejected() {
        assert!(Value::parse("{\"a\":1.5}").is_err());
    }
}
