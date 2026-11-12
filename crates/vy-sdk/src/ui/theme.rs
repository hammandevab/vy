//! Colours, words and text styles the components draw with.

use crate::api::{text_measure, text_style};
use crate::{Color, Font, StyleId, VyError};

/// The colours of a [`Theme`]. Every component draws with these and
/// nothing else, so a page restyles all of them at once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Palette {
    /// Background of fields, boxes, secondary buttons and lists.
    pub surface: Color,
    /// Text and the caret.
    pub ink: Color,
    /// Secondary text.
    pub muted: Color,
    /// A field's placeholder text.
    pub placeholder: Color,
    /// Borders of fields, boxes and unselected radio buttons.
    pub border: Color,
    /// Focused fields, checked boxes, primary buttons, the slider's fill.
    pub accent: Color,
    /// Text and ticks on the accent colour.
    pub on_accent: Color,
    /// The focus ring (SPEC-10.3.3).
    pub ring: Color,
    /// Selected text in a field and the selected row of a list.
    pub selection: Color,
    /// The slider's track and the scroll bar.
    pub track: Color,
    /// Lines between list rows.
    pub rule: Color,
}

impl Palette {
    /// Light: dark ink on white, blue accent, amber focus ring.
    pub const LIGHT: Palette = Palette {
        surface: Color::rgb(0xFF, 0xFF, 0xFF),
        ink: Color::rgb(0x1C, 0x1E, 0x26),
        muted: Color::rgb(0x6B, 0x70, 0x7C),
        placeholder: Color::rgb(0x6B, 0x70, 0x7C),
        border: Color::rgb(0xC5, 0xC9, 0xD2),
        accent: Color::rgb(0x2F, 0x5B, 0xD6),
        on_accent: Color::rgb(0xFF, 0xFF, 0xFF),
        ring: Color::rgb(0xF2, 0x9F, 0x05),
        selection: Color::rgba(0x2F, 0x5B, 0xD6, 0x38),
        track: Color::rgb(0xC9, 0xCD, 0xD6),
        rule: Color::rgb(0xE3, 0xE5, 0xE8),
    };

    /// Dark: light ink on a dark surface.
    pub const DARK: Palette = Palette {
        surface: Color::rgb(0x1F, 0x22, 0x2B),
        ink: Color::rgb(0xEC, 0xEE, 0xF2),
        muted: Color::rgb(0x9A, 0xA0, 0xAE),
        placeholder: Color::rgb(0x9A, 0xA0, 0xAE),
        border: Color::rgb(0x4A, 0x50, 0x60),
        accent: Color::rgb(0x6F, 0x95, 0xFF),
        on_accent: Color::rgb(0x14, 0x16, 0x1C),
        ring: Color::rgb(0xF2, 0x9F, 0x05),
        selection: Color::rgba(0x6F, 0x95, 0xFF, 0x50),
        track: Color::rgb(0x3A, 0x3F, 0x4E),
        rule: Color::rgb(0x33, 0x37, 0x44),
    };
}

/// The words a component adds to a node's text to say its state, since a
/// semantic node carries a role and a text and nothing else (SPEC-5.2.13).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Words {
    /// After the label of a checked checkbox.
    pub checked: &'static str,
    /// After the label of the selected radio button or list row.
    pub selected: &'static str,
}

impl Words {
    /// Swedish (the default): " (markerad)", " (vald)".
    pub const SV: Words = Words {
        checked: " (markerad)",
        selected: " (vald)",
    };
    /// English: " (checked)", " (selected)".
    pub const EN: Words = Words {
        checked: " (checked)",
        selected: " (selected)",
    };
}

/// Palette, words and the text styles every component draws with. Build it
/// once in `App::init`: [`Theme::new`] registers four styles with the host
/// (SPEC-5.2.5) and measures the line height.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The colours.
    pub palette: Palette,
    /// The state words of semantic nodes.
    pub words: Words,
    /// Control text: field values, labels, list rows, the slider's value.
    pub text: StyleId,
    /// A field's placeholder.
    pub placeholder: StyleId,
    /// A primary button's label (semibold, `on_accent`).
    pub button: StyleId,
    /// A secondary button's label (semibold, `accent`).
    pub button_secondary: StyleId,
    /// Height of one line of `text`, as the host measured it.
    pub line_h: f32,
}

impl Theme {
    /// The light palette, Swedish words, the sans-serif font at 16 px.
    pub fn new() -> Result<Self, VyError> {
        Self::custom(Palette::LIGHT, Font::Sans, 16.0)
    }

    /// A theme with another palette, font or size; the words are Swedish
    /// until the page sets [`Theme::words`].
    pub fn custom(palette: Palette, font: Font, size_px: f32) -> Result<Self, VyError> {
        let text = text_style(font, size_px, 400, palette.ink)?;
        Ok(Self {
            palette,
            words: Words::SV,
            text,
            placeholder: text_style(font, size_px, 400, palette.placeholder)?,
            button: text_style(font, size_px, 600, palette.on_accent)?,
            button_secondary: text_style(font, size_px, 600, palette.accent)?,
            line_h: text_measure(text, 0.0, "Ag")?.h,
        })
    }
}
