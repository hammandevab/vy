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
    pub const Z: i32 = 0x1D;
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
    /// The window, logical pixels at scale 1.
    size: (u32, u32),
    /// Fuel of the costliest frame so far, and what led to it.
    worst: (u64, String),
    /// The largest share of the rasterizer's budget a frame took, of
    /// those [`Page::raster`] looked at.
    raster_worst: f64,
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
            size: (width, height),
            worst: (0, String::new()),
            raster_worst: 0.0,
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
        self.size = (width, height);
        self.frames(&what);
    }

    /// Rasterize the last frame as the client does and hold it to the
    /// rasterizer's budget, the other half of SPEC-5.3.3; the share of
    /// the budget it took.
    fn raster(&mut self, after: &str) -> f64 {
        let list = self.rt.display_list().to_vec();
        let (width, height) = self.size;
        let (text, images) = self.rt.raster_parts();
        let report = vy_render::render(&list, width, height, 1.0, text, images)
            .unwrap_or_else(|e| panic!("{}: the image after {after}: {e}", self.name))
            .report;
        assert!(
            !report.truncated,
            "{}: rasterizing the frame after {after} stopped at its budget of {} units (SPEC-5.3.3)",
            self.name,
            report.budget
        );
        let share = report.units as f64 / report.budget as f64;
        self.raster_worst = self.raster_worst.max(share);
        share
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
/// the wheel, and a reload on what it saved. The costliest frame and what
/// led to it.
fn kalkyl_on(case: &str, blob: &[u8], width: u32, height: u32) -> (u64, String) {
    let (fuel, after, _) = kalkyl_visit(case, blob, width, height, false);
    (fuel, after)
}

/// [`kalkyl_on`]; with `look`, the frame is rasterized as well at each
/// stop and held to that budget. The costliest frame, what led to it, and
/// the largest share of the raster budget.
fn kalkyl_visit(
    case: &str,
    blob: &[u8],
    width: u32,
    height: u32,
    look: bool,
) -> (u64, String, f64) {
    let fail = |e: String| -> Page { panic!("kalkyl on {case}: {e}") };
    let mut page = Page::load_with("kalkyl", width, height, stored("kalkyl", blob))
        .unwrap_or_else(fail)
        .on(case);
    let look_at = |page: &mut Page, after: &str| {
        if look {
            page.raster(after);
        }
    };
    page.settle("the stored sheet");
    look_at(&mut page, "the stored sheet");
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
    look_at(&mut page, "a resize");
    page.event("the wheel", kind::SCROLL, 0, 60_000, 0, 0);
    page.settle("scrolling");
    look_at(&mut page, "scrolling");
    let saved = page.rt.take_storage();
    let mut again = Page::load_with("kalkyl", width, height, saved)
        .unwrap_or_else(fail)
        .on(case);
    again.settle("the sheet it saved");
    look_at(&mut again, "the sheet it saved");
    let raster = page.raster_worst.max(again.raster_worst);
    let (fuel, after) = if again.worst.0 > page.worst.0 {
        again.worst
    } else {
        page.worst
    };
    (fuel, after, raster)
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

// A page must never be able to stop itself with data it saved (granskning
// 0013, the rule from its findings 1 and 2). What a page reads back from
// `state_get` is whatever an earlier version of it, or a bug in it, put
// there: every example page is started here on values it never meant to
// save, and must start, keep every frame within the budget, and start
// again on what it saved meanwhile.

/// `count` values a page did not save, each at most `max` bytes: random
/// bytes; random bytes after `head` (the page's format byte), so that its
/// decoder gets past the first check; and small numbers after `head`,
/// which read as plausible counts and lengths.
fn random_blobs(seed: u64, count: usize, max: usize, head: &[u8]) -> Vec<Vec<u8>> {
    let mut state = seed;
    (0..count)
        .map(|n| {
            let len = (next(&mut state) % (max as u64 + 1)) as usize;
            let mut blob: Vec<u8> = (0..len)
                .map(|_| {
                    let byte = next(&mut state) as u8;
                    if n % 3 == 2 {
                        byte % 7
                    } else {
                        byte
                    }
                })
                .collect();
            if n % 3 != 0 {
                for (b, h) in blob.iter_mut().zip(head) {
                    *b = *h;
                }
            }
            blob
        })
        .collect()
}

/// `blob` cut short: at every length below 24 and at every `step`th
/// after that, as a write that did not finish would leave it.
fn truncations(blob: &[u8], step: usize) -> Vec<Vec<u8>> {
    (0..blob.len())
        .filter(|len| *len < 24 || len % step == 0)
        .map(|len| blob[..len].to_vec())
        .collect()
}

/// The largest value a page with this draft can have saved under `key`:
/// the quota less the key (SPEC-9.3.2).
fn quota_of(name: &str, key: &str) -> usize {
    let draft =
        fs::read_to_string(root().join(format!("conformance/manifests/{name}.json"))).unwrap();
    let draft = Draft::from_json(&draft).unwrap();
    let kb = draft
        .capabilities
        .storage
        .unwrap_or_else(|| panic!("{name} requests storage"))
        .quota_kb;
    kb as usize * 1024 - key.len()
}

impl Page {
    /// Use the page as someone who knows nothing about it does: Tab to
    /// something and press Enter, type a letter, Enter again, press and
    /// drag across the middle, turn the wheel, undo. What that changes,
    /// the page saves.
    fn poke(&mut self) {
        let (w, h) = (self.size.0 as i32 * 100, self.size.1 as i32 * 100);
        for _ in 0..3 {
            self.key(vy_abi::key::TAB, 0);
        }
        self.key(vy_abi::key::ENTER, 0);
        self.text("x");
        self.key(vy_abi::key::ENTER, 0);
        let (x, y) = (w * 45 / 100, h * 55 / 100);
        self.event("a press", kind::POINTER_DOWN, x, y, button::PRIMARY, 0);
        for step in 1..=6 {
            self.event(
                "a drag",
                kind::POINTER_MOVE,
                x + step * w / 40,
                y + step * h / 50,
                button::mask(button::PRIMARY),
                0,
            );
        }
        let (x, y) = (x + 6 * w / 40, y + 6 * h / 50);
        self.event("a release", kind::POINTER_UP, x, y, button::PRIMARY, 0);
        self.event("the wheel", kind::SCROLL, 0, 30_000, 0, 0);
        self.key(key::Z, modifier::CTRL);
    }
}

/// A visit to `name` that starts on `value` under `key`: `vy_init` and
/// the frames it asks for, a narrow window, some use, a wide window, and
/// a second visit on what the page saved meanwhile. With `look`, the
/// frame at each size is rasterized as well and held to that budget.
/// The costliest frame, what led to it, and the largest share of the
/// raster budget.
fn visit(
    name: &'static str,
    key: &str,
    case: &str,
    value: &[u8],
    look: bool,
) -> (u64, String, f64) {
    let fail = |e: String| -> Page { panic!("{name} on {case}: {e}") };
    let (width, height) = (1024, 720);
    let mut page = Page::load_with(name, width, height, stored(key, value))
        .unwrap_or_else(fail)
        .on(case);
    if look {
        page.raster("the stored value");
    }
    page.resize(400, 700);
    if look {
        page.raster("a resize");
    }
    page.poke();
    page.resize(1600, 1000);
    if look {
        page.raster("some use");
    }
    let saved = page.rt.take_storage();
    let mut again = Page::load_with(name, width, height, saved)
        .unwrap_or_else(fail)
        .on(case);
    again.poke();
    if look {
        again.raster("the second visit");
    }
    let raster = page.raster_worst.max(again.raster_worst);
    let (fuel, after) = if again.worst.0 > page.worst.0 {
        again.worst
    } else {
        page.worst
    };
    (fuel, after, raster)
}

/// Visit `name` on each of `values`; the costliest frame of them all.
fn visits(name: &'static str, key: &str, values: &[(String, Vec<u8>)], look: bool) {
    let quota = quota_of(name, key);
    let mut worst = (0, String::new());
    let mut raster_worst = (0.0, String::new());
    for (case, value) in values {
        assert!(value.len() <= quota, "{name} on {case}: within the quota");
        let (fuel, after, raster) = visit(name, key, case, value, look);
        if fuel > worst.0 {
            worst = (fuel, format!("{case}, after {after}"));
        }
        if raster > raster_worst.0 {
            raster_worst = (raster, case.clone());
        }
    }
    eprintln!(
        "{name} on {} stored values: costliest frame {} fuel, {}; most of the raster budget {:.0} % ({})",
        values.len(),
        worst.0,
        worst.1,
        raster_worst.0 * 100.0,
        raster_worst.1
    );
}

/// Random values and cuts of `whole` (a value the page could have
/// saved), named.
fn unmeant(
    seed: u64,
    max: usize,
    head: &[u8],
    whole: &[u8],
    step: usize,
) -> Vec<(String, Vec<u8>)> {
    let random = random_blobs(seed, 300, max, head)
        .into_iter()
        .enumerate()
        .map(|(n, blob)| (format!("random value {n} ({} bytes)", blob.len()), blob));
    let cut = truncations(whole, step)
        .into_iter()
        .map(|blob| (format!("a value cut at {} bytes", blob.len()), blob));
    random.chain(cut).collect()
}

/// `kanban`'s stored form (`Board::encode`): a format byte, then for each
/// of the three columns a count and for each card a length and the title.
fn board_blob(columns: [&[String]; 3]) -> Vec<u8> {
    let mut out = vec![1u8];
    for cards in columns {
        out.push(cards.len() as u8);
        for title in cards {
            out.push(title.len() as u8);
            out.extend_from_slice(title.as_bytes());
        }
    }
    out
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `kanban` starts on any
/// stored value: 300 random ones, a full board cut at every length, the
/// fullest boards its format can describe, and values of the whole
/// quota.
#[test]
fn spec_5_3_3_kanban_starts_on_any_stored_value() {
    let titles = |unit: &str, n: usize| -> Vec<String> {
        (0..n).map(|_| unit.repeat(80 / unit.len())).collect()
    };
    let none: &[String] = &[];
    let wide = titles("W", 60);
    let full = board_blob([&wide[..20], &wide[20..40], &wide[40..]]);
    let quota = quota_of("kanban", "tavlan");
    let mut after_board = board_blob([&wide[..1], none, none]);
    after_board.resize(quota, 0xFF);
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("60 cards of 80 x W".into(), full.clone()),
        (
            "60 cards of 80 x W in one column".into(),
            board_blob([none, &wide, none]),
        ),
        (
            "60 cards of 20 four-byte characters".into(),
            board_blob([&titles("\u{10348}", 60), none, none]),
        ),
        (
            "60 cards of 40 one-letter words".into(),
            board_blob([none, none, &titles("a ", 60)]),
        ),
        (
            "60 cards of line breaks and controls".into(),
            board_blob([&titles("\n\t\u{2028}\u{7f}\u{0}\u{202e}", 60), none, none]),
        ),
        ("61 cards".into(), board_blob([&wide, &wide[..1], none])),
        (
            "cards of 255 bytes".into(),
            board_blob([&vec!["W".repeat(255); 60], none, none]),
        ),
        (
            "a card without a title".into(),
            board_blob([&[String::new()], none, none]),
        ),
        (
            "a board without cards".into(),
            board_blob([none, none, none]),
        ),
        ("255 cards and no titles".into(), vec![1, 255, 255, 255]),
        ("a quota of zeros".into(), vec![0; quota]),
        ("a quota of ones".into(), vec![1; quota]),
        ("a quota of 0xFF".into(), vec![0xFF; quota]),
        ("a board and then a quota of 0xFF".into(), after_board),
    ];
    visits("kanban", "tavlan", &values, true);
    values = unmeant(0x7A51_A000_0000_0001, 5000, &[1], &full, 53);
    assert!(values.len() > 400);
    visits("kanban", "tavlan", &values, false);

    // The full board is one the page reads.
    let page = Page::load_with("kanban", 1024, 720, stored("tavlan", &full)).unwrap();
    assert!(page.shows("60 kort · tavlan är full"));
}

/// `rita`'s stored form (`ink::encode`): a format byte, the number of
/// strokes, and for each a byte of colour and pen, the number of points
/// and the points, each as the zigzag difference from the one before in
/// groups of seven bits. Points in eighths of a paper unit: the paper is
/// 9 600 × 6 400.
fn drawing_blob(strokes: &[(u8, Vec<(i32, i32)>)]) -> Vec<u8> {
    fn put(out: &mut Vec<u8>, delta: i32) {
        let mut v = ((delta << 1) ^ (delta >> 31)) as u32;
        while v >= 0x80 {
            out.push((v & 0x7F) as u8 | 0x80);
            v >>= 7;
        }
        out.push(v as u8);
    }
    let mut out = vec![1u8];
    out.extend_from_slice(&(strokes.len() as u16).to_le_bytes());
    for (tool, points) in strokes {
        out.push(*tool);
        out.extend_from_slice(&(points.len() as u16).to_le_bytes());
        let mut last = (0, 0);
        for &(x, y) in points {
            put(&mut out, x - last.0);
            put(&mut out, y - last.1);
            last = (x, y);
        }
    }
    out
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `rita` starts on any stored
/// value: 300 random ones, a full drawing cut at every length, the
/// drawings with the most points and the most ink its format can
/// describe, and values of the whole quota. The frames are rasterized
/// too: a drawing costs the rasterizer its ink, and 1 250 lines from
/// corner to corner, which the page would never take but did read back,
/// stopped the rasterizer at its budget.
#[test]
fn spec_5_3_3_rita_starts_on_any_stored_value() {
    const WIDEST: u8 = 2;
    let (w, h) = (9600, 6400);
    let strokes = |point: &dyn Fn(usize) -> (i32, i32), per: usize| -> Vec<(u8, Vec<(i32, i32)>)> {
        (0..2500 / per)
            .map(|s| {
                (
                    ((s % 8) as u8) << 2 | WIDEST,
                    (0..per).map(|i| point(s * per + i)).collect(),
                )
            })
            .collect()
    };
    // Corner to corner and back, 2 500 times: thirty times the ink the
    // page lets anyone draw.
    let across = drawing_blob(&strokes(&|i| [(0, 0), (w, h), (0, h), (w, 0)][i % 4], 500));
    let quota = quota_of("rita", "rita");
    let mut after_drawing = drawing_blob(&[(WIDEST, vec![(100, 100), (9000, 6000)])]);
    after_drawing.resize(quota, 0);
    // What the page takes: strokes until their ink (length × width, in
    // paper units, the length as `ink::distance` has it) reaches
    // 2 000 000, and the one that takes it there however long it is. A
    // line from corner to corner with the widest pen is 21 588.
    let diagonal = |s: usize| {
        let ends = [[(0, 0), (w, h)], [(0, h), (w, 0)]][s % 2];
        (WIDEST, ends.to_vec())
    };
    let mut fullest: Vec<(u8, Vec<(i32, i32)>)> = (0..92).map(diagonal).collect();
    fullest.push((
        WIDEST,
        (0..500)
            .map(|i| [(0, 0), (w, h), (0, h), (w, 0)][i % 4])
            .collect(),
    ));
    let fullest = drawing_blob(&fullest);
    let lines = drawing_blob(&(0..1250).map(diagonal).collect::<Vec<_>>());
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("2 500 points from corner to corner".into(), across.clone()),
        ("1 250 lines from corner to corner".into(), lines.clone()),
        (
            "92 lines and a stroke of 500 points from corner to corner".into(),
            fullest.clone(),
        ),
        (
            "2 500 points along the top edge".into(),
            drawing_blob(&strokes(&|i| ((i % 2) as i32 * w, 0), 500)),
        ),
        (
            "2 500 points in one place".into(),
            drawing_blob(&strokes(&|_| (w / 2, h / 2), 500)),
        ),
        (
            "2 500 points an eighth of a unit apart".into(),
            drawing_blob(&strokes(&|i| (4800 + (i % 2) as i32, 3200), 500)),
        ),
        (
            "2 500 dots".into(),
            drawing_blob(&strokes(
                &|i| ((i % 50) as i32 * 190, (i / 50) as i32 * 128),
                1,
            )),
        ),
        (
            "a scribble of 2 500 points".into(),
            drawing_blob(&{
                let mut seed = 0x51CA_77E0_0000_0001u64;
                let at: Vec<(i32, i32)> = (0..2500)
                    .map(|_| {
                        (
                            (next(&mut seed) % 9601) as i32,
                            (next(&mut seed) % 6401) as i32,
                        )
                    })
                    .collect();
                strokes(&|i| at[i], 500)
            }),
        ),
        (
            "a stroke of 501 points".into(),
            drawing_blob(&[(0, (0..501).map(|i| (i, i)).collect())]),
        ),
        (
            "2 501 points".into(),
            drawing_blob(&(0..2501).map(|i| (0u8, vec![(i, i)])).collect::<Vec<_>>()),
        ),
        (
            "a stroke without points".into(),
            drawing_blob(&[(0, Vec::new())]),
        ),
        (
            "a pen that does not exist".into(),
            drawing_blob(&[(3, vec![(10, 10), (500, 500)])]),
        ),
        (
            "a colour that does not exist".into(),
            drawing_blob(&[(63 << 2, vec![(10, 10), (500, 500)])]),
        ),
        (
            "a point off the paper".into(),
            drawing_blob(&[(0, vec![(10, 10), (9601, 10)])]),
        ),
        (
            "a point left of the paper".into(),
            drawing_blob(&[(0, vec![(-1, 10)])]),
        ),
        ("65 535 strokes and no more".into(), vec![1, 0xFF, 0xFF]),
        ("no strokes".into(), vec![1, 0, 0]),
        ("a quota of zeros".into(), vec![0; quota]),
        ("a quota of ones".into(), vec![1; quota]),
        ("a quota of 0xFF".into(), vec![0xFF; quota]),
        ("a stroke and then a quota of zeros".into(), after_drawing),
    ];
    visits("rita", "rita", &values, true);
    values = unmeant(0x717A_0000_0000_0001, 3000, &[1, 2, 0], &across, 97);
    assert!(values.len() > 400);
    visits("rita", "rita", &values, false);

    // The fullest drawing is one the page reads, the lines are not.
    let page = Page::load_with("rita", 1024, 720, stored("rita", &fullest)).unwrap();
    assert!(page.shows("Ritning med 93 streck"));
    let page = Page::load_with("rita", 1024, 720, stored("rita", &lines)).unwrap();
    assert!(page.shows("Ritning med 0 streck"));
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `block` starts on any stored
/// value: 300 random ones (four bytes are a best score, anything else is
/// none), every score with one bit set, and a value of the whole quota.
#[test]
fn spec_5_3_3_block_starts_on_any_stored_value() {
    let quota = quota_of("block", "block");
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("the highest score there is".into(), vec![0xFF; 4]),
        ("an empty value".into(), Vec::new()),
        ("three bytes".into(), vec![1, 2, 3]),
        ("five bytes".into(), vec![1, 2, 3, 4, 5]),
        ("a quota of 0xFF".into(), vec![0xFF; quota]),
    ];
    for bit in 0..32 {
        values.push((
            format!("a best score of 2^{bit}"),
            (1u32 << bit).to_le_bytes().to_vec(),
        ));
    }
    visits("block", "block", &values, true);
    let mut seed = 0xB10C_0000_0000_0001u64;
    values = random_blobs(seed, 150, quota, &[])
        .into_iter()
        .enumerate()
        .map(|(n, blob)| (format!("random value {n} ({} bytes)", blob.len()), blob))
        .collect();
    for n in 0..150 {
        let score = next(&mut seed) as u32;
        values.push((
            format!("random score {n} ({score})"),
            score.to_le_bytes().to_vec(),
        ));
    }
    visits("block", "block", &values, false);
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `skriv` on values that are
/// not the texts of `spec_5_3_3_skriv_starts_on_any_text_it_saved`: 300
/// random ones, texts cut inside a character, and the whole quota of the
/// constructs that cost it the most.
#[test]
fn spec_5_3_3_skriv_starts_on_any_stored_value() {
    let quota = quota_of("skriv", "skriv");
    let fill = |unit: &str| unit.repeat(quota / unit.len()).into_bytes();
    let text = "# Rubrik\n\nEtt stycke med **fet**, *betonad* och `kod` text och \
                [en länk](vy://x).\n\n- å\n  - \u{10348}\n\n```\nkod\n```\n"
        .repeat(160);
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("a quota of line breaks".into(), fill("\n")),
        ("a quota of [".into(), fill("[")),
        ("a quota of links".into(), fill("[a](b)")),
        ("a quota of W".into(), fill("W")),
        ("a quota of four-byte characters".into(), fill("\u{10348}")),
        ("a quota of paragraphs".into(), fill("a\n\n")),
        ("a quota of zeros".into(), vec![0; quota]),
        ("a quota of 0xFF".into(), vec![0xFF; quota]),
        (
            "a quota of text".into(),
            text.as_bytes()[..quota.min(text.len())].to_vec(),
        ),
    ];
    visits("skriv", "skriv", &values, true);
    // Cuts of a text fall inside its two- and four-byte characters.
    values = unmeant(
        0x5C81_0000_0000_0001,
        8000,
        b"# ",
        &text.as_bytes()[..6000],
        61,
    );
    assert!(values.len() > 400);
    visits("skriv", "skriv", &values, false);
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `kalkyl` on values that are
/// not the sheets of `spec_5_3_3_kalkyl_starts_on_any_sheet_it_saved`:
/// 300 random ones, a sheet cut at every length, sheets of random
/// formulas, and the whole quota of one cell set again and again.
#[test]
fn spec_5_3_3_kalkyl_starts_on_any_stored_value() {
    let quota = quota_of("kalkyl", "kalkyl");
    let sums = sheet_of(|col, row| {
        (row < 12).then(|| {
            if col == 0 {
                "1,5".into()
            } else {
                "=SUMMA(A1:A100)".into()
            }
        })
    });
    // One cell, set 65 000 times; every cell, cleared 87 000 times.
    let again: Vec<u8> = [1u8]
        .into_iter()
        .chain((0..(quota - 1) / 4).flat_map(|n| [7, 0, 1, b'0' + (n % 10) as u8]))
        .collect();
    let cleared: Vec<u8> = [1u8]
        .into_iter()
        .chain((0..(quota - 1) / 3).flat_map(|n| {
            let i = (n % 2600) as u16;
            [i as u8, (i >> 8) as u8, 0]
        }))
        .collect();
    let mut values: Vec<(String, Vec<u8>)> = vec![
        ("one cell set 65 000 times".into(), again),
        ("every cell cleared 87 000 times".into(), cleared),
        (
            "a cell that is not on the sheet".into(),
            sheet_blob(&[(0, 100, "1")]),
        ),
        (
            "a source that is no text".into(),
            vec![1, 0, 0, 2, 0xC3, 0x28],
        ),
        ("an empty value".into(), Vec::new()),
        ("a sheet without cells".into(), vec![1]),
        ("a quota of zeros".into(), vec![0; quota]),
        ("a quota of ones".into(), vec![1; quota]),
        ("a quota of 0xFF".into(), vec![0xFF; quota]),
    ];
    // Sheets of formulas no one would write: every token there is, in a
    // fixed pseudo-random order, in every cell.
    let tokens = [
        "=",
        "=",
        "SUMMA(",
        "MEDEL(",
        "MIN(",
        "MAX(",
        "ANTAL(",
        "A1",
        "Z100",
        "B7",
        "M50",
        ":",
        ";",
        "+",
        "-",
        "*",
        "/",
        "(",
        ")",
        "1",
        "0",
        ",",
        "5",
        " ",
        "å",
        "x",
        "999999999999",
    ];
    let mut seed = 0xCA1C_0000_0000_0001u64;
    for n in 0..24 {
        let cells = [26, 260, 1300, 2600][n % 4];
        let longest = [12, 40, 200][n % 3];
        let sources: Vec<String> = (0..cells)
            .map(|_| {
                let mut src = String::from(if next(&mut seed).is_multiple_of(4) {
                    ""
                } else {
                    "="
                });
                let pieces = 1 + next(&mut seed) % 30;
                for _ in 0..pieces {
                    let token = tokens[(next(&mut seed) % tokens.len() as u64) as usize];
                    if src.len() + token.len() > longest {
                        break;
                    }
                    src.push_str(token);
                }
                src
            })
            .collect();
        let sheet = sheet_of(|col, row| sources.get(row * 26 + col).cloned());
        values.push((format!("random formulas {n} ({cells} cells)"), sheet));
    }
    let mut worst = (0, String::new());
    let mut on = |values: &[(String, Vec<u8>)]| {
        for (case, value) in values {
            assert!(value.len() <= quota, "kalkyl on {case}: within the quota");
            let (fuel, after) = kalkyl_on(case, value, 1024, 720);
            if fuel > worst.0 {
                worst = (fuel, format!("{case}, after {after}"));
            }
        }
    };
    on(&values);
    on(&unmeant(0xCA1C_0000_0000_0002, 4000, &[1], &sums, 41));
    eprintln!(
        "kalkyl on stored values: costliest frame {} fuel, {}",
        worst.0, worst.1
    );
}

/// SPEC-5.3.3, SPEC-5.2.9 (granskning 0013): `rymd` requests no storage
/// and reads none: with values under every key the other pages use, and
/// its own name, it draws what it draws without them.
#[test]
fn spec_5_3_3_rymd_reads_no_stored_value() {
    let draft = fs::read_to_string(root().join("conformance/manifests/rymd.json")).unwrap();
    assert!(Draft::from_json(&draft)
        .unwrap()
        .capabilities
        .storage
        .is_none());
    let empty = Page::load("rymd", 1024, 720);
    let full: Storage = ["rymd", "tavlan", "kalkyl", "rita", "skriv", "block"]
        .iter()
        .map(|key| (key.as_bytes().to_vec(), vec![0xFF; 4096]))
        .collect();
    let mut page = Page::load_with("rymd", 1024, 720, full.clone())
        .unwrap()
        .on("a full storage");
    assert_eq!(
        page.rt.display_list(),
        empty.rt.display_list(),
        "the same frames"
    );
    page.raster("the stored values");
    page.resize(400, 700);
    page.poke();
    page.resize(1600, 1000);
    page.raster("some use");
    assert_eq!(page.rt.take_storage(), full, "and writes none");
    page.report("with a full storage");
}

/// SPEC-5.3.3 (granskning 0013:3): `kalkyl` with long texts in view. A
/// cell may hold 200 bytes and the quota 256 KB: 1 260 cells of 200
/// bytes were 14.1 M units of fuel for the first frame at 1024 × 720 and
/// 20.9 M at 1600 × 1000, and the rasterizer stopped at its budget. The
/// page sent every text in view whole to draw the dozen letters its cell
/// shows, and every text of the sheet whole to the tree, in every frame.
/// Whatever texts the sheet holds, every frame keeps both budgets.
#[test]
fn spec_5_3_3_kalkyl_long_texts_keep_the_budget() {
    let dense = |n: usize, src: &str| {
        let src = src.to_string();
        sheet_of(move |col, row| (row * 26 + col < n).then(|| src.clone()))
    };
    let words = "ord och text som inte ryms i sin cell ".repeat(3);
    let sheets: Vec<(&str, Vec<u8>)> = vec![
        ("1 260 cells of 200 x W", dense(1260, &"W".repeat(200))),
        ("1 260 cells of 200 x i", dense(1260, &"i".repeat(200))),
        (
            "1 260 cells of 100 combining marks",
            dense(1260, &"\u{301}".repeat(100)),
        ),
        ("2 600 cells of 97 letters", dense(2600, &"a".repeat(97))),
        ("2 600 cells of 48 x å", dense(2600, &"å".repeat(48))),
        (
            "2 600 cells of 32 characters of three bytes",
            dense(2600, &"\u{4e2d}".repeat(32)),
        ),
        (
            "2 600 cells of 24 characters of four bytes",
            dense(2600, &"\u{10348}".repeat(24)),
        ),
        ("2 600 cells of words", dense(2600, &words[..97])),
        // As long as a text may be and still be measured: 28 bytes.
        ("2 600 cells of 28 x m", dense(2600, &"m".repeat(28))),
        // Texts with empty cells to run over: enough for all of it, and
        // one cell too few.
        (
            "a text of 196 x i in every seventh column",
            sheet_of(|col, _| (col % 7 == 0).then(|| "i".repeat(196))),
        ),
        (
            "a text of 192 bytes in every other column",
            sheet_of(|col, _| (col % 2 == 0).then(|| "Wi".repeat(96))),
        ),
    ];
    let mut worst = (0, String::new());
    let mut raster_worst = (0.0, String::new());
    for (case, blob) in &sheets {
        assert!(blob.len() <= quota_of("kalkyl", "kalkyl"), "{case}");
        // The last is a window with every column and 50 rows in view.
        for (width, height) in [(1024, 720), (1600, 1000), (2560, 1440)] {
            let (fuel, after, raster) = kalkyl_visit(case, blob, width, height, true);
            if fuel > worst.0 {
                worst = (fuel, format!("{case} at {width} x {height}, after {after}"));
            }
            if raster > raster_worst.0 {
                raster_worst = (raster, format!("{case} at {width} x {height}"));
            }
        }
    }
    eprintln!(
        "kalkyl on long texts: costliest frame {} fuel, {}; most of the raster budget {:.0} % ({})",
        worst.0,
        worst.1,
        raster_worst.0 * 100.0,
        raster_worst.1
    );

    // The review's sheet: what its first frame costs now, and what the
    // tree has of it. Every cell is a node; the first texts are whole and
    // the rest are their start.
    for (width, height) in [(1024, 720), (1600, 1000)] {
        let mut page = Page::load_with("kalkyl", width, height, stored("kalkyl", &sheets[0].1))
            .unwrap()
            .on(sheets[0].0);
        let share = page.raster("the first frame");
        page.report(&format!(
            "at {width} x {height} ({:.0} % of the raster budget)",
            share * 100.0
        ));
        let cells: Vec<&SemNode> = page
            .rt
            .semantic_tree()
            .iter()
            .filter(|n| n.role == role::CELL)
            .collect();
        assert_eq!(cells.len(), 1260);
        assert_eq!(cells[0].text, format!("A1: {}", "W".repeat(200)));
        assert_eq!(cells[1259].text, "L49: WWWWWWWWWWWWWWW…");
    }
}
