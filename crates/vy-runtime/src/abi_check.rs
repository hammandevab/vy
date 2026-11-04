//! Static ABI validation of a module before it is compiled (SPEC §4.2, §5.1, §5.2).
//!
//! Used by `Runtime::load` and by `vy-cli check-abi`, so both report the same problems.

use crate::Error;
use std::collections::BTreeMap;

/// Name the module must export its linear memory under so the host can read
/// `(ptr, len)` arguments. See `docs/BUILD-NOTES.md` on SPEC-4.2.1.
pub const MEMORY_EXPORT: &str = "memory";

/// Result of validating a module against SPEC §4.2 and §5.
#[derive(Debug, Default)]
pub struct AbiReport {
    /// Human-readable problems; empty means the module conforms.
    pub problems: Vec<String>,
    /// Imports found, by name.
    pub imports: Vec<String>,
    /// Function exports found, by name.
    pub exports: Vec<String>,
    /// Declared memory limits in pages (min, max).
    pub memory: Option<(u64, Option<u64>)>,
}

impl AbiReport {
    /// True when no problems were found.
    pub fn ok(&self) -> bool {
        self.problems.is_empty()
    }
}

impl std::fmt::Display for AbiReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "exports: {}", self.exports.join(", "))?;
        writeln!(f, "imports: {}", self.imports.join(", "))?;
        match self.memory {
            Some((min, Some(max))) => writeln!(f, "memory: {min}..{max} pages")?,
            Some((min, None)) => writeln!(f, "memory: {min}.. pages (no max)")?,
            None => writeln!(f, "memory: none")?,
        }
        if self.problems.is_empty() {
            writeln!(f, "ABI 1.0: OK")
        } else {
            writeln!(f, "ABI 1.0: {} problem(s)", self.problems.len())?;
            for p in &self.problems {
                writeln!(f, "  - {p}")?;
            }
            Ok(())
        }
    }
}

fn ty(v: wasmparser::ValType) -> Option<vy_abi::Ty> {
    use wasmparser::ValType as V;
    match v {
        V::I32 => Some(vy_abi::Ty::I32),
        V::I64 => Some(vy_abi::Ty::I64),
        V::F32 => Some(vy_abi::Ty::F32),
        _ => None,
    }
}

fn sig_matches(ft: &wasmparser::FuncType, sig: &vy_abi::Sig) -> bool {
    let params: Option<Vec<_>> = ft.params().iter().map(|v| ty(*v)).collect();
    let results: Option<Vec<_>> = ft.results().iter().map(|v| ty(*v)).collect();
    match (params, results) {
        (Some(p), Some(r)) => p == sig.params && r.as_slice() == sig.result.as_slice(),
        _ => false,
    }
}

fn describe(ft: &wasmparser::FuncType) -> String {
    format!("{:?} -> {:?}", ft.params(), ft.results())
}

fn invalid(e: wasmparser::BinaryReaderError) -> Error {
    Error::Invalid(e.to_string())
}

/// Validate wasm bytes against the Vy ABI (SPEC-4.2.1, SPEC-4.2.2, SPEC-4.2.3, SPEC-5.1.*, SPEC-5.2.*).
///
/// `Err` means the bytes are not WebAssembly at all; a returned report may
/// still list problems.
pub fn check_abi_bytes(bytes: &[u8]) -> Result<AbiReport, Error> {
    use wasmparser::{ExternalKind, Parser, Payload, TypeRef};

    wasmparser::validate(bytes).map_err(invalid)?;

    let mut report = AbiReport::default();
    let mut types: Vec<wasmparser::FuncType> = Vec::new();
    // function index space: imported functions first, then defined
    let mut func_types: Vec<u32> = Vec::new();
    let mut exports: BTreeMap<String, u32> = BTreeMap::new();
    let mut memory_exported = false;

    for payload in Parser::new(0).parse_all(bytes) {
        match payload.map_err(invalid)? {
            Payload::TypeSection(reader) => {
                for t in reader.into_iter_err_on_gc_types() {
                    types.push(t.map_err(invalid)?);
                }
            }
            Payload::ImportSection(reader) => {
                for imp in reader {
                    let imp = imp.map_err(invalid)?;
                    match imp.ty {
                        TypeRef::Func(tidx) => {
                            func_types.push(tidx);
                            report.imports.push(imp.name.to_string());
                            if imp.module != vy_abi::IMPORT_MODULE {
                                let what = if imp.module.starts_with("wasi") {
                                    "WASI import (SPEC-4.2.3)"
                                } else {
                                    "import outside namespace `vy` (SPEC-4.2.2)"
                                };
                                report
                                    .problems
                                    .push(format!("{what}: {}::{}", imp.module, imp.name));
                                continue;
                            }
                            match vy_abi::import(imp.name) {
                                None => report
                                    .problems
                                    .push(format!("unknown import vy::{} (SPEC-5.2)", imp.name)),
                                Some(sig) => {
                                    let ft = &types[tidx as usize];
                                    if !sig_matches(ft, sig) {
                                        report.problems.push(format!(
                                            "import vy::{} has signature {} but SPEC-5.2 says {:?} -> {:?}",
                                            imp.name,
                                            describe(ft),
                                            sig.params,
                                            sig.result
                                        ));
                                    }
                                }
                            }
                        }
                        other => report.problems.push(format!(
                            "non-function import {}::{} ({other:?}) (SPEC-4.2.2)",
                            imp.module, imp.name
                        )),
                    }
                }
            }
            Payload::FunctionSection(reader) => {
                for t in reader {
                    func_types.push(t.map_err(invalid)?);
                }
            }
            Payload::MemorySection(reader) => {
                for m in reader {
                    let m = m.map_err(invalid)?;
                    report.memory = Some((m.initial, m.maximum));
                    if m.shared {
                        report
                            .problems
                            .push("shared memory requires threads (SPEC-4.2.3)".into());
                    }
                    if let Some(max) = m.maximum {
                        if max > u64::from(vy_abi::limits::MAX_MEMORY_PAGES) {
                            report.problems.push(format!(
                                "memory max {max} pages exceeds {} (SPEC-8.1.3)",
                                vy_abi::limits::MAX_MEMORY_PAGES
                            ));
                        }
                    }
                }
            }
            Payload::StartSection { .. } => {
                report.problems.push(
                    "module has a start function; the host enters a module only through the six exports (SPEC-5.3.4)"
                        .into(),
                );
            }
            Payload::ExportSection(reader) => {
                for e in reader {
                    let e = e.map_err(invalid)?;
                    match e.kind {
                        ExternalKind::Func => {
                            exports.insert(e.name.to_string(), e.index);
                        }
                        ExternalKind::Memory if e.name == MEMORY_EXPORT => memory_exported = true,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    if report.memory.is_none() {
        report
            .problems
            .push("module defines no linear memory".into());
    } else if !memory_exported {
        report.problems.push(format!(
            "linear memory is not exported as `{MEMORY_EXPORT}`; the host cannot read (ptr, len) arguments"
        ));
    }

    for sig in vy_abi::EXPORTS {
        match exports.get(sig.name) {
            None => report
                .problems
                .push(format!("missing export {} (SPEC-4.2.1)", sig.name)),
            Some(&idx) => {
                let tidx = func_types.get(idx as usize).copied().ok_or_else(|| {
                    Error::Invalid(format!("export {} has bad function index", sig.name))
                })?;
                let ft = &types[tidx as usize];
                if !sig_matches(ft, sig) {
                    report.problems.push(format!(
                        "export {} has signature {} but SPEC-5.1 says {:?} -> {:?}",
                        sig.name,
                        describe(ft),
                        sig.params,
                        sig.result
                    ));
                }
            }
        }
    }
    for name in exports.keys() {
        if vy_abi::export(name).is_none() {
            report
                .problems
                .push(format!("extra function export {name} (SPEC-4.2.1)"));
        }
    }
    report.exports = exports.keys().cloned().collect();
    if report.exports.is_empty() && report.imports.is_empty() {
        return Err(Error::Invalid(
            "module has neither imports nor exports".into(),
        ));
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"
        (module
          (import "vy" "dl_begin" (func))
          (import "vy" "dl_rect" (func (param f32 f32 f32 f32 f32 i32)))
          (memory (export "memory") 1 16)
          (func (export "vy_init") (param i32 i32 i32 f32) (result i32) i32.const 0)
          (func (export "vy_frame") (param i32) (result i32) i32.const 0)
          (func (export "vy_event") (param i32 i32 i32 i32 i32) (result i32) i32.const 0)
          (func (export "vy_resize") (param i32 i32 f32))
          (func (export "vy_suspend") (result i32) i32.const 0)
          (func (export "vy_alloc") (param i32) (result i32) i32.const 0))
    "#;

    fn wasm(wat: &str) -> Vec<u8> {
        wat::parse_str(wat).unwrap()
    }

    /// SPEC-4.2.1, SPEC-5.1.1..5.1.6: a module with exactly the six exports passes.
    #[test]
    fn spec_4_2_1_conforming_module_passes() {
        let r = check_abi_bytes(&wasm(GOOD)).unwrap();
        assert!(r.ok(), "{r}");
        assert_eq!(r.exports.len(), 6);
    }

    /// SPEC-4.2.1: a missing export is reported.
    #[test]
    fn spec_4_2_1_missing_export_fails() {
        let wat = GOOD.replace(
            r#"(func (export "vy_alloc") (param i32) (result i32) i32.const 0)"#,
            "",
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems
                .iter()
                .any(|p| p.contains("missing export vy_alloc")),
            "{r}"
        );
    }

    /// SPEC-4.2.1: an extra function export is reported.
    #[test]
    fn spec_4_2_1_extra_export_fails() {
        let wat = GOOD.replace("(memory", r#"(func (export "extra")) (memory"#);
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems
                .iter()
                .any(|p| p.contains("extra function export extra")),
            "{r}"
        );
    }

    /// SPEC-5.3.4: a start function is an entry point outside the six exports.
    #[test]
    fn spec_5_3_4_start_function_rejected() {
        let wat = GOOD.replace("(memory", "(func $s) (start $s) (memory");
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems.iter().any(|p| p.contains("start function")),
            "{r}"
        );
    }

    /// SPEC-4.2.1: memory must be exported as `memory` so the host can read arguments.
    #[test]
    fn spec_4_2_1_memory_export_required() {
        let wat = GOOD.replace(r#"(memory (export "memory") 1 16)"#, "(memory 1 16)");
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(r.problems.iter().any(|p| p.contains("not exported")), "{r}");
    }

    /// SPEC-5.1.2: a wrong export signature is reported.
    #[test]
    fn spec_5_1_2_wrong_signature_fails() {
        let wat = GOOD.replace(
            r#"(func (export "vy_frame") (param i32) (result i32) i32.const 0)"#,
            r#"(func (export "vy_frame") (param i64) (result i32) i32.const 0)"#,
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems
                .iter()
                .any(|p| p.starts_with("export vy_frame has signature")),
            "{r}"
        );
    }

    /// SPEC-4.2.2: imports outside namespace `vy` are reported.
    #[test]
    fn spec_4_2_2_foreign_namespace_fails() {
        let wat = GOOD.replace(
            r#"(import "vy" "dl_begin" (func))"#,
            r#"(import "env" "dl_begin" (func))"#,
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems.iter().any(|p| p.contains("outside namespace")),
            "{r}"
        );
    }

    /// SPEC-4.2.3: WASI imports are reported.
    #[test]
    fn spec_4_2_3_wasi_import_fails() {
        let wat = GOOD.replace(
            r#"(import "vy" "dl_begin" (func))"#,
            r#"(import "wasi_snapshot_preview1" "fd_write" (func (param i32 i32 i32 i32) (result i32)))"#,
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(r.problems.iter().any(|p| p.contains("WASI import")), "{r}");
    }

    /// SPEC-5.2.3: an import with the wrong signature is reported.
    #[test]
    fn spec_5_2_3_wrong_import_signature_fails() {
        let wat = GOOD.replace(
            r#"(import "vy" "dl_rect" (func (param f32 f32 f32 f32 f32 i32)))"#,
            r#"(import "vy" "dl_rect" (func (param f32 f32 f32 f32 i32)))"#,
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(
            r.problems
                .iter()
                .any(|p| p.starts_with("import vy::dl_rect has signature")),
            "{r}"
        );
    }

    /// SPEC-8.1.3: memory max above 4096 pages is reported.
    #[test]
    fn spec_8_1_3_memory_max_limit() {
        let wat = GOOD.replace(
            "(memory (export \"memory\") 1 16)",
            "(memory (export \"memory\") 1 5000)",
        );
        let r = check_abi_bytes(&wasm(&wat)).unwrap();
        assert!(r.problems.iter().any(|p| p.contains("exceeds 4096")), "{r}");
    }

    /// SPEC-2.1.6: bytes that are not WebAssembly are rejected, never interpreted.
    #[test]
    fn spec_2_1_6_non_wasm_rejected() {
        assert!(matches!(check_abi_bytes(b"<html>"), Err(Error::Invalid(_))));
    }
}
