//! Ed25519 signatures over the manifest (SPEC-7.4.1).

use crate::manifest::{from_hex, hex, Manifest, SigBytes, Signature};
use crate::Error;
use ed25519_dalek::{Signer, SigningKey as DalekSigning, VerifyingKey};
use sha2::{Digest, Sha256};
use std::fmt;
use std::str::FromStr;

/// Length of a key id in hex characters: the first 8 bytes of
/// SHA-256(public key).
pub const KEY_ID_HEX_LEN: usize = 16;

/// Key id of a public key: the first 16 hex characters of its SHA-256.
/// Derived, so a key has exactly one id (SPEC-2.1.1).
pub fn key_id(public: &[u8; 32]) -> String {
    let digest = Sha256::digest(public);
    hex(&digest[..KEY_ID_HEX_LEN / 2])
}

/// A publisher's signing key: a 32-byte seed.
#[derive(Clone)]
pub struct SigningKey(DalekSigning);

impl fmt::Debug for SigningKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "SigningKey({})", self.public().key_id())
    }
}

impl SigningKey {
    /// Key from its 32-byte seed.
    pub fn from_seed(seed: &[u8; 32]) -> Self {
        Self(DalekSigning::from_bytes(seed))
    }

    /// The seed.
    pub fn seed(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// The matching public key.
    pub fn public(&self) -> PublicKey {
        PublicKey(self.0.verifying_key())
    }

    /// Sign arbitrary bytes.
    pub fn sign_bytes(&self, message: &[u8]) -> [u8; 64] {
        self.0.sign(message).to_bytes()
    }
}

/// A publisher's public key (32 bytes). Text form: 64 lower-case hex digits.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct PublicKey(VerifyingKey);

impl fmt::Debug for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PublicKey({})", self.key_id())
    }
}

impl fmt::Display for PublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&hex(self.0.as_bytes()))
    }
}

impl FromStr for PublicKey {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bytes = from_hex::<32>(s.trim()).ok_or_else(|| Error::Field {
            field: "public key",
            reason: format!("{s:?} is not 64 hex digits"),
        })?;
        Self::from_bytes(&bytes)
    }
}

impl PublicKey {
    /// Key from its 32 bytes; fails for bytes that are not a curve point.
    pub fn from_bytes(bytes: &[u8; 32]) -> Result<Self, Error> {
        VerifyingKey::from_bytes(bytes)
            .map(PublicKey)
            .map_err(|e| Error::Field {
                field: "public key",
                reason: e.to_string(),
            })
    }

    /// The 32 bytes.
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0.to_bytes()
    }

    /// [`key_id`] of this key.
    pub fn key_id(&self) -> String {
        key_id(self.0.as_bytes())
    }

    /// Strict verification of `signature` over `message`.
    pub fn verify_bytes(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        let sig = ed25519_dalek::Signature::from_bytes(signature);
        self.0.verify_strict(message, &sig).is_ok()
    }
}

/// The `vy-key` text form of a publisher's public keys: one 64-hex-digit
/// Ed25519 key per line, `#` comments and blank lines ignored. This is the
/// body of `https://<host>/.well-known/vy-key` (SPEC-7.4.2) and of the
/// `.pub` file `vy-publish keygen` writes.
pub fn key_file(keys: &[PublicKey]) -> String {
    let mut out = String::from("# vy-key: ed25519 public keys, one per line\n");
    for k in keys {
        out.push_str(&format!("# key_id {}\n{k}\n", k.key_id()));
    }
    out
}

/// Parse the `vy-key` text form; every non-comment line must be a key.
pub fn parse_key_file(text: &str) -> Result<Vec<PublicKey>, Error> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::parse)
        .collect()
}

impl Manifest {
    /// Sign the canonical encoding without `sig` (SPEC-7.4.1) and store
    /// the result in `sig` with the key's id.
    pub fn sign(&mut self, key: &SigningKey) {
        let bytes = self.signed_bytes();
        self.sig = Some(Signature {
            key_id: key.public().key_id(),
            bytes: SigBytes(key.sign_bytes(&bytes)),
        });
    }

    /// True when `sig` is present, names `key` and verifies over the
    /// canonical encoding without `sig` (SPEC-7.4.1).
    pub fn verify(&self, key: &PublicKey) -> bool {
        match &self.sig {
            Some(sig) => {
                sig.key_id == key.key_id() && key.verify_bytes(&self.signed_bytes(), &sig.bytes.0)
            }
            None => false,
        }
    }
}
