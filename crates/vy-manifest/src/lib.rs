//! Manifest: CBOR schema, validation and capability policy (SPEC §4, §8).
//!
//! M1 implements parsing and validation; M4 adds signatures (SPEC-7.4.*).
#![deny(missing_docs)]

/// Capabilities defined in 1.0 (SPEC-8.2.1).
pub const CAPABILITIES: &[&str] = &[
    "net",
    "storage",
    "clipboard",
    "hires_time",
    "ime",
    "persist_frame",
];

/// Maximum manifest size in bytes (SPEC-4.1.3).
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// Maximum uncompressed module size in bytes (SPEC-4.1.4).
pub const MAX_MODULE_BYTES: usize = 8 * 1024 * 1024;
/// Maximum total resource size in bytes (SPEC-4.1.5).
pub const MAX_RESOURCES_BYTES: usize = 32 * 1024 * 1024;

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC-8.2.1: the six capabilities of 1.0.
    #[test]
    fn spec_8_2_1_capability_list() {
        assert_eq!(CAPABILITIES.len(), 6);
        for c in [
            "net",
            "storage",
            "clipboard",
            "hires_time",
            "ime",
            "persist_frame",
        ] {
            assert!(CAPABILITIES.contains(&c));
        }
    }

    /// SPEC-4.1.3, SPEC-4.1.4, SPEC-4.1.5: size limits.
    #[test]
    fn spec_4_1_3_size_limits() {
        assert_eq!(MAX_MANIFEST_BYTES, 65_536);
        assert_eq!(MAX_MODULE_BYTES, 8_388_608);
        assert_eq!(MAX_RESOURCES_BYTES, 33_554_432);
    }

    /// SPEC-4.1.1: a package is one manifest, exactly one module, zero or more resources.
    #[test]
    #[ignore = "M1: manifest parser"]
    fn spec_4_1_1_package_shape() {
        todo!()
    }

    /// SPEC-4.1.2, SPEC-2.1.2: package identity is SHA-256 of canonical CBOR.
    #[test]
    #[ignore = "M1: canonical CBOR + hash"]
    fn spec_4_1_2_identity_is_hash_of_canonical_cbor() {
        todo!()
    }

    /// SPEC-4.1.3: non-canonical CBOR or > 64 KB is rejected.
    #[test]
    #[ignore = "M1: canonical CBOR check"]
    fn spec_4_1_3_rejects_non_canonical() {
        todo!()
    }

    /// SPEC-4.3.1: unknown ABI major version is rejected before loading.
    #[test]
    #[ignore = "M1"]
    fn spec_4_3_1_unknown_major_rejected() {
        todo!()
    }

    /// SPEC-4.3.2: a 1.y module runs on a 1.x client when y <= x.
    #[test]
    #[ignore = "M1"]
    fn spec_4_3_2_minor_compat() {
        todo!()
    }

    /// SPEC-4.4.1: package media type and zstd+tar framing.
    #[test]
    #[ignore = "M3: package unpack"]
    fn spec_4_4_1_package_framing() {
        todo!()
    }

    /// SPEC-4.5.1: resource index 0 is the module.
    #[test]
    #[ignore = "M1"]
    fn spec_4_5_1_resource_index_zero_is_module() {
        todo!()
    }

    /// SPEC-8.1.1: required fields are enforced.
    #[test]
    #[ignore = "M1"]
    fn spec_8_1_1_required_fields() {
        todo!()
    }

    /// SPEC-8.1.2: optional fields are accepted and unknown fields rejected.
    #[test]
    #[ignore = "M1"]
    fn spec_8_1_2_optional_fields() {
        todo!()
    }

    /// SPEC-8.2.2: a capability absent from the manifest can never be granted.
    #[test]
    #[ignore = "M1: policy"]
    fn spec_8_2_2_no_late_grants() {
        todo!()
    }

    /// SPEC-8.2.3: default grant policy per capability.
    #[test]
    #[ignore = "M1: policy"]
    fn spec_8_2_3_default_policy() {
        todo!()
    }

    /// SPEC-8.3.1: fallback is shown when the module is rejected or the window is under min_size.
    #[test]
    #[ignore = "M3"]
    fn spec_8_3_1_fallback_conditions() {
        todo!()
    }
}
