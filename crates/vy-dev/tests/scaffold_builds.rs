//! `vy-dev new` writes a project that builds to wasm here, passes the ABI
//! check `vy-cli check-abi` runs, renders a first frame with a semantic
//! tree, has a manifest draft that describes the module, is rustfmt-clean
//! and passes its own native test. The starter's `ui::TextBox` and
//! `ui::Button` work in the real runtime, and so does the whole page in
//! `docs/SDK.md`, which a project built from the guide uses.
//!
//! The projects live in a temporary directory; cargo builds them into
//! `target/vy-dev-tests` of this workspace so repeated runs (and other
//! tests that build a scaffold) share one build of `vy-sdk`.

use std::path::{Path, PathBuf};
use std::process::Command;
use vy_abi::{event, key};

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

/// Scaffold `name` in a fresh temporary directory, on the SDK of this
/// checkout; returns (temporary directory, project).
fn scaffold(name: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("vy-dev-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let project = dir.join(name);
    vy_dev::scaffold::Scaffold {
        dir: project.clone(),
        name: name.into(),
        sdk: Some(root()),
    }
    .create()
    .unwrap();
    (dir, project)
}

/// Build the project for wasm32 without warnings; returns the module.
fn build_wasm(project: &Path, lib: &str) -> Vec<u8> {
    let out = cargo(
        project,
        &["build", "--release", "--target", "wasm32-unknown-unknown"],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "cargo build failed:\n{stderr}");
    assert!(
        !stderr.contains("warning"),
        "the project builds without warnings:\n{stderr}"
    );
    let wasm_path = target_dir().join(format!("wasm32-unknown-unknown/release/{lib}.wasm"));
    let wasm = std::fs::read(&wasm_path).unwrap();
    let report = vy_runtime::check_abi_bytes(&wasm).unwrap();
    assert!(report.ok(), "{report}");
    assert!(report.to_string().ends_with("ABI 1.0: OK\n"));
    wasm
}

/// The project's draft describes the module (SPEC-8.1.3: memory_max holds
/// it), requests `ime` for its text field (SPEC-10.4.2), and the module
/// starts under it with `ime` granted.
fn start(project: &Path, wasm: &[u8]) -> vy_runtime::Runtime {
    let draft = std::fs::read_to_string(project.join("manifest.json")).unwrap();
    let draft = vy_manifest::Draft::from_json(&draft).unwrap();
    assert!(draft.capabilities.ime.is_some(), "a text field needs ime");
    let manifest = draft.finish(wasm, &[]);
    let problems = manifest.validate(wasm, &[] as &[Vec<u8>]);
    assert!(problems.is_empty(), "{problems:?}");
    let options = vy_runtime::Options {
        memory_max_pages: manifest.memory_max,
        capabilities: vy_runtime::Capabilities::from_names(["ime"]).unwrap(),
        ..vy_runtime::Options::default()
    };
    let mut rt = vy_runtime::Runtime::load(wasm, &vy_runtime::sha256(wasm), options).unwrap();
    rt.init(800, 600, 1.0).unwrap();
    assert!(!rt.frame().unwrap().is_empty());
    rt
}

/// Tab to the first focusable node (the text field), type `text`, press
/// Enter and draw: what the tree says afterwards.
fn type_and_enter(rt: &mut vy_runtime::Runtime, text: &str) -> Vec<(i32, String)> {
    rt.event(event::KEY_DOWN, key::TAB, 0, 0, 0).unwrap();
    assert_eq!(rt.focus(), Some(-1), "Tab focuses the field");
    assert!(rt.ime_rect().is_some(), "the field opened composition");
    assert!(rt.text_input(text).unwrap());
    assert!(rt.event(event::KEY_DOWN, key::ENTER, 0, 0, 0).unwrap());
    rt.frame().unwrap();
    rt.semantic_tree()
        .iter()
        .map(|n| (n.role, n.text.clone()))
        .collect()
}

/// The project's own native test, against the SDK mock.
fn native_tests_pass(project: &Path) {
    let test = cargo(project, &["test", "--quiet"]);
    assert!(
        test.status.success(),
        "cargo test in the project failed:\n{}{}",
        String::from_utf8_lossy(&test.stdout),
        String::from_utf8_lossy(&test.stderr)
    );
}

#[test]
fn scaffold_builds_to_wasm_and_passes_check_abi() {
    let (dir, project) = scaffold("prov-sida");
    let wasm = build_wasm(&project, "prov_sida");
    let mut rt = start(&project, &wasm);

    // A first frame with the tree the template promises.
    let roles: Vec<i32> = rt.semantic_tree().iter().map(|n| n.role).collect();
    assert_eq!(
        roles,
        [15, 2, 1, 1, 5, 4, 1],
        "main, heading, text, label, textbox, button, greeting"
    );
    let ids: Vec<i32> = rt.semantic_tree().iter().map(|n| n.id).collect();
    assert_eq!((ids[4], ids[5]), (-1, -2), "fixed ids for the controls");

    // The starter's ui::TextBox and ui::Button in the real runtime.
    let tree = type_and_enter(&mut rt, "Anna");
    assert!(tree.contains(&(5, "Anna".into())), "{tree:?}");
    assert!(tree.contains(&(1, "Hej, Anna!".into())), "{tree:?}");

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

    native_tests_pass(&project);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The body of the first fenced block of `lang` after `marker` in `text`.
fn fenced_after(text: &str, marker: &str, lang: &str) -> String {
    let at = text
        .find(marker)
        .unwrap_or_else(|| panic!("{marker} not in docs/SDK.md"));
    let fence = format!("```{lang}\n");
    let start = at + text[at..].find(&fence).expect("a fenced block") + fence.len();
    let end = start + text[start..].find("\n```").expect("a closing fence");
    text[start..=end].to_string()
}

/// The whole page of docs/SDK.md (section 8, "En hel sida från början"),
/// with the manifest of section 4, builds to wasm without warnings, passes
/// the ABI check, starts, adds a task through its `ui::TextBox` and
/// `ui::List` in the real runtime, and passes its own native test: the
/// guide an outside agent reads cannot go stale.
#[test]
fn sdk_guide_example_builds() {
    let guide = std::fs::read_to_string(root().join("docs/SDK.md")).unwrap();
    let code = fenced_after(&guide, "<!-- exempel: src/lib.rs -->", "rust");
    let manifest = fenced_after(&guide, "## 4. Capabilities och manifestet", "json");
    let (dir, project) = scaffold("att-gora");
    std::fs::write(project.join("src/lib.rs"), code).unwrap();
    std::fs::write(project.join("manifest.json"), manifest).unwrap();
    let wasm = build_wasm(&project, "att_gora");
    let mut rt = start(&project, &wasm);
    let tree = type_and_enter(&mut rt, "Handla");
    assert!(tree.contains(&(9, "Handla".into())), "a listitem: {tree:?}");
    assert!(
        tree.contains(&(1, "1 av högst 100 uppgifter".into())),
        "{tree:?}"
    );
    native_tests_pass(&project);
    let _ = std::fs::remove_dir_all(&dir);
}
