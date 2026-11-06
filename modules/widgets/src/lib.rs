//! Vy reference module 12 (`widgets`): every control of `vy_sdk::ui` on
//! one page.
//!
//! A form card with a one-line and a multi-line [`TextBox`], a
//! [`Checkbox`], a [`RadioGroup`], a [`Slider`] and two [`Button`]s; a
//! card with a virtualised [`List`] of 24 rows; a card with a
//! [`ScrollView`] over a few paragraphs; the whole page in a main
//! `ScrollView`. Layout lives in `layout()`, built from [`Column`] and
//! [`Row`] stacks, and both `frame` and `event` use it.
//!
//! What it exercises: `TEXT_INPUT`, `IME_COMPOSE` and `ime_begin`/`ime_end`
//! through the fields (SPEC-5.2.6, capability `ime`, SPEC-10.4.2); Ctrl+C
//! and Ctrl+X writing the selection during the copy gesture (SPEC-8.2.4)
//! and the "Kopiera namnet" button writing outside it (capability
//! `clipboard`, SPEC-5.2.12); the host's focus on fixed node ids with the
//! rings the controls draw (SPEC-10.3.1, SPEC-10.3.3), and the main area
//! scrolling the control the keyboard focused into view; wheel, keys and
//! scroll bars; the whole semantic tree every frame, off-screen nodes too
//! (SPEC-10.1.1). A headless host has no clipboard, so the status line
//! shows its refusal there.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use vy_sdk::prelude::*;
use vy_sdk::ui::{
    self, Button, Checkbox, Column, Input, List, Palette, RadioGroup, Row, ScrollView, Slider,
    TextBox, Theme,
};
use vy_sdk::{clipboard_write, text_measure};

const PAPER: Color = Color::rgb(0xF2, 0xF3, 0xF5);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const FRAME: Color = Color::rgb(0xC5, 0xC9, 0xD2);

const MARGIN: f32 = 24.0;
const GAP: f32 = 16.0;
const PAD: f32 = 20.0;
const LABEL_GAP: f32 = 6.0;
const SECTION: f32 = 16.0;
const TWO_COLUMNS: f32 = 720.0;
const MAX_ONE: f32 = 560.0;
const MAX_TWO: f32 = 880.0;
const NOTE_H: f32 = 100.0;
const OPTION_H: f32 = 32.0;
const ROW_H: f32 = 36.0;
const LIST_ROWS: f32 = 6.0;
const BOX_H: f32 = 150.0;
const BOX_PAD: f32 = 12.0;

/// Fixed ids of the focusable nodes, in Tab order (SPEC-10.3.2).
const ID_NAME: i32 = -10;
const ID_NOTE: i32 = -11;
const ID_NEWS: i32 = -12;
const ID_SIZE: i32 = -13; // -13, -14, -15
const ID_VOLUME: i32 = -16;
const ID_SAVE: i32 = -17;
const ID_COPY: i32 = -18;

const TITLE: &str = "Komponenter";
const HELP: &str = "Alla kontroller i vy_sdk::ui på en sida. Tab flyttar fokus, hjulet rullar det som är under pekaren, och efter ett klick i listan eller rutan tar de piltangenterna.";
const HEADINGS: [&str; 3] = ["Formulär", "Lista", "Rullning"];
const SIZES: [&str; 3] = ["Liten", "Mellan", "Stor"];
const CITIES: [&str; 24] = [
    "Stockholm",
    "Göteborg",
    "Malmö",
    "Uppsala",
    "Västerås",
    "Örebro",
    "Linköping",
    "Helsingborg",
    "Jönköping",
    "Norrköping",
    "Lund",
    "Umeå",
    "Gävle",
    "Borås",
    "Södertälje",
    "Eskilstuna",
    "Halmstad",
    "Växjö",
    "Karlstad",
    "Sundsvall",
    "Östersund",
    "Trollhättan",
    "Luleå",
    "Kalmar",
];
const TERMS: [&str; 5] = [
    "Rutan är en ScrollView: hjulet rullar den när pekaren är över den, rullningslisten kan dras, och efter ett klick i rutan rullar piltangenterna, Page Up/Down och Home/End.",
    "Innehållet läggs ut med rutans förskjutning avdragen, så träfftest och ritning använder samma rektanglar. Det som hamnar utanför klipps bort, men varje stycke registreras ändå som en textnod.",
    "Listan ritar bara raderna i bild men registrerar alla 24 som listitem-noder. Upp och ned flyttar markeringen när listan är aktiv, och Enter öppnar raden.",
    "Fokus ägs av värden: Tab går genom textfälten, kryssrutan, radioknapparna, slidern och knapparna i dokumentordning, och varje kontroll ritar sin egen fokusring.",
    "Ctrl+C i ett textfält kopierar markeringen under kopieringsgesten. Huvudlöst finns inget urklipp, så statusraden visar värdens svar.",
];

#[derive(Clone, Copy)]
struct Styles {
    title: StyleId,
    help: StyleId,
    heading: StyleId,
    label: StyleId,
    body: StyleId,
}

/// Every rectangle on the page for the current window, scroll and status.
struct Layout {
    view: Rect,
    content_h: f32,
    title: Rect,
    help: Rect,
    cards: [Rect; 3],
    headings: [Rect; 3],
    name_label: Rect,
    name: Rect,
    note_label: Rect,
    note: Rect,
    news: Rect,
    size_label: Rect,
    sizes: Rect,
    volume_label: Rect,
    volume: Rect,
    save: Rect,
    copy: Rect,
    status: Rect,
    list: Rect,
    list_status: Rect,
    terms: Rect,
    terms_h: f32,
}

impl Layout {
    /// The rectangle of focusable node `id`, to scroll it into view.
    fn rect_of(&self, id: i32, size: &RadioGroup) -> Option<Rect> {
        Some(match id {
            ID_NAME => self.name,
            ID_NOTE => self.note,
            ID_NEWS => self.news,
            ID_VOLUME => self.volume,
            ID_SAVE => self.save,
            ID_COPY => self.copy,
            id if (ID_SIZE - 2..=ID_SIZE).contains(&id) => {
                size.option_rect(self.sizes, (ID_SIZE - id) as usize)
            }
            _ => return None,
        })
    }
}

struct Page {
    window: Window,
    theme: Theme,
    s: Styles,
    main: ScrollView,
    name: TextBox,
    note: TextBox,
    news: Checkbox,
    size: RadioGroup,
    volume: Slider,
    save: Button,
    copy: Button,
    list: List,
    terms: ScrollView,
    status: String,
    list_status: String,
}

fn measure(style: StyleId, w: f32, text: &str) -> f32 {
    text_measure(style, w, text).map_or(20.0, |s| s.h)
}

/// A text block with its node: drawn wrapped at `r.w`, registered over `r`.
fn block(f: &mut Frame, r: Rect, style: StyleId, role: Role, text: &str) -> Result<(), VyError> {
    f.text(r.x, r.y, r.w, style, text)?;
    f.sem.node(role, r, text)?;
    Ok(())
}

impl Page {
    fn layout(&self) -> Layout {
        let lh = self.theme.line_h;
        let w = self.window.width;
        let two = w >= TWO_COLUMNS;
        let content_w = (w - 2.0 * MARGIN).clamp(0.0, if two { MAX_TWO } else { MAX_ONE });
        let x = ((w - content_w) / 2.0).max(MARGIN);
        let top = MARGIN - self.main.offset();
        let title = Rect::new(x, top, content_w, measure(self.s.title, content_w, TITLE));
        let help = Rect::new(
            x,
            title.bottom() + 6.0,
            content_w,
            measure(self.s.help, content_w, HELP),
        );
        let y = help.bottom() + 20.0;
        let (left, right) = if two {
            let [a, b] = ui::split::<2>(Rect::new(x, y, content_w, 0.0), GAP);
            (a, b)
        } else {
            let all = Rect::new(x, y, content_w, 0.0);
            (all, all)
        };
        let heading_h = measure(self.s.heading, 0.0, HEADINGS[0]);

        let mut c = Column::new(left, PAD, 0.0);
        let h0 = c.next(heading_h);
        c.space(12.0);
        let name_label = c.next(lh);
        c.space(LABEL_GAP);
        let name = c.next(ui::CONTROL_H);
        c.space(SECTION);
        let note_label = c.next(lh);
        c.space(LABEL_GAP);
        let note = c.next(NOTE_H);
        c.space(SECTION);
        let news = c.next(28.0);
        c.space(12.0);
        let size_label = c.next(lh);
        c.space(LABEL_GAP);
        let sizes = c.next(3.0 * OPTION_H);
        c.space(SECTION);
        let volume_label = c.next(lh);
        c.space(LABEL_GAP);
        let volume = c.next(OPTION_H);
        c.space(20.0);
        let mut row = Row::new(c.next(ui::CONTROL_H), 0.0, 12.0);
        let save = row.next(110.0);
        let copy = row.next(170.0);
        c.space(12.0);
        let status_h = measure(self.s.body, c.width(), &self.status);
        let status = c.next(status_h);
        let form = Rect::new(left.x, left.y, left.w, c.height());

        let list_top = if two { right.y } else { form.bottom() + GAP };
        let mut c = Column::new(Rect::new(right.x, list_top, right.w, 0.0), PAD, 0.0);
        let h1 = c.next(heading_h);
        c.space(12.0);
        let list = c.next(LIST_ROWS * ROW_H);
        c.space(12.0);
        let list_status = c.next(lh);
        let list_card = Rect::new(right.x, list_top, right.w, c.height());

        let terms_top = list_card.bottom() + GAP;
        let mut c = Column::new(Rect::new(right.x, terms_top, right.w, 0.0), PAD, 0.0);
        let h2 = c.next(heading_h);
        c.space(12.0);
        let terms = c.next(BOX_H);
        let terms_card = Rect::new(right.x, terms_top, right.w, c.height());

        let bottom = form.bottom().max(terms_card.bottom());
        Layout {
            view: Rect::new(0.0, 0.0, w, self.window.height),
            content_h: bottom + self.main.offset() + MARGIN,
            title,
            help,
            cards: [form, list_card, terms_card],
            headings: [h0, h1, h2],
            name_label,
            name,
            note_label,
            note,
            news,
            size_label,
            sizes,
            volume_label,
            volume,
            save,
            copy,
            status,
            list,
            list_status,
            terms,
            terms_h: self.terms_height(terms.w),
        }
    }

    fn terms_width(box_w: f32) -> f32 {
        (box_w - 2.0 * BOX_PAD - ui::BAR_W).max(1.0)
    }

    /// Height of the paragraphs in the scroll box, with its padding.
    fn terms_height(&self, box_w: f32) -> f32 {
        let w = Self::terms_width(box_w);
        let text: f32 = TERMS.iter().map(|p| measure(self.s.body, w, p)).sum();
        text + (TERMS.len() - 1) as f32 * 10.0 + 2.0 * BOX_PAD
    }

    fn save_status(&mut self) {
        let name = self.name.text().trim();
        self.status = format!(
            "Sparat: {}, storlek {}, volym {}, nyheter {}.",
            if name.is_empty() { "inget namn" } else { name },
            SIZES[self.size.selected],
            self.volume.value(),
            if self.news.checked { "ja" } else { "nej" },
        );
    }

    fn copy_name(&mut self) {
        self.status = if self.name.text().is_empty() {
            String::from("Inget namn att kopiera.")
        } else {
            match clipboard_write(self.name.text()) {
                Ok(()) => String::from("Namnet är kopierat."),
                Err(e) => format!("Kopiera: urklippet nekades ({}).", e.name()),
            }
        };
    }

    fn draw_form(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let t = self.theme;
        let s = self.s;
        block(f, l.name_label, s.label, Role::Text, "Namn")?;
        self.name.draw(f, l.name, &t, "Förnamn Efternamn")?;
        block(f, l.note_label, s.label, Role::Text, "Anteckning")?;
        self.note
            .draw(f, l.note, &t, "Flera rader; Enter bryter raden.")?;
        self.news.draw(f, l.news, &t, "Skicka nyhetsbrevet")?;
        block(f, l.size_label, s.label, Role::Text, "Storlek")?;
        self.size.draw(f, l.sizes, &t, &SIZES)?;
        block(f, l.volume_label, s.label, Role::Text, "Volym")?;
        self.volume.draw(f, l.volume, &t, "Volym")?;
        self.save.draw(f, l.save, &t, "Spara")?;
        self.copy.draw(f, l.copy, &t, "Kopiera namnet")?;
        block(f, l.status, s.body, Role::Text, &self.status)
    }

    fn draw_terms(&self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let w = Self::terms_width(l.terms.w);
        let x = l.terms.x + BOX_PAD;
        let mut y = l.terms.y + BOX_PAD - self.terms.offset();
        for p in TERMS {
            let h = measure(self.s.body, w, p);
            block(f, Rect::new(x, y, w, h), self.s.body, Role::Text, p)?;
            y += h + 10.0;
        }
        Ok(())
    }

    fn draw_page(&mut self, f: &mut Frame, l: &Layout) -> Result<(), VyError> {
        let main = Rect::new(l.title.x, l.title.y, l.title.w, l.content_h);
        f.group(Role::Main, main, "", |f| {
            block(f, l.title, self.s.title, Role::Heading, TITLE)?;
            block(f, l.help, self.s.help, Role::Text, HELP)?;
            for (i, card) in l.cards.iter().enumerate() {
                f.rect(*card, 12.0, CARD)?;
                f.group(Role::Region, *card, HEADINGS[i], |f| {
                    block(f, l.headings[i], self.s.heading, Role::Heading, HEADINGS[i])?;
                    match i {
                        0 => self.draw_form(f, l),
                        1 => {
                            let t = self.theme;
                            self.list.draw(f, l.list, &t, &CITIES)?;
                            block(f, l.list_status, self.s.body, Role::Text, &self.list_status)
                        }
                        _ => {
                            f.rect(l.terms.inset(-1.0), ui::RADIUS + 1.0, FRAME)?;
                            f.rect(l.terms, ui::RADIUS, CARD)?;
                            let t = self.theme;
                            let mut terms = self.terms;
                            let drawn =
                                terms.draw(f, l.terms, &t, l.terms_h, |f| self.draw_terms(f, l));
                            self.terms = terms;
                            drawn
                        }
                    }
                })?;
            }
            Ok(())
        })
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let p = Palette::LIGHT;
        let theme = Theme::new()?;
        let mut main = ScrollView::new();
        // The page's own area takes the keys until something else does.
        main.set_active(true);
        Ok(Self {
            window,
            theme,
            s: Styles {
                title: text_style(Font::Sans, 26.0, 700, p.ink)?,
                help: text_style(Font::Sans, 14.0, 400, p.muted)?,
                heading: text_style(Font::Sans, 15.0, 700, p.muted)?,
                label: text_style(Font::Sans, 13.0, 600, p.muted)?,
                body: text_style(Font::Sans, 14.0, 400, p.ink)?,
            },
            main,
            name: TextBox::new(ID_NAME),
            note: TextBox::multiline(ID_NOTE),
            news: Checkbox::new(ID_NEWS, false),
            size: RadioGroup::new(ID_SIZE, SIZES.len(), 1),
            volume: Slider::new(ID_VOLUME, 0, 100, 40),
            save: Button::new(ID_SAVE),
            copy: Button::secondary(ID_COPY),
            list: List::new(ROW_H),
            terms: ScrollView::new(),
            status: String::from("Inget sparat än."),
            list_status: String::from("Ingen rad vald."),
        })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        f.rect(l.view, 0.0, PAPER)?;
        let t = self.theme;
        let mut main = self.main;
        let drawn = main.draw(f, l.view, &t, l.content_h, |f| self.draw_page(f, &l));
        self.main = main;
        drawn
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let t = self.theme;
        let copies = (self.name.last_copy(), self.note.last_copy());
        let mut ui = Input::new(event);
        // Inner components first, the main area last.
        let name = self.name.event(&mut ui, l.name, &t);
        self.note.event(&mut ui, l.note, &t);
        self.news.event(&mut ui, l.news, &t);
        self.size.event(&mut ui, l.sizes, &t);
        self.volume.event(&mut ui, l.volume, &t);
        let save = self.save.event(&mut ui, l.save, &t);
        let copy = self.copy.event(&mut ui, l.copy, &t);
        let list = self.list.event(&mut ui, l.list, &t, &CITIES);
        self.terms.event(&mut ui, l.terms, &t, l.terms_h);
        self.main.event(&mut ui, l.view, &t, l.content_h);

        if save.activated() || name.activated() {
            self.save_status();
            ui.set_redraw();
        }
        if copy.activated() {
            self.copy_name();
            ui.set_redraw();
        }
        let now = (self.name.last_copy(), self.note.last_copy());
        if now != copies {
            let last = if now.0 != copies.0 { now.0 } else { now.1 };
            self.status = match last {
                Some(Ok(())) => String::from("Ctrl+C: markeringen är kopierad."),
                Some(Err(e)) => format!("Ctrl+C: urklippet nekades ({}).", e.name()),
                None => String::new(),
            };
            ui.set_redraw();
        }
        if let Some(i) = self.list.selected() {
            if list.activated() {
                self.list_status = format!("Öppnad: {}", CITIES[i]);
            } else if list.changed() {
                self.list_status = format!("Vald: {}", CITIES[i]);
            }
        }
        // Keyboard focus moved to a control: bring it into view.
        if let Event::Focus {
            node,
            gained: true,
            keyboard: true,
        } = event
        {
            if let Some(r) = l.rect_of(node.0, &self.size) {
                if self.main.reveal(r.inset(-8.0), l.view, l.content_h) {
                    ui.set_redraw();
                }
            }
        }
        ui.redraw()
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
        let l = self.layout();
        self.main.clamp(l.view, l.content_h);
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::event::{key, Modifiers};
    use vy_sdk::mock;

    fn page(width: f32, height: f32) -> Page {
        mock::reset();
        mock::with(|h| h.caps.ime = true);
        Page::init(
            Window {
                width,
                height,
                scale: 1.0,
            },
            0,
        )
        .unwrap()
    }

    fn press(page: &mut Page, r: Rect) -> bool {
        page.event(Event::PointerDown {
            x: r.x + 4.0,
            y: r.y + r.h / 2.0,
            button: 1,
            modifiers: Modifiers(0),
        })
    }

    fn focus(page: &mut Page, id: i32) -> bool {
        page.event(Event::Focus {
            node: NodeId(id),
            gained: true,
            keyboard: true,
        })
    }

    /// SPEC-10.1.1, SPEC-10.3.2: one frame registers every control with its
    /// role and fixed id, in Tab order, and every list row.
    #[test]
    fn spec_10_1_1_whole_tree_in_tab_order() {
        let mut p = page(800.0, 600.0);
        assert!(mock::run_frame(&mut p, 0).unwrap() > 50);
        mock::with(|h| {
            let focusable: Vec<i32> = h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect();
            assert_eq!(focusable, [-10, -11, -12, -13, -14, -15, -16, -17, -18]);
            assert_eq!(
                h.sem
                    .iter()
                    .filter(|n| n.role == Role::ListItem as i32)
                    .count(),
                24
            );
            assert!(h.sem.iter().any(|n| n.text == "Mellan (vald)"));
            assert!(h.sem.iter().any(|n| n.text == "Volym: 40"));
            assert!(h.refusals.is_empty(), "{:?}", h.refusals);
        });
    }

    /// SPEC-5.1.6, SPEC-8.2.4: typed text lands in the focused field, Enter
    /// saves, and Ctrl+C reports the host's answer in the status line.
    #[test]
    fn spec_8_2_4_type_save_and_copy() {
        let mut p = page(800.0, 600.0);
        let l = p.layout();
        focus(&mut p, ID_NAME);
        assert!(p.event(Event::TextInput { text: "Anna" }));
        assert!(p.event(Event::KeyDown {
            code: key::ENTER,
            modifiers: Modifiers(0),
            repeat: false
        }));
        assert_eq!(
            p.status,
            "Sparat: Anna, storlek Mellan, volym 40, nyheter nej."
        );
        p.name.select_all();
        let ctrl_c = Event::KeyDown {
            code: key::C,
            modifiers: Modifiers(2),
            repeat: false,
        };
        assert!(p.event(ctrl_c));
        assert_eq!(p.status, "Ctrl+C: urklippet nekades (E_DENIED).");
        assert!(mock::as_copy_gesture(|| p.event(ctrl_c)));
        assert_eq!(p.status, "Ctrl+C: markeringen är kopierad.");
        mock::with(|h| assert_eq!(h.clipboard, "Anna"));
        assert!(press(&mut p, l.copy));
        assert_eq!(p.status, "Kopiera: urklippet nekades (E_DENIED).");
    }

    /// SPEC-10.3.1: keyboard focus on a control below the window scrolls
    /// the main area so the control is in view; the list takes the arrows
    /// after a press in it.
    #[test]
    fn spec_10_3_1_focus_scrolls_into_view() {
        let mut p = page(400.0, 600.0);
        let copy = p.layout().copy;
        assert!(copy.bottom() > 600.0, "below the window at first");
        assert!(focus(&mut p, ID_COPY));
        let l = p.layout();
        assert!(l.copy.bottom() <= 600.0 && l.copy.y >= 0.0, "{:?}", l.copy);
        assert!(p.main.offset() > 0.0);

        let mut p = page(800.0, 600.0);
        let list = p.list.row_rect(p.layout().list, 2);
        assert!(press(&mut p, list));
        assert_eq!(p.list_status, "Vald: Malmö");
        assert!(p.event(Event::KeyDown {
            code: key::DOWN,
            modifiers: Modifiers(0),
            repeat: false
        }));
        assert_eq!(p.list_status, "Vald: Uppsala");
        assert_eq!(p.main.offset(), 0.0, "the list took the key, not the page");
    }
}
