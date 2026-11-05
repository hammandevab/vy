//! On-disk caches: modules and resources by SHA-256 (SPEC-7.3.1) and
//! manifests by URL with `ETag`/`max-age` (SPEC-7.3.2).

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use vy_manifest::Hash;

/// Default size cap of the content-addressed cache: 2 GB.
pub const DEFAULT_CACHE_CAP_BYTES: u64 = 2 * 1024 * 1024 * 1024;
/// Default size cap of the HTTP cache of page responses: 64 MB. A manifest
/// is at most 64 KB, but a whole package (SPEC-7.2.1) is cached under its
/// URL too, so without a cap every package ever visited stayed on disk for
/// good, twice over with the blob cache (review 0003, finding 6).
pub const DEFAULT_HTTP_CACHE_CAP_BYTES: u64 = 64 * 1024 * 1024;

/// Seconds since the Unix epoch.
pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Content-addressed store of modules and resources (SPEC-7.3.1): a file
/// per SHA-256 under `blobs/`, verified on every read, evicted least
/// recently used first when the total passes the cap.
///
/// Recency is a logical sequence number kept in `blobs/index`, not file
/// times, so eviction order does not depend on the file system's clock
/// resolution.
#[derive(Debug)]
pub struct BlobCache {
    dir: PathBuf,
    cap: u64,
    /// hash -> (last use, size)
    index: BTreeMap<Hash, (u64, u64)>,
    next_seq: u64,
}

impl BlobCache {
    /// Open or create the cache under `dir` with a size cap in bytes.
    pub fn open(dir: &Path, cap: u64) -> io::Result<Self> {
        let dir = dir.join("blobs");
        fs::create_dir_all(&dir)?;
        let mut cache = BlobCache {
            dir,
            cap,
            index: BTreeMap::new(),
            next_seq: 1,
        };
        cache.load_index()?;
        Ok(cache)
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index")
    }

    fn blob_path(&self, hash: &Hash) -> PathBuf {
        self.dir.join(hash.hex())
    }

    /// Read `index`; rebuild it from the directory when missing or stale.
    fn load_index(&mut self) -> io::Result<()> {
        if let Ok(text) = fs::read_to_string(self.index_path()) {
            for line in text.lines() {
                let mut parts = line.split(' ');
                let (Some(h), Some(seq), Some(size)) = (parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                let (Some(bytes), Ok(seq), Ok(size)) = (
                    vy_manifest::from_hex::<32>(h),
                    seq.parse::<u64>(),
                    size.parse::<u64>(),
                ) else {
                    continue;
                };
                let hash = Hash(bytes);
                if self.blob_path(&hash).is_file() {
                    self.index.insert(hash, (seq, size));
                    self.next_seq = self.next_seq.max(seq + 1);
                }
            }
        }
        // Files the index does not know about: count them as oldest.
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(bytes) = name.to_str().and_then(vy_manifest::from_hex::<32>) else {
                continue;
            };
            let hash = Hash(bytes);
            self.index.entry(hash).or_insert_with(|| {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                (0, size)
            });
        }
        Ok(())
    }

    fn save_index(&self) -> io::Result<()> {
        let mut text = String::new();
        for (hash, (seq, size)) in &self.index {
            text.push_str(&format!("{} {seq} {size}\n", hash.hex()));
        }
        write_atomic(&self.index_path(), text.as_bytes())
    }

    fn touch(&mut self, hash: &Hash, size: u64) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.index.insert(*hash, (seq, size));
    }

    /// True when the cache holds this hash.
    pub fn contains(&self, hash: &Hash) -> bool {
        self.index.contains_key(hash)
    }

    /// Number of entries.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Bytes held.
    pub fn total_bytes(&self) -> u64 {
        self.index.values().map(|(_, s)| s).sum()
    }

    /// The bytes behind `hash`, verified against it; a file that does not
    /// hash to its name is deleted and reported as a miss.
    pub fn get(&mut self, hash: &Hash) -> Option<Vec<u8>> {
        if !self.index.contains_key(hash) {
            return None;
        }
        let path = self.blob_path(hash);
        match fs::read(&path) {
            Ok(bytes) if Hash::of(&bytes) == *hash => {
                self.touch(hash, bytes.len() as u64);
                let _ = self.save_index();
                Some(bytes)
            }
            _ => {
                let _ = fs::remove_file(&path);
                self.index.remove(hash);
                let _ = self.save_index();
                None
            }
        }
    }

    /// Store `bytes` under `hash` (which must be their SHA-256) and evict
    /// the least recently used entries until the cap holds.
    pub fn put(&mut self, hash: &Hash, bytes: &[u8]) -> io::Result<()> {
        debug_assert_eq!(Hash::of(bytes), *hash);
        write_atomic(&self.blob_path(hash), bytes)?;
        self.touch(hash, bytes.len() as u64);
        self.evict();
        self.save_index()
    }

    /// Drop oldest entries until the total is within the cap; the entry
    /// used most recently is never evicted.
    fn evict(&mut self) {
        while self.total_bytes() > self.cap && self.index.len() > 1 {
            let Some((&victim, _)) = self.index.iter().min_by_key(|(_, (seq, _))| *seq) else {
                break;
            };
            let _ = fs::remove_file(self.blob_path(&victim));
            self.index.remove(&victim);
        }
    }
}

/// A cached manifest (or package) response (SPEC-7.3.2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HttpRecord {
    /// Final URL after redirects.
    pub url: String,
    /// `ETag` of the response.
    pub etag: Option<String>,
    /// `max-age` in seconds.
    pub max_age: u64,
    /// When it was fetched or revalidated.
    pub fetched_at: u64,
    /// Media type.
    pub media_type: String,
    /// Body.
    pub body: Vec<u8>,
}

impl HttpRecord {
    /// True when `max-age` has not run out.
    pub fn is_fresh(&self, now: u64) -> bool {
        now < self.fetched_at.saturating_add(self.max_age)
    }

    fn encode(&self) -> Vec<u8> {
        let mut e = minicbor::Encoder::new(Vec::new());
        e.array(6)
            .and_then(|e| e.str(&self.url))
            .and_then(|e| match &self.etag {
                Some(t) => e.str(t),
                None => e.null(),
            })
            .and_then(|e| e.u64(self.max_age))
            .and_then(|e| e.u64(self.fetched_at))
            .and_then(|e| e.str(&self.media_type))
            .and_then(|e| e.bytes(&self.body))
            .expect("vec write cannot fail");
        e.into_writer()
    }

    fn decode(bytes: &[u8]) -> Option<HttpRecord> {
        let mut d = minicbor::Decoder::new(bytes);
        if d.array().ok()? != Some(6) {
            return None;
        }
        let url = d.str().ok()?.to_string();
        let etag = if d.datatype().ok()? == minicbor::data::Type::Null {
            d.null().ok()?;
            None
        } else {
            Some(d.str().ok()?.to_string())
        };
        let max_age = d.u64().ok()?;
        let fetched_at = d.u64().ok()?;
        let media_type = d.str().ok()?.to_string();
        let body = d.bytes().ok()?.to_vec();
        Some(HttpRecord {
            url,
            etag,
            max_age,
            fetched_at,
            media_type,
            body,
        })
    }
}

/// Ordinary HTTP cache for page responses, keyed by the URL asked for
/// (SPEC-7.3.2): one file per URL hash under `http/`, evicted least
/// recently used first when the total passes the cap, with the same
/// logical-sequence index as [`BlobCache`] (`http/index`).
#[derive(Debug)]
pub struct HttpCache {
    dir: PathBuf,
    cap: u64,
    /// URL hash (hex) -> (last use, file size)
    index: BTreeMap<String, (u64, u64)>,
    next_seq: u64,
}

impl HttpCache {
    /// Open or create under `dir` with a size cap in bytes.
    pub fn open(dir: &Path, cap: u64) -> io::Result<Self> {
        let dir = dir.join("http");
        fs::create_dir_all(&dir)?;
        let mut cache = HttpCache {
            dir,
            cap,
            index: BTreeMap::new(),
            next_seq: 1,
        };
        cache.load_index()?;
        Ok(cache)
    }

    fn index_path(&self) -> PathBuf {
        self.dir.join("index")
    }

    fn key(url: &str) -> String {
        Hash::of(url.as_bytes()).hex()
    }

    fn path(&self, key: &str) -> PathBuf {
        self.dir.join(key)
    }

    /// Read `index`; files it does not know about count as oldest.
    fn load_index(&mut self) -> io::Result<()> {
        if let Ok(text) = fs::read_to_string(self.index_path()) {
            for line in text.lines() {
                let mut parts = line.split(' ');
                let (Some(key), Some(seq), Some(size)) = (parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                let (Some(_), Ok(seq), Ok(size)) = (
                    vy_manifest::from_hex::<32>(key),
                    seq.parse::<u64>(),
                    size.parse::<u64>(),
                ) else {
                    continue;
                };
                if self.path(key).is_file() {
                    self.index.insert(key.to_string(), (seq, size));
                    self.next_seq = self.next_seq.max(seq + 1);
                }
            }
        }
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            let name = entry.file_name();
            let Some(key) = name
                .to_str()
                .filter(|n| vy_manifest::from_hex::<32>(n).is_some())
            else {
                continue;
            };
            self.index.entry(key.to_string()).or_insert_with(|| {
                let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
                (0, size)
            });
        }
        Ok(())
    }

    fn save_index(&self) -> io::Result<()> {
        let mut text = String::new();
        for (key, (seq, size)) in &self.index {
            text.push_str(&format!("{key} {seq} {size}\n"));
        }
        write_atomic(&self.index_path(), text.as_bytes())
    }

    fn touch(&mut self, key: &str, size: u64) {
        let seq = self.next_seq;
        self.next_seq += 1;
        self.index.insert(key.to_string(), (seq, size));
    }

    /// Number of records.
    pub fn len(&self) -> usize {
        self.index.len()
    }

    /// True when empty.
    pub fn is_empty(&self) -> bool {
        self.index.is_empty()
    }

    /// Bytes held.
    pub fn total_bytes(&self) -> u64 {
        self.index.values().map(|(_, s)| s).sum()
    }

    /// True when a record for `url` is held.
    pub fn contains(&self, url: &str) -> bool {
        self.index.contains_key(&Self::key(url))
    }

    /// The record for `url`, if any; a hit makes it the most recently used.
    pub fn get(&mut self, url: &str) -> Option<HttpRecord> {
        let key = Self::key(url);
        if !self.index.contains_key(&key) {
            return None;
        }
        let read = fs::read(self.path(&key)).ok().and_then(|b| {
            let size = b.len() as u64;
            HttpRecord::decode(&b).map(|r| (r, size))
        });
        match read {
            Some((record, size)) => {
                self.touch(&key, size);
                let _ = self.save_index();
                Some(record)
            }
            None => {
                self.forget(&key);
                None
            }
        }
    }

    /// Store the record for `url` and evict the least recently used
    /// records until the cap holds; the record just stored is never evicted.
    pub fn put(&mut self, url: &str, record: &HttpRecord) -> io::Result<()> {
        let key = Self::key(url);
        let bytes = record.encode();
        write_atomic(&self.path(&key), &bytes)?;
        self.touch(&key, bytes.len() as u64);
        while self.total_bytes() > self.cap && self.index.len() > 1 {
            let Some(victim) = self
                .index
                .iter()
                .min_by_key(|(_, (seq, _))| *seq)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            let _ = fs::remove_file(self.path(&victim));
            self.index.remove(&victim);
        }
        self.save_index()
    }

    /// Forget `url`.
    pub fn remove(&mut self, url: &str) {
        self.forget(&Self::key(url));
    }

    fn forget(&mut self, key: &str) {
        let _ = fs::remove_file(self.path(key));
        self.index.remove(key);
        let _ = self.save_index();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "vy-proto-{name}-{}-{}",
            std::process::id(),
            now_secs()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// SPEC-7.3.1: entries are keyed by hash, verified on read, and the
    /// least recently used go first when the cap is passed.
    #[test]
    fn blob_cache_verifies_and_evicts_lru() {
        let dir = temp_dir("blobs");
        let a = vec![1u8; 100];
        let b = vec![2u8; 100];
        let c = vec![3u8; 100];
        let (ha, hb, hc) = (Hash::of(&a), Hash::of(&b), Hash::of(&c));
        {
            let mut cache = BlobCache::open(&dir, 250).unwrap();
            cache.put(&ha, &a).unwrap();
            cache.put(&hb, &b).unwrap();
            assert_eq!(cache.get(&ha).unwrap(), a); // a is now newer than b
            cache.put(&hc, &c).unwrap(); // 300 > 250: b goes
            assert!(cache.contains(&ha));
            assert!(!cache.contains(&hb));
            assert!(cache.contains(&hc));
            assert_eq!(cache.total_bytes(), 200);
        }
        // The index survives reopening.
        let mut cache = BlobCache::open(&dir, 250).unwrap();
        assert_eq!(cache.len(), 2);
        assert_eq!(cache.get(&hc).unwrap(), c);
        // Tampered bytes are a miss and are removed.
        fs::write(dir.join("blobs").join(ha.hex()), b"tampered").unwrap();
        assert!(cache.get(&ha).is_none());
        assert!(!cache.contains(&ha));
        assert!(!dir.join("blobs").join(ha.hex()).exists());
        // A file dropped in without an index entry is picked up as oldest.
        fs::write(dir.join("blobs").join(hb.hex()), &b).unwrap();
        let cache = BlobCache::open(&dir, 250).unwrap();
        assert!(cache.contains(&hb));
        let _ = fs::remove_dir_all(&dir);
    }

    /// SPEC-7.3.2: records round-trip with their ETag and max-age.
    #[test]
    fn http_cache_roundtrip() {
        let dir = temp_dir("http");
        let mut cache = HttpCache::open(&dir, DEFAULT_HTTP_CACHE_CAP_BYTES).unwrap();
        let rec = HttpRecord {
            url: "http://localhost:1/x".into(),
            etag: Some("\"abc\"".into()),
            max_age: 60,
            fetched_at: 1000,
            media_type: "application/vy-manifest+cbor".into(),
            body: vec![1, 2, 3],
        };
        cache.put("http://localhost:1/x", &rec).unwrap();
        assert_eq!(cache.get("http://localhost:1/x").unwrap(), rec);
        assert!(cache.get("http://localhost:1/y").is_none());
        assert!(rec.is_fresh(1059));
        assert!(!rec.is_fresh(1060));
        let none = HttpRecord { etag: None, ..rec };
        cache.put("u", &none).unwrap();
        assert_eq!(cache.get("u").unwrap(), none);
        cache.remove("u");
        assert!(cache.get("u").is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// SPEC-7.3.2, SPEC-9.3.1: the HTTP cache has a size cap (64 MB by
    /// default) and evicts the least recently used record first, where a
    /// hit counts as use; the record just stored survives even alone over
    /// the cap; the index outlives a reopen; a damaged file is a miss and
    /// is dropped. Without this every package ever fetched stayed on disk
    /// (review 0003, finding 6).
    #[test]
    fn http_cache_evicts_least_recently_used_at_cap() {
        assert_eq!(DEFAULT_HTTP_CACHE_CAP_BYTES, 64 << 20);
        let dir = temp_dir("http-cap");
        let record = |n: u8| HttpRecord {
            url: format!("http://localhost:1/{n}"),
            etag: None,
            max_age: 60,
            fetched_at: 1000,
            media_type: "application/vy-package+zstd".into(),
            body: vec![n; 1000],
        };
        let size = record(0).encode().len() as u64;
        let cap = size * 2 + size / 2;
        {
            let mut cache = HttpCache::open(&dir, cap).unwrap();
            cache.put("a", &record(1)).unwrap();
            cache.put("b", &record(2)).unwrap();
            assert_eq!(cache.len(), 2);
            assert_eq!(cache.total_bytes(), 2 * size);
            // a is used, so b is the oldest when c arrives.
            assert_eq!(cache.get("a").unwrap().body, vec![1; 1000]);
            cache.put("c", &record(3)).unwrap();
            assert!(cache.contains("a") && !cache.contains("b") && cache.contains("c"));
            assert_eq!(cache.total_bytes(), 2 * size);
            assert!(cache.total_bytes() <= cap);
            // One record bigger than the whole cap still goes in, alone.
            let big = HttpRecord {
                body: vec![9; 4000],
                ..record(4)
            };
            cache.put("d", &big).unwrap();
            assert_eq!(cache.len(), 1, "only the newest survives");
            assert_eq!(cache.get("d").unwrap().body.len(), 4000);
            assert!(cache.get("a").is_none());
        }
        // Reopened: the index and the file are still there, and a damaged
        // file is dropped on read.
        let mut cache = HttpCache::open(&dir, cap).unwrap();
        assert_eq!(cache.len(), 1);
        assert!(cache.contains("d"));
        let key = HttpCache::key("d");
        fs::write(dir.join("http").join(&key), b"garbage").unwrap();
        assert!(cache.get("d").is_none());
        assert!(cache.is_empty());
        assert!(!dir.join("http").join(&key).exists());
        let _ = fs::remove_dir_all(&dir);
    }
}
