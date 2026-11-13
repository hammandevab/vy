//! Vy example page `rymd` ("Rymd"): a 3D scene rendered in software.
//!
//! What it shows beyond the reference pages:
//!
//! - **3D without a 3D API.** The host draws rectangles, text and paths
//!   and nothing else (SPEC-2.1.3). The page turns a solid, projects it,
//!   leaves out the faces turned away (back-face culling), lights each
//!   face from one direction (flat shading), sorts them the furthest
//!   first (the painter's algorithm) and draws each as a filled `PATH`
//!   (SPEC-C.1.2). A wireframe strokes all the faces, hidden ones dimmer.
//! - **Its own trigonometry.** `core` has no sine and the SDK brings no
//!   `libm`; `scene.rs` has a polynomial and whole-number angles, so the
//!   same angles give the same display list byte for byte (SPEC-2.1.5).
//! - **Hundreds of particles.** A star field in four depths behind the
//!   solid, one small rectangle a star, that shifts with the view
//!   (parallax). A slider sets how many.
//! - **Animation from `dt_ms`.** While "Rotera" is on, every frame asks
//!   for the next (SPEC-5.2.2) and turns the solid by the time that
//!   passed (SPEC-5.1.2); off, the page asks for nothing (SPEC-6.2.2).
//! - **A readout of its own cost:** the display-list ops of the frame and
//!   the time since the last one.
//!
//! The pointer drags the solid around, the wheel zooms; the arrow keys,
//! Page Up, Page Down and Home do the same from the keyboard. The page
//! asks for no capability.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod scene;

use alloc::string::String;
use alloc::vec::Vec;
use scene::{Drawn, Projector, Solid, Star, View, MAX_STARS, TURN};
use vy_sdk::event::{button, key};
use vy_sdk::math::{floor, snap};
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Checkbox, Input, Palette, Slider, Theme};

const SKY: Color = Color::rgb(0x06, 0x09, 0x13);
const PALETTE: Palette = Palette::DARK;

/// The solids, in Tab order. Solid `i` has node id `ID_SHAPE - i`.
const SHAPES: usize = 3;
const NAMES: [&str; SHAPES] = ["Ikosaeder", "Torus", "Kub"];
const NOUNS: [&str; SHAPES] = ["ikosaeder", "torus", "kub"];
/// Each solid's colour in full light.
const TINTS: [(u8, u8, u8); SHAPES] = [(0x6C, 0x9B, 0xFF), (0xFF, 0xAE, 0x45), (0x52, 0xD9, 0xA8)];
const ID_SHAPE: i32 = -10;
const ID_WIRE: i32 = -20;
const ID_SPIN: i32 = -21;
const ID_STARS: i32 = -22;

/// Each depth of stars, far to near: side in pixels and colour.
const STAR_LOOKS: [(f32, Color); 4] = [
    (1.0, Color::rgb(0x55, 0x60, 0x7E)),
    (1.0, Color::rgb(0x8C, 0x98, 0xB8)),
    (2.0, Color::rgb(0xB9, 0xC4, 0xE0)),
    (3.0, Color::rgb(0xFF, 0xFF, 0xFF)),
];

/// Below this width the settings are a strip under the scene.
const NARROW: f32 = 720.0;
const PANEL_W: f32 = 264.0;
const PANEL_H: f32 = 166.0;
/// The focal length as a share of the scene's shorter side: a solid of
/// radius one then fills about three fifths of it.
const FOCAL: f32 = 1.16;
/// Each zoom step's factor, and how many steps there are either way.
const ZOOM_STEP: f32 = 1.1;
const ZOOM_MAX: i32 = 6;
/// Pixels of wheel that make one zoom step; a line of wheel is as many.
const WHEEL_PX: f32 = 40.0;
/// Angle units a pixel of drag turns the solid: half a degree.
const DRAG_TURN: f32 = 91.0;
/// Angle units an arrow key turns it: five degrees.
const KEY_TURN: i32 = 910;
/// Angle units "Rotera" turns it in 100 ms: twenty degrees a second.
const SPIN: i32 = 364;
/// Most milliseconds one frame turns it for, so a stall is no jump.
const MAX_STEP_MS: i32 = 100;
/// The tilt stops short of looking straight down or up: 80 degrees.
const PITCH_MAX: i32 = TURN * 2 / 9;
/// The view the page starts with and Home returns to: from a little to
/// one side and a little above.
const HOME_YAW: i32 = TURN / 12;
const HOME_PITCH: i32 = -TURN / 18;
/// A wireframe's line, and the light of the faces turned away.
const WIRE_W: f32 = 1.5;
const WIRE_BACK: f32 = 0.36;
/// The stroke that closes the seams between filled faces.
const SEAM_W: f32 = 1.0;
/// How the page is used, for the column beside the scene.
const HINT: &str = "Dra i scenen för att vrida och rulla för att zooma. \
                    Piltangenterna vrider, Page Up och Page Down zoomar, Home återställer.";
const STARS_STEP: i32 = 50;
const STARS_DEFAULT: i32 = 300;

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    label: StyleId,
    read: StyleId,
    key: StyleId,
    key_on: StyleId,
}

/// Every rectangle on the page for the current window.
#[derive(Clone, Copy, Debug)]
struct Layout {
    narrow: bool,
    /// Where the stars and the solid are.
    scene: Rect,
    /// The settings: a column on the right, or a strip at the bottom.
    panel: Rect,
    /// The word over the solids' buttons, in the column only.
    form: Option<Rect>,
    shapes: [Rect; SHAPES],
    wire: Rect,
    spin: Rect,
    /// The word beside or over the slider.
    stars_label: Rect,
    stars: Rect,
    readout: [Rect; 2],
    /// How the page is used, in the column only.
    hint: Option<Rect>,
}

struct Page {
    window: Window,
    theme: Theme,
    s: Styles,
    solids: [Solid; SHAPES],
    shape: usize,
    /// The view: turn and tilt in angle units, zoom in steps.
    yaw: i32,
    pitch: i32,
    zoom: i32,
    /// Wheel not yet spent on a zoom step, in pixels.
    wheel: f32,
    wire: Checkbox,
    spin: Checkbox,
    count: Slider,
    stars: Vec<Star>,
    projector: Projector,
    /// Where the pointer was when it last dragged the solid.
    drag: Option<(f32, f32)>,
    /// The last frame asked for the next: `dt_ms` is animation time.
    animating: bool,
    /// The node the host's focus is on.
    focus: Option<i32>,
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
fn number(mut n: u32) -> String {
    let mut digits = [0u8; 10];
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

/// A solid's colour in light `shade`, 0 to 1.
fn lit(tint: (u8, u8, u8), shade: f32) -> Color {
    let part = |c: u8| (f32::from(c) * shade.clamp(0.0, 1.0)) as u8;
    Color::rgb(part(tint.0), part(tint.1), part(tint.2))
}

/// Add the outline of a projected face to `path`.
fn trace(path: &mut Path, face: &Drawn) {
    path.move_to(face.at[0].0, face.at[0].1);
    for &(x, y) in &face.at[1..usize::from(face.n)] {
        path.line_to(x, y);
    }
    path.close();
}

impl Page {
    fn layout(&self) -> Layout {
        let (w, h) = (self.window.width, self.window.height);
        let narrow = w < NARROW;
        let mut shapes = [Rect::default(); SHAPES];
        if narrow {
            let panel = Rect::new(0.0, (h - PANEL_H).max(0.0), w, PANEL_H);
            let (x, y) = (12.0, panel.y + 12.0);
            let inner = (w - 24.0).max(0.0);
            let key_w = floor((inner - 12.0) / 3.0);
            for (i, r) in shapes.iter_mut().enumerate() {
                *r = Rect::new(x + i as f32 * (key_w + 6.0), y, key_w, 36.0);
            }
            let half = floor((inner - 12.0) / 2.0);
            let second = x + half + 12.0;
            Layout {
                narrow,
                scene: Rect::new(0.0, 0.0, w, panel.y),
                panel,
                form: None,
                shapes,
                wire: Rect::new(x, y + 42.0, half, 36.0),
                spin: Rect::new(second, y + 42.0, half, 36.0),
                stars_label: Rect::new(x, y + 84.0, 76.0, 36.0),
                stars: Rect::new(x + 82.0, y + 84.0, (inner - 82.0).max(0.0), 36.0),
                readout: [
                    Rect::new(x, y + 124.0, half, 20.0),
                    Rect::new(second, y + 124.0, half, 20.0),
                ],
                hint: None,
            }
        } else {
            let panel = Rect::new(w - PANEL_W, 0.0, PANEL_W, h);
            let x = panel.x + 16.0;
            let inner = PANEL_W - 32.0;
            for (i, r) in shapes.iter_mut().enumerate() {
                *r = Rect::new(x, 42.0 + i as f32 * 42.0, inner, 36.0);
            }
            Layout {
                narrow,
                scene: Rect::new(0.0, 0.0, panel.x, h),
                panel,
                form: Some(Rect::new(x, 16.0, inner, 20.0)),
                shapes,
                wire: Rect::new(x, 176.0, inner, 36.0),
                spin: Rect::new(x, 212.0, inner, 36.0),
                stars_label: Rect::new(x, 262.0, inner, 20.0),
                stars: Rect::new(x, 282.0, inner, 36.0),
                readout: [
                    Rect::new(x, 334.0, inner, 20.0),
                    Rect::new(x, 356.0, inner, 20.0),
                ],
                hint: Some(Rect::new(x, 394.0, inner, (h - 394.0 - 16.0).max(0.0))),
            }
        }
    }

    /// How the solid is seen in the scene of layout `l`.
    fn view(&self, l: &Layout) -> View {
        let step = if self.zoom < 0 {
            1.0 / ZOOM_STEP
        } else {
            ZOOM_STEP
        };
        let mut zoom = 1.0f32;
        for _ in 0..self.zoom.unsigned_abs() {
            zoom *= step;
        }
        View {
            yaw: self.yaw,
            pitch: self.pitch,
            focal: FOCAL * l.scene.w.min(l.scene.h) * zoom,
            centre: (l.scene.x + l.scene.w / 2.0, l.scene.y + l.scene.h / 2.0),
        }
    }

    /// Turn the view by angle units; true when it moved.
    fn turn(&mut self, yaw: i32, pitch: i32) -> bool {
        let before = (self.yaw, self.pitch);
        self.yaw = self.yaw.wrapping_add(yaw);
        self.pitch = (self.pitch + pitch).clamp(-PITCH_MAX, PITCH_MAX);
        before != (self.yaw, self.pitch)
    }

    /// Zoom by steps, in when positive; true when it moved.
    fn zoom_by(&mut self, steps: i32) -> bool {
        let before = self.zoom;
        self.zoom = (self.zoom + steps).clamp(-ZOOM_MAX, ZOOM_MAX);
        before != self.zoom
    }

    /// True when `event` presses the page's own control `id` in `r`: a
    /// primary press inside it, or Enter or Space while it has the focus.
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

    /// The scene's own pointer, wheel and keys, for what no control took.
    fn scene_event(&mut self, ui: &mut Input<'_>, l: &Layout) -> bool {
        let held = button::mask(button::PRIMARY);
        match ui.unhandled() {
            Some(Event::PointerDown { x, y, button, .. })
                if button == button::PRIMARY && l.scene.contains(x, y) =>
            {
                self.drag = Some((x, y));
                true
            }
            Some(Event::PointerMove { x, y, buttons, .. }) => match self.drag {
                // Released outside the page: the drag is over.
                Some(_) if buttons & held == 0 => {
                    self.drag = None;
                    true
                }
                Some((px, py)) => {
                    self.drag = Some((x, y));
                    // The near side follows the pointer.
                    let yaw = ((px - x) * DRAG_TURN) as i32;
                    let pitch = ((py - y) * DRAG_TURN) as i32;
                    self.turn(yaw, pitch)
                }
                None => false,
            },
            Some(Event::PointerUp { .. } | Event::PointerLeave) => self.drag.take().is_some(),
            Some(Event::Scroll { dy, lines, .. }) => {
                self.wheel += if lines { dy * WHEEL_PX } else { dy };
                let steps = (self.wheel / WHEEL_PX) as i32;
                self.wheel -= steps as f32 * WHEEL_PX;
                // The wheel away from the hand zooms in.
                self.zoom_by(-steps)
            }
            Some(Event::KeyDown {
                code, modifiers, ..
            }) if !modifiers.shortcut() => match code {
                key::LEFT => self.turn(KEY_TURN, 0),
                key::RIGHT => self.turn(-KEY_TURN, 0),
                key::UP => self.turn(0, KEY_TURN),
                key::DOWN => self.turn(0, -KEY_TURN),
                key::PAGE_UP => self.zoom_by(1),
                key::PAGE_DOWN => self.zoom_by(-1),
                key::HOME => {
                    let before = (self.yaw, self.pitch, self.zoom);
                    (self.yaw, self.pitch, self.zoom) = (HOME_YAW, HOME_PITCH, 0);
                    before != (self.yaw, self.pitch, self.zoom)
                }
                _ => false,
            },
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

    /// The stars: one small rectangle each, on whole device pixels.
    fn draw_stars(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let scale = self.window.scale;
        let shown = (self.count.value().max(0) as usize).min(self.stars.len());
        let (kx, ky) = (l.scene.w / TURN as f32, l.scene.h / TURN as f32);
        let sides = STAR_LOOKS.map(|(side, _)| snap(side, scale));
        for star in &self.stars[..shown] {
            let (x, y) = star.at(self.yaw, self.pitch);
            let layer = usize::from(star.layer);
            let side = sides[layer];
            let r = Rect::new(
                snap(l.scene.x + x as f32 * kx, scale),
                snap(l.scene.y + y as f32 * ky, scale),
                side,
                side,
            );
            // The nearest are round.
            let radius = if layer == 3 { side / 2.0 } else { 0.0 };
            f.rect(r, radius, STAR_LOOKS[layer].1)?;
        }
        Ok(())
    }

    /// The solid: filled faces the furthest first, or as a wireframe the
    /// outlines of all faces, those turned away dimmer and underneath.
    fn draw_solid(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let view = self.view(l);
        let wire = self.wire.checked;
        self.projector
            .project(&self.solids[self.shape], &view, wire);
        let tint = TINTS[self.shape];
        if !wire {
            for face in self.projector.drawn() {
                // Filled, and stroked in its own colour: two faces that
                // meet would otherwise show the sky through the seam,
                // where each covers its edge pixels only in part.
                let mut path = Path::new();
                trace(&mut path, face);
                let color = lit(tint, face.shade);
                f.path(&path, color, 0.0)?;
                f.path(&path, color, SEAM_W)?;
            }
            return Ok(());
        }
        // One path for each side: an edge two faces share is one line.
        for front in [false, true] {
            let mut path = Path::new();
            for face in self.projector.drawn().filter(|d| d.front == front) {
                trace(&mut path, face);
            }
            if !path.is_empty() {
                let shade = if front { 1.0 } else { WIRE_BACK };
                f.path(&path, lit(tint, shade), WIRE_W)?;
            }
        }
        Ok(())
    }

    /// One of the solids' buttons.
    fn draw_shape(&self, f: &mut Frame, i: usize, r: Rect) -> Result<(), VyError> {
        let p = &self.theme.palette;
        let id = ID_SHAPE - i as i32;
        if self.focus == Some(id) {
            f.rect(r.inset(-ui::RING), ui::RADIUS + ui::RING, p.ring)?;
        }
        let on = self.shape == i;
        f.rect(r, ui::RADIUS, if on { p.accent } else { p.border })?;
        if !on {
            f.rect(r.inset(1.0), ui::RADIUS - 1.0, p.surface)?;
        }
        let style = if on { self.s.key_on } else { self.s.key };
        Self::centred(f, r, style, NAMES[i])?;
        let chosen = if on { self.theme.words.selected } else { "" };
        f.sem
            .node_with_id(id, Role::Button, r, &cat(&[NAMES[i], chosen]))?;
        Ok(())
    }

    /// A line of the panel's own text with its node.
    fn line(f: &mut Frame, r: Rect, style: StyleId, text: &str) -> Result<(), VyError> {
        let size = f.text(r.x, r.y, r.w, style, text)?;
        f.sem
            .node(Role::Text, Rect::new(r.x, r.y, size.w, size.h), text)?;
        Ok(())
    }

    /// The settings, and last of all the readout, which counts itself.
    fn draw_panel(&self, f: &mut Frame, l: &Layout, frame_ms: Option<i32>) -> Result<(), VyError> {
        let t = &self.theme;
        let scale = self.window.scale;
        f.rect(l.panel, 0.0, t.palette.surface)?;
        let rule = snap(1.0, scale);
        let edge = if l.narrow {
            Rect::new(l.panel.x, snap(l.panel.y, scale), l.panel.w, rule)
        } else {
            Rect::new(snap(l.panel.x, scale), l.panel.y, rule, l.panel.h)
        };
        f.rect(edge, 0.0, t.palette.rule)?;
        f.group(Role::Region, l.panel, "Inställningar", |f| {
            if let Some(r) = l.form {
                Self::line(f, r, self.s.label, "Form")?;
            }
            for (i, r) in l.shapes.iter().enumerate() {
                self.draw_shape(f, i, *r)?;
            }
            self.wire.draw(f, l.wire, t, "Trådmodell")?;
            self.spin.draw(f, l.spin, t, "Rotera")?;
            let label = l.stars_label;
            let line_h = text_measure(self.s.label, 0.0, "Partiklar")?.h;
            let at = Rect::new(label.x, label.y + (label.h - line_h) / 2.0, label.w, line_h);
            Self::line(f, at, self.s.label, "Partiklar")?;
            self.count.draw(f, l.stars, t, "Partiklar")?;
            // The hint is drawn before the readout, whose two lines are
            // the frame's last ops, and comes after it in the tree, as
            // it does on the page.
            let hint = match l.hint {
                Some(r) => Some((r, f.text(r.x, r.y, r.w, self.s.label, HINT)?)),
                None => None,
            };
            let ops = cat(&["Ritoperationer: ", &number(f.ops() + 2)]);
            Self::line(f, l.readout[0], self.s.read, &ops)?;
            let time = match frame_ms {
                Some(ms) => cat(&["Bildtid: ", &number(ms.max(0) as u32), " ms"]),
                None => String::from("Bildtid: –"),
            };
            Self::line(f, l.readout[1], self.s.read, &time)?;
            if let Some((r, size)) = hint {
                let at = Rect::new(r.x, r.y, size.w, size.h);
                f.sem.node(Role::Text, at, HINT)?;
            }
            Ok(())
        })
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let theme = Theme::custom(PALETTE, Font::Sans, 15.0)?;
        Ok(Self {
            window,
            theme,
            s: Styles {
                title: text_style(Font::Sans, 20.0, 700, PALETTE.ink)?,
                label: text_style(Font::Sans, 13.0, 400, PALETTE.muted)?,
                read: text_style(Font::Sans, 13.0, 400, PALETTE.ink)?,
                key: text_style(Font::Sans, 14.0, 600, PALETTE.ink)?,
                key_on: text_style(Font::Sans, 14.0, 600, PALETTE.on_accent)?,
            },
            solids: [Solid::icosahedron(), Solid::torus(), Solid::cube()],
            shape: 0,
            yaw: HOME_YAW,
            pitch: HOME_PITCH,
            zoom: 0,
            wheel: 0.0,
            wire: Checkbox::new(ID_WIRE, false),
            spin: Checkbox::new(ID_SPIN, true),
            count: Slider::new(ID_STARS, 0, MAX_STARS as i32, STARS_DEFAULT).with_step(STARS_STEP),
            stars: scene::stars(),
            projector: Projector::default(),
            drag: None,
            animating: false,
            focus: None,
        })
    }

    fn frame(&mut self, f: &mut Frame, dt_ms: i32) -> Result<(), VyError> {
        // The solid turns by itself unless the pointer holds it. `dt_ms`
        // is time to turn for only when the last frame asked for this
        // one; after a rest it is the time the page stood still.
        let spinning = self.spin.checked && self.drag.is_none();
        let ran = core::mem::replace(&mut self.animating, spinning);
        if spinning {
            if ran {
                self.turn(dt_ms.clamp(0, MAX_STEP_MS) * SPIN / 100, 0);
            }
            request_frame();
        }
        let l = self.layout();
        let window = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(window, 0.0, SKY)?;
        self.draw_stars(f, &l)?;
        self.draw_solid(f, &l)?;
        f.group(Role::Main, window, "", |f| {
            let size = f.text(16.0, 12.0, 0.0, self.s.title, "Rymd")?;
            let title = Rect::new(16.0, 12.0, size.w, size.h);
            f.sem.node(Role::Heading, title, "Rymd")?;
            let wire = if self.wire.checked {
                " som trådmodell"
            } else {
                ""
            };
            let text = cat(&[
                "3D-scen: ",
                NOUNS[self.shape],
                " med ",
                &number(self.solids[self.shape].faces.len() as u32),
                " ytor",
                wire,
                ", ",
                &number(self.count.value().max(0) as u32),
                " partiklar",
            ]);
            f.sem.node(Role::Image, l.scene, &text)?;
            self.draw_panel(f, &l, ran.then_some(dt_ms))
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let t = self.theme;
        let mut ui = Input::new(event);
        if let Event::Focus { node, gained, .. } = event {
            if gained {
                self.focus = Some(node.0);
            } else if self.focus == Some(node.0) {
                self.focus = None;
            }
            ui.set_redraw();
        }
        if let Event::Visibility { visible } = event {
            // Hidden, the page gets no frames; shown again it draws one,
            // whose time is the time it was away and turns nothing.
            self.animating = false;
            self.drag = None;
            return visible;
        }
        for (i, r) in l.shapes.iter().enumerate() {
            if self.pressed(&ui, ID_SHAPE - i as i32, *r) {
                ui.take();
                ui.set_redraw();
                self.shape = i;
            }
        }
        self.wire.event(&mut ui, l.wire, &t);
        self.spin.event(&mut ui, l.spin, &t);
        self.count.event(&mut ui, l.stars, &t);
        if self.scene_event(&mut ui, &l) {
            ui.set_redraw();
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
