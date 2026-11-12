//! The main process's side of the compilation cache (SPEC-13.1.4,
//! SPEC-9.4.2).

#[cfg(test)]
mod tests {
    /// SPEC-9.4.2: compiled code is kept by the main process alone, under
    /// a key it makes itself from the module bytes it gave the tab; a tab
    /// process can hand over code only for the module it was loaded with,
    /// once per load, and kept code runs only in a tab process loaded
    /// with the same module.
    #[test]
    #[ignore = "M5: the compilation cache in the shell"]
    fn spec_9_4_2_code_is_kept_under_the_main_process_key() {
        unimplemented!("the compilation cache in the shell (SPEC-9.4.2)");
    }
}
