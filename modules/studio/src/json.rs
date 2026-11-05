//! A small JSON reader for the dev API's answers (RFC 8259): objects,
//! arrays, strings with every escape (surrogate pairs too), numbers,
//! booleans and null, nested at most [`MAX_DEPTH`] deep. Anything else,
//! trailing bytes included, is `None`: the page shows that it could not
//! read the answer instead of guessing.

use alloc::string::String;
use alloc::vec::Vec;

/// Deepest nesting accepted.
pub const MAX_DEPTH: usize = 32;

/// A JSON value. Objects keep their keys in order.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    /// `null`.
    Null,
    /// `true` or `false`.
    Bool(bool),
    /// A number.
    Num(f64),
    /// A string.
    Str(String),
    /// An array.
    Arr(Vec<Json>),
    /// An object.
    Obj(Vec<(String, Json)>),
}

static NULL: Json = Json::Null;

impl Json {
    /// The value at `key` of an object; `null` when absent or not an object.
    pub fn get(&self, key: &str) -> &Json {
        match self {
            Json::Obj(pairs) => pairs
                .iter()
                .find(|(k, _)| k == key)
                .map_or(&NULL, |(_, v)| v),
            _ => &NULL,
        }
    }

    /// The string, if this is one.
    pub fn str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The number, if this is one.
    pub fn num(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// The boolean, if this is one.
    pub fn bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// The elements of an array; empty for anything else.
    pub fn arr(&self) -> &[Json] {
        match self {
            Json::Arr(v) => v,
            _ => &[],
        }
    }
}

/// Parse one JSON text.
pub fn parse(bytes: &[u8]) -> Option<Json> {
    let mut p = Parser { b: bytes, i: 0 };
    let v = p.value(0)?;
    p.ws();
    (p.i == bytes.len()).then_some(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.b.get(self.i).copied()
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            self.i += 1;
        }
    }

    fn eat(&mut self, c: u8) -> Option<()> {
        (self.peek() == Some(c)).then(|| self.i += 1)
    }

    fn word(&mut self, w: &[u8], v: Json) -> Option<Json> {
        if self.b.get(self.i..self.i + w.len()) == Some(w) {
            self.i += w.len();
            Some(v)
        } else {
            None
        }
    }

    fn value(&mut self, depth: usize) -> Option<Json> {
        if depth > MAX_DEPTH {
            return None;
        }
        self.ws();
        match self.peek()? {
            b'{' => self.object(depth),
            b'[' => self.array(depth),
            b'"' => self.string().map(Json::Str),
            b't' => self.word(b"true", Json::Bool(true)),
            b'f' => self.word(b"false", Json::Bool(false)),
            b'n' => self.word(b"null", Json::Null),
            b'-' | b'0'..=b'9' => self.number(),
            _ => None,
        }
    }

    fn object(&mut self, depth: usize) -> Option<Json> {
        self.eat(b'{')?;
        let mut pairs = Vec::new();
        self.ws();
        if self.eat(b'}').is_some() {
            return Some(Json::Obj(pairs));
        }
        loop {
            self.ws();
            let key = self.string()?;
            self.ws();
            self.eat(b':')?;
            let v = self.value(depth + 1)?;
            pairs.push((key, v));
            self.ws();
            if self.eat(b',').is_some() {
                continue;
            }
            self.eat(b'}')?;
            return Some(Json::Obj(pairs));
        }
    }

    fn array(&mut self, depth: usize) -> Option<Json> {
        self.eat(b'[')?;
        let mut items = Vec::new();
        self.ws();
        if self.eat(b']').is_some() {
            return Some(Json::Arr(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.ws();
            if self.eat(b',').is_some() {
                continue;
            }
            self.eat(b']')?;
            return Some(Json::Arr(items));
        }
    }

    fn hex4(&mut self) -> Option<u32> {
        let digits = self.b.get(self.i..self.i + 4)?;
        let mut v = 0u32;
        for &d in digits {
            v = v * 16 + char::from(d).to_digit(16)?;
        }
        self.i += 4;
        Some(v)
    }

    fn string(&mut self) -> Option<String> {
        self.eat(b'"')?;
        let mut out: Vec<u8> = Vec::new();
        loop {
            let c = self.peek()?;
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(out).ok(),
                b'\\' => {
                    let e = self.peek()?;
                    self.i += 1;
                    let ch = match e {
                        b'"' => '"',
                        b'\\' => '\\',
                        b'/' => '/',
                        b'b' => '\u{8}',
                        b'f' => '\u{c}',
                        b'n' => '\n',
                        b'r' => '\r',
                        b't' => '\t',
                        b'u' => {
                            let hi = self.hex4()?;
                            let code = if (0xD800..0xDC00).contains(&hi) {
                                // A high surrogate needs its low half.
                                if self.b.get(self.i..self.i + 2) != Some(b"\\u") {
                                    return None;
                                }
                                self.i += 2;
                                let lo = self.hex4()?;
                                if !(0xDC00..0xE000).contains(&lo) {
                                    return None;
                                }
                                0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
                            } else {
                                hi
                            };
                            char::from_u32(code)?
                        }
                        _ => return None,
                    };
                    let mut buf = [0u8; 4];
                    out.extend_from_slice(ch.encode_utf8(&mut buf).as_bytes());
                }
                0x00..=0x1F => return None,
                c => out.push(c),
            }
        }
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while matches!(
            self.peek(),
            Some(b'-' | b'+' | b'.' | b'e' | b'E' | b'0'..=b'9')
        ) {
            self.i += 1;
        }
        let text = core::str::from_utf8(&self.b[start..self.i]).ok()?;
        text.parse::<f64>().ok().map(Json::Num)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_dev_api_answers() {
        let v = parse(
            br#" {"next": 9, "entries": [{"seq": 4, "kind": "tool", "text": "Edit src/lib.rs"},
                 {"seq": 9, "kind": "build", "text": "klar", "status": "ok"}], "x": null,
                 "b": true, "f": -1.5e2} "#,
        )
        .unwrap();
        assert_eq!(v.get("next").num(), Some(9.0));
        assert_eq!(v.get("entries").arr().len(), 2);
        assert_eq!(v.get("entries").arr()[1].get("status").str(), Some("ok"));
        assert_eq!(v.get("x"), &Json::Null);
        assert_eq!(v.get("missing"), &Json::Null);
        assert_eq!(v.get("b").bool(), Some(true));
        assert_eq!(v.get("f").num(), Some(-150.0));
        assert!(v.get("next").arr().is_empty());
    }

    #[test]
    fn strings_unescape() {
        // `~` stands for the backslash, so the escapes are the reader's to undo.
        let input = r#""a~"b~~c~/d~n~t~u00e5~ud83d~ude00 ~u2014 rå""#
            .replace('~', &char::from(92).to_string());
        let v = parse(input.as_bytes()).unwrap();
        assert_eq!(v.str(), Some("a\"b\\c/d\n\tå😀 — rå"));
        for bad in [
            &br#""\ud83d""#[..],
            br#""\ud83dx""#,
            br#""\x""#,
            b"\"a\nb\"",
            br#""open"#,
            b"\"\xff\"",
        ] {
            assert_eq!(parse(bad), None, "{:?}", core::str::from_utf8(bad));
        }
    }

    #[test]
    fn refuses_what_is_not_json() {
        for bad in [
            &b""[..],
            b"{",
            b"[1,]",
            b"{\"a\" 1}",
            b"{'a': 1}",
            b"tru",
            b"1 2",
            b"{} x",
            b"-",
        ] {
            assert_eq!(parse(bad), None, "{:?}", core::str::from_utf8(bad));
        }
        let deep: Vec<u8> = core::iter::repeat_n(b'[', MAX_DEPTH + 2)
            .chain(core::iter::repeat_n(b']', MAX_DEPTH + 2))
            .collect();
        assert_eq!(parse(&deep), None, "too deep");
        assert_eq!(parse(b"[]"), Some(Json::Arr(Vec::new())));
        assert_eq!(parse(b"{}"), Some(Json::Obj(Vec::new())));
    }
}
