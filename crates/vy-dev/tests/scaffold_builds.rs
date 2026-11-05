//! `vy-dev new` writes a project that builds to wasm here, passes the ABI
//! check `vy-cli check-abi` runs, renders a first frame with a semantic
//! tree, has a manifest draft that describes the module, is rustfmt-clean
//! and passes its own native test.
//!
//! The project lives in a temporary directory; cargo builds it into
//! `target/vy-dev-tests` of this workspace so repeated runs (and other
//! tests that build a scaffold) share one build of `vy-sdk`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The shared target directory for scaffolded test projects.
fn target_dir() -> PathBuf {
    root().join("target").join("vy-dev-tests")
}

fn cargo(dir: &Path, args: &[&str]) -> std::process::Output {
    Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(args)
        .current_dir(dir)
        .env("CARGO_TARGET_DIR", target_dir())
        .env("CARGO_TERM_COLOR", "never")
        .output()
        .expect("cargo runs")
}

#[test]
fn scaffold_builds_to_wasm_and_passes_check_abi() {
    let dir = std::env::temp_dir().join(format!("vy-dev-scaffold-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join("prov-sida");
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: "prov-sida".into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();

    let out = cargo(
        &project,
        &["build", "--release", "--target", "wasm32-unknown-unknown"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "cargo build failed:\n{stderr}");
    assert!(
        !stderr.contains("warning"),
        "the scaffold builds without warnings:\n{stderr}"
    );

    let wasm_path = target_dir().join("wasm32-unknown-unknown/release/prov_sida.wasm");
    let wasm = std::fs::read(&wasm_path).unwrap();
    let report = vy_runtime::check_abi_bytes(&wasm).unwrap();
    assert!(report.ok(), "{report}");
    assert!(report.to_string().ends_with("ABI 1.0: OK\n"));

    // The draft describes the module (SPEC-8.1.3: memory_max holds it).
    let draft = std::fs::read_to_string(project.join("manifest.json")).unwrap();
    let draft = vy_manifest::Draft::from_json(&draft).unwrap();
    let manifest = draft.finish(&wasm, &[]);
    let problems = manifest.validate(&wasm, &[] as &[Vec<u8>]);
    assert!(problems.is_empty(), "{problems:?}");

    // A first frame with the tree the template promises.
    let options = vy_runtime::Options {
        memory_max_pages: manifest.memory_max,
        ..vy_runtime::Options::default()
    };
    let mut rt = vy_runtime::Runtime::load(&wasm, &vy_runtime::sha256(&wasm), options).unwrap();
    rt.init(800, 600, 1.0).unwrap();
    assert!(!rt.frame().unwrap().is_empty());
    let roles: Vec<i32> = rt.semantic_tree().iter().map(|n| n.role).collect();
    assert_eq!(
        roles.len(),
        5,
        "main, heading, two texts, button: {roles:?}"
    );

    // rustfmt-clean, so `cargo fmt --check` in a new project is quiet.
    let fmt = Command::new("rustfmt")
        .args(["--edition", "2021", "--check"])
        .arg(project.join("src/lib.rs"))
        .output()
        .expect("rustfmt runs");
    assert!(
        fmt.status.success(),
        "the template is not rustfmt-clean:\n{}",
        String::from_utf8_lossy(&fmt.stdout)
    );

    // Its own native test, against the SDK mock.
    let test = cargo(&project, &["test", "--quiet"]);
    assert!(
        test.status.success(),
        "cargo test in the scaffold failed:\n{}{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}
