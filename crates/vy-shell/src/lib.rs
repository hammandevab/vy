//! Shell: window, tabs, address bar, history, capability dialogs (SPEC §9, §11).
//!
//! Not touched before M3 (see CLAUDE.md). Headless work lives in `vy-cli`.
//! The client has no extension system (SPEC-11.5.1) and never renders HTML (SPEC-1.1.1).
#![deny(missing_docs)]

#[cfg(test)]
mod tests {
    /// SPEC-1.1.2, SPEC-11.1.1: crate boundaries match the spec; shell depends on no module code.
    #[test]
    #[ignore = "M3: shell"]
    fn spec_11_1_1_crate_boundaries() {
        todo!()
    }

    /// SPEC-2.1.7: https:// links open in the system browser.
    #[test]
    #[ignore = "M3: shell"]
    fn spec_2_1_7_https_opens_externally() {
        todo!()
    }

    /// SPEC-9.1.1, SPEC-9.1.2: one OS process per tab; GPU only in the main process.
    #[test]
    #[ignore = "M3: process model"]
    fn spec_9_1_1_process_isolation() {
        todo!()
    }

    /// SPEC-9.2.1: granted capabilities are visible and individually revocable.
    #[test]
    #[ignore = "M3: shell"]
    fn spec_9_2_1_capability_ui() {
        todo!()
    }

    /// SPEC-9.6.1: the address bar is always drawn by the client; modules cannot draw outside their area.
    #[test]
    #[ignore = "M3: shell"]
    fn spec_9_6_1_chrome_owned_by_client() {
        todo!()
    }

    /// SPEC-9.7.1: reproducible builds publish hash and recipe.
    #[test]
    #[ignore = "M5: release"]
    fn spec_9_7_1_reproducible_build() {
        todo!()
    }

    /// SPEC-10.2.1: semantic tree exposed through AccessKit.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_2_1_accesskit() {
        todo!()
    }

    /// SPEC-10.2.2: text selection copies logical strings.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_2_2_text_selection() {
        todo!()
    }

    /// SPEC-10.3.1: focus order from the tree; Tab/Shift+Tab/Enter/Escape as KIND_FOCUS.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_3_1_focus_order() {
        todo!()
    }

    /// SPEC-10.4.1: a page without a semantic tree is marked inaccessible.
    #[test]
    #[ignore = "M4: accessibility"]
    fn spec_10_4_1_inaccessible_marker() {
        todo!()
    }

    /// SPEC-11.2.1, SPEC-11.2.2: display lists over shared-memory ring buffers; tab crash is contained.
    #[test]
    #[ignore = "M3: process model"]
    fn spec_11_2_1_ring_buffer_and_crash_containment() {
        todo!()
    }

    /// SPEC-11.4.1: record and replay a session exactly.
    #[test]
    #[ignore = "M4: devtools"]
    fn spec_11_4_1_record_replay() {
        todo!()
    }

    /// SPEC-13.1.1: cold start to empty window <= 150 ms.
    #[test]
    #[ignore = "M3: bench"]
    fn spec_13_1_1_cold_start() {
        todo!()
    }

    /// SPEC-13.3.1, SPEC-13.3.2: memory with 10 tabs <= 400 MB; hidden tab 0 % CPU.
    #[test]
    #[ignore = "M3: bench"]
    fn spec_13_3_1_memory_and_background() {
        todo!()
    }
}
