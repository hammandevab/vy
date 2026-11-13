//! Frames after input keep the 8 ms budget (SPEC-5.3.3): pages driven on
//! `vy-runtime` with the text that broke it. Neither the SDK's mock (it
//! counts no fuel) nor `vy-publish build` (it sends no events) sees a frame
//! like these.

use std::fs;
use std::path::{Path, PathBuf};
use vy_abi::{button, event as kind, modifier, role};
use vy_manifest::Draft;
use vy_runtime::{Capabilities, Options, Runtime, SemNode, FRAME_BUDGET_FUEL};

/// USB HID usage ids of the keys the fields take (SPEC-A.2.1).
mod key {
    pub const A: i32 = 0x04;
    pub const BACKSPACE: i32 = 0x2A;
    pub const HOME: i32 = 0x4A;
    pub const END: i32 = 0x4D;
    pub const DOWN: i32 = 0x51;
    pub const UP: i32 = 0x52;
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

/// A reference module running as its conformance cases run it: the
/// draft's memory cap, every capability the draft requests granted, the
/// headless clock.
struct Page {
    name: &'static str,
    rt: Runtime,
    /// Fuel of the costliest frame so far, and what led to it.
    worst: (u64, String),
}

impl Page {
    fn load(name: &'static str, width: u32, height: u32) -> Self {
        let root = root();
        let path = root.join(format!("target/wasm/{name}.wasm"));
        let wasm = fs::read(&path)
            .unwrap_or_else(|e| panic!("{}: {e}\nrun tools/build_modules.sh", path.display()));
        let draft =
            fs::read_to_string(root.join(format!("conformance/manifests/{name}.json"))).unwrap();
        let draft = Draft::from_json(&draft).unwrap();
        let granted = vy_abi::cap::NAMES
            .into_iter()
            .filter(|n| draft.capabilities.requests(n));
        let options = Options {
            memory_max_pages: draft.memory_max,
            capabilities: Capabilities::from_names(granted).unwrap(),
            clipboard: Some(Vec::new()),
            ..Options::default()
        };
        let mut rt = Runtime::load(&wasm, &vy_runtime::sha256(&wasm), options).unwrap();
        rt.init(width, height, 1.0).unwrap();
        let mut page = Self {
            name,
            rt,
            worst: (0, String::new()),
        };
        page.frames("init");
        page
    }

    /// Run the frames the page asks for (at most eight: some pages
    /// animate) and hold each to the budget.
    fn frames(&mut self, after: &str) {
        let mut ran = 0;
        while self.rt.wants_frame() && ran < 8 {
            self.rt
                .frame()
                .unwrap_or_else(|e| panic!("{}: the frame after {after}: {e}", self.name));
            ran += 1;
            let fuel = self.rt.last_frame().fuel;
            if fuel > self.worst.0 {
                self.worst = (fuel, after.to_string());
            }
            assert!(
                fuel <= FRAME_BUDGET_FUEL,
                "{}: the frame after {after} used {fuel} fuel, over the budget of {FRAME_BUDGET_FUEL} (SPEC-5.3.3)",
                self.name
            );
        }
        assert!(
            !self.rt.is_suspended(),
            "{}: suspended after {after}",
            self.name
        );
    }

    fn event(&mut self, what: &str, kind: i32, a: i32, b: i32, c: i32, d: i32) {
        self.rt
            .event(kind, a, b, c, d)
            .unwrap_or_else(|e| panic!("{}: {what}: {e}", self.name));
        self.frames(what);
    }

    /// A primary click at (`x`, `y`), logical pixels.
    fn click(&mut self, x: f32, y: f32) {
        let (a, b) = ((x * 100.0).round() as i32, (y * 100.0).round() as i32);
        let what = format!("a press at {x}, {y}");
        self.event(&what, kind::POINTER_DOWN, a, b, button::PRIMARY, 0);
        self.event(&what, kind::POINTER_UP, a, b, button::PRIMARY, 0);
    }

    fn key(&mut self, code: i32, modifiers: i32) {
        let what = format!("key {code:#x} with modifiers {modifiers}");
        self.event(&what, kind::KEY_DOWN, code, modifiers, 0, 0);
        self.event(&what, kind::KEY_UP, code, modifiers, 0, 0);
    }

    fn text(&mut self, text: &str) {
        let what = format!("text_input of {} bytes", text.len());
        self.rt
            .text_input(text)
            .unwrap_or_else(|e| panic!("{}: {what}: {e}", self.name));
        self.frames(&what);
    }

    fn resize(&mut self, width: u32, height: u32) {
        let what = format!("a resize to {width} x {height}");
        self.rt
            .resize(width, height, 1.0)
            .unwrap_or_else(|e| panic!("{}: {what}: {e}", self.name));
        self.frames(&what);
    }

    /// The largest `textbox` node of the last frame.
    fn largest_textbox(&self) -> SemNode {
        self.rt
            .semantic_tree()
            .iter()
            .filter(|n| n.role == role::TEXTBOX)
            .max_by(|a, b| (a.w * a.h).total_cmp(&(b.w * b.h)))
            .unwrap_or_else(|| panic!("{}: no textbox node", self.name))
            .clone()
    }

    /// Click in the middle of the largest text field, which gives it the
    /// host's focus (SPEC-10.3.1).
    fn focus_largest_textbox(&mut self) -> i32 {
        let n = self.largest_textbox();
        self.click(n.x + n.w / 2.0, n.y + n.h / 2.0);
        assert_eq!(
            self.rt.focus(),
            Some(n.id),
            "{}: the field has focus",
            self.name
        );
        n.id
    }

    fn textbox(&self, id: i32) -> SemNode {
        self.rt
            .semantic_tree()
            .iter()
            .find(|n| n.id == id)
            .unwrap_or_else(|| panic!("{}: node {id} is gone", self.name))
            .clone()
    }

    /// Print the costliest frame (`cargo test -- --nocapture` shows it).
    fn report(&self, case: &str) {
        eprintln!(
            "{} {case}: costliest frame {} fuel ({:.2} ms), after {}",
            self.name,
            self.worst.0,
            self.worst.0 as f64 / vy_runtime::FUEL_PER_MS as f64,
            self.worst.1
        );
    }
}

/// Paste `text` into the largest text field of `widgets` (the multi-line
/// one), then resize twice, press and drag in the field and move the caret
/// through it: every frame within the budget.
fn widgets_with(text: &str, case: &str) {
    let mut page = Page::load("widgets", 800, 600);
    let id = page.focus_largest_textbox();
    page.text(text);
    let shown = page.textbox(id);
    assert_eq!(shown.text, text, "the field took the text");
    page.resize(1280, 800);
    page.resize(400, 700);
    // A press in the field finds the caret's line and offset, a drag
    // selects, and the keys move through the wrapped lines.
    let n = page.largest_textbox();
    page.click(n.x + n.w / 2.0, n.y + n.h / 2.0);
    let (x, y) = (
        ((n.x + n.w / 2.0) * 100.0) as i32,
        ((n.y + 30.0) * 100.0) as i32,
    );
    page.event("a press", kind::POINTER_DOWN, x, y, button::PRIMARY, 0);
    page.event(
        "a drag",
        kind::POINTER_MOVE,
        x + 4000,
        y + 4000,
        button::mask(button::PRIMARY),
        0,
    );
    page.event(
        "a release",
        kind::POINTER_UP,
        x + 4000,
        y + 4000,
        button::PRIMARY,
        0,
    );
    for code in [key::UP, key::DOWN, key::HOME, key::END] {
        page.key(code, 0);
    }
    page.key(key::HOME, modifier::CTRL);
    page.key(key::END, modifier::CTRL);
    page.text("!");
    page.key(key::BACKSPACE, 0);
    page.resize(800, 600);
    page.report(case);
}

/// SPEC-5.3.3, SPEC-6.4.3 (granskning 0010:1): a word of 4 096 bytes
/// without a space, pasted into a multi-line `ui::TextBox`, wraps within
/// the frame budget, also after the window changed size. Breaking it used
/// to measure the whole rest of the word for every line: 63.9 M fuel for
/// the frame after the paste.
#[test]
fn spec_5_3_3_long_word_in_a_multiline_field_keeps_the_budget() {
    widgets_with(&"W".repeat(4096), "4 096 x W");
}

/// SPEC-5.3.3, SPEC-6.4.3 (granskning 0010:1): the same for 1 024
/// characters of four bytes each (13.0 M before).
#[test]
fn spec_5_3_3_long_word_of_four_byte_characters_keeps_the_budget() {
    let text = "\u{10348}".repeat(1024);
    assert_eq!(text.len(), 4096);
    widgets_with(&text, "1 024 four-byte characters");
}

/// SPEC-5.3.3, SPEC-6.4.3 (granskning 0010:1, 0013): `skriv` takes 6 000
/// bytes; one unbroken word of that length in its editor, in the split
/// view, in the narrow view's editor and in its preview, at every size.
#[test]
fn spec_5_3_3_long_word_in_skriv_keeps_the_budget() {
    let mut page = Page::load("skriv", 800, 600);
    let id = page.focus_largest_textbox();
    page.key(key::A, modifier::CTRL);
    let word = "W".repeat(6000);
    page.text(&word);
    assert_eq!(
        page.textbox(id).text.len(),
        4096,
        "the node holds 4 KB of it"
    );
    page.resize(1280, 800);
    // Below 720 px the editor and the preview are behind tabs.
    page.resize(400, 700);
    page.key(key::HOME, modifier::CTRL);
    page.text("x");
    let tabs: Vec<SemNode> = page
        .rt
        .semantic_tree()
        .iter()
        .filter(|n| n.role == role::TAB)
        .cloned()
        .collect();
    assert_eq!(tabs.len(), 2, "Skriv and Läs");
    page.click(tabs[1].x + tabs[1].w / 2.0, tabs[1].y + tabs[1].h / 2.0);
    page.resize(360, 640);
    page.click(tabs[0].x + tabs[0].w / 2.0, tabs[0].y + tabs[0].h / 2.0);
    page.resize(800, 600);
    page.report("6 000 x W");
}
