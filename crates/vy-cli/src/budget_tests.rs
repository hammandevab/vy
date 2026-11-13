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
    /// The module, and after [`Page::on`] the data it was started on.
    name: String,
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
            name: name.to_string(),
            rt,
            worst: (0, String::new()),
        };
        page.frames("init");
        Ok(page)
    }

    /// The same page, named in every failure with what it was started on.
    fn on(mut self, case: &str) -> Self {
        self.name = format!("{} on {case}", self.name);
        self
    }

    /// Run the frames the page asks for (at most eight: some pages
    /// animate) and hold each to the budget; how many ran.
    fn frames(&mut self, after: &str) -> u32 {
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
        ran
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
    let mut page = Page::load_with("skriv", width, height, stored("skriv", text))
        .unwrap_or_else(fail)
        .on(case);
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
    let mut again = Page::load_with("skriv", width, height, saved)
        .unwrap_or_else(fail)
        .on(case);
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

/// `kalkyl`'s stored form (`Sheet::encode`): a format byte, then for each
/// cell its index (column + 26 × row, two bytes), the length of its
/// source (one byte) and the source.
fn sheet_blob(cells: &[(usize, usize, &str)]) -> Vec<u8> {
    let mut out = vec![1u8];
    for (col, row, src) in cells {
        out.extend_from_slice(&((col + 26 * row) as u16).to_le_bytes());
        out.push(src.len() as u8);
        out.extend_from_slice(src.as_bytes());
    }
    out
}

/// Every cell of the sheet for which `source` gives one, row by row.
fn sheet_of(source: impl Fn(usize, usize) -> Option<String>) -> Vec<u8> {
    let mut cells = Vec::new();
    for row in 0..100 {
        for col in 0..26 {
            if let Some(src) = source(col, row) {
                cells.push((col, row, src));
            }
        }
    }
    let cells: Vec<(usize, usize, &str)> =
        cells.iter().map(|(c, r, s)| (*c, *r, s.as_str())).collect();
    sheet_blob(&cells)
}

/// The name of cell `i`, counted row by row: `B3`.
fn cell_name(i: usize) -> String {
    format!("{}{}", (b'A' + (i % 26) as u8) as char, i / 26 + 1)
}

impl Page {
    /// Run frames until the page asks for no more, each within the
    /// budget; how many it took.
    fn settle(&mut self, after: &str) -> u32 {
        let mut ran = 0;
        while self.rt.wants_frame() {
            ran += self.frames(after);
            assert!(
                ran < 1000,
                "{}: still asking for frames after {after}",
                self.name
            );
        }
        ran
    }

    /// True when the last frame's tree has a node with this text.
    fn shows(&self, text: &str) -> bool {
        self.rt.semantic_tree().iter().any(|n| n.text == text)
    }
}

/// Open `kalkyl` on `blob` as its stored sheet and use it: the frames it
/// asks for until the sheet is calculated, a cell changed, another size,
/// and a reload on what it saved. Returns the page after the first
/// recalculation, for the caller to look at.
fn kalkyl_on(case: &str, blob: &[u8], width: u32, height: u32) -> (u64, String) {
    let fail = |e: String| -> Page { panic!("kalkyl on {case}: {e}") };
    let mut page = Page::load_with("kalkyl", width, height, stored("kalkyl", blob))
        .unwrap_or_else(fail)
        .on(case);
    page.settle("the stored sheet");
    let calculating = |page: &Page| {
        page.rt
            .semantic_tree()
            .iter()
            .any(|n| n.text.contains("räknar"))
    };
    assert!(
        !calculating(&page),
        "{case}: calculated when the frames end"
    );
    // A press on a cell in the grid, a number typed and Enter: the sheet
    // is recalculated, a step in the event and a step a frame.
    page.click(200.0, 200.0);
    page.text("12,5");
    page.key(vy_abi::key::ENTER, 0);
    page.settle("a cell changed");
    assert!(!calculating(&page), "{case}: calculated after the change");
    page.resize(width + 300, height + 200);
    page.event("the wheel", kind::SCROLL, 0, 60_000, 0, 0);
    page.settle("scrolling");
    let saved = page.rt.take_storage();
    let mut again = Page::load_with("kalkyl", width, height, saved)
        .unwrap_or_else(fail)
        .on(case);
    again.settle("the sheet it saved");
    if again.worst.0 > page.worst.0 {
        again.worst
    } else {
        page.worst
    }
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013:2): `kalkyl` saves the sheet
/// before it is calculated, so a sheet it could not calculate within a
/// call stopped it at every visit: 100 numbers and 2 500 sums of them,
/// and 200 formulas that each sum the whole sheet (3.6 KB stored), both
/// aborted `vy_init` at the hard limit. A recalculation is now a step of
/// bounded work in each frame until it is done, formulas are kept as
/// their text (1 260 formulas of 200 bytes as trees were more memory
/// than the page may have: a trap), and are first read through by the
/// steps too. Whatever sheet is stored, the page starts, calculates it
/// and keeps every frame within the budget.
#[test]
fn spec_5_3_3_kalkyl_starts_on_any_sheet_it_saved() {
    let sums = sheet_of(|col, _| {
        Some(if col == 0 {
            "1,5".into()
        } else {
            "=SUMMA(A1:A100)".into()
        })
    });
    let tangle = sheet_of(|col, row| (row * 26 + col < 200).then(|| "=SUMMA(A1:Z100)".into()));
    assert_eq!(tangle.len(), 3601, "the review's 3.6 KB");
    let chain = sheet_of(|col, row| {
        let i = row * 26 + col;
        Some(if i == 2599 {
            "1".into()
        } else {
            format!("={}+1", cell_name(i + 1))
        })
    });
    let widest = format!("=SUMMA({})", ["B1:Z100"; 24].join(";"));
    let nested = format!("={}1{}", "(".repeat(99), ")".repeat(99));
    let terms = format!("=1{}", "+1".repeat(99));
    let minus = format!("={}1", "-".repeat(198));
    let dense = |src: &str| {
        let src = src.to_string();
        sheet_of(move |col, row| (row * 26 + col < 1260).then(|| src.clone()))
    };
    let sheets: Vec<(&str, Vec<u8>)> = vec![
        ("100 numbers and 2 500 sums of them", sums),
        ("200 sums of the whole sheet", tangle),
        (
            "2 600 sums of the whole sheet",
            sheet_of(|_, _| Some("=SUMMA(A1:Z100)".into())),
        ),
        ("a chain through every cell", chain),
        (
            "a column of formulas that read the sheet 24 times",
            sheet_of(|col, _| Some(if col == 0 { widest.clone() } else { "1".into() })),
        ),
        (
            "2 600 numbers",
            sheet_of(|col, row| Some(format!("{},{row}", col * row))),
        ),
        ("1 260 formulas of 99 nested brackets", dense(&nested)),
        ("1 260 formulas of 100 terms", dense(&terms)),
        ("1 260 formulas of 198 minus signs", dense(&minus)),
        (
            "1 260 formulas that do not parse",
            dense(&"=SUMMA(".repeat(28)),
        ),
        ("1 260 divisions by zero", dense("=1/0+SUMMA(A1:Z2)")),
    ];
    let mut worst = (0, String::new());
    for (case, blob) in &sheets {
        assert!(blob.len() <= 256 * 1024, "{case}: within the quota");
        for (width, height) in [(1024, 720), (1600, 1000)] {
            let (fuel, after) = kalkyl_on(case, blob, width, height);
            if fuel > worst.0 {
                worst = (fuel, format!("{case} at {width} x {height}, after {after}"));
            }
        }
    }
    eprintln!(
        "kalkyl on hostile sheets: costliest frame {} fuel, {}",
        worst.0, worst.1
    );

    // What the two sheets of the review come to once they are calculated.
    let mut page = Page::load_with("kalkyl", 1024, 720, stored("kalkyl", &sheets[0].1)).unwrap();
    assert!(
        page.shows("2600 celler · räknar…"),
        "the first frame says so"
    );
    assert!(page.settle("the sums") > 20, "over many frames");
    assert!(page.shows("B1: 150") && page.shows("Z100: 150"));
    assert!(page.shows("2600 celler"));
    let mut page = Page::load_with("kalkyl", 1024, 720, stored("kalkyl", &sheets[1].1)).unwrap();
    page.settle("the tangle");
    assert!(page.shows("A1: #CIRKEL") && page.shows("R8: #CIRKEL"));
}
