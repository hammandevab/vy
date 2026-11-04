//! Runtime: wasmtime instance per tab, ABI imports, fuel budget, memory cap (SPEC §5, §6.2, §9).
//!
//! M1 implements the display list and text imports headless; M3 adds network and storage.
//! The client contains no interpreter for any language (SPEC-2.1.6): modules are
//! precompiled WebAssembly (SPEC-4.1.4) executed by wasmtime.
#![deny(missing_docs)]

#[cfg(test)]
mod tests {
    /// SPEC-5.2.2, SPEC-6.2.1: vy_frame runs only after request_frame, an event returning 1, or a resize.
    #[test]
    #[ignore = "M1: runtime"]
    fn spec_6_2_1_frame_only_on_request() {
        todo!()
    }

    /// SPEC-6.2.2: an idle page consumes 0 module CPU.
    #[test]
    #[ignore = "M1: runtime"]
    fn spec_6_2_2_idle_is_free() {
        todo!()
    }

    /// SPEC-5.3.1, SPEC-2.1.4: a call without its capability returns E_DENIED and does not kill the module.
    #[test]
    #[ignore = "M1: capability gating"]
    fn spec_5_3_1_denied_call_is_harmless() {
        todo!()
    }

    /// SPEC-5.2.8: fetch_begin to an unlisted origin returns E_ORIGIN with no network call.
    #[test]
    #[ignore = "M3: network"]
    fn spec_5_2_8_origin_gate() {
        todo!()
    }

    /// SPEC-5.3.3: two frames over 8 ms in a row yield KIND_BUDGET then a forced suspend.
    #[test]
    #[ignore = "M1: fuel budget"]
    fn spec_5_3_3_frame_budget() {
        todo!()
    }

    /// SPEC-5.3.4: the host never calls into the module outside the six exports.
    #[test]
    #[ignore = "M1: runtime"]
    fn spec_5_3_4_no_callbacks() {
        todo!()
    }

    /// SPEC-2.1.5: same module, same input, same window gives the same bytes.
    #[test]
    #[ignore = "M1: determinism"]
    fn spec_2_1_5_determinism() {
        todo!()
    }

    /// SPEC-4.2.4: memory may not grow past memory_max.
    #[test]
    #[ignore = "M1: memory cap"]
    fn spec_4_2_4_memory_cap() {
        todo!()
    }

    /// SPEC-9.3.1: resource limits enforced by the host.
    #[test]
    #[ignore = "M3: limits"]
    fn spec_9_3_1_resource_limits() {
        todo!()
    }

    /// SPEC-9.4.1: a module whose hash mismatches is never run.
    #[test]
    #[ignore = "M1: hash check"]
    fn spec_9_4_1_hash_mismatch_never_runs() {
        todo!()
    }

    /// SPEC-9.5.1, SPEC-9.5.2: storage and state are per origin and never shared.
    #[test]
    #[ignore = "M3: storage"]
    fn spec_9_5_1_per_origin_isolation() {
        todo!()
    }

    /// SPEC-10.1.1, SPEC-10.1.2: semantic tree built via sem_node with valid roles only.
    #[test]
    #[ignore = "M1: semantic tree"]
    fn spec_10_1_1_semantic_tree() {
        todo!()
    }

    /// SPEC-5.2.11: now_ms is monotonic with 1 ms resolution unless hires_time.
    #[test]
    #[ignore = "M1: time"]
    fn spec_5_2_11_time_resolution() {
        todo!()
    }
}
