//! # settings
//!
//! Vy reference module 6 (`settings`): switches, radio groups, a slider the
//! pointer drags, a reset button, and values that persist through the host.
//!
//! What the page exercises in the ABI:
//! - `state_get` on init and `state_put` on every change (SPEC-5.2.9,
//!   capability `storage`), plus `state_delete` from the reset button and a
//!   final `state_put` in `vy_suspend` (SPEC-5.1.5). The value is five bytes
//!   with a version prefix; a refusal (`E_DENIED` without the capability or
//!   in a headless host, SPEC-5.3.1) leaves the defaults in place and says so
//!   in a status line, a missing key (`E_RESOURCE`) is "nothing saved yet".
//! - `POINTER_DOWN`/`POINTER_MOVE`/`POINTER_UP` as a drag gesture on the
//!   slider (SPEC-A.2.1), and hit-testing of every control from the same
//!   arithmetic the frame draws with.
//! - `KEY_DOWN` for Space, arrows, Home/End on the focused control, and
//!   `KIND_FOCUS` when the host moves focus with Tab, Shift+Tab or a pointer
//!   press (SPEC-10.3.1); the ring is the page's (SPEC-10.3.3); node ids
//!   come from the draw order and are recorded per frame.
//! - A semantic tree with `checkbox`, `radio`, `slider`, `button` and
//!   `region` nodes whose text carries the current value (SPEC-10.1.1).
//! - Visible state: the dark switch swaps the palette, the compact switch
//!   tightens the rows, the text-size radios change the preview, the slider
//!   moves. Requires capability `storage`.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::event::key;
use vy_sdk::prelude::*;
use vy_sdk::{state_delete, state_get, state_put};

/// Storage key; the value's first byte is the format version.
const KEY: &str = "settings.v1";
const FORMAT: u8 = 1;

const TITLE: &str = "Inställningar";
const RESET: &str = "Återställ standard";
const PREVIEW: &str = "Så här ser brödtexten ut med den valda storleken.";
const SECTIONS: [&str; 4] = ["Utseende", "Text", "Startsida", "Ljud"];
const SWITCHES: [&str; 3] = ["Mörkt tema", "Kompakt läge", "Animationer"];
const SIZES: [&str; 3] = ["Liten", "Normal", "Stor"];
const HOMES: [&str; 3] = ["Senaste flikarna", "Tom sida", "Bokmärken"];
const VOLUME: &str = "Volym";

const MARGIN: f32 = 24.0;
const MAX_CARD: f32 = 560.0;
/// Windows at least this wide get the four cards in two columns.
const TWO_COLUMNS_MIN: f32 = 720.0;
const MAX_WIDE: f32 = 880.0;
const PAD: f32 = 16.0;
const CARD_GAP: f32 = 16.0;
const SWITCH_W: f32 = 44.0;
const SWITCH_H: f32 = 24.0;
const RADIO: f32 = 20.0;
const SLIDER_H: f32 = 6.0;
const KNOB: f32 = 20.0;
const BUTTON_H: f32 = 40.0;
const SLIDER_STEP: u8 = 5;
/// Space kept around a control scrolled into view.
const VIEW_PAD: f32 = 24.0;

/// Everything the page persists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Values {
    dark: bool,
    compact: bool,
    animations: bool,
    /// Index into `SIZES`.
    size: u8,
    /// Index into `HOMES`.
    home: u8,
    /// 0..=100.
    volume: u8,
}

impl Values {
    const DEFAULT: Values = Values {
        dark: false,
        compact: false,
        animations: true,
        size: 1,
        home: 0,
        volume: 40,
    };

    fn encode(self) -> [u8; 5] {
        let flags =
            u8::from(self.dark) | u8::from(self.compact) << 1 | u8::from(self.animations) << 2;
        [FORMAT, flags, self.size, self.home, self.volume]
    }

    fn decode(bytes: &[u8]) -> Option<Values> {
        let [FORMAT, flags, size, home, volume] = *bytes else {
            return None;
        };
        if size > 2 || home > 2 || volume > 100 {
            return None;
        }
        Some(Values {
            dark: flags & 1 != 0,
            compact: flags & 2 != 0,
            animations: flags & 4 != 0,
            size,
            home,
            volume,
        })
    }
}

/// What the host said about storage at init.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Storage {
    /// Saved values were read.
    Loaded,
    /// Nothing saved yet; defaults shown.
    Empty,
    /// The host refused; defaults shown and changes will not be kept.
    Unavailable(VyError),
}

/// Keyboard focus stops in document order (the host's Tab order).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Control {
    Switch(usize),
    Size(usize),
    Home(usize),
    Volume,
    Reset,
}

const STOPS: usize = SWITCHES.len() + SIZES.len() + HOMES.len() + 2;

impl Control {
    fn nth(i: usize) -> Control {
        let mut i = i;
        if i < SWITCHES.len() {
            return Control::Switch(i);
        }
        i -= SWITCHES.len();
        if i < SIZES.len() {
            return Control::Size(i);
        }
        i -= SIZES.len();
        if i < HOMES.len() {
            return Control::Home(i);
        }
        if i == HOMES.len() {
            Control::Volume
        } else {
            Control::Reset
        }
    }

    fn index(self) -> usize {
        match self {
            Control::Switch(i) => i,
            Control::Size(i) => SWITCHES.len() + i,
            Control::Home(i) => SWITCHES.len() + SIZES.len() + i,
            Control::Volume => SWITCHES.len() + SIZES.len() + HOMES.len(),
            Control::Reset => STOPS - 1,
        }
    }
}

struct Palette {
    bg: Color,
    card: Color,
    ink: Color,
    muted: Color,
    rule: Color,
    accent: Color,
    track: Color,
    knob: Color,
}

const LIGHT: Palette = Palette {
    bg: Color::rgb(0xF2, 0xF3, 0xF5),
    card: Color::rgb(0xFF, 0xFF, 0xFF),
    ink: Color::rgb(0x1C, 0x1E, 0x26),
    muted: Color::rgb(0x6B, 0x70, 0x7C),
    rule: Color::rgb(0xDE, 0xE1, 0xE6),
    accent: Color::rgb(0x2F, 0x5B, 0xD6),
    track: Color::rgb(0xC9, 0xCD, 0xD6),
    knob: Color::rgb(0xFF, 0xFF, 0xFF),
};

const DARK: Palette = Palette {
    bg: Color::rgb(0x14, 0x16, 0x1C),
    card: Color::rgb(0x1F, 0x22, 0x2B),
    ink: Color::rgb(0xEC, 0xEE, 0xF2),
    muted: Color::rgb(0x9A, 0xA0, 0xAE),
    rule: Color::rgb(0x33, 0x37, 0x44),
    accent: Color::rgb(0x6F, 0x95, 0xFF),
    track: Color::rgb(0x3A, 0x3F, 0x4E),
    knob: Color::rgb(0xEC, 0xEE, 0xF2),
};

const RING: Color = Color::rgb(0xF2, 0x9F, 0x05);

/// Per-palette text styles.
#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    status: StyleId,
    heading: StyleId,
    label: StyleId,
    value: StyleId,
    button: StyleId,
    preview: [StyleId; 3],
}

/// Rectangles of every control for the current values and window.
struct Layout {
    card: [Rect; 4],
    switches: [Rect; 3],
    sizes: [Rect; 3],
    homes: [Rect; 3],
    preview: Rect,
    /// The full slider track.
    track: Rect,
    reset: Rect,
    status_y: f32,
    content_h: f32,
}

struct SettingsPage {
    window: Window,
    light: Styles,
    dark: Styles,
    values: Values,
    storage: Storage,
    /// Outcome of the last `state_put`, once something changed.
    last_save: Option<Result<(), VyError>>,
    focus: Option<Control>,
    dragging: bool,
    scroll: f32,
    /// Node id each focus stop got in the last frame (SPEC-10.3.1).
    node_ids: [i32; STOPS],
}

impl SettingsPage {
    fn palette(&self) -> &'static Palette {
        if self.values.dark {
            &DARK
        } else {
            &LIGHT
        }
    }

    fn styles(&self) -> Styles {
        if self.values.dark {
            self.dark
        } else {
            self.light
        }
    }

    fn row_h(&self) -> f32 {
        if self.values.compact {
            36.0
        } else {
            44.0
        }
    }

    fn layout(&self) -> Layout {
        let two = self.window.width >= TWO_COLUMNS_MIN;
        let max_w = if two { MAX_WIDE } else { MAX_CARD };
        let w = (self.window.width - 2.0 * MARGIN).clamp(0.0, max_w);
        let x = ((self.window.width - w) / 2.0).max(MARGIN);
        let col_w = if two { (w - CARD_GAP) / 2.0 } else { w };
        let inner_w = (col_w - 2.0 * PAD).max(0.0);
        let row = self.row_h();
        let head = 30.0;
        let top = MARGIN + 32.0 + 20.0 + 12.0 - self.scroll;

        let h_switches = PAD + head + 3.0 * row + PAD;
        let preview_h = (18.0 + 4.0 * self.values.size as f32) * 1.25 * 2.0;
        let h_text = PAD + head + 3.0 * row + 6.0 + preview_h + PAD;
        let h_homes = h_switches;
        let h_sound = PAD + head + 20.0 + SLIDER_H + 20.0 + PAD;

        let card = if two {
            let c0 = Rect::new(x, top, col_w, h_switches);
            let c2 = Rect::new(x, c0.bottom() + CARD_GAP, col_w, h_homes);
            let c1 = Rect::new(x + col_w + CARD_GAP, top, col_w, h_text);
            let c3 = Rect::new(c1.x, c1.bottom() + CARD_GAP, col_w, h_sound);
            [c0, c1, c2, c3]
        } else {
            let c0 = Rect::new(x, top, col_w, h_switches);
            let c1 = Rect::new(x, c0.bottom() + CARD_GAP, col_w, h_text);
            let c2 = Rect::new(x, c1.bottom() + CARD_GAP, col_w, h_homes);
            let c3 = Rect::new(x, c2.bottom() + CARD_GAP, col_w, h_sound);
            [c0, c1, c2, c3]
        };
        let bottom = card[2].bottom().max(card[3].bottom());
        let reset = Rect::new(x, bottom + CARD_GAP, w, BUTTON_H);
        let status_y = reset.bottom() + 12.0;

        let mut switches = [Rect::default(); 3];
        for (i, r) in switches.iter_mut().enumerate() {
            *r = Rect::new(
                card[0].right() - PAD - SWITCH_W,
                card[0].y + PAD + head + i as f32 * row + (row - SWITCH_H) / 2.0,
                SWITCH_W,
                SWITCH_H,
            );
        }
        let mut sizes = [Rect::default(); 3];
        for (i, r) in sizes.iter_mut().enumerate() {
            *r = Rect::new(
                card[1].x + PAD,
                card[1].y + PAD + head + i as f32 * row,
                inner_w,
                row,
            );
        }
        let preview = Rect::new(
            card[1].x + PAD,
            card[1].y + PAD + head + 3.0 * row + 6.0,
            inner_w,
            preview_h,
        );
        let mut homes = [Rect::default(); 3];
        for (i, r) in homes.iter_mut().enumerate() {
            *r = Rect::new(
                card[2].x + PAD,
                card[2].y + PAD + head + i as f32 * row,
                inner_w,
                row,
            );
        }
        let track = Rect::new(
            card[3].x + PAD,
            card[3].y + PAD + head + 20.0,
            (inner_w - 56.0).max(40.0),
            SLIDER_H,
        );
        Layout {
            card,
            switches,
            sizes,
            homes,
            preview,
            track,
            reset,
            status_y,
            content_h: status_y + 20.0 + MARGIN + self.scroll,
        }
    }

    /// Where the knob's centre is for `value` on `track`.
    fn knob_x(track: Rect, value: u8) -> f32 {
        track.x + track.w * value as f32 / 100.0
    }

    /// Slider value for a pointer x on `track`.
    fn value_at(track: Rect, x: f32) -> u8 {
        let t = ((x - track.x) / track.w).clamp(0.0, 1.0);
        (t * 100.0 + 0.5) as u8
    }

    fn slider_hit(track: Rect) -> Rect {
        Rect::new(
            track.x - KNOB / 2.0,
            track.y - 14.0,
            track.w + KNOB,
            SLIDER_H + 28.0,
        )
    }

    fn load(&mut self) {
        let mut buf = [0u8; 16];
        self.storage = match state_get(KEY, &mut buf) {
            Ok(n) => match Values::decode(&buf[..n.min(buf.len())]) {
                Some(v) => {
                    self.values = v;
                    Storage::Loaded
                }
                None => {
                    let _ = log(Level::Warn, "settings: saved value has an unknown format");
                    Storage::Empty
                }
            },
            Err(VyError::Resource) => Storage::Empty,
            Err(e) => {
                let _ = log(Level::Warn, &format!("settings: state_get refused: {e}"));
                Storage::Unavailable(e)
            }
        };
    }

    fn save(&mut self) {
        let result = state_put(KEY, &self.values.encode());
        if let Err(e) = result {
            let _ = log(Level::Warn, &format!("settings: state_put refused: {e}"));
        }
        self.last_save = Some(result);
    }

    fn change(&mut self, f: impl FnOnce(&mut Values)) -> bool {
        let before = self.values;
        f(&mut self.values);
        if self.values == before {
            return false;
        }
        self.save();
        true
    }

    fn reset(&mut self) -> bool {
        self.values = Values::DEFAULT;
        if let Err(e) = state_delete(KEY) {
            let _ = log(Level::Warn, &format!("settings: state_delete refused: {e}"));
        }
        self.last_save = None;
        self.storage = match self.storage {
            Storage::Unavailable(e) => Storage::Unavailable(e),
            _ => Storage::Empty,
        };
        true
    }

    fn set_focus(&mut self, next: Option<Control>) -> bool {
        let changed = next != self.focus;
        self.focus = next;
        changed
    }

    /// The hit rectangle of a control in window coordinates.
    fn control_rect(&self, c: Control) -> Rect {
        let l = self.layout();
        match c {
            Control::Switch(i) => l.switches[i],
            Control::Size(i) => l.sizes[i],
            Control::Home(i) => l.homes[i],
            Control::Volume => Self::slider_hit(l.track),
            Control::Reset => l.reset,
        }
    }

    /// Scroll so the focused control is on screen with some padding.
    fn reveal(&mut self, c: Control) {
        let r = self.control_rect(c);
        let max = (self.layout().content_h - self.window.height).max(0.0);
        let delta = if r.y < VIEW_PAD {
            r.y - VIEW_PAD
        } else if r.bottom() > self.window.height - VIEW_PAD {
            r.bottom() - self.window.height + VIEW_PAD
        } else {
            0.0
        };
        self.scroll = (self.scroll + delta).clamp(0.0, max);
    }

    /// Focus from the keyboard: the control comes into view (SPEC-10.3.1).
    fn focus_by_keyboard(&mut self, next: Control) -> bool {
        let changed = self.set_focus(Some(next));
        self.reveal(next);
        changed
    }

    /// Space or Enter on the focused control.
    fn activate(&mut self) -> bool {
        match self.focus {
            Some(Control::Switch(i)) => self.change(|v| match i {
                0 => v.dark = !v.dark,
                1 => v.compact = !v.compact,
                _ => v.animations = !v.animations,
            }),
            Some(Control::Size(i)) => self.change(|v| v.size = i as u8),
            Some(Control::Home(i)) => self.change(|v| v.home = i as u8),
            Some(Control::Reset) => self.reset(),
            _ => false,
        }
    }

    fn nudge_volume(&mut self, delta: i32) -> bool {
        self.change(|v| v.volume = (v.volume as i32 + delta).clamp(0, 100) as u8)
    }

    fn status_text(&self) -> String {
        let base = match self.storage {
            Storage::Loaded => String::from("Sparade inställningar laddade."),
            Storage::Empty => String::from("Inga sparade inställningar: standardvärden visas."),
            Storage::Unavailable(e) => {
                format!("Lagring otillgänglig ({}): standardvärden visas.", e.name())
            }
        };
        match self.last_save {
            None => base,
            Some(Ok(())) => format!("{base} Ändringen är sparad."),
            Some(Err(e)) => format!("{base} Ändringen kunde inte sparas ({}).", e.name()),
        }
    }

    fn ring(&self, f: &mut Frame, r: Rect, radius: f32) -> Result<(), VyError> {
        f.rect(r.inset(-3.0), radius + 3.0, RING)?;
        f.rect(r.inset(-1.0), radius + 1.0, self.palette().card)
    }

    fn draw_switch(&mut self, f: &mut Frame, i: usize, r: Rect, card: Rect) -> Result<(), VyError> {
        let p = self.palette();
        let s = self.styles();
        let on = match i {
            0 => self.values.dark,
            1 => self.values.compact,
            _ => self.values.animations,
        };
        let row_h = self.row_h();
        let label_y = r.y + (SWITCH_H - 20.0) / 2.0;
        f.text(card.x + PAD, label_y, 0.0, s.label, SWITCHES[i])?;
        if self.focus == Some(Control::Switch(i)) {
            self.ring(f, r, SWITCH_H / 2.0)?;
        }
        f.rect(r, SWITCH_H / 2.0, if on { p.accent } else { p.track })?;
        let kx = if on {
            r.right() - SWITCH_H + 2.0
        } else {
            r.x + 2.0
        };
        f.rect(
            Rect::new(kx, r.y + 2.0, SWITCH_H - 4.0, SWITCH_H - 4.0),
            (SWITCH_H - 4.0) / 2.0,
            p.knob,
        )?;
        let row = Rect::new(
            card.x + PAD,
            r.y - (row_h - SWITCH_H) / 2.0,
            card.w - 2.0 * PAD,
            row_h,
        );
        let text = format!("{}: {}", SWITCHES[i], if on { "på" } else { "av" });
        let id = f.sem.node(Role::Checkbox, row, &text)?;
        self.node_ids[Control::Switch(i).index()] = id.0;
        Ok(())
    }

    fn draw_radio(
        &mut self,
        f: &mut Frame,
        control: Control,
        r: Rect,
        label: &str,
        selected: bool,
    ) -> Result<(), VyError> {
        let p = self.palette();
        let s = self.styles();
        let circle = Rect::new(r.x, r.y + (r.h - RADIO) / 2.0, RADIO, RADIO);
        if self.focus == Some(control) {
            self.ring(f, circle, RADIO / 2.0)?;
        }
        f.rect(
            circle,
            RADIO / 2.0,
            if selected { p.accent } else { p.track },
        )?;
        f.rect(circle.inset(2.0), RADIO / 2.0 - 2.0, p.card)?;
        if selected {
            f.rect(circle.inset(6.0), RADIO / 2.0 - 6.0, p.accent)?;
        }
        f.text(
            circle.right() + 12.0,
            r.y + (r.h - 20.0) / 2.0,
            0.0,
            s.label,
            label,
        )?;
        let text = if selected {
            format!("{label} (vald)")
        } else {
            String::from(label)
        };
        let id = f.sem.node(Role::Radio, r, &text)?;
        self.node_ids[control.index()] = id.0;
        Ok(())
    }

    fn draw_slider(&mut self, f: &mut Frame, track: Rect) -> Result<(), VyError> {
        let p = self.palette();
        let s = self.styles();
        let kx = Self::knob_x(track, self.values.volume);
        f.rect(track, SLIDER_H / 2.0, p.track)?;
        f.rect(
            Rect::new(track.x, track.y, kx - track.x, track.h),
            SLIDER_H / 2.0,
            p.accent,
        )?;
        let knob = Rect::new(
            kx - KNOB / 2.0,
            track.y + SLIDER_H / 2.0 - KNOB / 2.0,
            KNOB,
            KNOB,
        );
        if self.focus == Some(Control::Volume) {
            self.ring(f, knob, KNOB / 2.0)?;
        }
        f.rect(knob, KNOB / 2.0, p.accent)?;
        f.rect(knob.inset(3.0), KNOB / 2.0 - 3.0, p.knob)?;
        let value = format!("{}", self.values.volume);
        let vw = text_measure(s.value, 0.0, &value)?.w;
        f.text(
            track.right() + 56.0 - vw,
            knob.y - 1.0,
            0.0,
            s.value,
            &value,
        )?;
        f.text(track.x, track.y - 20.0 - 4.0, 0.0, s.label, VOLUME)?;
        let hit = Self::slider_hit(track);
        let id = f.sem.node(
            Role::Slider,
            hit,
            &format!("{VOLUME}: {}", self.values.volume),
        )?;
        self.node_ids[Control::Volume.index()] = id.0;
        Ok(())
    }
}

fn styles_for(p: &Palette) -> Result<Styles, VyError> {
    let style = |font, size, weight, color| text_style(font, size, weight, color);
    Ok(Styles {
        title: style(Font::Sans, 26.0, 700, p.ink)?,
        status: style(Font::Sans, 13.0, 400, p.muted)?,
        heading: style(Font::Sans, 15.0, 700, p.muted)?,
        label: style(Font::Sans, 16.0, 400, p.ink)?,
        value: style(Font::Sans, 16.0, 600, p.ink)?,
        button: style(Font::Sans, 15.0, 600, p.accent)?,
        preview: [
            style(Font::Serif, 14.0, 400, p.muted)?,
            style(Font::Serif, 16.0, 400, p.muted)?,
            style(Font::Serif, 18.0, 400, p.muted)?,
        ],
    })
}

impl App for SettingsPage {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let _ = log(Level::Debug, "settings: init");
        let mut page = Self {
            window,
            light: styles_for(&LIGHT)?,
            dark: styles_for(&DARK)?,
            values: Values::DEFAULT,
            storage: Storage::Empty,
            last_save: None,
            focus: None,
            dragging: false,
            scroll: 0.0,
            node_ids: [0; STOPS],
        };
        page.load();
        Ok(page)
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let p = self.palette();
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, p.bg)?;
        let x = l.reset.x;
        let w = l.reset.w;
        f.clipped(window, 0.0, |f| {
            f.group(Role::Main, Rect::new(x, 0.0, w, l.content_h), "", |f| {
                let s = self.styles();
                let ty = MARGIN - self.scroll;
                let th = f.text(x, ty, w, s.title, TITLE)?.h;
                f.sem.node(Role::Heading, Rect::new(x, ty, w, th), TITLE)?;
                let status = self.status_text();
                let sh = f.text(x, ty + th + 4.0, w, s.status, &status)?.h;
                f.sem
                    .node(Role::Text, Rect::new(x, ty + th + 4.0, w, sh), &status)?;

                for (c, card) in l.card.iter().enumerate() {
                    f.rect(*card, 10.0, p.card)?;
                    f.group(Role::Region, *card, SECTIONS[c], |f| {
                        let s = self.styles();
                        let hy = card.y + PAD;
                        let hh = f
                            .text(card.x + PAD, hy, card.w - 2.0 * PAD, s.heading, SECTIONS[c])?
                            .h;
                        f.sem.node(
                            Role::Heading,
                            Rect::new(card.x + PAD, hy, card.w - 2.0 * PAD, hh),
                            SECTIONS[c],
                        )?;
                        match c {
                            0 => {
                                for i in 0..SWITCHES.len() {
                                    if i > 0 {
                                        let sep = l.switches[i].y - (self.row_h() - SWITCH_H) / 2.0;
                                        f.rect(
                                            Rect::new(card.x + PAD, sep, card.w - 2.0 * PAD, 1.0),
                                            0.0,
                                            p.rule,
                                        )?;
                                    }
                                    self.draw_switch(f, i, l.switches[i], *card)?;
                                }
                            }
                            1 => {
                                for (i, r) in l.sizes.iter().enumerate() {
                                    let selected = self.values.size as usize == i;
                                    self.draw_radio(f, Control::Size(i), *r, SIZES[i], selected)?;
                                }
                                let s = self.styles();
                                let ph = f
                                    .text(
                                        l.preview.x,
                                        l.preview.y,
                                        l.preview.w,
                                        s.preview[self.values.size as usize],
                                        PREVIEW,
                                    )?
                                    .h;
                                f.sem.node(
                                    Role::Text,
                                    Rect::new(l.preview.x, l.preview.y, l.preview.w, ph),
                                    PREVIEW,
                                )?;
                            }
                            2 => {
                                for (i, r) in l.homes.iter().enumerate() {
                                    let selected = self.values.home as usize == i;
                                    self.draw_radio(f, Control::Home(i), *r, HOMES[i], selected)?;
                                }
                            }
                            _ => self.draw_slider(f, l.track)?,
                        }
                        Ok(())
                    })?;
                }

                // Reset button: outlined, accent text.
                let s = self.styles();
                let b = l.reset;
                if self.focus == Some(Control::Reset) {
                    self.ring(f, b, 8.0)?;
                }
                f.rect(b, 8.0, p.accent)?;
                f.rect(b.inset(1.5), 6.5, p.card)?;
                let tw = text_measure(s.button, 0.0, RESET)?.w;
                f.text(
                    b.x + (b.w - tw) / 2.0,
                    b.y + (BUTTON_H - 19.0) / 2.0,
                    0.0,
                    s.button,
                    RESET,
                )?;
                let id = f.sem.node(Role::Button, b, RESET)?;
                self.node_ids[Control::Reset.index()] = id.0;

                let note = "Ändringar sparas direkt via state_put; vy_suspend sparar en gång till.";
                let nh = f.text(x, l.status_y, w, s.status, note)?.h;
                f.sem
                    .node(Role::Text, Rect::new(x, l.status_y, w, nh), note)?;
                Ok(())
            })
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } => {
                let l = self.layout();
                for (i, r) in l.switches.iter().enumerate() {
                    let row = Rect::new(l.card[0].x, r.y - 10.0, l.card[0].w, SWITCH_H + 20.0);
                    if row.contains(x, y) {
                        self.set_focus(Some(Control::Switch(i)));
                        return self.activate() || true;
                    }
                }
                for (i, r) in l.sizes.iter().enumerate() {
                    if r.contains(x, y) {
                        self.set_focus(Some(Control::Size(i)));
                        return self.activate() || true;
                    }
                }
                for (i, r) in l.homes.iter().enumerate() {
                    if r.contains(x, y) {
                        self.set_focus(Some(Control::Home(i)));
                        return self.activate() || true;
                    }
                }
                if Self::slider_hit(l.track).contains(x, y) {
                    self.set_focus(Some(Control::Volume));
                    self.dragging = true;
                    let v = Self::value_at(l.track, x);
                    self.change(|val| val.volume = v);
                    return true;
                }
                if l.reset.contains(x, y) {
                    self.set_focus(Some(Control::Reset));
                    return self.reset();
                }
                self.set_focus(None)
            }
            Event::PointerMove { x, .. } if self.dragging => {
                let track = self.layout().track;
                let v = Self::value_at(track, x);
                // Only the value moves while dragging; it is saved on release.
                if v != self.values.volume {
                    self.values.volume = v;
                    true
                } else {
                    false
                }
            }
            Event::PointerUp { x, .. } if self.dragging => {
                self.dragging = false;
                let track = self.layout().track;
                self.values.volume = Self::value_at(track, x);
                self.save();
                true
            }
            Event::KeyDown { code, .. } => match code {
                key::SPACE | key::ENTER => self.activate(),
                key::ESCAPE => self.set_focus(None),
                key::RIGHT | key::UP if self.focus == Some(Control::Volume) => {
                    self.nudge_volume(i32::from(SLIDER_STEP))
                }
                key::LEFT | key::DOWN if self.focus == Some(Control::Volume) => {
                    self.nudge_volume(-i32::from(SLIDER_STEP))
                }
                key::HOME if self.focus == Some(Control::Volume) => self.nudge_volume(-100),
                key::END if self.focus == Some(Control::Volume) => self.nudge_volume(100),
                _ => false,
            },
            Event::Scroll { dy, lines, .. } => {
                let px = if lines { dy * 24.0 } else { dy };
                let max = (self.layout().content_h - self.window.height).max(0.0);
                let next = (self.scroll + px).clamp(0.0, max);
                let changed = next != self.scroll;
                self.scroll = next;
                changed
            }
            Event::Focus {
                node,
                gained,
                keyboard,
            } => {
                let stop = self.node_ids.iter().position(|id| *id == node.0);
                match (stop, gained) {
                    (Some(i), true) if keyboard => self.focus_by_keyboard(Control::nth(i)),
                    (Some(i), true) => self.set_focus(Some(Control::nth(i))),
                    (Some(i), false) if self.focus == Some(Control::nth(i)) => self.set_focus(None),
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        let max = (self.layout().content_h - self.window.height).max(0.0);
        self.scroll = self.scroll.clamp(0.0, max);
    }

    /// The tab is being hidden: write the values once more (SPEC-5.1.5).
    fn suspend(&mut self) -> i32 {
        self.save();
        0
    }
}

vy_sdk::export!(SettingsPage);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::mock;

    fn window() -> Window {
        Window {
            width: 800.0,
            height: 600.0,
            scale: 1.0,
        }
    }

    /// SPEC-5.2.9, SPEC-5.1.5: values written with `state_put` on change and
    /// in `vy_suspend` come back through `state_get` in a fresh instance of
    /// the page; the headless runner cannot re-init with retained state, so
    /// the SDK mock stands in for the host's storage here.
    #[test]
    fn spec_5_2_9_values_persist_across_instances() {
        mock::reset();
        mock::with(|h| h.caps.storage = true);
        let mut page = SettingsPage::init(window(), 0).unwrap();
        assert_eq!(page.storage, Storage::Empty);
        assert_eq!(page.values, Values::DEFAULT);

        // Toggle dark, pick the large text size, drag the slider to the end.
        page.set_focus(Some(Control::Switch(0)));
        assert!(page.activate());
        page.set_focus(Some(Control::Size(2)));
        assert!(page.activate());
        let track = page.layout().track;
        assert!(page.event(Event::PointerDown {
            x: track.x + 1.0,
            y: track.y,
            button: 1,
            modifiers: Modifiers(0),
        }));
        assert!(page.event(Event::PointerMove {
            x: track.right(),
            y: track.y,
            buttons: 1,
            modifiers: Modifiers(0),
        }));
        assert!(page.event(Event::PointerUp {
            x: track.right() + 50.0,
            y: track.y,
            button: 1,
            modifiers: Modifiers(0),
        }));
        assert_eq!(page.last_save, Some(Ok(())));
        assert_eq!(page.suspend(), 0);
        let saved = mock::with(|h| h.state.get(KEY.as_bytes()).cloned()).unwrap();
        assert_eq!(saved, [FORMAT, 0b101, 2, 0, 100]);

        let again = SettingsPage::init(window(), 0).unwrap();
        assert_eq!(again.storage, Storage::Loaded);
        assert_eq!(
            again.values,
            Values {
                dark: true,
                size: 2,
                volume: 100,
                ..Values::DEFAULT
            }
        );

        // Reset deletes the key: the next instance starts from the defaults.
        let mut again = again;
        assert!(again.reset());
        let third = SettingsPage::init(window(), 0).unwrap();
        assert_eq!(third.storage, Storage::Empty);
        assert_eq!(third.values, Values::DEFAULT);
    }

    /// SPEC-5.3.1: without `storage` the page runs on the defaults, says
    /// so, and a change is kept for the session but reported as unsaved.
    #[test]
    fn spec_5_3_1_denied_storage_is_visible_not_fatal() {
        mock::reset();
        let mut page = SettingsPage::init(window(), 0).unwrap();
        assert_eq!(page.storage, Storage::Unavailable(VyError::Denied));
        page.set_focus(Some(Control::Switch(1)));
        assert!(page.activate());
        assert!(page.values.compact);
        assert_eq!(page.last_save, Some(Err(VyError::Denied)));
        assert!(page.status_text().contains("E_DENIED"));
        let mut f = Frame::begin();
        page.frame(&mut f, 16).unwrap();
        assert!(f.end().unwrap() > 20);
        mock::with(|h| {
            assert!(h.sem.iter().any(|n| n.text == "Kompakt läge: på"));
            assert!(h.sem.iter().any(|n| n.text == "Volym: 40"));
        });
    }
}
