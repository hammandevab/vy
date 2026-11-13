//! Vy example page `skriv` ("Anteckningar"): a Markdown editor with a
//! live preview.
//!
//! What it shows beyond the reference pages:
//!
//! - **Rich text laid out by the module.** `md.rs` parses the source into
//!   blocks of styled runs; the page wraps them itself, one
//!   `text_measure` per word (SPEC-5.2.5), into fragments that each are
//!   one `dl_text` in one style: headings, paragraphs, bold, inline code
//!   and code blocks in the monospace role, quotes, lists in two levels,
//!   rules and links. The layout is kept until the text or the width
//!   changes, so a frame that only scrolls measures nothing.
//! - **Links** drawn as links, registered with `link_register` every
//!   frame (SPEC-5.2.10) and present in the tree as `link` nodes with
//!   fixed ids: Tab reaches them, Enter follows them with `navigate`.
//! - **A multi-line `ui::TextBox`** for the source and a `ui::ScrollView`
//!   for the preview, which follows the block the caret is in.
//! - **Tabs** "Skriv" and "Läs" instead of the split below 720 px.
//! - **Storage** of the text after every change (SPEC-5.2.9); without the
//!   capability the page says "sparas inte".
//!
//! `text_style` has no slant (SPEC-5.2.5 takes a font role, a size, a
//! weight and a colour), so emphasis is set in the serif role instead of
//! in italics.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod md;

use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;
use md::{Doc, Kind, Run, BOLD, CODE, ITALIC};
use vy_sdk::event::{button, key};
use vy_sdk::math::snap;
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Input, Label, Multiline, Palette, ScrollView, TextBox, Theme};
use vy_sdk::{link_register, navigate, state_get, state_put};

const PAGE: Color = Color::rgb(0xF1, 0xF3, 0xF7);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const EDGE: Color = Color::rgb(0xC5, 0xC9, 0xD2);
const CODE_BG: Color = Color::rgb(0xEE, 0xF0, 0xF4);
const QUOTE_BAR: Color = Color::rgb(0xB9, 0xC6, 0xEE);
const TAB_OFF: Color = Color::rgb(0xE3, 0xE6, 0xED);

/// Fixed ids of the focusable nodes; link `i` of the preview has
/// `ID_LINK - i`.
const ID_EDITOR: i32 = -1;
const ID_TAB: i32 = -2; // -2, -3
const ID_LINK: i32 = -100;

/// Below this width one pane is shown at a time, behind tabs.
const NARROW: f32 = 720.0;
const MARGIN: f32 = 16.0;
const HEADER_H: f32 = 52.0;
const TAB_H: f32 = 36.0;
const STATUS_H: f32 = 36.0;
/// Padding inside the preview card.
const PAD: f32 = 18.0;
/// How far the plate behind inline code reaches past its text.
const PLATE: f32 = 3.0;
/// Longest text the editor takes, in bytes.
const MAX_TEXT: usize = 6000;
const STORE: &str = "skriv";

/// Indices into the page's text styles.
const S_BODY: usize = 0;
const S_BOLD: usize = 1;
const S_EMPH: usize = 2;
const S_CODE: usize = 3;
const S_LINK: usize = 4;
const S_H1: usize = 5;
const S_MARK: usize = 8;
const STYLES: usize = 9;

/// What a first visit shows: one of everything the preview knows.
const SAMPLE: &str = "# Anteckningar

Skriv **Markdown** till vänster och läs resultatet till höger. Texten *sparas* medan du skriver.

## Det här går att skriva

- Rubriker med `#`, `##` och `###`
- **Fet** och *betonad* text och `kod` i löptexten
  - punkter i två nivåer
  - länkar som [Vy på GitHub](https://github.com/hammandevab/vy)
- Citat, kodblock och linjer

1. Numrerade listor
2. behåller siffrorna du skriver

> Ett citat kan vara flera rader långt och bryts som vanlig text när fönstret är smalt.

### Ett kodblock

```
fn main() {
    println!(\"Hej från Vy\");
}
```

---

Mer om formatet finns på [startsidan](vy://localhost:8090/hello).";

/// One text style with what the layout needs to know about it.
#[derive(Clone, Copy)]
struct Type {
    id: StyleId,
    /// Height of the host's line box for the style.
    line: f32,
    /// Width of a space.
    space: f32,
}

/// One `dl_text` of the preview: a stretch of a block's text in one
/// style on one line, relative to the content's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frag {
    block: usize,
    start: usize,
    end: usize,
    x: f32,
    y: f32,
    w: f32,
    style: usize,
    link: Option<u16>,
    /// Inline code: drawn on a tinted plate.
    plate: bool,
}

/// Where a block landed.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Placed {
    y: f32,
    h: f32,
    /// Left edge of the block's text.
    x: f32,
    /// Distance between its lines.
    pitch: f32,
    /// Its fragments: first and one past the last.
    frags: (usize, usize),
}

/// The parsed text laid out for a width.
#[derive(Clone, Debug, Default)]
struct Preview {
    doc: Doc,
    placed: Vec<Placed>,
    frags: Vec<Frag>,
    height: f32,
}

/// Every rectangle on the page for the current window and tab.
#[derive(Clone, Copy, Debug)]
struct Layout {
    narrow: bool,
    tabs: [Rect; 2],
    editor: Rect,
    /// The preview's card and the scroll viewport inside it.
    card: Rect,
    view: Rect,
    status: Rect,
    show_editor: bool,
    show_preview: bool,
}

impl Layout {
    /// Width the preview's text wraps at.
    fn text_w(&self) -> f32 {
        (self.view.w - 2.0 * PAD - ui::BAR_W).max(60.0)
    }
}

struct Page {
    window: Window,
    /// The editor's theme: the monospace role.
    theme: Theme,
    types: [Type; STYLES],
    title: StyleId,
    status: StyleId,
    tab: StyleId,
    tab_on: StyleId,
    editor: TextBox<Multiline>,
    /// Counts every change of the text.
    revision: u32,
    preview: Preview,
    /// The revision and width `preview` was laid out for.
    laid: Option<(u32, f32)>,
    scroll: ScrollView,
    /// The pane a narrow window shows: 0 the editor, 1 the preview.
    pane: usize,
    /// The node the host's focus is on.
    focus: Option<i32>,
    /// False when the text could not be read or written: "sparas inte".
    stored: bool,
    /// The caret as the preview last followed it.
    followed: usize,
    /// A link the keyboard focused, to bring into view.
    reveal: Option<u16>,
    words: (usize, usize),
    /// Why a link could not be followed, until the next change.
    message: String,
}

/// The parts joined; the page builds its few strings without `core::fmt`.
fn cat(parts: &[&str]) -> String {
    let mut s = String::new();
    for part in parts {
        s.push_str(part);
    }
    s
}

/// `n` in decimal.
fn number(mut n: usize) -> String {
    let mut digits = [0u8; 20];
    let mut len = 0;
    loop {
        digits[len] = b'0' + (n % 10) as u8;
        len += 1;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    digits[..len].iter().rev().map(|d| *d as char).collect()
}

/// The style a run of a block is set in.
fn style_of(kind: Kind, run: &Run) -> usize {
    match kind {
        Kind::Heading(level) => S_H1 + usize::from(level.clamp(1, 3)) - 1,
        _ if run.style & CODE != 0 => S_CODE,
        _ if run.link.is_some() => S_LINK,
        _ if run.style & BOLD != 0 => S_BOLD,
        _ if run.style & ITALIC != 0 => S_EMPH,
        _ => S_BODY,
    }
}

/// A block's indent, the gap above it and the distance between its lines.
fn metrics(kind: Kind, after_item: bool) -> (f32, f32, f32) {
    match kind {
        Kind::Heading(1) => (0.0, 20.0, 36.0),
        Kind::Heading(2) => (0.0, 20.0, 30.0),
        Kind::Heading(_) => (0.0, 16.0, 26.0),
        Kind::Para => (0.0, 12.0, 24.0),
        Kind::Quote => (18.0, 12.0, 24.0),
        Kind::Bullet { nested } | Kind::Number { nested, .. } => (
            if nested { 48.0 } else { 24.0 },
            if after_item { 4.0 } else { 12.0 },
            24.0,
        ),
        Kind::Code => (12.0, 12.0, 20.0),
        Kind::Rule => (0.0, 16.0, 1.0),
    }
}

/// True for a target `navigate` and `link_register` take (SPEC-5.2.10).
fn followable(url: &str) -> bool {
    url.starts_with("https://") || url.starts_with("vy://")
}

impl Page {
    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let narrow = w < NARROW;
        let inner = (w - 2.0 * MARGIN).max(0.0);
        let mut top = HEADER_H;
        let mut tabs = [Rect::default(); 2];
        if narrow {
            tabs = ui::split::<2>(Rect::new(MARGIN, top, inner, TAB_H), 6.0);
            top += TAB_H + 8.0;
        }
        let body = Rect::new(MARGIN, top, inner, (h - top - STATUS_H).max(0.0));
        let (editor, card) = if narrow {
            (body, body)
        } else {
            let [a, b] = ui::split::<2>(body, 16.0);
            (a, b)
        };
        Layout {
            narrow,
            tabs,
            editor,
            card,
            view: card.inset(1.0),
            status: Rect::new(MARGIN, h - STATUS_H, inner, STATUS_H),
            show_editor: !narrow || self.pane == 0,
            show_preview: !narrow || self.pane == 1,
        }
    }

    fn content_h(&self) -> f32 {
        self.preview.height + 2.0 * PAD
    }

    /// Parse the text and wrap it at `width`, unless the preview is
    /// already laid out for this text and width.
    fn lay_out(&mut self, width: f32) -> Result<(), VyError> {
        if self.laid == Some((self.revision, width)) {
            return Ok(());
        }
        let doc = md::parse(self.editor.text());
        let mut frags: Vec<Frag> = Vec::new();
        let mut placed = Vec::with_capacity(doc.blocks.len());
        let mut y = 0.0;
        let mut before: Option<Kind> = None;
        for (bi, block) in doc.blocks.iter().enumerate() {
            // Items of one list sit close; a new list starts apart.
            let after_item = before.is_some_and(|b| b.is_item() && block.kind.continues(b));
            let (indent, gap, pitch) = metrics(block.kind, after_item);
            if before.is_some() {
                y += gap;
            }
            let top = y;
            let first = frags.len();
            match block.kind {
                Kind::Rule => y += pitch,
                Kind::Code => {
                    // One fragment a line, not wrapped.
                    y += 10.0;
                    let mut at = 0;
                    for line in block.text.split('\n') {
                        if !line.is_empty() {
                            frags.push(Frag {
                                block: bi,
                                start: at,
                                end: at + line.len(),
                                x: indent,
                                y,
                                w: 0.0,
                                style: S_CODE,
                                link: None,
                                plate: false,
                            });
                        }
                        at += line.len() + 1;
                        y += pitch;
                    }
                    y += 10.0;
                }
                _ => {
                    let room = (width - indent).max(40.0);
                    let mut x = 0.0;
                    for run in &block.runs {
                        let style = style_of(block.kind, run);
                        let ty = self.types[style];
                        let text = &block.text[run.start..run.end];
                        // The fragment the run's next word can join: the
                        // one before it on this line.
                        let mut open: Option<usize> = None;
                        let mut i = 0;
                        // Room for the plate around inline code.
                        let plate = style == S_CODE;
                        if plate && x > 0.0 {
                            x += PLATE;
                        }
                        while i < text.len() {
                            let rest = &text[i..];
                            let spaces = rest.len() - rest.trim_start_matches(' ').len();
                            if spaces > 0 {
                                if x > 0.0 {
                                    x += spaces as f32 * ty.space;
                                }
                                i += spaces;
                                continue;
                            }
                            let end = i + rest.find(' ').unwrap_or(rest.len());
                            let w = text_measure(ty.id, 0.0, &text[i..end])?.w;
                            if x > 0.0 && x + w > room {
                                x = 0.0;
                                y += pitch;
                                open = None;
                            }
                            match open.and_then(|k| frags.get_mut(k)) {
                                Some(frag) => {
                                    frag.end = run.start + end;
                                    frag.w = indent + x + w - frag.x;
                                }
                                None => {
                                    open = Some(frags.len());
                                    frags.push(Frag {
                                        block: bi,
                                        start: run.start + i,
                                        end: run.start + end,
                                        x: indent + x,
                                        y,
                                        w,
                                        style,
                                        link: run.link,
                                        plate,
                                    });
                                }
                            }
                            x += w;
                            i = end;
                        }
                        if plate {
                            x += PLATE;
                        }
                    }
                    y += pitch;
                }
            }
            placed.push(Placed {
                y: top,
                h: y - top,
                x: indent,
                pitch,
                frags: (first, frags.len()),
            });
            before = Some(block.kind);
        }
        self.preview = Preview {
            doc,
            placed,
            frags,
            height: y,
        };
        self.laid = Some((self.revision, width));
        Ok(())
    }

    fn save(&mut self) {
        let result = state_put(STORE, self.editor.text().as_bytes());
        if let Err(e) = result {
            let _ = log(Level::Warn, &cat(&["skriv: state_put refused: ", e.name()]));
        }
        self.stored = result.is_ok();
    }

    /// Enter on the focused link: follow it.
    fn follow(&mut self, link: usize) {
        let Some(url) = self.preview.doc.links.get(link) else {
            return;
        };
        self.message = match navigate(url, false) {
            Ok(()) => String::new(),
            Err(e) => cat(&["Länken kunde inte öppnas (", e.name(), ")."]),
        };
    }

    fn status_text(&self) -> String {
        let mut s = cat(&[
            &number(self.words.0),
            " ord · ",
            &number(self.words.1),
            " tecken",
        ]);
        if !self.stored {
            s.push_str(" · sparas inte");
        }
        if !self.message.is_empty() {
            s.push_str(" · ");
            s.push_str(&self.message);
        }
        s
    }

    /// True when the event presses the control `id` in `r`: a primary
    /// press inside it, or Enter or Space while it has the host's focus.
    fn pressed(&self, ui: &Input<'_>, id: i32, r: Rect) -> bool {
        match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. }) => {
                button == button::PRIMARY && r.contains(x, y)
            }
            Some(Event::KeyDown {
                code: key::ENTER | key::SPACE,
                repeat: false,
                ..
            }) => self.focus == Some(id),
            _ => false,
        }
    }

    /// `text` centred in `r`.
    fn centred(f: &mut Frame, r: Rect, style: StyleId, text: &str) -> Result<(), VyError> {
        let size = text_measure(style, 0.0, text)?;
        f.text(
            r.x + (r.w - size.w) / 2.0,
            r.y + (r.h - size.h) / 2.0,
            0.0,
            style,
            text,
        )?;
        Ok(())
    }

    fn draw_tabs(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let p = self.theme.palette;
        for (i, name) in ["Skriv", "Läs"].iter().enumerate() {
            let r = l.tabs[i];
            let on = i == self.pane;
            let id = ID_TAB - i as i32;
            if self.focus == Some(id) {
                f.rect(r.inset(-ui::RING), 8.0 + ui::RING, p.ring)?;
            }
            f.rect(r, 8.0, if on { CARD } else { TAB_OFF })?;
            Self::centred(f, r, if on { self.tab_on } else { self.tab }, name)?;
            if on {
                let bar = Rect::new(r.x + 12.0, r.bottom() - 4.0, (r.w - 24.0).max(0.0), 3.0);
                f.rect(bar, 1.5, p.accent)?;
            }
            let chosen = if on { self.theme.words.selected } else { "" };
            f.sem
                .node_with_id(id, Role::Tab, r, &cat(&[name, chosen]))?;
        }
        Ok(())
    }

    /// A block's marker, plate or bar: what is drawn besides its text.
    fn draw_decor(
        &self,
        f: &mut Frame,
        kind: Kind,
        at: Placed,
        origin: (f32, f32),
        width: f32,
    ) -> Result<(), VyError> {
        let p = self.theme.palette;
        let (x, y) = (origin.0, origin.1 + at.y);
        let line = snap(1.0, self.window.scale);
        match kind {
            Kind::Rule => f.rect(
                Rect::new(x, snap(y, self.window.scale), width, line),
                0.0,
                EDGE,
            ),
            Kind::Code => f.rect(Rect::new(x, y, width, at.h), 8.0, CODE_BG),
            Kind::Quote => f.rect(Rect::new(x, y, 4.0, at.h), 2.0, QUOTE_BAR),
            Kind::Bullet { nested } => {
                let dot = Rect::new(x + at.x - 15.0, y + at.pitch / 2.0 - 3.0, 6.0, 6.0);
                f.rect(dot, 3.0, p.ink)?;
                if nested {
                    f.rect(dot.inset(1.5), 1.5, CARD)?;
                }
                Ok(())
            }
            Kind::Number { n, .. } => {
                let mark = self.types[S_MARK];
                let label = cat(&[&number(usize::from(n)), "."]);
                let w = text_measure(mark.id, 0.0, &label)?.w;
                let ty = y + (at.pitch - mark.line) / 2.0;
                f.text(x + at.x - 7.0 - w, ty, 0.0, mark.id, &label)?;
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The preview inside its scroll viewport: the blocks in view drawn,
    /// every block and link registered (SPEC-10.1.1).
    fn draw_preview(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let p = self.theme.palette;
        let width = l.text_w();
        let origin = (l.view.x + PAD, l.view.y + PAD - self.scroll.offset());
        let line = snap(1.0, self.window.scale);
        let blocks = &self.preview.doc.blocks;
        // The first item of the list the walk is inside.
        let mut list: Option<Kind> = None;
        let mut links_seen = 0usize;
        for (bi, block) in blocks.iter().enumerate() {
            let at = self.preview.placed[bi];
            let area = Rect::new(origin.0, origin.1 + at.y, width, at.h);
            let visible = area.bottom() > l.view.y && area.y < l.view.bottom();
            // A run of list items of one sort is one `list` node.
            if list.is_some_and(|first| !block.kind.continues(first)) {
                f.sem.end();
                list = None;
            }
            if block.kind.is_item() && list.is_none() {
                let last = blocks[bi..]
                    .iter()
                    .position(|b| !b.kind.continues(block.kind))
                    .map_or(blocks.len(), |n| bi + n)
                    - 1;
                let end = self.preview.placed[last];
                let all = Rect::new(area.x, area.y, width, end.y + end.h - at.y);
                f.sem.begin(Role::List, all, "")?;
                list = Some(block.kind);
            }
            if visible {
                self.draw_decor(f, block.kind, at, origin, width)?;
            }
            let role = match block.kind {
                Kind::Heading(_) => Role::Heading,
                Kind::Rule => Role::Separator,
                k if k.is_item() => Role::ListItem,
                _ => Role::Text,
            };
            let text = &block.text[..block.text.floor_char_boundary(4000)];
            f.sem.node(role, area, text)?;

            let mut link_node: Option<u16> = None;
            for frag in &self.preview.frags[at.frags.0..at.frags.1] {
                let ty = self.types[frag.style];
                let x = origin.0 + frag.x;
                let y = origin.1 + frag.y + (at.pitch - ty.line) / 2.0;
                let piece = &block.text[frag.start..frag.end];
                let r = Rect::new(x, y, frag.w, ty.line);
                // Only the lines in view are drawn: a long paragraph is
                // mostly outside it.
                let visible = r.bottom() > l.view.y && r.y < l.view.bottom();
                if visible {
                    if frag.plate {
                        let plate = Rect::new(x - PLATE, y, frag.w + 2.0 * PLATE, ty.line);
                        f.rect(plate, 4.0, CODE_BG)?;
                    }
                    if let Some(link) = frag.link {
                        if self.focus == Some(ID_LINK - i32::from(link)) {
                            f.rect(r.inset(-ui::RING), 4.0, p.ring)?;
                            f.rect(r.inset(-1.0), 2.0, CARD)?;
                        }
                    }
                    f.text(x, y, 0.0, ty.id, piece)?;
                }
                let Some(link) = frag.link else {
                    continue;
                };
                if visible {
                    let under =
                        Rect::new(x, snap(y + ty.line - 2.0, self.window.scale), frag.w, line);
                    f.rect(under, 0.0, p.accent)?;
                }
                // Every piece of a wrapped link is its own rectangle for
                // the host; the link is one node, at its first piece.
                let url = &self.preview.doc.links[usize::from(link)];
                if followable(url) {
                    links_seen += 1;
                    link_register(links_seen as i32, r, url)?;
                }
                if link_node != Some(link) {
                    link_node = Some(link);
                    let whole = block
                        .runs
                        .iter()
                        .find(|run| run.link == Some(link))
                        .map_or(piece, |run| &block.text[run.start..run.end]);
                    f.sem
                        .node_with_id(ID_LINK - i32::from(link), Role::Link, r, whole)?;
                }
            }
        }
        if list.is_some() {
            f.sem.end();
        }
        Ok(())
    }

    /// The window rectangle of the first piece of link `link`.
    fn link_rect(&self, l: &Layout, link: u16) -> Option<Rect> {
        let frag = self.preview.frags.iter().find(|f| f.link == Some(link))?;
        let at = self.preview.placed[frag.block];
        Some(Rect::new(
            l.view.x + PAD + frag.x,
            l.view.y + PAD - self.scroll.offset() + frag.y,
            frag.w,
            at.pitch,
        ))
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let p = Palette::LIGHT;
        let theme = Theme::custom(p, Font::Mono, 14.0)?;
        // The stored text: its length first, then the bytes.
        let loaded = state_get(STORE, &mut []).and_then(|len| {
            let mut bytes = vec![0u8; len];
            let n = state_get(STORE, &mut bytes)?;
            bytes.truncate(n);
            Ok(String::from_utf8(bytes).ok())
        });
        let (text, stored) = match loaded {
            Ok(text) => (text, true),
            Err(VyError::Resource) => (None, true),
            Err(e) => {
                let _ = log(Level::Info, &cat(&["skriv: no storage: ", e.name()]));
                (None, false)
            }
        };
        let text = text.unwrap_or_else(|| String::from(SAMPLE));
        let style = |font, size: f32, weight, color| -> Result<Type, VyError> {
            let id = text_style(font, size, weight, color)?;
            let wide = text_measure(id, 0.0, "a b")?;
            Ok(Type {
                id,
                line: wide.h,
                space: wide.w - text_measure(id, 0.0, "ab")?.w,
            })
        };
        let mut editor = TextBox::multiline(ID_EDITOR)
            .with_max_len(MAX_TEXT)
            .with_text(&text);
        editor.select(0..0);
        let words = md::count(editor.text());
        Ok(Self {
            window,
            theme,
            types: [
                style(Font::Sans, 16.0, 400, p.ink)?,
                style(Font::Sans, 16.0, 700, p.ink)?,
                style(Font::Serif, 16.0, 400, p.ink)?,
                style(Font::Mono, 14.0, 400, p.ink)?,
                style(Font::Sans, 16.0, 400, p.accent)?,
                style(Font::Sans, 27.0, 700, p.ink)?,
                style(Font::Sans, 21.0, 700, p.ink)?,
                style(Font::Sans, 17.0, 700, p.ink)?,
                style(Font::Sans, 16.0, 400, p.muted)?,
            ],
            title: text_style(Font::Sans, 20.0, 700, p.ink)?,
            status: text_style(Font::Sans, 13.0, 400, p.muted)?,
            tab: text_style(Font::Sans, 14.0, 600, p.muted)?,
            tab_on: text_style(Font::Sans, 14.0, 600, p.accent)?,
            editor,
            revision: 0,
            preview: Preview::default(),
            laid: None,
            scroll: ScrollView::new(),
            pane: 0,
            focus: None,
            stored,
            followed: 0,
            reveal: None,
            words,
            message: String::new(),
        })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        let t = self.theme;
        if l.show_preview {
            self.lay_out(l.text_w())?;
            self.scroll.clamp(l.view, self.content_h());
            // The preview follows the block the caret is in, and a link
            // the keyboard focused.
            let caret = self.editor.caret();
            if caret != self.followed {
                self.followed = caret;
                if let Some(at) = self.preview.doc.block_at(caret) {
                    let at = self.preview.placed[at];
                    let y = l.view.y + PAD - self.scroll.offset() + at.y;
                    let block = Rect::new(l.view.x, y - PAD, l.view.w, at.h + 2.0 * PAD);
                    self.scroll.reveal(block, l.view, self.content_h());
                }
            }
            if let Some(r) = self.reveal.take().and_then(|k| self.link_rect(&l, k)) {
                self.scroll.reveal(r.inset(-8.0), l.view, self.content_h());
            }
        }
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, PAGE)?;
        f.group(Role::Main, window, "", |f| {
            let h = f.text(MARGIN, 13.0, 0.0, self.title, "Anteckningar")?.h;
            let title = Rect::new(MARGIN, 13.0, 160.0, h);
            f.sem.node(Role::Heading, title, "Anteckningar")?;
            if l.narrow {
                self.draw_tabs(f, &l)?;
            }
            if l.show_editor {
                let label = Label::hidden("Anteckningens text i Markdown");
                self.editor
                    .draw(f, l.editor, &t, label, "Skriv Markdown här …")?;
            }
            if l.show_preview {
                f.rect(l.card, ui::RADIUS, EDGE)?;
                f.rect(l.view, ui::RADIUS - 1.0, CARD)?;
                let mut scroll = self.scroll;
                let content_h = self.content_h();
                let drawn = f.group(Role::Region, l.card, "Förhandsvisning", |f| {
                    scroll.draw(f, l.view, &t, content_h, |f| self.draw_preview(f, &l))
                });
                self.scroll = scroll;
                drawn?;
            }
            let status = self.status_text();
            let size = text_measure(self.status, l.status.w, &status)?;
            let y = l.status.y + (l.status.h - size.h) / 2.0;
            f.text(l.status.x, y, l.status.w, self.status, &status)?;
            let line = Rect::new(l.status.x, y, size.w, size.h);
            f.sem.node(Role::Text, line, &status)?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let t = self.theme;
        let mut ui = Input::new(event);
        let is_focus = matches!(event, Event::Focus { .. });
        if let Event::Focus {
            node,
            gained,
            keyboard,
        } = event
        {
            if gained {
                self.focus = Some(node.0);
                // A link the keyboard reached may be out of view.
                let link = ID_LINK - node.0;
                if keyboard && (0..=i32::from(u16::MAX)).contains(&link) {
                    self.reveal = Some(link as u16);
                }
            } else if self.focus == Some(node.0) {
                self.focus = None;
            }
            ui.set_redraw();
        }
        let shown = l.show_editor || is_focus;
        if shown && self.editor.event(&mut ui, l.editor, &t).changed() {
            self.revision = self.revision.wrapping_add(1);
            self.words = md::count(self.editor.text());
            self.message.clear();
            self.save();
        }
        if l.narrow {
            for pane in 0..2 {
                if self.pressed(&ui, ID_TAB - pane as i32, l.tabs[pane]) {
                    ui.take();
                    ui.set_redraw();
                    self.pane = pane;
                }
            }
        }
        if let (
            Some(Event::KeyDown {
                code: key::ENTER, ..
            }),
            Some(focus),
        ) = (ui.unhandled(), self.focus)
        {
            let link = ID_LINK - focus;
            if l.show_preview && link >= 0 {
                ui.take();
                ui.set_redraw();
                self.follow(link as usize);
            }
        }
        if l.show_preview {
            self.scroll.event(&mut ui, l.view, &t, self.content_h());
        }
        ui.redraw()
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests;
