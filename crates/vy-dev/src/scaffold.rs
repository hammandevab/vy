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

/// The release tag of that repository a project made without `--sdk` pins:
/// the version of this `vy-dev`, which is the workspace version, which
/// release.yml requires to be the release's tag. A project then builds
/// against the SDK of the release whose `vy-dev` made it, not against main.
pub const SDK_TAG: &str = concat!("v", env!("CARGO_PKG_VERSION"));

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
                "{:?} is not a valid page name: lowercase letters, digits, - and _, starting with a letter, not a name cargo reserves (--name sets it; the title users see is \"name\" in manifest.json)",
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
            None => format!("vy-sdk = {{ git = \"{SDK_GIT}\", tag = \"{SDK_TAG}\" }}"),
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
//! (drawing) and `event` (hit-testing) use the same rectangles. The text
//! field and the button are `vy_sdk::ui` components: they draw themselves,
//! register their semantic nodes and handle their own events.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Button, Column, Input, Label, Row, TextBox, Theme};

const BG: Color = Color::rgb(0xF2, 0xF3, 0xF5);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);

const TITLE: &str = "Hej från Vy";
const BODY: &str = "Den här sidan är en WebAssembly-modul. Skriv ditt namn och tryck Enter eller Hälsa – och ändra sedan koden.";
const LABEL: &str = "Ditt namn";

const MARGIN: f32 = 24.0;
const PAD: f32 = 20.0;
const BUTTON_W: f32 = 120.0;

/// Fixed semantic ids of the controls, in Tab order: `Event::Focus` names
/// them. Ids a page chooses are negative; automatic ones are positive.
const ID_NAME: i32 = -1;
const ID_GREET: i32 = -2;

/// Every rectangle of the page, computed from the window.
struct Layout {
    card: Rect,
    title: Rect,
    body: Rect,
    label: Rect,
    name: Rect,
    greet: Rect,
    greeting: Rect,
}

struct Page {
    window: Window,
    theme: Theme,
    title: StyleId,
    body: StyleId,
    label: StyleId,
    name: TextBox,
    greet: Button,
    greeting: String,
}

impl Page {
    fn layout(&self) -> Result<Layout, VyError> {
        let area = Rect::new(
            MARGIN,
            MARGIN,
            (self.window.width - 2.0 * MARGIN).clamp(0.0, 640.0),
            0.0,
        );
        let mut c = Column::new(area, PAD, 12.0);
        let w = c.width();
        // Measure first, so the card wraps its content at this width.
        let title = c.next(text_measure(self.title, w, TITLE)?.h);
        let body = c.next(text_measure(self.body, w, BODY)?.h);
        c.space(8.0);
        let label = c.next(text_measure(self.label, w, LABEL)?.h);
        let mut row = Row::new(c.next(ui::CONTROL_H), 0.0, 12.0);
        let name = row.next((w - BUTTON_W - 12.0).max(0.0));
        let greet = row.next(BUTTON_W);
        let greeting = c.next(self.theme.line_h);
        let card = Rect::new(area.x, area.y, area.w, c.height());
        Ok(Layout {
            card,
            title,
            body,
            label,
            name,
            greet,
            greeting,
        })
    }

    /// The button was pressed, or Enter typed in the field.
    fn press(&mut self) {
        let who = self.name.text().trim();
        self.greeting = if who.is_empty() {
            String::from("Skriv ett namn först.")
        } else {
            format!("Hej, {who}!")
        };
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        // The theme registers the components' text styles once.
        let theme = Theme::new()?;
        let p = theme.palette;
        Ok(Self {
            window,
            title: text_style(Font::Sans, 28.0, 600, p.ink)?,
            body: text_style(Font::Sans, 16.0, 400, p.muted)?,
            label: text_style(Font::Sans, 13.0, 600, p.muted)?,
            theme,
            name: TextBox::new(ID_NAME).with_max_len(100),
            greet: Button::new(ID_GREET),
            greeting: String::from("Ingen hälsning än."),
        })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout()?;
        let x = l.card.x + PAD;
        let t = self.theme;
        f.rect(
            Rect::new(0.0, 0.0, self.window.width, self.window.height),
            0.0,
            BG,
        )?;
        f.rect(l.card, 12.0, CARD)?;
        // Every visible text and control gets a semantic node, in paint order.
        f.group(Role::Main, l.card, "", |f| {
            f.text(x, l.title.y, l.title.w, self.title, TITLE)?;
            f.sem.node(Role::Heading, l.title, TITLE)?;
            f.text(x, l.body.y, l.body.w, self.body, BODY)?;
            f.sem.node(Role::Text, l.body, BODY)?;
            // A field's label is drawn here; the field registers it as its
            // name, the node just before its own.
            f.text(x, l.label.y, l.label.w, self.label, LABEL)?;
            let label = Label::shown(l.label, LABEL);
            self.name.draw(f, l.name, &t, label, "Förnamn")?;
            self.greet.draw(f, l.greet, &t, "Hälsa")?;
            f.text(x, l.greeting.y, l.greeting.w, t.text, &self.greeting)?;
            f.sem.node(Role::Text, l.greeting, &self.greeting)?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let Ok(l) = self.layout() else {
            return false;
        };
        let t = self.theme;
        // Every component gets the event; the one that uses it takes it.
        let mut ui = Input::new(event);
        let enter = self.name.event(&mut ui, l.name, &t).activated();
        let pressed = self.greet.event(&mut ui, l.greet, &t).activated();
        if enter || pressed {
            self.press();
        }
        ui.redraw()
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::event::key;
    use vy_sdk::mock;

    fn page() -> Page {
        mock::reset();
        let window = Window {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        };
        Page::init(window, 0).unwrap()
    }

    /// One frame registers the whole tree, the controls with their fixed ids.
    #[test]
    fn first_frame_registers_the_tree() {
        let mut page = page();
        assert!(mock::run_frame(&mut page, 0).unwrap() > 0);
        mock::with(|h| {
            let roles: Vec<i32> = h.sem.iter().map(|n| n.role).collect();
            assert_eq!(
                roles.len(),
                7,
                "main, heading, two texts, textbox, button, text"
            );
            // The field's name is the node before it: its label.
            let name = h.sem.iter().position(|n| n.id == ID_NAME).unwrap();
            assert_eq!(h.sem[name].role, Role::Textbox as i32);
            assert_eq!(h.sem[name - 1].text, LABEL);
            assert!(h
                .sem
                .iter()
                .any(|n| n.id == ID_GREET && n.role == Role::Button as i32));
        });
    }

    /// Typed text lands in the focused field; Enter and the button greet.
    #[test]
    fn type_and_greet() {
        let mut page = page();
        let focus = Event::Focus {
            node: NodeId(ID_NAME),
            gained: true,
            keyboard: true,
        };
        assert!(page.event(focus));
        assert!(page.event(Event::TextInput { text: "Anna" }));
        let enter = Event::KeyDown {
            code: key::ENTER,
            modifiers: Modifiers(0),
            repeat: false,
        };
        assert!(page.event(enter));
        assert_eq!(page.greeting, "Hej, Anna!");
        page.name.set_text("");
        let button = page.layout().unwrap().greet;
        assert!(page.event(Event::PointerDown {
            x: button.x + 1.0,
            y: button.y + 1.0,
            button: 1,
            modifiers: Modifiers(0),
        }));
        assert_eq!(page.greeting, "Skriv ett namn först.");
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
  "capabilities": {"ime": {}},
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
4. `vy-cli render target/wasm32-unknown-unknown/release/{lib}.wasm --out preview.png --size 800x600 --grant ime --dump-semantic semantic.json`
   – `--grant` med de capabilities manifestet begär (`ime,net,storage`, …) och en
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
  `frame` ritar dem och `event` träfftestar mot samma rektanglar. Det finns ingen layoutmotor;
  `ui::Column`, `ui::Row` och `ui::split` staplar rektanglar.
- Använd komponenterna i `vy_sdk::ui` (textfält, knappar, kryssrutor, radioknappar, slider, lista,
  rullning) i stället för att rita egna kontroller; se *Komponenter* nedan.
- En semantisk nod (`f.sem.node`) för varje synlig text och kontroll, registrerad varje frame i
  ritordning, även utanför fönstret. Fokus ägs av värden: Tab når aldrig sidan, `Event::Focus`
  säger vilken nod som har fokus och sidan ritar ringen själv. Ett nod-id sidan väljer är negativt
  (`node_with_id(-1, …)`); välj fasta id för kontroller.
- Capabilities bara genom manifestet: `"capabilities": {"ime": {}, "net": {"origins":
  ["https://api.example"]}, "storage": {"quota_kb": 64}}`. Ett anrop utan beviljad capability ger
  `Err(VyError::Denied)` och sidan ska fortsätta fungera (visa det, krascha inte). `vy-cli render`
  nekar `fetch_begin` och lagring om du inte ger `--grant`, och även då svarar inget nät.
- `ime` i manifestet så snart sidan har ett textfält (`ui::TextBox`, `Role::Textbox`); startsidan
  begär det redan. Fältet sköter `ime_begin`/`ime_end` och `Event::TextInput` själv.
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

## Komponenter (`vy_sdk::ui`)

Allt finns i `vy_sdk::ui`; importera det sidan använder, t.ex. `use vy_sdk::ui::{self, Button, Input, Label, TextBox, Theme};`
(en import som inte används är en varning, och bygget ska gå utan varningar).

| Komponent | Nod | Gör |
| --- | --- | --- |
| `TextBox::new(id)`, `TextBox::multiline(id)` (typ `TextBox<Multiline>`) | `text` (fältets namn) + `textbox` | markör, markering (Skift, pekaren), Ctrl+A/C/X, inklistring, IME, radbrytning; `.text()`, `.set_text()`, `.with_max_len(n)`; `draw(f, r, &theme, Label::shown(etikettens_rektangel, "Namn"), platshållare)` för en etikett sidan ritar, `Label::hidden("Sök")` för ett namn som inte ritas |
| `Button::new(id)`, `Button::secondary(id)` | `button` | tryck, Enter, mellanslag |
| `Checkbox::new(id, checked)` | `checkbox` | tryck, mellanslag; `.checked` |
| `RadioGroup::new(first_id, n, selected)` | `radio` × n (id `first`, `first − 1`, …) | tryck, mellanslag; `.selected` |
| `Slider::new(id, min, max, value)` | `slider` ("Volym: 40") | dra, pilar, Page Up/Down, Home/End; `.value()` |
| `List::new(row_h)` | `list` + `listitem` per rad | ritar bara raderna i bild; pilar, Enter; `.selected()` |
| `ScrollView::new()` | – | klipper, hjul, tangenter, dragbar list; `.offset()`, `.reveal(r, vy, höjd)` |
| `Column::new(område, marginal, mellanrum)`, `Row::new(…)`, `ui::split::<N>(område, mellanrum)` | – | `next(h)`/`next(w)` ger nästa rektangel, `height()` vad ett kort behöver |

- Ett tema, byggt i `init`: `Theme::new()?` (ljust, svenska tillståndsord, 16 px). Andra färger:
  `Theme::custom(Palette { accent: …, ..Palette::LIGHT }, Font::Sans, 16.0)?`; `theme.words = Words::EN`.
- Samma form för alla: `x.draw(f, rektangel, &theme, …)` i `frame`,
  `x.event(&mut ui, rektangel, &theme, …) -> Response` i `event`, med samma rektangel.
  `List` tar raderna (`&items[..]` eller en egen `impl Rows`), `ScrollView` innehållets höjd.
- I `App::event`: `let mut ui = Input::new(event);`, ge `&mut ui` till varje komponent – inre före
  yttre (en lista eller ett fält i en `ScrollView` före den) – och returnera `ui.redraw()`. En
  komponent som använder en pekar-, tangent- eller texthändelse tar den; fokus tas aldrig. Sidans
  egna tangenter: `ui.unhandled()`.
- `Response`: `.activated()` (knapp, Enter i ett enradsfält, öppnad rad), `.changed()`, `.redraw()`.
- Fasta negativa id för allt fokuserbart (`-1`, `-2`, …) i den ordning Tab ska gå. Komponenten
  ritar sin fokusring själv.
- `List` och `ScrollView` har ingen fokuserbar roll: de tar tangenter efter ett tryck i dem, eller
  efter `set_active(true)`. När Tab flyttar fokus till en kontroll utanför bild i en `ScrollView`:
  `scroll.reveal(rektangel, vy, höjd)`.
- Rita ett fälts etikett själv ovanför med `f.text` och registrera den som `Role::Text`, före fältet.
- `ScrollView::draw` tar en closure; eftersom den är `Copy`: `let mut s = self.scroll;
  s.draw(f, vy, &t, höjd, |f| self.draw_content(f, &l))?; self.scroll = s;`.

## Determinism och budget

- Samma modul, indata och fönster ger samma display list, byte för byte. Tid bara via `now_ms()`
  och `dt_ms`, slump bara via `random()`; ingen `HashMap`, inga globala räknare.
- `frame` har 8 ms logisk tid, och värdens textformning räknas in; rasteriseringen av listan har
  samma budget räknad i pixlar. Rita bara det som syns, mät inte samma text många gånger per frame,
  sätt tak på listor och fält. `vy-dev build` varnar för en sida över budgeten.

## Vanliga misstag

| Misstag | Följd | Rätt |
| --- | --- | --- |
| Textfält utan `"ime": {}` i manifestet | sidan avvisas av konformanssviten; IME nekas | begär `ime` |
| Bild utan `w`/`h` i manifestet, eller fel storlek | manifestet valideras inte, bilden avvisas (`E_RESOURCE`) | bildens verkliga storlek i pixlar |
| Glömd semantisk nod | texten finns inte för skärmläsare, sök och markering | `f.sem.node(…)` för varje text; komponenterna för kontroller |
| Automatiskt id på en kontroll | fokus hamnar fel när en nod läggs till före | fasta negativa id |
| Obegränsad allokering per frame | minnet tar slut, budgeten spricker | tak på data, rita bara det som syns |
| `event` returnerar `false` efter en ändring | ingen ny bild | `ui.redraw()`, eller `true` när sidan ändrat något |
| `request_frame()` utan animation | sidan bränner CPU | bara medan något rör sig |
| `unwrap` och panik | fliken stoppas | returnera och hantera `Err` |
| Layout räknad på två ställen | träffytor och ritning glider isär | en `layout()` |

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
  `KeyDown { code, modifiers, repeat }`, `KeyUp { code, modifiers }` (`code: i32`, `vy_sdk::event::key`), `TextInput { text }`,
  `ImeCompose { text, caret }`, `Focus { node, gained, keyboard }`,
  `FetchDone { request, status, bytes }`, `Visibility { visible }`,
  `Budget { used_ms, allowed_ms, last_warning }`, `Capability { cap, granted }`,
  `Unknown { kind, a, b, c, d }`. Tangentkoder (USB HID) i `vy_sdk::event::key`: `ENTER`,
  `ESCAPE`, `BACKSPACE`, `SPACE`, `DELETE`, `HOME`, `END`, `PAGE_UP`, `PAGE_DOWN`, `LEFT`,
  `RIGHT`, `UP`, `DOWN`.
- Text: `text_style(Font::Sans | Serif | Mono | Ui | Emoji | Resource(i), size_px, weight,
  color) -> StyleId` (vikt 1–1000, 400 normal, 700 fet), `text_measure(style, max_w, text) ->
  Size { w, h }`, `text_caret(style, text, byte_offset) -> (x, y)`;
  `f.text_centred(r, style, text)` ritar en rad mitt i en rektangel.
- Tal som text utan `format!` (som kostar modulen omkring 4 KB): `vy_sdk::num::push_int(&mut s,
  v)` och `push_decimal(&mut s, v, decimaler, ',')` för fixtal (`1995` med två decimaler blir
  `19,95`).
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
- Komponenter: `vy_sdk::ui` (se *Komponenter* ovan) – `Theme`, `Palette`, `Words`, `Input`,
  `Response`, `TextBox` (radläget `Lines`: `OneLine` som standard, `Multiline`), `Button`
  (`ButtonKind`), `Checkbox`, `RadioGroup`, `Slider`, `List`, `Rows`, `ScrollView`, `Column`, `Row`,
  `split`, `CONTROL_H` (40, höjden på ett fält eller en knapp).
- Nativa tester: `vy_sdk::mock::reset()`, `mock::run_frame(&mut sida, dt) -> Result<u32, _>`
  (antal opkoder), `mock::with(|h| h.sem …)` för trädet; `h.caps.storage = true` med flera
  beviljar capabilities i mocken; `mock::as_copy_gesture(|| …)` kör en händelse under
  kopieringsgesten (Ctrl+C).

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
    /// A project without `--sdk` pins the SDK to the release tag of this
    /// `vy-dev`: `v` and the version in `[workspace.package]` of the root
    /// `Cargo.toml`, which release.yml requires the tag to be.
    #[test]
    fn sdk_tag_is_the_release_version() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let manifest = std::fs::read_to_string(root.join("Cargo.toml")).unwrap();
        let section = manifest
            .split("[workspace.package]")
            .nth(1)
            .expect("[workspace.package]");
        let version = section
            .lines()
            .find_map(|l| l.strip_prefix("version = \""))
            .and_then(|v| v.strip_suffix('"'))
            .expect("version in [workspace.package]");
        assert_eq!(SDK_TAG, format!("v{version}"));
        let dir = std::env::temp_dir().join(format!("vy-dev-sdk-tag-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        Scaffold {
            dir: dir.join("sida"),
            name: "sida".into(),
            sdk: None,
        }
        .create()
        .unwrap();
        let cargo = std::fs::read_to_string(dir.join("sida/Cargo.toml")).unwrap();
        let line = format!(
            "vy-sdk = {{ git = \"https://github.com/hammandevab/vy\", tag = \"v{version}\" }}"
        );
        assert!(cargo.lines().any(|l| l == line), "{line} in {cargo}");
        let _ = std::fs::remove_dir_all(&dir);
    }

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
        assert!(cargo.contains(&format!(
            "vy-sdk = {{ git = \"{SDK_GIT}\", tag = \"{SDK_TAG}\" }}"
        )));
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
        // The components: every type and function `vy_sdk::ui` re-exports
        // (constants aside).
        let ui = std::fs::read_to_string(sdk.join("ui/mod.rs")).unwrap();
        for line in ui.lines().filter_map(|l| l.strip_prefix("pub use ")) {
            let list = line.split_once("::").map_or(line, |(_, rest)| rest);
            names.extend(
                list.trim_matches(|c| c == '{' || c == '}' || c == ';')
                    .split(',')
                    .map(str::trim)
                    .filter(|n| n.chars().any(|c| c.is_ascii_lowercase()))
                    .map(str::to_string),
            );
        }
        assert!(names.iter().any(|n| n == "TextBox"), "{names:?}");
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
