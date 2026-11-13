//! Frames after input keep the 8 ms budget (SPEC-5.3.3): pages driven on
//! `vy-runtime` with the text that broke it, and with stored data that
//! broke it. Neither the SDK's mock (it counts no fuel) nor
//! `vy-publish build` (it sends no events and presets no storage) sees a
//! frame like these.
//!
//! The runtime reports fuel for frames only. `vy_init` and `vy_event` have
//! the hard limit of eight budgets and no other, so what these tests can
//! say of them is that they were not aborted.

use std::fs;
use std::path::{Path, PathBuf};
use vy_abi::{button, event as kind, modifier, role};
use vy_manifest::Draft;
use vy_runtime::{
    Capabilities, CompileCache, Options, Runtime, SemNode, Storage, FRAME_BUDGET_FUEL,
};

/// USB HID usage ids of the keys the fields take (SPEC-A.2.1).
mod key {
    pub const A: i32 = 0x04;
    pub const BACKSPACE: i32 = 0x2A;
    pub const HOME: i32 = 0x4A;
    pub const END: i32 = 0x4D;
    pub const DOWN: i32 = 0x51;
    pub const UP: i32 = 0x52;
}

/// The compiled code of the modules these tests start, by name. A test
/// here starts a page hundreds of times; compiling it once and handing the
/// code to every later load, as the client's main process does for its
/// tab processes (SPEC-13.1.4), keeps the tests about frames.
fn compiled(name: &str) -> Option<Vec<u8>> {
    CODE.lock().unwrap().get(name).cloned()
}

static CODE: std::sync::Mutex<std::collections::BTreeMap<String, Vec<u8>>> =
    std::sync::Mutex::new(std::collections::BTreeMap::new());

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
        Self::load_with(name, width, height, Storage::new())
            .unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// The page started on `storage`, as a visit after an earlier one that
    /// saved it (SPEC-5.2.9); `Err` when `vy_init` fails or is aborted.
    fn load_with(
        name: &'static str,
        width: u32,
        height: u32,
        storage: Storage,
    ) -> Result<Self, String> {
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
            storage_quota_bytes: draft
                .capabilities
                .storage
                .as_ref()
                .map_or(vy_runtime::DEFAULT_STORAGE_QUOTA_BYTES, |s| {
                    u64::from(s.quota_kb) * 1024
                }),
            storage,
            compile_cache: CompileCache::Host {
                code: compiled(name),
            },
            ..Options::default()
        };
        let mut rt = Runtime::load(&wasm, &vy_runtime::sha256(&wasm), options).unwrap();
        if let Some(code) = rt.take_code() {
            CODE.lock().unwrap().insert(name.to_string(), code);
        }
        rt.init(width, height, 1.0)
            .map_err(|e| format!("vy_init at {width} x {height}: {e}"))?;
        let mut page = Self {
            name,
            rt,
            worst: (0, String::new()),
        };
        page.frames("init");
        Ok(page)
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

/// A storage holding `value` under `key`: what an earlier visit saved.
fn stored(key: &str, value: &[u8]) -> Storage {
    Storage::from([(key.as_bytes().to_vec(), value.to_vec())])
}

/// xorshift64*, as the headless host's `random`: the same blobs in every
/// run.
fn next(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Texts within `skriv`'s 6 000 bytes that are as bad as Markdown gets for
/// a parser, a layout or a tree: one construct over and over, openers
/// that never close, and mixes of all the markup there is.
fn hostile_markdown() -> Vec<(String, String)> {
    let fill = |unit: &str| unit.repeat(6000 / unit.len());
    let mut texts: Vec<(String, String)> = [
        "W",
        "[",
        "]",
        "(",
        "[a](",
        "[a]",
        "[a](b)",
        "[a](b) ",
        "[[a](b)",
        "`",
        "`a",
        "`a` ",
        "*",
        "*a",
        "*a* ",
        "**",
        "**a",
        "**a** ",
        "***a*** ",
        "_",
        "_a",
        "_a_ ",
        "a_",
        "\\[",
        "\\",
        "a ",
        "å ",
        "\u{10348}",
        "\n",
        "a\n\n",
        "- a\n",
        "  - a\n",
        "1. a\n",
        "# a\n",
        "### \n",
        "---\n",
        "> a\n",
        ">\n",
        "```\n",
        "```\na\n```\n",
        "a\n```\n",
        "*a _b `c [d](e) **f\n",
        "- [a](b) `c` **d** *e*\n",
    ]
    .iter()
    .map(|unit| (format!("{unit:?} repeated"), fill(unit)))
    .collect();
    texts.push(("5 999 x [ and one ]".into(), "[".repeat(5999) + "]"));
    texts.push((
        "[ x 2 000, ]( x 1 000, no )".into(),
        "[".repeat(2000) + &"](".repeat(1000) + &"a".repeat(2000),
    ));
    texts.push((
        "one link with a 5 990-byte target".into(),
        format!("[a]({})", "b".repeat(5990)),
    ));
    texts.push((
        "one link with a 5 990-byte text".into(),
        format!("[{}](b)", "a".repeat(5990)),
    ));
    // Every piece of markup there is, in a fixed pseudo-random order.
    let alphabet: Vec<&str> = vec![
        "[", "]", "(", ")", "*", "**", "_", "`", "\\", "\n", "\n\n", "- ", "  - ", "1. ", "# ",
        "> ", "```\n", "---\n", " ", "a", "ord", "å", "W",
    ];
    let mut seed = 0x5EED_5EED_0000_0001u64;
    for n in 0..12 {
        let mut text = String::new();
        while text.len() < 5990 {
            text.push_str(alphabet[(next(&mut seed) % alphabet.len() as u64) as usize]);
        }
        texts.push((format!("mixed markup {n}"), text));
    }
    texts
}

/// Open `skriv` on `text` as its stored note at `width` × `height` and use
/// it: the narrow view's two tabs, Tab through the first links, the wheel
/// over the preview, a resize, a character typed first in the note (the
/// text is parsed, wrapped and saved again) and a reload on what that
/// saved.
fn skriv_on(case: &str, text: &[u8], width: u32, height: u32) -> (u64, String) {
    let fail = |e: String| -> Page { panic!("skriv on {case}: {e}") };
    let mut page =
        Page::load_with("skriv", width, height, stored("skriv", text)).unwrap_or_else(fail);
    let tabs: Vec<SemNode> = page
        .rt
        .semantic_tree()
        .iter()
        .filter(|n| n.role == role::TAB)
        .cloned()
        .collect();
    for tab in tabs.iter().rev() {
        page.click(tab.x + tab.w / 2.0, tab.y + tab.h / 2.0);
    }
    for _ in 0..4 {
        page.key(vy_abi::key::TAB, 0);
    }
    let (x, y) = ((width as i32 - 60) * 100, (height as i32 / 2) * 100);
    page.event("a pointer move", kind::POINTER_MOVE, x, y, 0, 0);
    page.event("the wheel", kind::SCROLL, 0, 40_000, 0, 0);
    page.resize(width + 123, height - 77);
    if let Some(editor) = page
        .rt
        .semantic_tree()
        .iter()
        .find(|n| n.role == role::TEXTBOX)
        .cloned()
    {
        page.click(editor.x + editor.w / 2.0, editor.y + editor.h / 2.0);
        page.key(key::HOME, modifier::CTRL);
        page.key(key::BACKSPACE, 0);
        page.text("x");
    }
    // What the page saved now is what the next visit starts on.
    let saved = page.rt.take_storage();
    let mut again = Page::load_with("skriv", width, height, saved).unwrap_or_else(fail);
    again.resize(width, height + 50);
    if again.worst.0 > page.worst.0 {
        again.worst
    } else {
        page.worst
    }
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013:1): `skriv` saves the note in
/// the event, before the frame that shows it, so a text it cannot show
/// within the budget stopped it at every reload. Whatever 6 000 bytes it
/// holds, the page starts on them and keeps every frame within the
/// budget, in the split view and behind the tabs: (a) a word without a
/// space (49.3 M and an aborted frame before, the SDK's wrapping), (b)
/// `[` after `[` (aborted: the parser looked for a `]` to the end of the
/// text for each), and every other construct of the Markdown it reads,
/// repeated to the limit.
#[test]
fn spec_5_3_3_skriv_starts_on_any_text_it_saved() {
    let mut worst = (0, String::new());
    for (case, text) in hostile_markdown() {
        for (width, height) in [(1024, 720), (400, 700)] {
            let (fuel, after) = skriv_on(&case, text.as_bytes(), width, height);
            if fuel > worst.0 {
                worst = (fuel, format!("{case} at {width} x {height}, after {after}"));
            }
        }
    }
    eprintln!(
        "skriv on hostile notes: costliest frame {} fuel, {}",
        worst.0, worst.1
    );
    // More than the editor takes, and bytes that are no text at all: the
    // page cuts the one and starts on its sample for the other.
    let long = "ord [a](b) ".repeat(1489);
    assert_eq!(long.len(), 16_379, "what the quota of 16 KB holds");
    skriv_on("16 KB of words and links", long.as_bytes(), 1024, 720);
    let mut seed = 0xC0FF_EE00_0000_0001u64;
    for n in 0..20 {
        let len = (next(&mut seed) % 16_000) as usize;
        let blob: Vec<u8> = (0..len).map(|_| next(&mut seed) as u8).collect();
        skriv_on(&format!("random bytes {n}"), &blob, 800, 600);
    }
    skriv_on("an empty note", b"", 800, 600);
}
