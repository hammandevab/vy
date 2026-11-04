//! Protocol: `vy://` addresses, fetching, content-addressed cache, signatures (SPEC §7).
//!
//! M3 implements fetching and cache; M4 adds signatures.
#![deny(missing_docs)]

/// Media type of a whole package (SPEC-7.2.1).
pub const MEDIA_PACKAGE: &str = "application/vy-package+zstd";
/// Media type of a bare manifest (SPEC-7.2.1).
pub const MEDIA_MANIFEST: &str = "application/vy-manifest+cbor";

/// Translate a `vy://` address to the HTTPS URL it is fetched from (SPEC-7.1.1).
///
/// Returns `None` for addresses that are not valid `vy://` addresses, including
/// any with a fragment (SPEC-7.1.2).
pub fn to_https(addr: &str) -> Option<String> {
    let rest = addr.strip_prefix("vy://")?;
    if rest.is_empty() || rest.contains('#') {
        return None;
    }
    Some(format!("https://{rest}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-7.1.1: vy://host/path?q becomes https://host/path?q.
    #[test]
    fn spec_7_1_1_translation() {
        assert_eq!(
            to_https("vy://exempel.se/butik?x=1").as_deref(),
            Some("https://exempel.se/butik?x=1")
        );
        assert_eq!(
            to_https("vy://exempel.se:8443/").as_deref(),
            Some("https://exempel.se:8443/")
        );
        assert_eq!(to_https("https://exempel.se/"), None);
    }

    /// SPEC-7.1.2: fragments are not allowed.
    #[test]
    fn spec_7_1_2_no_fragments() {
        assert_eq!(to_https("vy://exempel.se/sida#avsnitt"), None);
    }

    /// SPEC-7.2.1: media types.
    #[test]
    fn spec_7_2_1_media_types() {
        assert_eq!(MEDIA_PACKAGE, "application/vy-package+zstd");
        assert_eq!(MEDIA_MANIFEST, "application/vy-manifest+cbor");
    }

    /// SPEC-7.1.3: TLS is required; localhost is the only exception.
    #[test]
    #[ignore = "M3: fetch"]
    fn spec_7_1_3_tls_required() {
        todo!()
    }

    /// SPEC-7.2.2, SPEC-1.1.1: a response with another media type (e.g. HTML) is never rendered.
    #[test]
    #[ignore = "M3: fetch"]
    fn spec_7_2_2_other_media_type_not_rendered() {
        todo!()
    }

    /// SPEC-7.3.1: cache is keyed by SHA-256; a matching hash is never refetched.
    #[test]
    #[ignore = "M3: cache"]
    fn spec_7_3_1_content_addressed_cache() {
        todo!()
    }

    /// SPEC-7.3.2: manifests use ordinary HTTP caching.
    #[test]
    #[ignore = "M3: cache"]
    fn spec_7_3_2_manifest_http_cache() {
        todo!()
    }

    /// SPEC-7.4.1: Ed25519 signature over canonical CBOR without `sig`.
    #[test]
    #[ignore = "M4: signatures"]
    fn spec_7_4_1_signature_scope() {
        todo!()
    }

    /// SPEC-7.4.2: key from DNS TXT and .well-known must agree on first visit.
    #[test]
    #[ignore = "M4: signatures"]
    fn spec_7_4_2_key_discovery() {
        todo!()
    }

    /// SPEC-7.4.3: unsigned page runs in restricted mode with no capabilities.
    #[test]
    #[ignore = "M4: signatures"]
    fn spec_7_4_3_restricted_mode() {
        todo!()
    }

    /// SPEC-7.4.4: key rotation via old-key-signed manifest naming the new key id.
    #[test]
    #[ignore = "M4: signatures"]
    fn spec_7_4_4_key_rotation() {
        todo!()
    }

    /// SPEC-7.5.1: no Referer, at most 3 redirects.
    #[test]
    #[ignore = "M3: fetch"]
    fn spec_7_5_1_no_referer_max_redirects() {
        todo!()
    }
}
