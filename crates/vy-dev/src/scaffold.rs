//! `vy-dev new`: a page project a person or an agent can build at once.
//!
//! The project is a crate of its own (its own `[workspace]`, so it builds
//! inside any other project) on `vy-sdk`, with the wasm32 release profile
//! of the reference modules, a manifest draft in the form of
//! `conformance/manifests/*.json`, and a `CLAUDE.md` that holds the whole
//! loop, the rules, a summary of the SDK's API and the SDK's own README, so
//! a page-authoring agent needs nothing from the network.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The SDK README, embedded so the scaffold's `CLAUDE.md` always carries the
/// README of the SDK this binary was built from.
pub const SDK_README: &str = include_str!("../../vy-sdk/README.md");

/// The git repository the SDK comes from without `--sdk`.
pub const SDK_GIT: &str = "https://github.com/hammandevab/vy";

/// File name of the manifest draft in a project.
pub const MANIFEST_FILE: &str = "manifest.json";

/// What `vy-dev new` writes.
#[derive(Clone, Debug)]
pub struct Scaffold {
    /// The directory to create (it may exist but must be empty).
    pub dir: PathBuf,
    /// Crate name and address segment ([`valid_name`]).
    pub name: String,
    /// A checkout of the Vy repository, to depend on `crates/vy-sdk` by
    /// path; without one the dependency is the git repository.
    pub sdk: Option<PathBuf>,
}

/// Names cargo refuses for a package or that would clash with the page's
/// own dependency: Rust's built-in crates, the SDK, and the device names
/// Windows reserves for files.
const RESERVED: &[&str] = &[
    "alloc",
    "core",
    "std",
    "test",
    "proc-macro",
    "proc_macro",
    "vy-sdk",
    "vy_sdk",
    "vy-abi",
    "vy_abi",
    "con",
    "prn",
    "aux",
    "nul",
    "com1",
    "com2",
    "com3",
    "com4",
    "com5",
    "com6",
    "com7",
    "com8",
    "com9",
    "lpt1",
    "lpt2",
    "lpt3",
    "lpt4",
    "lpt5",
    "lpt6",
    "lpt7",
    "lpt8",
    "lpt9",
];

/// A valid crate name and address segment: lowercase ASCII letters, digits,
/// `-` and `_`, starting with a letter, at most 64 characters, and not one
/// of the names cargo or Windows reserve.
pub fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(c) if c.is_ascii_lowercase())
        && name.len() <= 64
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
        && !RESERVED.contains(&name)
}

/// A name derived from a directory name: lowercased, every other character
/// replaced by `-`, prefixed with `page-` when it does not start with a
/// letter, without a trailing `-`, and `page` when nothing is left.
pub fn derive_name(dir_name: &str) -> String {
    let mut name: String = dir_name
        .chars()
        .map(|c| match c.to_ascii_lowercase() {
            c if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' => c,
            _ => '-',
        })
        .collect();
    if !name.chars().next().is_some_and(|c| c.is_ascii_lowercase()) {
        name = format!("page-{name}");
    }
    name.truncate(64);
    let name = name.trim_end_matches('-').to_string();
    if valid_name(&name) {
        name
    } else if valid_name(&format!("page-{name}")) {
        format!("page-{name}")
    } else {
        "page".to_string()
    }
}

/// The library name cargo gives a crate: `-` becomes `_`. The module is
/// `<lib>.wasm`.
pub fn lib_name(crate_name: &str) -> String {
    crate_name.replace('-', "_")
}

/// `min-sida` as a title: `Min sida`.
pub fn display_name(name: &str) -> String {
    let mut s: String = name.replace(['-', '_'], " ");
    if let Some(first) = s.get(..1) {
        let upper = first.to_ascii_uppercase();
        s.replace_range(..1, &upper);
    }
    s
}

/// A path as a TOML basic string value: forward slashes, so a Windows
/// path needs no escaping, and `"` is never part of a path we accept.
fn toml_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Today's date in UTC as `YYYY-MM-DD` (the manifest's `published`,
/// SPEC-8.1.1).
pub fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    civil_date(secs / 86_400)
}

/// `YYYY-MM-DD` of a day count since 1970-01-01 (Howard Hinnant's
/// `civil_from_days`).
pub fn civil_date(days: u64) -> String {
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

impl Scaffold {
    /// Write the project and return the files written. Fails, before
    /// writing anything, when `dir` exists and is not empty, when `name` is
    /// not [`valid_name`], or when `sdk` has no `crates/vy-sdk/Cargo.toml`.
    pub fn create(&self) -> Result<Vec<PathBuf>> {
        if !valid_name(&self.name) {
            bail!(
                "{:?} is not a valid page name: lowercase letters, digits, - and _, starting with a letter, not a name cargo reserves (pass --name)",
                self.name
            );
        }
        if self.dir.exists() {
            let mut entries = std::fs::read_dir(&self.dir)
                .with_context(|| format!("reading {}", self.dir.display()))?;
            if entries.next().is_some() {
                bail!("{} exists and is not empty", self.dir.display());
            }
        }
        let sdk_dep = match &self.sdk {
            Some(sdk) => {
                let sdk = std::path::absolute(sdk)
                    .with_context(|| format!("resolving {}", sdk.display()))?;
                let crate_dir = sdk.join("crates").join("vy-sdk");
                if !crate_dir.join("Cargo.toml").is_file() {
                    bail!(
                        "{} has no crates/vy-sdk/Cargo.toml: --sdk must name a checkout of {SDK_GIT}",
                        sdk.display()
                    );
                }
                let path = toml_path(&crate_dir);
                if path.contains('"') {
                    bail!("{path}: a path with a quote cannot be written into Cargo.toml");
                }
                format!("vy-sdk = {{ path = \"{path}\" }}")
            }
            None => format!("vy-sdk = {{ git = \"{SDK_GIT}\" }}"),
        };
        let name = self.name.as_str();
        let lib = lib_name(name);
        let display = display_name(name);
        let date = today();
        let files = [
            (
                "Cargo.toml",
                CARGO_TOML
                    .replace("{name}", name)
                    .replace("{sdk_dep}", &sdk_dep),
            ),
            ("src/lib.rs", LIB_RS.replace("{name}", name)),
            (
                MANIFEST_FILE,
                MANIFEST_JSON
                    .replace("{display}", &display)
                    .replace("{date}", &date),
            ),
            (".gitignore", GITIGNORE.to_string()),
            // The SDK README goes in last, so nothing in it is taken for a
            // placeholder.
            (
                "CLAUDE.md",
                CLAUDE_MD
                    .replace("{name}", name)
                    .replace("{lib}", &lib)
                    .replace("{sdk_readme}", SDK_README.trim_end()),
            ),
        ];
        let mut written = Vec::new();
        for (rel, text) in files {
            let path = self.dir.join(rel);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("creating {}", parent.display()))?;
            }
            std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
            written.push(path);
        }
        Ok(written)
    }
}

const CARGO_TOML: &str = r#"[package]
name = "{name}"
version = "0.1.0"
edition = "2021"
publish = false
description = "A Vy page"

[lib]
crate-type = ["cdylib"]

[dependencies]
{sdk_dep}

# The wasm32 release profile of the reference pages: small, no unwinding.
[profile.release]
opt-level = "s"
lto = true
codegen-units = 1
panic = "abort"
strip = true

[profile.dev]
panic = "abort"

# A page is its own workspace, even inside another project.
[workspace]
"#;

const LIB_RS: &str = r#"//! {name}: a Vy page built on vy-sdk.
//!
//! The page is a `struct` that implements `App`; `vy_sdk::export!` turns it
//! into the six exports the client calls. Layout and state live here:
//! `layout()` computes every rectangle from the window, and both `frame`
//! (drawing) and `event` (hit-testing) use the same rectangles.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;

const BG: Color = Color::rgb(0xF2, 0xF3, 0xF5);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const INK: Color = Color::rgb(0x1C, 0x1E, 0x26);
const MUTED: Color = Color::rgb(0x6B, 0x70, 0x7C);
const ACCENT: Color = Color::rgb(0x2F, 0x5B, 0xD6);
const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const RING: Color = Color::rgb(0xF2, 0x9F, 0x05);

const TITLE: &str = "Hej från Vy";
const BODY: &str =
    "Den här sidan är en WebAssembly-modul. Ändra texten, lägg till kontroller och bygg om.";
const BUTTON: &str = "Räkna";

const MARGIN: f32 = 24.0;
const PAD: f32 = 20.0;
const BUTTON_W: f32 = 120.0;
const BUTTON_H: f32 = 40.0;

/// Fixed semantic id of the button: `Event::Focus` names it. Ids a page
/// chooses are negative; automatic ones are positive.
const ID_BUTTON: i32 = -1;

/// Every rectangle of the page, computed from the window.
struct Layout {
    card: Rect,
    text_w: f32,
    title_y: f32,
    body_y: f32,
    count_y: f32,
    button: Rect,
}

struct Page {
    window: Window,
    title: StyleId,
    body: StyleId,
    label: StyleId,
    clicks: u32,
    /// The host moved keyboard focus to the button (the page draws the ring).
    focused: bool,
}

impl Page {
    fn layout(&self) -> Result<Layout, VyError> {
        let card_w = (self.window.width - 2.0 * MARGIN).max(0.0);
        let text_w = (card_w - 2.0 * PAD).max(0.0);
        // Measure first, so the card wraps its content at this width.
        let title_h = text_measure(self.title, text_w, TITLE)?.h;
        let body_h = text_measure(self.body, text_w, BODY)?.h;
        let title_y = MARGIN + PAD;
        let body_y = title_y + title_h + 12.0;
        let count_y = body_y + body_h + 16.0;
        let button = Rect::new(MARGIN + PAD, count_y + 24.0 + 12.0, BUTTON_W, BUTTON_H);
        let card_h = button.bottom() + PAD - MARGIN;
        Ok(Layout {
            card: Rect::new(MARGIN, MARGIN, card_w, card_h),
            text_w,
            title_y,
            body_y,
            count_y,
            button,
        })
    }

    fn count_text(&self) -> String {
        match self.clicks {
            0 => String::from("Inga klick än."),
            1 => String::from("Ett klick."),
            n => format!("{n} klick."),
        }
    }

    fn press(&mut self) -> bool {
        self.clicks += 1;
        true
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        Ok(Self {
            window,
            title: text_style(Font::Sans, 28.0, 600, INK)?,
            body: text_style(Font::Sans, 16.0, 400, MUTED)?,
            label: text_style(Font::Sans, 15.0, 600, WHITE)?,
            clicks: 0,
            focused: false,
        })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout()?;
        let x = l.card.x + PAD;
        f.rect(
            Rect::new(0.0, 0.0, self.window.width, self.window.height),
            0.0,
            BG,
        )?;
        f.rect(l.card, 12.0, CARD)?;
        // Every visible text and control gets a semantic node, in paint order.
        f.group(Role::Main, l.card, "", |f| {
            let h = f.text(x, l.title_y, l.text_w, self.title, TITLE)?.h;
            f.sem
                .node(Role::Heading, Rect::new(x, l.title_y, l.text_w, h), TITLE)?;
            let h = f.text(x, l.body_y, l.text_w, self.body, BODY)?.h;
            f.sem
                .node(Role::Text, Rect::new(x, l.body_y, l.text_w, h), BODY)?;
            let count = self.count_text();
            let h = f.text(x, l.count_y, l.text_w, self.body, &count)?.h;
            f.sem
                .node(Role::Text, Rect::new(x, l.count_y, l.text_w, h), &count)?;
            if self.focused {
                f.rect(l.button.inset(-3.0), 9.0, RING)?;
            }
            f.rect(l.button, 6.0, ACCENT)?;
            let tw = text_measure(self.label, 0.0, BUTTON)?.w;
            f.text(
                l.button.x + (l.button.w - tw) / 2.0,
                l.button.y + 10.0,
                0.0,
                self.label,
                BUTTON,
            )?;
            f.sem
                .node_with_id(ID_BUTTON, Role::Button, l.button, BUTTON)?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } => {
                self.layout().is_ok_and(|l| l.button.contains(x, y)) && self.press()
            }
            Event::KeyDown {
                code: key::ENTER | key::SPACE,
                ..
            } if self.focused => self.press(),
            Event::Focus { node, gained, .. } if node == NodeId(ID_BUTTON) => {
                self.focused = gained;
                true
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::mock;

    /// One frame registers the whole tree; a press on the button counts.
    #[test]
    fn first_frame_registers_the_tree() {
        mock::reset();
        let window = Window {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        };
        let mut page = Page::init(window, 0).unwrap();
        assert!(mock::run_frame(&mut page, 0).unwrap() > 0);
        mock::with(|h| assert_eq!(h.sem.len(), 5, "main, heading, two texts, button"));
        let button = page.layout().unwrap().button;
        assert!(page.event(Event::PointerDown {
            x: button.x + 1.0,
            y: button.y + 1.0,
            button: 1,
            modifiers: Modifiers(0),
        }));
        assert_eq!(page.clicks, 1);
    }
}
"#;

const MANIFEST_JSON: &str = r##"{
  "abi": "1.0",
  "name": "{display}",
  "summary": "En ny Vy-sida, byggd med vy-sdk.",
  "lang": "sv",
  "memory_max": 32,
  "published": "{date}",
  "fallback": "# {display}\n\nSidan kräver en Vy-klient."
}
"##;

const GITIGNORE: &str = "/target\n/preview*.png\n/semantic*.json\n*.vyp\n";

const CLAUDE_MD: &str = r##"# {name} – en Vy-sida

Du bygger en sida för **Vy**: en WebAssembly-modul som ritar sig själv genom ett litet värd-API.
Ingen HTML, ingen CSS, ingen JavaScript. Sidan är `src/lib.rs`, byggd på `vy-sdk`;
`manifest.json` beskriver den (namn, sammanfattning, capabilities, resurser, reservtext).
Allt du behöver står i den här filen: du behöver inget från nätet och ska inte hämta något.

## Loopen

1. Ändra `src/lib.rs` (och `manifest.json` om sidan behöver capabilities eller resurser).
2. `cargo build --release --target wasm32-unknown-unknown`
3. `vy-cli check-abi target/wasm32-unknown-unknown/release/{lib}.wasm`
4. `vy-cli render target/wasm32-unknown-unknown/release/{lib}.wasm --out preview.png --size 800x600 --dump-semantic semantic.json`
   – lägg till `--grant ime,net,storage` (de capabilities manifestet begär) och en
   `--resource <fil>` per resurs i manifestets ordning. `--size 400x700 --out preview-smal.png`
   prövar en smal vy.
5. Titta på `preview.png` (läs bilden) och `semantic.json`. Rätta och börja om från 1.
6. `vy-dev build` kontrollerar manifestet mot modulen och skriver det signerade paketet.

`cargo test` kör sidans egna tester nativt mot SDK:ts mock (ingen wasm-motor behövs).
Står `CARGO_TARGET_DIR` i miljön ligger modulen där i stället för under `target/`.
`vy-dev serve`, som den som startade dig kör, bygger om när du är klar och visar sidan på
`vy://localhost:8080/{name}`; du behöver inte starta något själv.

Du kör utan någon att fråga. Bash är begränsat till `cargo`, `vy-cli`, `vy-dev build` och `ls`;
filverktygen når bara den här katalogen. Avsluta med två meningar om vad du ändrade.

## Regler

- `#![no_std]` + `alloc` (`#![cfg_attr(not(test), no_std)]` så att `cargo test` fungerar).
  Bara `vy-sdk` som beroende; lägg inte till crates.
- Deterministisk: samma indata och fönster ger samma display list, byte för byte. Tid och slump
  bara via `now_ms` och `random`; inga egna klockor, inga statiska räknare som överlever en frame
  utan att vara tillstånd i sidan.
- Layout och tillstånd lever i modulen: en `layout()`-funktion räknar rektanglarna ur fönstret;
  `frame` ritar dem och `event` träfftestar mot samma rektanglar. Det finns ingen layoutmotor.
- En semantisk nod (`f.sem.node`) för varje synlig text och kontroll, registrerad varje frame i
  ritordning, även utanför fönstret. Fokus ägs av värden: Tab når aldrig sidan, `Event::Focus`
  säger vilken nod som har fokus och sidan ritar ringen själv. Ett nod-id sidan väljer är negativt
  (`node_with_id(-1, …)`); välj fasta id för kontroller.
- Capabilities bara genom manifestet: `"capabilities": {"ime": {}, "net": {"origins":
  ["https://api.example"]}, "storage": {"quota_kb": 64}}`. Ett anrop utan beviljad capability ger
  `Err(VyError::Denied)` och sidan ska fortsätta fungera (visa det, krascha inte). `vy-cli render`
  nekar `fetch_begin` och lagring om du inte ger `--grant`, och även då svarar inget nät.
- `ime` i manifestet så snart sidan har ett textfält (`Role::Textbox`): `ime_begin(rect)` när
  fältet får fokus, `ime_end()` när det tappar det; texten kommer som `Event::TextInput`.
- Mät text med `text_measure` före ritning så att block följer på varandra; `max_w <= 0` betyder
  ingen radbrytning. `f.text` returnerar storleken på det som ritades.
- En panik stoppar sidan. Returnera `Err` och hantera `Err` från SDK:t i stället.
- Rör inte `[profile.release]` i `Cargo.toml`: paketet ska vara litet (sikta under 300 KB).

## Klart när

- `cargo build --release --target wasm32-unknown-unknown` går utan varningar.
- `vy-cli check-abi` slutar med `ABI 1.0: OK`.
- `vy-cli render` skriver bilden utan fel i loggen (raderna på stderr), och bilden visar det
  uppdraget bad om, i både 800x600 och 400x700.
- `semantic.json` har en nod för varje synlig text och kontroll.
- `vy-dev build` går igenom: manifestet beskriver modulen (`memory_max`, resurser, capabilities).
- `cargo test` är grönt.

## API i korthet

`use vy_sdk::prelude::*;` ger `App`, `Window`, `Frame`, `Event`, `Modifiers`, `Capability`,
`Role`, `NodeId`, `Sem`, `Rect`, `Size`, `Color`, `Font`, `StyleId`, `ResourceId`, `Fit`, `Path`,
`VyError`, `Level`, `log`, `request_frame`, `text_style` och `text_measure`. Resten importeras
med namn: `use vy_sdk::{fetch_begin, fetch_poll, fetch_read, Method, state_get, state_put};`.

- `trait App`: `fn init(window: Window, abi_minor: i32) -> Result<Self, VyError>`,
  `fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError>`,
  `fn event(&mut self, event: Event<'_>) -> bool` (sant = rita om),
  `fn resize(&mut self, window: Window)`, `fn suspend(&mut self) -> i32` (valfri).
  `vy_sdk::export!(Typ);` sist i filen. `Window { width, height, scale }` i logiska pixlar.
- `Frame`: `rect(r, radius, color)`, `text(x, y, max_w, style, text) -> Size`,
  `image(r, ResourceId(i), Fit::Cover | Fit::Contain | Fit::Stretch)`, `path(&Path, color,
  stroke_w)` (`Path::new().move_to(x, y).line_to(x, y).quad_to(cx, cy, x, y).close()`),
  `clip_push(r, radius)`/`clip_pop()` eller `clipped(r, radius, |f| …)`, `group(role, r, text,
  |f| …)`, och `f.sem`: `node(role, r, text) -> NodeId`, `node_with_id(id, role, r, text)`,
  `begin`/`end`.
- `Role`: `Text`, `Heading`, `Link`, `Button`, `Textbox`, `Checkbox`, `Radio`, `List`,
  `ListItem`, `Image`, `Table`, `Row`, `Cell`, `Nav`, `Main`, `Region`, `Dialog`, `Menu`,
  `MenuItem`, `Tab`, `TabPanel`, `Progress`, `Slider`, `Separator`.
- `Event`: `PointerMove { x, y, buttons, modifiers }`, `PointerDown { x, y, button, modifiers }`,
  `PointerUp { … }`, `PointerLeave`, `Scroll { dx, dy, lines, modifiers }`,
  `KeyDown { code, modifiers, repeat }`, `KeyUp { code, modifiers }`, `TextInput { text }`,
  `ImeCompose { text, caret }`, `Focus { node, gained, keyboard }`,
  `FetchDone { request, status, bytes }`, `Visibility { visible }`,
  `Budget { used_ms, allowed_ms, last_warning }`, `Capability { cap, granted }`,
  `Unknown { kind, a, b, c, d }`. Tangentkoder (USB HID) i `vy_sdk::event::key`: `ENTER`,
  `ESCAPE`, `BACKSPACE`, `SPACE`, `DELETE`, `HOME`, `END`, `PAGE_UP`, `PAGE_DOWN`, `LEFT`,
  `RIGHT`, `UP`, `DOWN`.
- Text: `text_style(Font::Sans | Serif | Mono | Ui | Emoji | Resource(i), size_px, weight,
  color) -> StyleId` (vikt 1–1000, 400 normal, 700 fet), `text_measure(style, max_w, text) ->
  Size { w, h }`, `text_caret(style, text, byte_offset) -> (x, y)`.
- Färg och geometri: `Color::rgb(r, g, b)`, `Color::rgba(r, g, b, a)`; `Rect::new(x, y, w, h)`
  med `right()`, `bottom()`, `contains(x, y)`, `inset(d)`, `offset(dx, dy)`;
  `vy_sdk::math::{floor, round, ceil, snap}`.
- Livscykel: `abi_version()`, `log(Level::Debug | Info | Warn | Error, text)`,
  `request_frame()` (nästa frame utan händelse, för animation), `exit(code)`,
  `now_ms() -> i64`, `random(&mut buf)`, `key_down(code) -> bool`.
- Inmatning (`ime`): `ime_begin(rect)`, `ime_end()`.
- Nät (`net`): `fetch_begin(Method::Get | Post | Put | Delete, url, body) -> Request`;
  svaret kommer som `Event::FetchDone`, därefter `fetch_poll(req)` (1 = klart) och
  `fetch_read(req, &mut buf) -> usize` tills 0; `fetch_cancel(req)`. Bara origins i manifestet.
- Lagring (`storage`): `state_put(key, bytes)`, `state_get(key, &mut buf) -> usize` (hela
  längden; `Err(VyError::Resource)` om nyckeln saknas), `state_delete(key)`.
- Navigering: `navigate(url, replace)` (`vy://` eller `https://`), `link_register(id, rect, url)`
  varje frame för en klickbar länk.
- Urklipp (`clipboard`): `clipboard_read(&mut buf)`, `clipboard_write(text)`.
- Nativa tester: `vy_sdk::mock::reset()`, `mock::run_frame(&mut sida, dt) -> Result<u32, _>`
  (antal opkoder), `mock::with(|h| h.sem …)` för trädet; `h.caps.storage = true` med flera
  beviljar capabilities i mocken.

## vy-sdk

Det här är SDK:ts README, som den såg ut när `vy-dev` byggdes.

{sdk_readme}
"##;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert!(valid_name("min-sida"));
        assert!(valid_name("a1_b"));
        for bad in [
            "Min", "1a", "", "a b", "test", "std", "vy-sdk", "con", "lpt9",
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
        assert!(!valid_name(&"a".repeat(65)));
        assert_eq!(derive_name("Min Sida"), "min-sida");
        assert_eq!(derive_name("2024"), "page-2024");
        assert_eq!(derive_name("hej_då"), "hej_d");
        assert_eq!(derive_name("test"), "page-test");
        assert_eq!(derive_name("---"), "page");
        assert_eq!(lib_name("min-sida"), "min_sida");
        assert_eq!(display_name("min-sida"), "Min sida");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_date(0), "1970-01-01");
        assert_eq!(civil_date(19_723), "2024-01-01");
        assert_eq!(civil_date(20_762), "2026-11-05");
        assert_eq!(civil_date(11_016), "2000-02-29");
        assert_eq!(today().len(), 10);
    }

    /// The scaffold writes the five files with the name filled in, refuses
    /// a non-empty directory, a checkout without the SDK and a bad name,
    /// and the `CLAUDE.md` carries the loop, the API summary and the SDK
    /// README.
    #[test]
    fn writes_a_project() {
        let dir = std::env::temp_dir().join(format!("vy-dev-scaffold-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let s = Scaffold {
            dir: dir.join("min-sida"),
            name: "min-sida".into(),
            sdk: Some(root.clone()),
        };
        let files = s.create().unwrap();
        assert_eq!(files.len(), 5);
        let cargo = std::fs::read_to_string(dir.join("min-sida/Cargo.toml")).unwrap();
        assert!(cargo.contains("name = \"min-sida\""));
        assert!(cargo.contains("vy-sdk = { path = \""));
        assert!(cargo.contains("/crates/vy-sdk\" }"));
        assert!(!cargo.contains('\\'), "forward slashes only: {cargo}");
        assert!(cargo.contains("[workspace]"));
        let claude = std::fs::read_to_string(dir.join("min-sida/CLAUDE.md")).unwrap();
        assert!(claude.contains("release/min_sida.wasm"));
        assert!(claude.contains("vy://localhost:8080/min-sida"));
        assert!(claude.contains("# vy-sdk"));
        assert!(claude.contains("vy_sdk::export!(Page);"));
        assert!(!claude.contains("{lib}") && !claude.contains("{name}"));
        let lib = std::fs::read_to_string(dir.join("min-sida/src/lib.rs")).unwrap();
        assert!(lib.starts_with("//! min-sida: a Vy page"));
        let manifest = std::fs::read_to_string(dir.join("min-sida/manifest.json")).unwrap();
        let draft = vy_manifest::Draft::from_json(&manifest).unwrap();
        assert_eq!(draft.name, "Min sida");
        assert_eq!(draft.published, today());
        assert!(s.create().is_err(), "not empty");

        let git = Scaffold {
            dir: dir.join("annan"),
            name: "annan".into(),
            sdk: None,
        };
        git.create().unwrap();
        let cargo = std::fs::read_to_string(dir.join("annan/Cargo.toml")).unwrap();
        assert!(cargo.contains(&format!("vy-sdk = {{ git = \"{SDK_GIT}\" }}")));
        let bad = Scaffold {
            dir: dir.join("x"),
            name: "x".into(),
            sdk: Some(dir.clone()),
        };
        assert!(bad.create().is_err(), "no SDK there");
        assert!(!dir.join("x").exists(), "nothing written");
        assert!(Scaffold {
            dir: dir.join("y"),
            name: "Y".into(),
            sdk: None
        }
        .create()
        .is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The API summary in `CLAUDE.md` names every import the SDK exports,
    /// every event and every role, so it cannot drift from the SDK without
    /// this test failing.
    #[test]
    fn api_summary_covers_the_sdk() {
        let sdk = Path::new(env!("CARGO_MANIFEST_DIR")).join("../vy-sdk/src");
        let api = std::fs::read_to_string(sdk.join("api.rs")).unwrap();
        let mut names: Vec<String> = api
            .lines()
            .filter_map(|l| l.strip_prefix("pub fn "))
            .map(|l| l.split('(').next().unwrap().to_string())
            .collect();
        assert!(names.len() >= 20, "{names:?}");
        let events = std::fs::read_to_string(sdk.join("event.rs")).unwrap();
        let body = events.split("pub enum Event<'a> {").nth(1).unwrap();
        let body = body.split("\n}\n").next().unwrap();
        names.extend(
            body.lines()
                .map(str::trim)
                .filter(|l| l.chars().next().is_some_and(|c| c.is_ascii_uppercase()))
                .map(|l| {
                    l.split([' ', ',', '{'])
                        .next()
                        .unwrap()
                        .trim_end_matches(',')
                        .to_string()
                }),
        );
        let roles = std::fs::read_to_string(sdk.join("sem.rs")).unwrap();
        let body = roles.split("pub enum Role {").nth(1).unwrap();
        let body = body.split("\n}\n").next().unwrap();
        names.extend(
            body.lines()
                .map(str::trim)
                .filter_map(|l| l.split_once(" = role::").map(|(n, _)| n.to_string())),
        );
        for name in &names {
            assert!(
                CLAUDE_MD.contains(&format!("`{name}"))
                    || CLAUDE_MD.contains(&format!(" {name}("))
                    || CLAUDE_MD.contains(&format!("`{name}`")),
                "CLAUDE.md does not mention {name}"
            );
        }
    }
}
