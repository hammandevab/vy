//! Canonical CBOR (RFC 8949 §4.2) on top of minicbor's tokenizer (SPEC-4.1.3).
//!
//! The manifest uses five major types: unsigned integers, byte strings, text
//! strings, arrays and maps with text keys. Everything else (negative
//! integers, floats, tags, simple values, indefinite lengths) is rejected.
//! The reader enforces the core deterministic encoding requirements:
//!
//! - integers and lengths in their shortest form (checked by comparing the
//!   bytes minicbor consumed with the shortest head for the value),
//! - definite lengths only,
//! - map keys in bytewise lexicographic order of their encoding, without
//!   duplicates.
//!
//! The writer produces exactly that form: [`MapWriter`] sorts entries by
//! their encoded key before emitting the map, so a caller cannot get the
//! order wrong.

use crate::Error;
use minicbor::data::Type;
use minicbor::decode::Decoder;
use minicbor::Encoder;
use std::cmp::Ordering;

/// Encoder into a growable buffer.
pub(crate) type Enc = Encoder<Vec<u8>>;

/// Length of the shortest CBOR head carrying the argument `n`.
fn head_len(n: u64) -> usize {
    if n < 24 {
        1
    } else if n <= 0xff {
        2
    } else if n <= 0xffff {
        3
    } else if n <= 0xffff_ffff {
        5
    } else {
        9
    }
}

/// Encoded form of a text key, used for ordering (RFC 8949 §4.2.1).
pub(crate) fn encode_key(key: &str) -> Vec<u8> {
    let mut e = Enc::new(Vec::new());
    e.str(key).expect("vec write cannot fail");
    e.into_writer()
}

/// Strict reader over one CBOR item tree.
pub(crate) struct Reader<'b> {
    d: Decoder<'b>,
}

impl<'b> Reader<'b> {
    pub(crate) fn new(bytes: &'b [u8]) -> Self {
        Self {
            d: Decoder::new(bytes),
        }
    }

    pub(crate) fn position(&self) -> usize {
        self.d.position()
    }

    pub(crate) fn remaining(&self) -> usize {
        self.d.input().len() - self.d.position()
    }

    fn datatype(&self) -> Result<Type, Error> {
        self.d.datatype().map_err(|e| self.malformed(e))
    }

    fn malformed(&self, e: minicbor::decode::Error) -> Error {
        if e.is_end_of_input() {
            Error::Truncated
        } else {
            Error::Malformed {
                at: self.d.position(),
                message: e.to_string(),
            }
        }
    }

    fn check_head(&self, start: usize, argument: u64, payload: usize) -> Result<(), Error> {
        let consumed = self.d.position() - start;
        if consumed == head_len(argument) + payload {
            Ok(())
        } else {
            Err(Error::NonCanonical {
                at: start,
                reason: "integer or length not in shortest form",
            })
        }
    }

    fn type_error(&self, expected: &'static str) -> Error {
        Error::Malformed {
            at: self.d.position(),
            message: format!(
                "expected {expected}, found {}",
                self.d.datatype().map(|t| t.to_string()).unwrap_or_default()
            ),
        }
    }

    /// An unsigned integer in shortest form.
    pub(crate) fn uint(&mut self) -> Result<u64, Error> {
        let at = self.position();
        match self.datatype()? {
            Type::U8 | Type::U16 | Type::U32 | Type::U64 => {}
            _ => return Err(self.type_error("unsigned integer")),
        }
        let v = self.d.u64().map_err(|e| self.malformed(e))?;
        self.check_head(at, v, 0)?;
        Ok(v)
    }

    /// An unsigned integer that fits `u32`.
    pub(crate) fn u32(&mut self, field: &'static str) -> Result<u32, Error> {
        let v = self.uint()?;
        u32::try_from(v).map_err(|_| Error::Field {
            field,
            reason: format!("{v} does not fit 32 bits"),
        })
    }

    /// A definite-length text string.
    pub(crate) fn text(&mut self) -> Result<&'b str, Error> {
        let at = self.position();
        match self.datatype()? {
            Type::String => {}
            Type::StringIndef => {
                return Err(Error::NonCanonical {
                    at,
                    reason: "indefinite-length text string",
                })
            }
            _ => return Err(self.type_error("text string")),
        }
        let s = self.d.str().map_err(|e| self.malformed(e))?;
        self.check_head(at, s.len() as u64, s.len())?;
        Ok(s)
    }

    /// A definite-length byte string.
    pub(crate) fn bytes(&mut self) -> Result<&'b [u8], Error> {
        let at = self.position();
        match self.datatype()? {
            Type::Bytes => {}
            Type::BytesIndef => {
                return Err(Error::NonCanonical {
                    at,
                    reason: "indefinite-length byte string",
                })
            }
            _ => return Err(self.type_error("byte string")),
        }
        let b = self.d.bytes().map_err(|e| self.malformed(e))?;
        self.check_head(at, b.len() as u64, b.len())?;
        Ok(b)
    }

    /// A fixed-size byte string.
    pub(crate) fn bytes_exact<const N: usize>(
        &mut self,
        field: &'static str,
    ) -> Result<[u8; N], Error> {
        let b = self.bytes()?;
        <[u8; N]>::try_from(b).map_err(|_| Error::Field {
            field,
            reason: format!("expected {N} bytes, found {}", b.len()),
        })
    }

    /// Head of a definite-length array; returns its length.
    pub(crate) fn array(&mut self) -> Result<u64, Error> {
        let at = self.position();
        match self.datatype()? {
            Type::Array => {}
            Type::ArrayIndef => {
                return Err(Error::NonCanonical {
                    at,
                    reason: "indefinite-length array",
                })
            }
            _ => return Err(self.type_error("array")),
        }
        let n = self
            .d
            .array()
            .map_err(|e| self.malformed(e))?
            .expect("definite array");
        self.check_head(at, n, 0)?;
        if n as usize > self.remaining() {
            // Every element takes at least one byte.
            return Err(Error::Truncated);
        }
        Ok(n)
    }

    /// Head of a definite-length map; returns its entry count.
    pub(crate) fn map(&mut self) -> Result<u64, Error> {
        let at = self.position();
        match self.datatype()? {
            Type::Map => {}
            Type::MapIndef => {
                return Err(Error::NonCanonical {
                    at,
                    reason: "indefinite-length map",
                })
            }
            _ => return Err(self.type_error("map")),
        }
        let n = self
            .d
            .map()
            .map_err(|e| self.malformed(e))?
            .expect("definite map");
        self.check_head(at, n, 0)?;
        if n as usize > self.remaining() / 2 {
            // Every entry takes at least two bytes.
            return Err(Error::Truncated);
        }
        Ok(n)
    }

    /// A map with text keys in canonical order and no duplicates; `field`
    /// is called once per entry with the key and must consume the value.
    pub(crate) fn map_fields(
        &mut self,
        mut field: impl FnMut(&mut Reader<'b>, &'b str) -> Result<(), Error>,
    ) -> Result<(), Error> {
        let n = self.map()?;
        let mut prev: Option<Vec<u8>> = None;
        for _ in 0..n {
            let at = self.position();
            let key = self.text()?;
            let encoded = encode_key(key);
            if let Some(p) = &prev {
                match encoded.cmp(p) {
                    Ordering::Greater => {}
                    Ordering::Equal => return Err(Error::DuplicateField(key.to_string())),
                    Ordering::Less => {
                        return Err(Error::NonCanonical {
                            at,
                            reason: "map keys not in canonical order",
                        })
                    }
                }
            }
            prev = Some(encoded);
            field(self, key)?;
        }
        Ok(())
    }

    /// An array of text strings.
    pub(crate) fn text_array(&mut self) -> Result<Vec<String>, Error> {
        let n = self.array()?;
        let mut out = Vec::with_capacity(n.min(1024) as usize);
        for _ in 0..n {
            out.push(self.text()?.to_string());
        }
        Ok(out)
    }

    /// Fails unless the whole input was consumed.
    pub(crate) fn finish(self) -> Result<(), Error> {
        if self.remaining() == 0 {
            Ok(())
        } else {
            Err(Error::Trailing {
                at: self.position(),
            })
        }
    }
}

/// Collects map entries and emits them sorted by encoded key.
pub(crate) struct MapWriter {
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

impl MapWriter {
    pub(crate) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    /// Add one entry; `write` encodes exactly one value.
    pub(crate) fn field(&mut self, key: &str, write: impl FnOnce(&mut Enc)) {
        let mut value = Enc::new(Vec::new());
        write(&mut value);
        self.entries.push((encode_key(key), value.into_writer()));
    }

    /// Emit the map into `enc`.
    pub(crate) fn finish(mut self, enc: &mut Enc) {
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));
        enc.map(self.entries.len() as u64)
            .expect("vec write cannot fail");
        for (k, v) in self.entries {
            enc.writer_mut().extend_from_slice(&k);
            enc.writer_mut().extend_from_slice(&v);
        }
    }
}

/// Encode a text string.
pub(crate) fn text(enc: &mut Enc, s: &str) {
    enc.str(s).expect("vec write cannot fail");
}

/// Encode an unsigned integer.
pub(crate) fn uint(enc: &mut Enc, v: u64) {
    enc.u64(v).expect("vec write cannot fail");
}

/// Encode a byte string.
pub(crate) fn bytes(enc: &mut Enc, b: &[u8]) {
    enc.bytes(b).expect("vec write cannot fail");
}

/// Encode an array of text strings.
pub(crate) fn text_array(enc: &mut Enc, items: &[String]) {
    enc.array(items.len() as u64)
        .expect("vec write cannot fail");
    for s in items {
        text(enc, s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_uint(bytes: &[u8]) -> Result<u64, Error> {
        let mut r = Reader::new(bytes);
        let v = r.uint()?;
        r.finish()?;
        Ok(v)
    }

    /// SPEC-4.1.3: integers must use the shortest head.
    #[test]
    fn shortest_form_integers() {
        assert_eq!(read_uint(&[0x17]).unwrap(), 23);
        assert_eq!(read_uint(&[0x18, 0x18]).unwrap(), 24);
        assert_eq!(read_uint(&[0x19, 0x01, 0x00]).unwrap(), 256);
        assert!(matches!(
            read_uint(&[0x18, 0x17]),
            Err(Error::NonCanonical { at: 0, .. })
        ));
        assert!(matches!(
            read_uint(&[0x19, 0x00, 0x18]),
            Err(Error::NonCanonical { .. })
        ));
        assert!(matches!(
            read_uint(&[0x1a, 0, 0, 1, 0]),
            Err(Error::NonCanonical { .. })
        ));
        assert!(matches!(
            read_uint(&[0x1b, 0, 0, 0, 0, 0, 0, 1, 0]),
            Err(Error::NonCanonical { .. })
        ));
    }

    /// SPEC-4.1.3: indefinite lengths and wrong types are rejected.
    #[test]
    fn indefinite_and_types() {
        let mut r = Reader::new(&[0x9f, 0xff]);
        assert!(matches!(r.array(), Err(Error::NonCanonical { .. })));
        let mut r = Reader::new(&[0xbf, 0xff]);
        assert!(matches!(r.map(), Err(Error::NonCanonical { .. })));
        let mut r = Reader::new(&[0x7f, 0x61, 0x61, 0xff]);
        assert!(matches!(r.text(), Err(Error::NonCanonical { .. })));
        let mut r = Reader::new(&[0x20]);
        assert!(matches!(r.uint(), Err(Error::Malformed { .. })));
        let mut r = Reader::new(&[0xf9, 0x3c, 0x00]);
        assert!(matches!(r.uint(), Err(Error::Malformed { .. })));
        let mut r = Reader::new(&[]);
        assert!(matches!(r.uint(), Err(Error::Truncated)));
    }

    /// SPEC-4.1.3: map keys sorted bytewise by their encoding; the writer
    /// produces that order and the reader rejects any other.
    #[test]
    fn map_order() {
        let mut w = MapWriter::new();
        w.field("name", |e| uint(e, 1));
        w.field("abi", |e| uint(e, 2));
        w.field("sig", |e| uint(e, 3));
        w.field("memory_max", |e| uint(e, 4));
        let mut enc = Enc::new(Vec::new());
        w.finish(&mut enc);
        let bytes = enc.into_writer();
        // Shorter keys first (their head byte is smaller), then bytewise.
        let mut r = Reader::new(&bytes);
        let mut keys = Vec::new();
        r.map_fields(|r, k| {
            keys.push(k.to_string());
            r.uint().map(|_| ())
        })
        .unwrap();
        assert_eq!(keys, ["abi", "sig", "name", "memory_max"]);

        let mut swapped = bytes.clone();
        // Swap the first two entries ("abi" 2, "sig" 3): each is 5 bytes.
        let (a, b) = (swapped[1..6].to_vec(), swapped[6..11].to_vec());
        swapped[1..6].copy_from_slice(&b);
        swapped[6..11].copy_from_slice(&a);
        let mut r = Reader::new(&swapped);
        assert!(matches!(
            r.map_fields(|r, _| r.uint().map(|_| ())),
            Err(Error::NonCanonical { .. })
        ));

        let mut dup = Vec::new();
        dup.push(0xa2);
        dup.extend(encode_key("abi"));
        dup.push(1);
        dup.extend(encode_key("abi"));
        dup.push(2);
        let mut r = Reader::new(&dup);
        assert!(matches!(
            r.map_fields(|r, _| r.uint().map(|_| ())),
            Err(Error::DuplicateField(k)) if k == "abi"
        ));
    }

    /// Trailing bytes after the item are an error.
    #[test]
    fn trailing_bytes() {
        assert!(matches!(
            read_uint(&[0x01, 0x02]),
            Err(Error::Trailing { at: 1 })
        ));
    }

    /// A length larger than the remaining input is truncation, not an allocation.
    #[test]
    fn huge_length_is_truncated() {
        let mut r = Reader::new(&[0x9b, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
        assert!(matches!(r.array(), Err(Error::Truncated)));
        let mut r = Reader::new(&[0xba, 0xff, 0xff, 0xff, 0xff]);
        assert!(matches!(r.map(), Err(Error::Truncated)));
    }
}
