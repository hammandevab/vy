//! A small JSON reader for the repository's manifest files.
//!
//! The wire form of a manifest is canonical CBOR (SPEC-4.1.3); JSON is the
//! readable form kept under `conformance/manifests/` and what the publisher
//! (M3) turns into CBOR. This reader covers exactly RFC 8259: objects,
//! arrays, strings with escapes, numbers, `true`, `false`, `null`. It keeps
//! object members in file order and rejects duplicate keys, trailing commas
//! and anything after the value. Under 200 lines, so no dependency
//! (CLAUDE.md).

use std::fmt;

/// A JSON value.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// Any number; manifests use integers, read back through [`Value::as_u64`].
    Number(f64),
    /// A string.
    String(String),
    /// An array.
    Array(Vec<Value>),
    /// An object, members in file order.
    Object(Vec<(String, Value)>),
}

/// Where and why parsing stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JsonError {
    /// Byte offset into the text.
    pub at: usize,
    /// What was wrong.
    pub message: String,
}

impl fmt::Display for JsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "JSON error at byte {}: {}", self.at, self.message)
    }
}

impl std::error::Error for JsonError {}

impl Value {
    /// Parse one JSON document.
    pub fn parse(text: &str) -> Result<Value, JsonError> {
        let mut p = Parser {
            b: text.as_bytes(),
            i: 0,
        };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.i != p.b.len() {
            return Err(p.err("trailing characters after the value"));
        }
        Ok(v)
    }

    /// Member `key` of an object, if present.
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(members) => members.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// The string, if this is one.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }

    /// The number as a non-negative integer, if it is one exactly.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Value::Number(n) if *n >= 0.0 && n.fract() == 0.0 && *n < 9.007_199_254_740_992e15 => {
                Some(*n as u64)
            }
            _ => None,
        }
    }

    /// The elements, if this is an array.
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Value::Array(a) => Some(a),
            _ => None,
        }
    }

    /// The members, if this is an object.
    pub fn as_object(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Object(o) => Some(o),
            _ => None,
        }
    }
}

/// Deepest nesting accepted, so a crafted file cannot exhaust the stack.
const MAX_DEPTH: usize = 64;

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn err(&self, message: &str) -> JsonError {
        JsonError {
            at: self.i,
            message: message.into(),
        }
    }

    fn ws(&mut self) {
        while matches!(self.b.get(self.i), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> bool {
        if self.b.get(self.i) == Some(&c) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, c: u8) -> Result<(), JsonError> {
        if self.eat(c) {
            Ok(())
        } else {
            Err(self.err(&format!("expected `{}`", c as char)))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonError> {
        if depth > MAX_DEPTH {
            return Err(self.err("nesting deeper than 64"));
        }
        match self.b.get(self.i).copied() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => self.string().map(Value::String),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(_) => Err(self.err("unexpected character")),
            None => Err(self.err("unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &str, v: Value) -> Result<Value, JsonError> {
        if self.b[self.i..].starts_with(word.as_bytes()) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(self.err("unknown literal"))
        }
    }

    fn number(&mut self) -> Result<Value, JsonError> {
        let start = self.i;
        self.eat(b'-');
        let digits = |p: &mut Self| {
            let s = p.i;
            while matches!(p.b.get(p.i), Some(b'0'..=b'9')) {
                p.i += 1;
            }
            p.i > s
        };
        let int_start = self.i;
        if !digits(self) {
            return Err(self.err("expected a digit"));
        }
        if self.b[int_start] == b'0' && self.i - int_start > 1 {
            return Err(self.err("leading zero"));
        }
        if self.eat(b'.') && !digits(self) {
            return Err(self.err("expected a digit after `.`"));
        }
        if self.eat(b'e') || self.eat(b'E') {
            let _ = self.eat(b'+') || self.eat(b'-');
            if !digits(self) {
                return Err(self.err("expected an exponent"));
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i]).map_err(|_| self.err("utf-8"))?;
        text.parse::<f64>()
            .map(Value::Number)
            .map_err(|_| self.err("bad number"))
    }

    fn string(&mut self) -> Result<String, JsonError> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            match self.b.get(self.i).copied() {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    break;
                }
                Some(b'\\') => {
                    self.i += 1;
                    let c = self.b.get(self.i).copied();
                    self.i += 1;
                    match c {
                        Some(b'"') => out.push(b'"'),
                        Some(b'\\') => out.push(b'\\'),
                        Some(b'/') => out.push(b'/'),
                        Some(b'b') => out.push(8),
                        Some(b'f') => out.push(12),
                        Some(b'n') => out.push(b'\n'),
                        Some(b'r') => out.push(b'\r'),
                        Some(b't') => out.push(b'\t'),
                        Some(b'u') => {
                            let code = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&code) {
                                if !(self.eat(b'\\') && self.eat(b'u')) {
                                    return Err(self.err("lone high surrogate"));
                                }
                                let low = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&low) {
                                    return Err(self.err("bad low surrogate"));
                                }
                                0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00)
                            } else {
                                code
                            };
                            let ch =
                                char::from_u32(ch).ok_or_else(|| self.err("bad code point"))?;
                            out.extend_from_slice(ch.encode_utf8(&mut [0; 4]).as_bytes());
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
                Some(c) if c < 0x20 => return Err(self.err("control character in string")),
                Some(c) => {
                    out.push(c);
                    self.i += 1;
                }
            }
        }
        String::from_utf8(out).map_err(|_| self.err("invalid UTF-8 in string"))
    }

    fn hex4(&mut self) -> Result<u32, JsonError> {
        let s = self
            .b
            .get(self.i..self.i + 4)
            .and_then(|s| std::str::from_utf8(s).ok())
            .ok_or_else(|| self.err("expected four hex digits"))?;
        let v = u32::from_str_radix(s, 16).map_err(|_| self.err("expected four hex digits"))?;
        self.i += 4;
        Ok(v)
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']') {
            return Ok(Value::Array(items));
        }
        loop {
            self.ws();
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b',') {
                continue;
            }
            self.expect(b']')?;
            return Ok(Value::Array(items));
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonError> {
        self.expect(b'{')?;
        let mut members: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.eat(b'}') {
            return Ok(Value::Object(members));
        }
        loop {
            self.ws();
            let key = self.string()?;
            if members.iter().any(|(k, _)| *k == key) {
                return Err(self.err(&format!("duplicate key `{key}`")));
            }
            self.ws();
            self.expect(b':')?;
            self.ws();
            let v = self.value(depth + 1)?;
            members.push((key, v));
            self.ws();
            if self.eat(b',') {
                continue;
            }
            self.expect(b'}')?;
            return Ok(Value::Object(members));
        }
    }
}
