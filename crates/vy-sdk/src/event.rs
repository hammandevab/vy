//! `vy_event` arguments decoded per SPEC appendix A (SPEC-A.2.1, SPEC-5.1.3).

use crate::from_x100;
use crate::sem::NodeId;
use vy_abi::{event as kind, modifier};

/// Modifier keys held during an input event.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modifiers(pub i32);

impl Modifiers {
    /// Shift.
    pub fn shift(self) -> bool {
        self.0 & modifier::SHIFT != 0
    }
    /// Control.
    pub fn ctrl(self) -> bool {
        self.0 & modifier::CTRL != 0
    }
    /// Alt/Option.
    pub fn alt(self) -> bool {
        self.0 & modifier::ALT != 0
    }
    /// Command/Windows.
    pub fn meta(self) -> bool {
        self.0 & modifier::META != 0
    }
}

/// One event as the host delivers it. Coordinates are logical pixels; the
/// ABI carries them × 100.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Event<'a> {
    /// Pointer moved; `buttons` is the mask of buttons held.
    PointerMove {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// Buttons held.
        buttons: i32,
        /// Modifier keys.
        modifiers: Modifiers,
    },
    /// A pointer button was pressed.
    PointerDown {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// The button.
        button: i32,
        /// Modifier keys.
        modifiers: Modifiers,
    },
    /// A pointer button was released.
    PointerUp {
        /// X.
        x: f32,
        /// Y.
        y: f32,
        /// The button.
        button: i32,
        /// Modifier keys.
        modifiers: Modifiers,
    },
    /// Scroll by `(dx, dy)`, in pixels or in lines.
    Scroll {
        /// Horizontal delta.
        dx: f32,
        /// Vertical delta; positive scrolls content up.
        dy: f32,
        /// True when the deltas are lines, not pixels.
        lines: bool,
        /// Modifier keys.
        modifiers: Modifiers,
    },
    /// A key went down.
    KeyDown {
        /// USB HID usage code.
        code: i32,
        /// Modifier keys.
        modifiers: Modifiers,
        /// True for auto-repeat.
        repeat: bool,
    },
    /// A key went up.
    KeyUp {
        /// USB HID usage code.
        code: i32,
        /// Modifier keys.
        modifiers: Modifiers,
    },
    /// Committed text, written by the host into a `vy_alloc` buffer (SPEC-5.1.6).
    TextInput {
        /// The text.
        text: &'a str,
    },
    /// Composition in progress; `caret` is a byte offset into `text`.
    ImeCompose {
        /// The composing text.
        text: &'a str,
        /// Caret position.
        caret: i32,
    },
    /// The pointer left the page; hover state should be cleared.
    PointerLeave,
    /// Keyboard focus moved to or from a semantic node: the host moved it
    /// with Tab, Shift+Tab or a pointer press (SPEC-10.3.1); the page draws
    /// the ring (SPEC-10.3.3).
    Focus {
        /// The node.
        node: NodeId,
        /// True when the node gained focus.
        gained: bool,
        /// True when the change came from the keyboard, false from the pointer.
        keyboard: bool,
    },
    /// A `fetch_begin` request finished.
    FetchDone {
        /// The request.
        request: i32,
        /// HTTP status or negative error code.
        status: i32,
        /// Bytes available to `fetch_read`.
        bytes: i32,
    },
    /// The tab became visible or hidden.
    Visibility {
        /// True when visible.
        visible: bool,
    },
    /// The frame budget was exceeded (SPEC-5.3.3).
    Budget {
        /// Logical milliseconds used.
        used_ms: i32,
        /// Logical milliseconds allowed.
        allowed_ms: i32,
        /// True on the final warning before a forced `vy_suspend`.
        last_warning: bool,
    },
    /// A capability was granted or revoked (`vy_abi::cap`).
    Capability {
        /// Capability id.
        id: i32,
        /// True when granted.
        granted: bool,
    },
    /// A kind this SDK does not know (SPEC-A.2.1: new kinds get new numbers).
    Unknown {
        /// Kind.
        kind: i32,
        /// First argument.
        a: i32,
        /// Second argument.
        b: i32,
        /// Third argument.
        c: i32,
        /// Fourth argument.
        d: i32,
    },
}

impl<'a> Event<'a> {
    /// Decode the arguments of `vy_event`. `text` resolves the `(ptr, len)`
    /// of `TEXT_INPUT` and `IME_COMPOSE` into the buffer the host wrote; when
    /// it returns `None` the event is delivered as [`Event::Unknown`].
    pub fn decode(
        kind: i32,
        a: i32,
        b: i32,
        c: i32,
        d: i32,
        text: impl FnOnce(i32, i32) -> Option<&'a str>,
    ) -> Self {
        let unknown = Event::Unknown { kind, a, b, c, d };
        match kind {
            kind::POINTER_MOVE => Event::PointerMove {
                x: from_x100(a),
                y: from_x100(b),
                buttons: c,
                modifiers: Modifiers(d),
            },
            kind::POINTER_DOWN => Event::PointerDown {
                x: from_x100(a),
                y: from_x100(b),
                button: c,
                modifiers: Modifiers(d),
            },
            kind::POINTER_UP => Event::PointerUp {
                x: from_x100(a),
                y: from_x100(b),
                button: c,
                modifiers: Modifiers(d),
            },
            kind::SCROLL => Event::Scroll {
                dx: from_x100(a),
                dy: from_x100(b),
                lines: c == 1,
                modifiers: Modifiers(d),
            },
            kind::KEY_DOWN => Event::KeyDown {
                code: a,
                modifiers: Modifiers(b),
                repeat: c != 0,
            },
            kind::KEY_UP => Event::KeyUp {
                code: a,
                modifiers: Modifiers(b),
            },
            kind::TEXT_INPUT => match text(a, b) {
                Some(text) => Event::TextInput { text },
                None => unknown,
            },
            kind::IME_COMPOSE => match text(a, b) {
                Some(text) => Event::ImeCompose { text, caret: c },
                None => unknown,
            },
            kind::POINTER_LEAVE => Event::PointerLeave,
            kind::FOCUS => Event::Focus {
                node: NodeId(a),
                gained: b == 1,
                keyboard: c == 1,
            },
            kind::FETCH_DONE => Event::FetchDone {
                request: a,
                status: b,
                bytes: c,
            },
            kind::VISIBILITY => Event::Visibility { visible: a == 1 },
            kind::BUDGET => Event::Budget {
                used_ms: a,
                allowed_ms: b,
                last_warning: c == 1,
            },
            kind::CAPABILITY => Event::Capability {
                id: a,
                granted: b == 1,
            },
            _ => unknown,
        }
    }
}

/// USB HID usage codes of keys pages commonly handle (SPEC-A.2.1: `KEY_DOWN`
/// carries USB HID keycodes). Tab never reaches a page: the host turns it
/// into `FOCUS` events (SPEC-10.3.1).
pub mod key {
    /// Return / Enter.
    pub const ENTER: i32 = vy_abi::key::ENTER;
    /// Escape.
    pub const ESCAPE: i32 = vy_abi::key::ESCAPE;
    /// Backspace.
    pub const BACKSPACE: i32 = 0x2A;
    /// Tab; consumed by the host, listed so a page can name it.
    pub const TAB: i32 = vy_abi::key::TAB;
    /// Space.
    pub const SPACE: i32 = 0x2C;
    /// Delete forward.
    pub const DELETE: i32 = 0x4C;
    /// Home.
    pub const HOME: i32 = 0x4A;
    /// Page Up.
    pub const PAGE_UP: i32 = 0x4B;
    /// End.
    pub const END: i32 = 0x4D;
    /// Page Down.
    pub const PAGE_DOWN: i32 = 0x4E;
    /// Right arrow.
    pub const RIGHT: i32 = 0x4F;
    /// Left arrow.
    pub const LEFT: i32 = 0x50;
    /// Down arrow.
    pub const DOWN: i32 = 0x51;
    /// Up arrow.
    pub const UP: i32 = 0x52;
}
