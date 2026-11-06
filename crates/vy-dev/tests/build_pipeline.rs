//! The build pipeline on a scaffolded project: cargo for wasm32, the ABI
//! check, the draft against the module, a signed package written whole; a
//! broken source or draft is a failed outcome with the reason in the log
//! and no package, never a panic.

use std::path::{Path, PathBuf};
use vy_dev::{build_project, Project};
use vy_manifest::{Package, SigningKey};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

#[test]
fn build_pipeline_signs_a_valid_package_and_reports_failures() {
    let tmp = std::env::temp_dir().join(format!("vy-dev-pipeline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    let dir = tmp.join("rör-sida");
    vy_dev::scaffold::Scaffold {
        dir: dir.clone(),
        name: "ror-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    let mut project = Project::open(&dir).unwrap();
    project.target_dir = Some(root().join("target").join("vy-dev-tests"));
    let key = SigningKey::from_seed(&[9; 32]);
    let out = tmp.join("out").join("ror-sida.vyp");

    let mut lines = Vec::new();
    let outcome = build_project(&project, &key, &out, &mut |l| lines.push(l.to_string()));
    assert!(outcome.ok(), "{}", outcome.log);
    assert_eq!(outcome.first_error(), None);
    assert!(outcome.log.contains("ABI 1.0: OK"));
    assert!(outcome.log.ends_with('\n'));
    assert_eq!(
        lines.join("\n") + "\n",
        outcome.log,
        "progress saw every line"
    );
    let wasm = outcome.wasm_path.clone().unwrap();
    assert!(wasm.ends_with("wasm32-unknown-unknown/release/ror_sida.wasm"));
    let package = Package::unpack(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(Some(&package), outcome.package.as_ref());
    assert!(package.manifest.verify(&key.public()).is_some());
    assert!(package.validate().is_empty());
    assert_eq!(package.module, std::fs::read(&wasm).unwrap());
    // The frame check `vy-publish build` makes (SPEC-13.4.1) ran, quietly.
    assert!(outcome.log.contains("SPEC-13.4.1"), "{}", outcome.log);
    assert!(outcome.log.contains("frames: "), "{}", outcome.log);
    assert!(!outcome.log.contains("warning:"), "{}", outcome.log);

    // A source that does not compile.
    let lib = dir.join("src/lib.rs");
    let good = std::fs::read_to_string(&lib).unwrap();
    std::fs::write(&lib, good.replace("fn press(", "fn press(,")).unwrap();
    std::fs::remove_file(&out).unwrap();
    let broken = build_project(&project, &key, &out, &mut |_| {});
    assert!(!broken.ok());
    assert!(broken.log.contains("cargo build failed"), "{}", broken.log);
    assert!(broken.first_error().unwrap().starts_with("error"));
    assert!(!out.exists(), "no package from a failed build");

    // A draft that does not describe the module: memory_max below it.
    std::fs::write(&lib, &good).unwrap();
    let manifest = dir.join("manifest.json");
    let draft = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        draft.replace("\"memory_max\": 32", "\"memory_max\": 1"),
    )
    .unwrap();
    let bad_draft = build_project(&project, &key, &out, &mut |_| {});
    assert!(!bad_draft.ok());
    assert!(bad_draft.log.contains("memory_max"), "{}", bad_draft.log);
    assert!(!out.exists());

    // A frame over the 8 ms budget (review 0005, finding 3): the dev build
    // still serves the page, and the log warns that vy-publish refuses it.
    std::fs::write(&manifest, &draft).unwrap();
    let anchor = "let l = self.layout()?;\n        let x = l.card.x + PAD;";
    assert_eq!(good.matches(anchor).count(), 1, "the template's frame");
    std::fs::write(
        &lib,
        good.replace(
            anchor,
            &format!(
                "let mut spin = 0u32;\n        for i in 0..20_000_000u32 {{\n            \
                 spin = core::hint::black_box(spin ^ i);\n        }}\n        \
                 core::hint::black_box(spin);\n        {anchor}"
            ),
        ),
    )
    .unwrap();
    let slow = build_project(&project, &key, &out, &mut |_| {});
    assert!(slow.ok(), "{}", slow.log);
    assert!(out.exists(), "the slow page is still served");
    let warning = slow
        .log
        .lines()
        .find(|l| l.starts_with("warning:"))
        .unwrap_or_else(|| panic!("no warning in {}", slow.log));
    assert!(warning.contains("frame 1"), "{warning}");
    assert!(warning.contains("SPEC-13.4.1"), "{warning}");
    assert!(warning.contains("--allow-slow"), "{warning}");
    assert_eq!(slow.first_error(), None);
    let _ = std::fs::remove_dir_all(&tmp);
}
