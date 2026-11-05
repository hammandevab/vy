//! Protocol (SPEC §7): `vy://` addresses, fetching over HTTPS, the
//! content-addressed cache, ordinary HTTP caching of manifests, and trust
//! in publisher keys.
//!
//! Signatures themselves live in `vy-manifest`; this crate decides which
//! key to believe for an origin (SPEC-7.4.2, SPEC-7.4.4) and reports a
//! page that fails as untrusted so the shell runs it in restricted mode
//! (SPEC-7.4.3).
#![deny(missing_docs)]

mod address;
mod cache;
mod fetch;
mod keys;
mod url;

#[cfg(test)]
mod test_server;
#[cfg(test)]
mod tests;

pub use address::{to_https, Address, AddressError, DEFAULT_PORT};
pub use cache::{BlobCache, HttpCache, HttpRecord, DEFAULT_CACHE_CAP_BYTES};
pub use fetch::{
    Error, Fetched, Fetcher, Form, Options, ACCEPT, BLOB_PATH_PREFIX, MAX_BLOB_BYTES,
    MAX_KEY_FILE_BYTES, MAX_REDIRECTS, WELL_KNOWN_KEY_PATH,
};
pub use keys::{KeyStore, OriginKeys};
pub use url::{accepts, cache_control, media_type, resolve, CacheControl, HttpUrl};
pub use vy_manifest::{MEDIA_MANIFEST, MEDIA_PACKAGE};
