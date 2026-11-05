//! The chrome (SPEC-9.6.1): tab strip, address bar, markers, capability
//! badges, the capability question, the loading bar, and the views the
//! shell draws in the page area itself (new tab, loading, error,
//! fallback, stopped). All of it is a display list rasterized by
//! `vy-render` with the bundled faces, like a page; no toolkit. A page can
//! never draw here: its own display list is rasterized into its own area
//! and composited under the chrome's (see `compose`). The capability
//! question is the one piece drawn over the page area instead of above
//! it ([`draw_dialog`]): the page keeps its place, so a page cannot move
//! what is under the user's pointer by making the shell ask.
//!
//! Everything is laid out in logical pixels and depends only on the
//! [`View`], so a screenshot of the same state is the same pixels
//! (SPEC-2.1.5).

use crate::address::AddressField;
use crate::markdown::Block;
use crate::process::TabId;
use vy_abi::{font, weight};
use vy_dl::DisplayList;
use vy_render::TextEngine;

/// Height of the tab strip in logical pixels.
pub const TAB_STRIP_H: f32 = 34.0;
/// Height of the address row.
pub const ADDRESS_H: f32 = 44.0;
/// Height of the capability question, laid over the top of the page area.
pub const DIALOG_H: f32 = 40.0;
/// Height of the loading bar under the address row.
pub const LOADING_H: f32 = 3.0;
/// Width of a tab.
pub const TAB_W: f32 = 184.0;

const CHROME_BG: u32 = 0xECEB_E7FF;
const STRIP_BG: u32 = 0xDEDC_D6FF;
const TAB_ACTIVE: u32 = 0xF7F6_F2FF;
const TAB_INACTIVE: u32 = 0xE6E4_DEFF;
const INK: u32 = 0x2221_1FFF;
const MUTED: u32 = 0x6E6B_66FF;
const RULE: u32 = 0xC9C6_BEFF;
const FIELD_BG: u32 = 0xFFFF_FFFF;
const ACCENT: u32 = 0x2F5B_D6FF;
const SELECTION: u32 = 0xC7D6_F7FF;
const RESTRICTED: u32 = 0xC94A_2BFF;
const RESTRICTED_BG: u32 = 0xFBE6_E0FF;
const BADGE_BG: u32 = 0xE3EE_E9FF;
const BADGE_INK: u32 = 0x1F6F_5AFF;
const BADGE_OFF_BG: u32 = 0xEDEC_E8FF;
const DIALOG_BG: u32 = 0xFFF4_D6FF;
const BUTTON_BG: u32 = 0x2F5B_D6FF;
const BUTTON_INK: u32 = 0xFFFF_FFFF;
const BUTTON_PLAIN: u32 = 0xDAD8_D1FF;
const PAGE_BG: u32 = 0xFFFF_FFFF;
const STATUS_BG: u32 = 0x2221_1FE6;
/// The host's text selection over the page area (SPEC-10.2.2): the
/// chrome's accent, translucent.
pub const SELECTION_OVERLAY: u32 = 0x2F5B_D660;
/// Find matches over the page area.
pub const FIND_MATCH: u32 = 0xF2C9_4C80;
/// The current find match.
pub const FIND_CURRENT: u32 = 0xEE7F_1AB0;
/// Width of the find bar.
pub const FIND_W: f32 = 380.0;
/// Height of the find bar.
pub const FIND_H: f32 = 36.0;

/// A rectangle in logical pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    /// Left.
    pub x: f32,
    /// Top.
    pub y: f32,
    /// Width.
    pub w: f32,
    /// Height.
    pub h: f32,
}

impl Rect {
    /// A rectangle.
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }
    /// True when `(px, py)` is inside.
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && py >= self.y && px < self.x + self.w && py < self.y + self.h
    }
    /// Right edge.
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    /// Bottom edge.
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
}

/// One tab as the strip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabInfo {
    /// The tab.
    pub id: TabId,
    /// Its title (the manifest's name, the address, or the file name).
    pub title: String,
    /// True while its page loads.
    pub loading: bool,
}

/// What the page area shows when the page itself is not drawing there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageView<'a> {
    /// The page's own frame (composited over this area afterwards).
    Page,
    /// A new, empty tab.
    Empty,
    /// The page is being fetched.
    Loading,
    /// The page could not be loaded (SPEC-7.2.2).
    Error {
        /// Short title.
        title: &'a str,
        /// What happened.
        detail: &'a str,
        /// An address to offer to open externally.
        external: Option<&'a str>,
    },
    /// The manifest's `fallback` (SPEC-8.3.1).
    Fallback(&'a [Block]),
    /// The page stopped (a trap, `exit`, the budget) or its process is gone.
    Stopped(&'a str),
}

/// Everything the chrome draws from.
#[derive(Clone, Debug)]
pub struct View<'a> {
    /// Window width in logical pixels.
    pub width: f32,
    /// Window height in logical pixels.
    pub height: f32,
    /// The tabs in order.
    pub tabs: &'a [TabInfo],
    /// Index of the active tab.
    pub active: usize,
    /// The address field.
    pub address: &'a AddressField,
    /// Back is possible.
    pub can_back: bool,
    /// Forward is possible.
    pub can_forward: bool,
    /// The page runs in restricted mode (SPEC-7.4.3).
    pub restricted: bool,
    /// The page has no semantic tree (SPEC-10.4.1).
    pub inaccessible: bool,
    /// The page is loading (the bar under the address row).
    pub loading: bool,
    /// Capabilities requested by the page: name and whether granted (SPEC-9.2.1).
    pub badges: &'a [(&'static str, bool)],
    /// A capability the user is being asked about (SPEC-8.2.3).
    pub ask: Option<&'static str>,
    /// The page area's content.
    pub page: PageView<'a>,
    /// Font scale of the page (Ctrl+Plus/Minus); shown when not 1.
    pub zoom: f32,
    /// The find bar is open (Ctrl+F).
    pub find: bool,
}

/// Where everything ended up, for hit testing.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Layout {
    /// Height of the chrome above the page area.
    pub chrome_h: f32,
    /// The page area.
    pub page: Rect,
    /// Each tab's rectangle and its close button.
    pub tabs: Vec<(TabId, Rect, Rect)>,
    /// The new-tab button.
    pub new_tab: Rect,
    /// Back.
    pub back: Rect,
    /// Forward.
    pub forward: Rect,
    /// Reload.
    pub reload: Rect,
    /// The address field.
    pub address: Rect,
    /// Where the address text starts and its style, for caret placement.
    pub address_text_x: f32,
    /// Capability badges: name and rectangle; a click revokes or re-grants.
    pub badges: Vec<(&'static str, Rect)>,
    /// The capability question, over the top of the page area: what is
    /// under it is the shell's, not the page's.
    pub dialog: Option<Rect>,
    /// "Tillåt" in the capability question.
    pub allow: Option<Rect>,
    /// "Neka" in the capability question.
    pub deny: Option<Rect>,
    /// "Öppna externt" in an error view.
    pub external: Option<Rect>,
    /// The find bar, over the top right of the page area: what is under
    /// it is the shell's.
    pub find: Option<Rect>,
    /// The find bar's text field.
    pub find_field: Option<Rect>,
    /// Previous match.
    pub find_prev: Option<Rect>,
    /// Next match.
    pub find_next: Option<Rect>,
    /// Close the bar.
    pub find_close: Option<Rect>,
}

/// The chrome's text styles, registered once per text engine.
#[derive(Clone, Copy, Debug)]
pub struct Styles {
    ui: u16,
    ui_bold: u16,
    small: u16,
    small_muted: u16,
    small_restricted: u16,
    small_badge: u16,
    small_white: u16,
    address: u16,
    address_muted: u16,
    title: u16,
    body: u16,
    heading1: u16,
    heading2: u16,
    heading3: u16,
    status: u16,
}

impl Styles {
    /// The address field's style, for caret placement.
    pub fn address(&self) -> u16 {
        self.address
    }

    /// Register the chrome's styles in `text`.
    pub fn register(text: &mut TextEngine) -> Styles {
        let mut s = |size: f32, w: i32, color: u32| {
            text.style(font::UI, size, w, color)
                .expect("the chrome's styles fit the table")
        };
        Styles {
            ui: s(13.0, weight::NORMAL, INK),
            ui_bold: s(13.0, weight::BOLD, INK),
            small: s(12.0, weight::NORMAL, INK),
            small_muted: s(12.0, weight::NORMAL, MUTED),
            small_restricted: s(12.0, weight::BOLD, RESTRICTED),
            small_badge: s(12.0, weight::NORMAL, BADGE_INK),
            small_white: s(12.0, weight::BOLD, BUTTON_INK),
            address: s(14.0, weight::NORMAL, INK),
            address_muted: s(14.0, weight::NORMAL, MUTED),
            title: s(22.0, weight::BOLD, INK),
            body: s(15.0, weight::NORMAL, INK),
            heading1: s(26.0, weight::BOLD, INK),
            heading2: s(20.0, weight::BOLD, INK),
            heading3: s(16.0, weight::BOLD, INK),
            status: s(12.0, weight::NORMAL, BUTTON_INK),
        }
    }
}

/// Text cut to fit `max_w`, with an ellipsis when cut.
fn fit(text: &mut TextEngine, style: u16, s: &str, max_w: f32) -> String {
    let width = |text: &mut TextEngine, s: &str| text.measure(style, 0.0, s).map_or(0.0, |m| m.w);
    if width(text, s) <= max_w {
        return s.to_string();
    }
    let chars: Vec<char> = s.chars().collect();
    let mut n = chars.len();
    while n > 0 {
        n -= 1;
        let candidate: String = chars[..n].iter().collect::<String>() + "…";
        if width(text, &candidate) <= max_w {
            return candidate;
        }
    }
    "…".into()
}

/// A line of text vertically centred in `r`.
fn centred_text(dl: &mut DisplayList, text: &mut TextEngine, style: u16, r: Rect, s: &str) {
    let size = text.measure(style, 0.0, s).unwrap_or_default();
    let y = r.y + (r.h - size.h) / 2.0;
    let _ = dl.text(r.x, y, 0.0, style, s.as_bytes());
}

/// A line of text centred both ways in `r`.
fn centred_both(dl: &mut DisplayList, text: &mut TextEngine, style: u16, r: Rect, s: &str) {
    let size = text.measure(style, 0.0, s).unwrap_or_default();
    let x = r.x + (r.w - size.w) / 2.0;
    let y = r.y + (r.h - size.h) / 2.0;
    let _ = dl.text(x, y, 0.0, style, s.as_bytes());
}

/// A pill: rounded background and centred text at `(x, y)` with height
/// `h`; returns its rectangle.
fn pill(
    dl: &mut DisplayList,
    text: &mut TextEngine,
    style: u16,
    bg: u32,
    at: (f32, f32, f32),
    s: &str,
) -> Rect {
    let (x, y, h) = at;
    let size = text.measure(style, 0.0, s).unwrap_or_default();
    let r = Rect::new(x, y, size.w + 14.0, h);
    let _ = dl.rect(r.x, r.y, r.w, r.h, h / 2.0, bg);
    let _ = dl.text(
        r.x + 7.0,
        r.y + (h - size.h) / 2.0,
        0.0,
        style,
        s.as_bytes(),
    );
    r
}

/// Draw the chrome for `view` into a display list; `styles` must come
/// from the same `text`.
pub fn draw(view: &View<'_>, text: &mut TextEngine, styles: &Styles) -> (Vec<u8>, Layout) {
    let mut dl = DisplayList::new();
    dl.begin().expect("fresh list");
    let mut layout = Layout::default();
    let (w, h) = (view.width, view.height);

    // Tab strip.
    let _ = dl.rect(0.0, 0.0, w, TAB_STRIP_H, 0.0, STRIP_BG);
    let mut x = 8.0;
    let tab_w = TAB_W.min(((w - 60.0) / view.tabs.len().max(1) as f32).max(60.0));
    for (i, tab) in view.tabs.iter().enumerate() {
        let r = Rect::new(x, 6.0, tab_w, TAB_STRIP_H - 6.0);
        let active = i == view.active;
        let _ = dl.rect(
            r.x,
            r.y,
            r.w,
            r.h + 6.0,
            6.0,
            if active { TAB_ACTIVE } else { TAB_INACTIVE },
        );
        let close = Rect::new(r.right() - 24.0, r.y + 5.0, 18.0, 18.0);
        let label = if tab.loading && tab.title.is_empty() {
            "Laddar…".to_string()
        } else if tab.title.is_empty() {
            "Ny flik".to_string()
        } else {
            tab.title.clone()
        };
        let label = fit(text, styles.ui, &label, r.w - 36.0);
        centred_text(
            &mut dl,
            text,
            if active { styles.ui_bold } else { styles.ui },
            Rect::new(r.x + 10.0, r.y, r.w - 36.0, r.h),
            &label,
        );
        centred_both(&mut dl, text, styles.small_muted, close, "×");
        layout.tabs.push((tab.id, r, close));
        x += tab_w + 6.0;
    }
    layout.new_tab = Rect::new(x, 8.0, 26.0, 24.0);
    let _ = dl.rect(x, 8.0, 26.0, 24.0, 5.0, TAB_INACTIVE);
    centred_both(&mut dl, text, styles.ui, layout.new_tab, "+");

    // Address row.
    let row_y = TAB_STRIP_H;
    let _ = dl.rect(0.0, row_y, w, ADDRESS_H, 0.0, CHROME_BG);
    let button = |dl: &mut DisplayList, text: &mut TextEngine, x: f32, label: &str, on: bool| {
        let r = Rect::new(x, row_y + 8.0, 30.0, 28.0);
        let _ = dl.rect(
            r.x,
            r.y,
            r.w,
            r.h,
            5.0,
            if on { TAB_INACTIVE } else { CHROME_BG },
        );
        centred_both(
            dl,
            text,
            if on {
                styles.ui_bold
            } else {
                styles.small_muted
            },
            r,
            label,
        );
        r
    };
    layout.back = button(&mut dl, text, 8.0, "‹", view.can_back);
    layout.forward = button(&mut dl, text, 42.0, "›", view.can_forward);
    layout.reload = button(&mut dl, text, 76.0, "↻", true);

    let field = Rect::new(114.0, row_y + 7.0, (w - 122.0).max(40.0), 30.0);
    layout.address = field;
    let _ = dl.rect(field.x, field.y, field.w, field.h, 6.0, RULE);
    let _ = dl.rect(
        field.x + 1.0,
        field.y + 1.0,
        field.w - 2.0,
        field.h - 2.0,
        5.0,
        FIELD_BG,
    );
    // Markers and badges from the right, inside the field.
    let mut right = field.right() - 6.0;
    let mut mark =
        |dl: &mut DisplayList, text: &mut TextEngine, style: u16, bg: u32, label: &str| -> Rect {
            let size = text.measure(style, 0.0, label).unwrap_or_default();
            let pw = size.w + 14.0;
            let r = pill(
                dl,
                text,
                style,
                bg,
                (right - pw, field.y + 5.0, 20.0),
                label,
            );
            right -= pw + 4.0;
            r
        };
    if view.zoom != 1.0 {
        mark(
            &mut dl,
            text,
            styles.small_muted,
            BADGE_OFF_BG,
            &format!("{} %", (view.zoom * 100.0).round() as i32),
        );
    }
    if view.restricted {
        mark(
            &mut dl,
            text,
            styles.small_restricted,
            RESTRICTED_BG,
            "begränsat läge",
        );
    }
    if view.inaccessible {
        mark(
            &mut dl,
            text,
            styles.small_muted,
            BADGE_OFF_BG,
            "otillgänglig",
        );
    }
    for (name, granted) in view.badges.iter().rev() {
        let r = if *granted {
            mark(&mut dl, text, styles.small_badge, BADGE_BG, name)
        } else {
            mark(&mut dl, text, styles.small_muted, BADGE_OFF_BG, name)
        };
        layout.badges.push((name, r));
    }
    layout.badges.reverse();
    // The address text, clipped to the room left of the markers.
    let text_x = field.x + 10.0;
    layout.address_text_x = text_x;
    let room = (right - 4.0 - text_x).max(0.0);
    let _ = dl.clip_push(text_x, field.y + 1.0, room, field.h - 2.0, 0.0);
    let shown = view.address.text();
    let style = if view.address.focused() {
        styles.address
    } else if shown.is_empty() {
        styles.address_muted
    } else {
        styles.address
    };
    let line_h = 14.0 * vy_render::text::LINE_HEIGHT_FACTOR;
    let text_y = field.y + (field.h - line_h) / 2.0;
    if view.address.focused() {
        if let Some((a, b)) = view.address.selection() {
            let x0 = text.caret(style, shown, a).map_or(0.0, |c| c.0);
            let x1 = text.caret(style, shown, b).map_or(0.0, |c| c.0);
            let _ = dl.rect(text_x + x0, text_y, x1 - x0, line_h, 2.0, SELECTION);
        }
    }
    if shown.is_empty() && !view.address.focused() {
        let _ = dl.text(
            text_x,
            text_y,
            0.0,
            styles.address_muted,
            "Skriv en vy://-adress (Ctrl+L)".as_bytes(),
        );
    } else {
        let _ = dl.text(text_x, text_y, 0.0, style, shown.as_bytes());
    }
    if view.address.focused() {
        let cx = text
            .caret(style, shown, view.address.caret())
            .map_or(0.0, |c| c.0);
        let _ = dl.rect(text_x + cx, text_y + 1.0, 1.0, line_h - 2.0, 0.0, INK);
    }
    let _ = dl.clip_pop();

    let y = row_y + ADDRESS_H;
    // Loading bar.
    if view.loading {
        let _ = dl.rect(0.0, y - LOADING_H, w, LOADING_H, 0.0, TAB_INACTIVE);
        let _ = dl.rect(0.0, y - LOADING_H, w * 0.35, LOADING_H, 0.0, ACCENT);
    }
    layout.chrome_h = y;
    layout.page = Rect::new(0.0, y, w, (h - y).max(0.0));
    let page = layout.page;
    // The capability question lies over the top of the page area and
    // moves nothing: its pixels come from `draw_dialog`, composited after
    // the page; here only where it is and where its buttons are.
    if view.ask.is_some() {
        let dialog = Rect::new(page.x, page.y, page.w, DIALOG_H);
        let (allow, deny) = dialog_buttons(dialog.w);
        layout.dialog = Some(dialog);
        layout.allow = Some(Rect::new(
            dialog.x + allow.x,
            dialog.y + allow.y,
            allow.w,
            allow.h,
        ));
        layout.deny = Some(Rect::new(
            dialog.x + deny.x,
            dialog.y + deny.y,
            deny.w,
            deny.h,
        ));
    }

    // The find bar lies over the top right of the page area (below the
    // question, if there is one); its pixels come from `draw_find`.
    if view.find {
        let fw = FIND_W.min(page.w - 16.0).max(160.0);
        let top = page.y + 8.0 + if view.ask.is_some() { DIALOG_H } else { 0.0 };
        let bar = Rect::new(page.right() - fw - 8.0, top, fw, FIND_H);
        let parts = find_parts(fw);
        let at = |r: Rect| Rect::new(bar.x + r.x, bar.y + r.y, r.w, r.h);
        layout.find = Some(bar);
        layout.find_field = Some(at(parts.0));
        layout.find_prev = Some(at(parts.2));
        layout.find_next = Some(at(parts.3));
        layout.find_close = Some(at(parts.4));
    }

    // The page area, when the shell draws it.
    let _ = dl.rect(page.x, page.y, page.w, page.h, 0.0, PAGE_BG);
    let _ = dl.clip_push(page.x, page.y, page.w, page.h, 0.0);
    let margin = 32.0_f32.min(page.w / 8.0);
    let column = (page.w - 2.0 * margin).clamp(10.0, 720.0);
    match view.page {
        PageView::Page => {}
        PageView::Empty => {
            let r = Rect::new(page.x, page.y, page.w, page.h);
            let mut rr = r;
            rr.y = page.y + page.h * 0.3;
            rr.h = 40.0;
            centred_both(&mut dl, text, styles.heading1, rr, "Vy");
            rr.y += 44.0;
            rr.h = 24.0;
            centred_both(
                &mut dl,
                text,
                styles.address_muted,
                rr,
                "Ctrl+L och en vy://-adress, eller en .vyp- eller .wasm-fil.",
            );
        }
        PageView::Loading => {
            let r = Rect::new(page.x, page.y + page.h * 0.3, page.w, 30.0);
            centred_both(&mut dl, text, styles.address_muted, r, "Hämtar sidan…");
        }
        PageView::Error {
            title,
            detail,
            external,
        } => {
            let mut ty = page.y + margin;
            let _ = dl.text(page.x + margin, ty, column, styles.title, title.as_bytes());
            ty += 22.0 * vy_render::text::LINE_HEIGHT_FACTOR + 12.0;
            let size = text
                .measure(styles.body, column, detail)
                .unwrap_or_default();
            let _ = dl.text(page.x + margin, ty, column, styles.body, detail.as_bytes());
            ty += size.h + 20.0;
            if let Some(url) = external {
                let label = format!("Öppna {url} i webbläsaren");
                let label = fit(text, styles.small_white, &label, column - 24.0);
                let size = text
                    .measure(styles.small_white, 0.0, &label)
                    .unwrap_or_default();
                let b = Rect::new(page.x + margin, ty, size.w + 24.0, 30.0);
                let _ = dl.rect(b.x, b.y, b.w, b.h, 5.0, BUTTON_BG);
                centred_both(&mut dl, text, styles.small_white, b, &label);
                layout.external = Some(b);
            }
        }
        PageView::Fallback(blocks) => {
            let mut ty = page.y + margin;
            for block in blocks {
                let (style, s, indent, after) = match block {
                    Block::Heading { level: 1, text } => (styles.heading1, text.clone(), 0.0, 10.0),
                    Block::Heading { level: 2, text } => (styles.heading2, text.clone(), 0.0, 8.0),
                    Block::Heading { text, .. } => (styles.heading3, text.clone(), 0.0, 6.0),
                    Block::Paragraph(text) => (styles.body, text.clone(), 0.0, 12.0),
                    Block::Item { marker, text } => {
                        (styles.body, format!("{marker} {text}"), 16.0, 4.0)
                    }
                };
                let size = text.measure(style, column - indent, &s).unwrap_or_default();
                let _ = dl.text(
                    page.x + margin + indent,
                    ty,
                    column - indent,
                    style,
                    s.as_bytes(),
                );
                ty += size.h + after;
            }
        }
        PageView::Stopped(reason) => {
            let mut ty = page.y + margin;
            let _ = dl.text(
                page.x + margin,
                ty,
                column,
                styles.title,
                "Sidan stannade".as_bytes(),
            );
            ty += 22.0 * vy_render::text::LINE_HEIGHT_FACTOR + 12.0;
            let _ = dl.text(page.x + margin, ty, column, styles.body, reason.as_bytes());
            let size = text
                .measure(styles.body, column, reason)
                .unwrap_or_default();
            ty += size.h + 12.0;
            let _ = dl.text(
                page.x + margin,
                ty,
                column,
                styles.address_muted,
                "Andra flikar påverkas inte. F5 laddar om.".as_bytes(),
            );
        }
    }
    let _ = dl.clip_pop();
    dl.end().expect("balanced list");
    (dl.bytes().to_vec(), layout)
}

/// The find bar's field, label, previous, next and close, in the bar's
/// own coordinates for a bar `w` wide.
fn find_parts(w: f32) -> (Rect, Rect, Rect, Rect, Rect) {
    let close = Rect::new(w - 32.0, 6.0, 24.0, 24.0);
    let next = Rect::new(w - 60.0, 6.0, 24.0, 24.0);
    let prev = Rect::new(w - 88.0, 6.0, 24.0, 24.0);
    let label = Rect::new(prev.x - 92.0, 6.0, 88.0, 24.0);
    let field = Rect::new(8.0, 5.0, (label.x - 12.0).max(20.0), 26.0);
    (field, label, prev, next, close)
}

/// The find bar (Ctrl+F) `width` wide: its own display list, `(bytes,
/// width, height)` in logical pixels, composited over the page area after
/// the page and its highlights. Opaque, so a page cannot draw a fake one.
pub fn draw_find(
    text: &mut TextEngine,
    styles: &Styles,
    field: &AddressField,
    label: &str,
    width: f32,
) -> (Vec<u8>, f32, f32) {
    let (w, h) = (width.max(1.0), FIND_H);
    let (f, l, prev, next, close) = find_parts(w);
    let mut dl = DisplayList::new();
    dl.begin().expect("fresh list");
    let _ = dl.rect(0.0, 0.0, w, h, 6.0, RULE);
    let _ = dl.rect(1.0, 1.0, w - 2.0, h - 2.0, 5.0, CHROME_BG);
    let _ = dl.rect(f.x, f.y, f.w, f.h, 4.0, RULE);
    let _ = dl.rect(f.x + 1.0, f.y + 1.0, f.w - 2.0, f.h - 2.0, 3.0, FIELD_BG);
    let tx = f.x + 6.0;
    let line_h = 13.0 * vy_render::text::LINE_HEIGHT_FACTOR;
    let ty = f.y + (f.h - line_h) / 2.0;
    let shown = field.text();
    let _ = dl.clip_push(tx, f.y + 1.0, f.w - 10.0, f.h - 2.0, 0.0);
    if field.focused() {
        if let Some((a, b)) = field.selection() {
            let x0 = text.caret(styles.ui, shown, a).map_or(0.0, |c| c.0);
            let x1 = text.caret(styles.ui, shown, b).map_or(0.0, |c| c.0);
            let _ = dl.rect(tx + x0, ty, x1 - x0, line_h, 2.0, SELECTION);
        }
    }
    if shown.is_empty() {
        let _ = dl.text(tx, ty, 0.0, styles.small_muted, "Sök på sidan".as_bytes());
    } else {
        let _ = dl.text(tx, ty, 0.0, styles.ui, shown.as_bytes());
    }
    if field.focused() {
        let cx = text
            .caret(styles.ui, shown, field.caret())
            .map_or(0.0, |c| c.0);
        let _ = dl.rect(tx + cx, ty + 1.0, 1.0, line_h - 2.0, 0.0, INK);
    }
    let _ = dl.clip_pop();
    let label = fit(text, styles.small_muted, label, l.w);
    centred_text(&mut dl, text, styles.small_muted, l, &label);
    for (r, glyph) in [(prev, "‹"), (next, "›"), (close, "×")] {
        let _ = dl.rect(r.x, r.y, r.w, r.h, 4.0, TAB_INACTIVE);
        centred_both(&mut dl, text, styles.ui_bold, r, glyph);
    }
    dl.end().expect("balanced list");
    (dl.bytes().to_vec(), w, h)
}

/// "Tillåt" and "Neka" within a question `w` wide, in its own coordinates.
fn dialog_buttons(w: f32) -> (Rect, Rect) {
    (
        Rect::new(w - 168.0, 7.0, 72.0, 26.0),
        Rect::new(w - 88.0, 7.0, 72.0, 26.0),
    )
}

/// The capability question (SPEC-8.2.3) about `cap`, laid over the top
/// of the page area without moving it: its own display list, `(bytes,
/// width, height)` in logical pixels, composited after the page (see
/// `compose`). Full width, so it is the same question wherever the
/// pointer is, and opaque, so nothing a page draws shows through it.
pub fn draw_dialog(
    text: &mut TextEngine,
    styles: &Styles,
    cap: &str,
    width: f32,
) -> (Vec<u8>, f32, f32) {
    let (w, h) = (width.max(1.0), DIALOG_H);
    let mut dl = DisplayList::new();
    dl.begin().expect("fresh list");
    let _ = dl.rect(0.0, 0.0, w, h, 0.0, DIALOG_BG);
    let _ = dl.rect(0.0, h - 1.0, w, 1.0, 0.0, RULE);
    let question = format!("Sidan vill använda {} ({}).", cap_label(cap), cap);
    let question = fit(text, styles.ui, &question, (w - 200.0).max(20.0));
    centred_text(
        &mut dl,
        text,
        styles.ui,
        Rect::new(12.0, 0.0, (w - 200.0).max(20.0), h),
        &question,
    );
    let (allow, deny) = dialog_buttons(w);
    let _ = dl.rect(allow.x, allow.y, allow.w, allow.h, 5.0, BUTTON_BG);
    centred_both(&mut dl, text, styles.small_white, allow, "Tillåt");
    let _ = dl.rect(deny.x, deny.y, deny.w, deny.h, 5.0, BUTTON_PLAIN);
    centred_both(&mut dl, text, styles.small, deny, "Neka");
    dl.end().expect("balanced list");
    (dl.bytes().to_vec(), w, h)
}

/// The status pill shown over the page's bottom-left corner while the
/// pointer is on a link: its own small display list, `(bytes, width,
/// height)` in logical pixels.
pub fn draw_status(
    text: &mut TextEngine,
    styles: &Styles,
    url: &str,
    max_w: f32,
) -> (Vec<u8>, f32, f32) {
    let label = fit(text, styles.status, url, (max_w - 20.0).max(20.0));
    let size = text.measure(styles.status, 0.0, &label).unwrap_or_default();
    let (w, h) = (size.w + 16.0, 22.0);
    let mut dl = DisplayList::new();
    dl.begin().expect("fresh list");
    let _ = dl.rect(0.0, 0.0, w, h, 4.0, STATUS_BG);
    let _ = dl.text(
        8.0,
        (h - size.h) / 2.0,
        0.0,
        styles.status,
        label.as_bytes(),
    );
    dl.end().expect("balanced list");
    (dl.bytes().to_vec(), w, h)
}

/// What a capability is called when the user is asked (SPEC-8.2.3).
pub fn cap_label(name: &str) -> &'static str {
    match name {
        "net" => "nätverket",
        "storage" => "lagring",
        "clipboard" => "urklippet",
        "hires_time" => "en finare klocka",
        "ime" => "textinmatning",
        "persist_frame" => "ritning i bakgrunden",
        _ => "en okänd capability",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn view<'a>(tabs: &'a [TabInfo], address: &'a AddressField) -> View<'a> {
        View {
            width: 800.0,
            height: 600.0,
            tabs,
            active: 0,
            address,
            can_back: false,
            can_forward: false,
            restricted: false,
            inaccessible: false,
            loading: false,
            badges: &[],
            ask: None,
            page: PageView::Page,
            zoom: 1.0,
            find: false,
        }
    }

    /// SPEC-9.6.1, SPEC-8.2.3: the chrome is a display list of the shell's
    /// own with the address text in it; its layout gives the page area
    /// below the chrome and places every button; the same view draws the
    /// same bytes. A capability question lies over the top of the page
    /// area as its own list and moves neither the chrome nor the page.
    #[test]
    fn spec_9_6_1_chrome_layout_is_deterministic() {
        let mut text = TextEngine::new();
        let styles = Styles::register(&mut text);
        let tabs = vec![
            TabInfo {
                id: 1,
                title: "Hej från Vy".into(),
                loading: false,
            },
            TabInfo {
                id: 2,
                title: String::new(),
                loading: true,
            },
        ];
        let mut address = AddressField::new();
        address.set_committed("vy://docs.example/rapport");
        let v = view(&tabs, &address);
        let (bytes, layout) = draw(&v, &mut text, &styles);
        let (again, layout2) = draw(&v, &mut text, &styles);
        assert_eq!(bytes, again);
        assert_eq!(layout, layout2);
        assert_eq!(layout.chrome_h, TAB_STRIP_H + ADDRESS_H);
        assert_eq!(layout.page, Rect::new(0.0, 78.0, 800.0, 522.0));
        assert_eq!(layout.tabs.len(), 2);
        assert!(layout.tabs[0].1.contains(20.0, 20.0));
        assert!(layout.tabs[0]
            .2
            .contains(layout.tabs[0].1.right() - 15.0, 20.0));
        assert!(layout.new_tab.x > layout.tabs[1].1.right());
        assert!(layout.address.contains(300.0, 56.0));
        assert!(!layout.address.contains(300.0, 80.0));
        let ops = vy_dl::decode(&bytes).unwrap();
        let texts: Vec<String> = ops
            .iter()
            .filter_map(|op| match op {
                vy_dl::Op::Text { text, .. } => Some(String::from_utf8_lossy(text).into_owned()),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"vy://docs.example/rapport".to_string()));
        assert!(texts.contains(&"Hej från Vy".to_string()));
        assert!(texts.contains(&"Laddar…".to_string()));
        assert!(layout.allow.is_none() && layout.dialog.is_none());

        let mut asking = view(&tabs, &address);
        asking.ask = Some("clipboard");
        asking.badges = &[("net", true), ("clipboard", false)];
        asking.restricted = true;
        asking.inaccessible = true;
        asking.zoom = 1.25;
        let (asking_bytes, layout) = draw(&asking, &mut text, &styles);
        // The question moves nothing: the chrome's height and the page
        // area are those of the view without it; the question lies over
        // the top of the page area, its buttons inside it.
        assert_eq!(layout.chrome_h, TAB_STRIP_H + ADDRESS_H);
        assert_eq!(layout.page, Rect::new(0.0, 78.0, 800.0, 522.0));
        let dialog = layout.dialog.unwrap();
        assert_eq!(dialog, Rect::new(0.0, 78.0, 800.0, DIALOG_H));
        let allow = layout.allow.unwrap();
        assert!(allow.contains(800.0 - 168.0 + 10.0, 78.0 + 20.0));
        assert!(layout.deny.unwrap().x > allow.right());
        for r in [allow, layout.deny.unwrap()] {
            assert!(r.y >= dialog.y && r.bottom() <= dialog.bottom());
            assert!(r.x >= dialog.x && r.right() <= dialog.right());
        }
        assert_eq!(layout.badges.len(), 2);
        assert_eq!(layout.badges[0].0, "net");
        assert!(
            layout.badges[0].1.x < layout.badges[1].1.x,
            "in request order"
        );
        let decode = |bytes: &[u8]| -> Vec<String> {
            vy_dl::decode(bytes)
                .unwrap()
                .iter()
                .filter_map(|op| match op {
                    vy_dl::Op::Text { text, .. } => {
                        Some(String::from_utf8_lossy(text).into_owned())
                    }
                    _ => None,
                })
                .collect()
        };
        let texts = decode(&asking_bytes);
        for expected in [
            "begränsat läge",
            "otillgänglig",
            "net",
            "clipboard",
            "125 %",
        ] {
            assert!(
                texts.contains(&expected.to_string()),
                "{expected}: {texts:?}"
            );
        }
        assert!(
            !texts
                .iter()
                .any(|t| t == "Tillåt" || t.contains("urklippet")),
            "the question is not part of the chrome's list: {texts:?}"
        );
        let (dialog_bytes, dw, dh) = draw_dialog(&mut text, &styles, "clipboard", dialog.w);
        assert_eq!((dw, dh), (800.0, DIALOG_H));
        let texts = decode(&dialog_bytes);
        assert!(texts.contains(&"Tillåt".to_string()));
        assert!(texts.contains(&"Neka".to_string()));
        assert!(texts.iter().any(|t| t.contains("urklippet")));
        assert_eq!(
            draw_dialog(&mut text, &styles, "clipboard", dialog.w).0,
            dialog_bytes,
            "deterministic"
        );
        let (narrow_dialog, _, _) = draw_dialog(&mut text, &styles, "persist_frame", 150.0);
        assert!(!narrow_dialog.is_empty());

        // Error and fallback views draw their content in the page area.
        let mut err = view(&tabs, &address);
        err.page = PageView::Error {
            title: "Inte en Vy-sida",
            detail: "text/html",
            external: Some("https://docs.example/"),
        };
        let (bytes, layout) = draw(&err, &mut text, &styles);
        let ext = layout.external.unwrap();
        assert!(ext.y > layout.chrome_h);
        assert!(String::from_utf8_lossy(&bytes).contains("Inte en Vy-sida"));
        let blocks = crate::markdown::parse("# Reserv\n\nEn text.");
        let mut fb = view(&tabs, &address);
        fb.page = PageView::Fallback(&blocks);
        let (bytes, _) = draw(&fb, &mut text, &styles);
        assert!(String::from_utf8_lossy(&bytes).contains("Reserv"));
        let (status, w, h) = draw_status(&mut text, &styles, "vy://shop.example/produkt/1", 400.0);
        assert!(w > 50.0 && h == 22.0);
        assert!(String::from_utf8_lossy(&status).contains("vy://shop.example/produkt/1"));
        // A narrow window still lays out without overlap or panics.
        let mut narrow = view(&tabs, &address);
        narrow.width = 200.0;
        narrow.height = 100.0;
        let (_, layout) = draw(&narrow, &mut text, &styles);
        assert!(layout.page.h > 0.0);
        assert!(layout.address.w >= 40.0);
    }
}
