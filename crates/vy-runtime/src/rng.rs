//! The two sources behind `random` (SPEC-5.2.11): the seeded xorshift64*
//! of the headless host, which makes a render repeat byte for byte
//! (SPEC-2.1.5), and a ChaCha20 keystream keyed by 256 bits the client
//! drew from the operating system, which is the client's CSPRNG.
//!
//! ChaCha20 is the stream cipher of RFC 8439, used here only as a random
//! generator: the same construction as `rand`'s `StdRng`. It is written
//! out rather than taken as a dependency because it is forty lines of
//! add-rotate-xor and the keystream is checked against the RFC's test
//! vector in [`tests`]; keying it from the operating system is the
//! client's job (the seed is part of a session record, so a replay sees
//! the same bytes, SPEC-11.4.1).

/// What `random` draws from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RandomSource {
    /// xorshift64* from this seed: the headless and conformance source,
    /// deterministic by design. Not a CSPRNG.
    Xorshift(u64),
    /// ChaCha20 keystream under this key; the client keys it from the
    /// operating system's entropy for every page instance.
    ChaCha20([u8; 32]),
}

impl Default for RandomSource {
    fn default() -> Self {
        Self::Xorshift(crate::host::HEADLESS_RANDOM_SEED)
    }
}

/// The running generator behind a [`RandomSource`].
pub(crate) enum Rng {
    Xorshift(u64),
    ChaCha(ChaCha20),
}

impl Rng {
    pub(crate) fn new(source: RandomSource) -> Self {
        match source {
            RandomSource::Xorshift(seed) => Self::Xorshift(seed),
            RandomSource::ChaCha20(key) => Self::ChaCha(ChaCha20::new(&key, 0)),
        }
    }

    /// Fill `dst` with the next bytes of the stream.
    pub(crate) fn fill(&mut self, dst: &mut [u8]) {
        match self {
            Self::Xorshift(state) => {
                for chunk in dst.chunks_mut(8) {
                    let mut x = *state;
                    x ^= x >> 12;
                    x ^= x << 25;
                    x ^= x >> 27;
                    *state = x;
                    let bytes = x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes();
                    chunk.copy_from_slice(&bytes[..chunk.len()]);
                }
            }
            Self::ChaCha(c) => c.fill(dst),
        }
    }
}

const CONSTANTS: [u32; 4] = [0x6170_7865, 0x3320_646e, 0x7962_2d32, 0x6b20_6574];

/// ChaCha20 keystream generator (RFC 8439 §2.3) with a 64-bit block
/// counter in words 12–13 and a 64-bit nonce in words 14–15 (the original
/// layout; the RFC's 32-bit counter and 96-bit nonce occupy the same words).
pub(crate) struct ChaCha20 {
    state: [u32; 16],
    block: [u8; 64],
    pos: usize,
}

impl ChaCha20 {
    pub(crate) fn new(key: &[u8; 32], nonce: u64) -> Self {
        let mut state = [0u32; 16];
        state[..4].copy_from_slice(&CONSTANTS);
        for (i, word) in key.chunks_exact(4).enumerate() {
            state[4 + i] = u32::from_le_bytes([word[0], word[1], word[2], word[3]]);
        }
        state[12] = 0;
        state[13] = 0;
        state[14] = nonce as u32;
        state[15] = (nonce >> 32) as u32;
        Self {
            state,
            block: [0; 64],
            pos: 64,
        }
    }

    /// Start the keystream at block `counter` (tests: the RFC vector).
    #[cfg(test)]
    fn seek(&mut self, counter: u64) {
        self.state[12] = counter as u32;
        self.state[13] = (counter >> 32) as u32;
        self.pos = 64;
    }

    fn quarter_round(s: &mut [u32; 16], a: usize, b: usize, c: usize, d: usize) {
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(16);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(12);
        s[a] = s[a].wrapping_add(s[b]);
        s[d] = (s[d] ^ s[a]).rotate_left(8);
        s[c] = s[c].wrapping_add(s[d]);
        s[b] = (s[b] ^ s[c]).rotate_left(7);
    }

    /// One block of keystream from the current counter, which then advances.
    fn next_block(&mut self) {
        let mut w = self.state;
        for _ in 0..10 {
            Self::quarter_round(&mut w, 0, 4, 8, 12);
            Self::quarter_round(&mut w, 1, 5, 9, 13);
            Self::quarter_round(&mut w, 2, 6, 10, 14);
            Self::quarter_round(&mut w, 3, 7, 11, 15);
            Self::quarter_round(&mut w, 0, 5, 10, 15);
            Self::quarter_round(&mut w, 1, 6, 11, 12);
            Self::quarter_round(&mut w, 2, 7, 8, 13);
            Self::quarter_round(&mut w, 3, 4, 9, 14);
        }
        for (i, word) in w.iter().enumerate() {
            let v = word.wrapping_add(self.state[i]);
            self.block[i * 4..i * 4 + 4].copy_from_slice(&v.to_le_bytes());
        }
        let counter = (u64::from(self.state[13]) << 32 | u64::from(self.state[12])).wrapping_add(1);
        self.state[12] = counter as u32;
        self.state[13] = (counter >> 32) as u32;
        self.pos = 0;
    }

    fn fill(&mut self, dst: &mut [u8]) {
        let mut done = 0;
        while done < dst.len() {
            if self.pos >= 64 {
                self.next_block();
            }
            let n = (dst.len() - done).min(64 - self.pos);
            dst[done..done + n].copy_from_slice(&self.block[self.pos..self.pos + n]);
            self.pos += n;
            done += n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 8439 §2.3.2 and §2.4.2: key 00..1f, nonce 00:00:00:09:00:00:00:4a:00:00:00:00,
    /// counter 1. The first 128 bytes of keystream, as OpenSSL 3 produces
    /// them for the same key and IV.
    const KEYSTREAM: [u8; 128] = [
        0x10, 0xf1, 0xe7, 0xe4, 0xd1, 0x3b, 0x59, 0x15, 0x50, 0x0f, 0xdd, 0x1f, 0xa3, 0x20, 0x71,
        0xc4, 0xc7, 0xd1, 0xf4, 0xc7, 0x33, 0xc0, 0x68, 0x03, 0x04, 0x22, 0xaa, 0x9a, 0xc3, 0xd4,
        0x6c, 0x4e, 0xd2, 0x82, 0x64, 0x46, 0x07, 0x9f, 0xaa, 0x09, 0x14, 0xc2, 0xd7, 0x05, 0xd9,
        0x8b, 0x02, 0xa2, 0xb5, 0x12, 0x9c, 0xd1, 0xde, 0x16, 0x4e, 0xb9, 0xcb, 0xd0, 0x83, 0xe8,
        0xa2, 0x50, 0x3c, 0x4e, 0x0a, 0x88, 0x83, 0x77, 0x39, 0xd7, 0xbf, 0x4e, 0xf8, 0xcc, 0xac,
        0xb0, 0xea, 0x2b, 0xb9, 0xd6, 0x9d, 0x56, 0xc3, 0x94, 0xaa, 0x35, 0x1d, 0xfd, 0xa5, 0xbf,
        0x45, 0x9f, 0x0a, 0x2e, 0x9f, 0xe8, 0xe7, 0x21, 0xf8, 0x92, 0x55, 0xf9, 0xc4, 0x86, 0xbf,
        0x21, 0x67, 0x9c, 0x68, 0x3d, 0x4f, 0x9c, 0x5c, 0xf2, 0xfa, 0x27, 0x86, 0x55, 0x26, 0x00,
        0x5b, 0x06, 0xca, 0x37, 0x4c, 0x86, 0xaf, 0x3b,
    ];

    /// SPEC-5.2.11: the client's CSPRNG is ChaCha20; its keystream matches
    /// the RFC 8439 test vector byte for byte, across block boundaries and
    /// for reads of any length.
    #[test]
    fn spec_5_2_11_chacha20_matches_rfc_8439() {
        let key: [u8; 32] = std::array::from_fn(|i| i as u8);
        // The RFC's 96-bit nonce 00000009 0000004a 00000000 sits in words
        // 13–15: word 13 is the high half of this generator's counter.
        let mut c = ChaCha20::new(&key, 0x4a00_0000);
        c.seek(0x0900_0000 << 32 | 1);
        let mut out = [0u8; 128];
        c.fill(&mut out);
        assert_eq!(out, KEYSTREAM);
        // Same stream when read in odd pieces.
        let mut c = ChaCha20::new(&key, 0x4a00_0000);
        c.seek(0x0900_0000 << 32 | 1);
        let mut pieces = Vec::new();
        for n in [1, 7, 56, 3, 61] {
            let mut buf = vec![0u8; n];
            c.fill(&mut buf);
            pieces.extend(buf);
        }
        assert_eq!(pieces, KEYSTREAM);
        // Two keys, two streams; one key, one stream.
        let mut a = Rng::new(RandomSource::ChaCha20([1; 32]));
        let mut b = Rng::new(RandomSource::ChaCha20([2; 32]));
        let mut a2 = Rng::new(RandomSource::ChaCha20([1; 32]));
        let (mut x, mut y, mut z) = ([0u8; 100], [0u8; 100], [0u8; 100]);
        a.fill(&mut x);
        b.fill(&mut y);
        a2.fill(&mut z);
        assert_ne!(x, y);
        assert_eq!(x, z);
        assert_ne!(x, [0u8; 100]);
    }

    /// The headless source is the xorshift64* of before, so goldens that
    /// use `random` do not move.
    #[test]
    fn spec_5_2_11_xorshift_is_the_headless_source() {
        let mut r = Rng::new(RandomSource::default());
        let mut out = [0u8; 16];
        r.fill(&mut out);
        let mut x = crate::host::HEADLESS_RANDOM_SEED;
        let mut expect = Vec::new();
        for _ in 0..2 {
            x ^= x >> 12;
            x ^= x << 25;
            x ^= x >> 27;
            expect.extend(x.wrapping_mul(0x2545_F491_4F6C_DD1D).to_le_bytes());
        }
        assert_eq!(out.to_vec(), expect);
    }
}
