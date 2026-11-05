//! ABI constants shared by the Vy host and Vy modules.
//!
//! Everything in this crate is normative and traced to `SPEC.md`. The numeric
//! values never change within 1.x (SPEC-5.3.5, SPEC-A.1.1, SPEC-A.2.1).
#![no_std]
#![deny(missing_docs)]

/// ABI major version.
pub const ABI_MAJOR: u32 = 1;
/// ABI minor version.
pub const ABI_MINOR: u32 = 0;
/// Value returned by the `abi_version` import: `MAJOR * 100 + MINOR` (SPEC-5.2.1).
pub const ABI_VERSION: i32 = (ABI_MAJOR * 100 + ABI_MINOR) as i32;
/// Import namespace every module import must use (SPEC-4.2.2).
pub const IMPORT_MODULE: &str = "vy";

/// Status codes returned by host imports (SPEC appendix A, SPEC-A.1.1).
pub mod err {
    /// Success.
    pub const OK: i32 = 0;
    /// Capability missing or revoked.
    pub const E_DENIED: i32 = -1;
    /// Invalid argument (pointer outside memory, negative length).
    pub const E_ARG: i32 = -2;
    /// Module memory cap reached.
    pub const E_OOM: i32 = -3;
    /// Storage quota exhausted.
    pub const E_QUOTA: i32 = -4;
    /// Network or call rate limit exceeded.
    pub const E_RATE: i32 = -5;
    /// Display list call outside `dl_begin`/`dl_end`.
    pub const E_DL_STATE: i32 = -10;
    /// 65 536 opcodes reached.
    pub const E_DL_FULL: i32 = -11;
    /// Clip stack deeper than 32 or pop without push.
    pub const E_DL_CLIP: i32 = -12;
    /// Unknown resource id or wrong type.
    pub const E_RESOURCE: i32 = -20;
    /// Unknown style id.
    pub const E_STYLE: i32 = -21;
    /// Origin not listed in the manifest's `net` capability.
    pub const E_ORIGIN: i32 = -30;
    /// Transport error.
    pub const E_NET: i32 = -31;
    /// Response larger than 64 MB.
    pub const E_TOO_LARGE: i32 = -32;
    /// Call requires a user gesture within 1 s.
    pub const E_GESTURE: i32 = -40;

    /// Every status code with its name, in the order of appendix A.
    pub const ALL: [(i32, &str); 15] = [
        (OK, "OK"),
        (E_DENIED, "E_DENIED"),
        (E_ARG, "E_ARG"),
        (E_OOM, "E_OOM"),
        (E_QUOTA, "E_QUOTA"),
        (E_RATE, "E_RATE"),
        (E_DL_STATE, "E_DL_STATE"),
        (E_DL_FULL, "E_DL_FULL"),
        (E_DL_CLIP, "E_DL_CLIP"),
        (E_RESOURCE, "E_RESOURCE"),
        (E_STYLE, "E_STYLE"),
        (E_ORIGIN, "E_ORIGIN"),
        (E_NET, "E_NET"),
        (E_TOO_LARGE, "E_TOO_LARGE"),
        (E_GESTURE, "E_GESTURE"),
    ];

    /// Name of a status code, or `None` for a code 1.0 does not define.
    pub fn name(code: i32) -> Option<&'static str> {
        ALL.iter().find(|(c, _)| *c == code).map(|(_, n)| *n)
    }
}

/// Event kinds delivered through `vy_event` (SPEC appendix A, SPEC-A.2.1).
pub mod event {
    /// Pointer moved: a = x*100, b = y*100, c = buttons, d = modifiers.
    pub const POINTER_MOVE: i32 = 1;
    /// Pointer button pressed.
    pub const POINTER_DOWN: i32 = 2;
    /// Pointer button released.
    pub const POINTER_UP: i32 = 3;
    /// Scroll: a = dx*100, b = dy*100, c = 0 pixels / 1 lines, d = modifiers.
    pub const SCROLL: i32 = 4;
    /// Key pressed: a = USB HID keycode, b = modifiers, c = repeat.
    pub const KEY_DOWN: i32 = 5;
    /// Key released.
    pub const KEY_UP: i32 = 6;
    /// Committed text: a = ptr, b = len (written via `vy_alloc`).
    pub const TEXT_INPUT: i32 = 7;
    /// IME composition in progress: a = ptr, b = len, c = caret.
    pub const IME_COMPOSE: i32 = 8;
    /// The pointer left the page's area; no arguments. Until the next
    /// `POINTER_MOVE` the page has no pointer position.
    pub const POINTER_LEAVE: i32 = 9;
    /// Focus change: a = semantic node id, b = 1 in / 0 out, c = 0 mouse / 1 keyboard.
    pub const FOCUS: i32 = 10;
    /// Fetch finished: a = req_id, b = status or error, c = bytes.
    pub const FETCH_DONE: i32 = 20;
    /// Visibility: a = 1 visible / 0 hidden.
    pub const VISIBILITY: i32 = 30;
    /// Frame budget warning: a = ms used, b = ms allowed, c = 1 if final warning.
    pub const BUDGET: i32 = 31;
    /// Capability granted or revoked: a = capability id, b = 1 granted / 0 revoked.
    pub const CAPABILITY: i32 = 32;

    /// One event kind as appendix A lists it: number, name and what
    /// `a`..`d` carry (`-` for an unused argument).
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Kind {
        /// The `kind` argument of `vy_event`.
        pub kind: i32,
        /// Name as the spec writes it.
        pub name: &'static str,
        /// Meaning of `a`, `b`, `c` and `d`.
        pub args: [&'static str; 4],
    }

    /// Every kind of 1.0 in numeric order (SPEC-A.2.1).
    pub const ALL: [Kind; 14] = [
        Kind {
            kind: POINTER_MOVE,
            name: "POINTER_MOVE",
            args: ["x × 100", "y × 100", "knappar", "modifierare"],
        },
        Kind {
            kind: POINTER_DOWN,
            name: "POINTER_DOWN",
            args: ["x × 100", "y × 100", "knapp", "modifierare"],
        },
        Kind {
            kind: POINTER_UP,
            name: "POINTER_UP",
            args: ["x × 100", "y × 100", "knapp", "modifierare"],
        },
        Kind {
            kind: SCROLL,
            name: "SCROLL",
            args: ["dx × 100", "dy × 100", "0 pixlar / 1 rader", "modifierare"],
        },
        Kind {
            kind: KEY_DOWN,
            name: "KEY_DOWN",
            args: ["keycode (USB HID)", "modifierare", "repeat", "-"],
        },
        Kind {
            kind: KEY_UP,
            name: "KEY_UP",
            args: ["keycode", "modifierare", "-", "-"],
        },
        Kind {
            kind: TEXT_INPUT,
            name: "TEXT_INPUT",
            args: ["ptr", "len", "-", "-"],
        },
        Kind {
            kind: IME_COMPOSE,
            name: "IME_COMPOSE",
            args: ["ptr", "len", "markör", "-"],
        },
        Kind {
            kind: POINTER_LEAVE,
            name: "POINTER_LEAVE",
            args: ["-", "-", "-", "-"],
        },
        Kind {
            kind: FOCUS,
            name: "FOCUS",
            args: ["nod-id", "1 in / 0 ut", "0 mus / 1 tangentbord", "-"],
        },
        Kind {
            kind: FETCH_DONE,
            name: "FETCH_DONE",
            args: ["req_id", "HTTP-status eller negativ felkod", "bytes", "-"],
        },
        Kind {
            kind: VISIBILITY,
            name: "VISIBILITY",
            args: ["1 synlig / 0 gömd", "-", "-", "-"],
        },
        Kind {
            kind: BUDGET,
            name: "BUDGET",
            args: ["ms använt", "ms tillåtet", "1 = sista varningen", "-"],
        },
        Kind {
            kind: CAPABILITY,
            name: "CAPABILITY",
            args: ["capability-id", "1 beviljad / 0 återkallad", "-", "-"],
        },
    ];

    /// Name of a kind, or `None` for one 1.0 does not define.
    pub fn name(kind: i32) -> Option<&'static str> {
        ALL.iter().find(|k| k.kind == kind).map(|k| k.name)
    }
}

/// `level` argument of `log` (SPEC-5.2.1). A value outside 0..=3 is logged
/// as `ERROR`: a log call never fails on its level.
pub mod level {
    /// Development chatter.
    pub const DEBUG: i32 = 0;
    /// Normal operation.
    pub const INFO: i32 = 1;
    /// Something is off but the page goes on.
    pub const WARN: i32 = 2;
    /// The page cannot do what it meant to.
    pub const ERROR: i32 = 3;
    /// Highest level.
    pub const MAX: i32 = 3;

    /// Names in level order (SPEC-5.2.1).
    pub const NAMES: [&str; 4] = ["debug", "info", "warn", "error"];

    /// The level a host records a `log` call at: the argument when it is
    /// one of the four, else `ERROR` (SPEC-5.2.1).
    pub const fn clamp(level: i32) -> i32 {
        if level >= DEBUG && level <= MAX {
            level
        } else {
            MAX
        }
    }

    /// Name of a level after [`clamp`].
    pub fn name(level: i32) -> &'static str {
        NAMES[clamp(level) as usize]
    }
}

/// `method` argument of `fetch_begin` (SPEC-5.2.7). Any other value is
/// `E_ARG` and no request is made.
pub mod method {
    /// `GET`.
    pub const GET: i32 = 0;
    /// `POST`.
    pub const POST: i32 = 1;
    /// `PUT`.
    pub const PUT: i32 = 2;
    /// `DELETE`.
    pub const DELETE: i32 = 3;
    /// Highest valid method.
    pub const MAX: i32 = 3;

    /// HTTP method names in value order (SPEC-5.2.7).
    pub const NAMES: [&str; 4] = ["GET", "POST", "PUT", "DELETE"];

    /// HTTP method name of a value, or `None` outside 0..=3.
    pub fn name(method: i32) -> Option<&'static str> {
        if (GET..=MAX).contains(&method) {
            Some(NAMES[method as usize])
        } else {
            None
        }
    }
}

/// Built-in font roles: the negative `font_resource_id` values `text_style`
/// takes (SPEC-5.2.5, SPEC-6.4.2). A positive id names a font resource of
/// the package (SPEC-4.5.1); 0 is the module and never a font.
pub mod font {
    /// Proportional sans-serif.
    pub const SANS: i32 = -1;
    /// Proportional serif.
    pub const SERIF: i32 = -2;
    /// Monospace.
    pub const MONO: i32 = -3;
    /// The host's interface face.
    pub const UI: i32 = -4;
    /// Emoji.
    pub const EMOJI: i32 = -5;
    /// Lowest (most negative) role id.
    pub const MIN: i32 = -5;

    /// Role names in id order, from `SANS` (−1) to `EMOJI` (−5) (SPEC-6.4.2).
    pub const NAMES: [&str; 5] = ["sans", "serif", "mono", "ui", "emoji"];

    /// Name of a built-in role, or `None` for an id that is not one.
    pub fn name(id: i32) -> Option<&'static str> {
        if (MIN..=SANS).contains(&id) {
            Some(NAMES[(-id - 1) as usize])
        } else {
            None
        }
    }
}

/// `weight` argument of `text_style` (SPEC-5.2.5): 1..=1000 as CSS numbers
/// it. The host clamps a value outside the range and draws with the face
/// whose weight is nearest the requested one, the heavier on a tie.
pub mod weight {
    /// Lightest weight.
    pub const MIN: i32 = 1;
    /// Heaviest weight.
    pub const MAX: i32 = 1000;
    /// Regular text.
    pub const NORMAL: i32 = 400;
    /// Bold text.
    pub const BOLD: i32 = 700;

    /// The weight a host shapes with: the argument clamped to 1..=1000.
    pub const fn clamp(weight: i32) -> i32 {
        if weight < MIN {
            MIN
        } else if weight > MAX {
            MAX
        } else {
            weight
        }
    }

    /// The face a host picks for `weight` among the weights it has for a
    /// role: the nearest, and the heavier when two are equally near
    /// (SPEC-5.2.5). `None` only for an empty list.
    pub fn nearest(weight: i32, available: &[i32]) -> Option<i32> {
        let want = clamp(weight);
        let mut best: Option<i32> = None;
        for &have in available {
            best = Some(match best {
                None => have,
                Some(b) => {
                    let (db, dh) = ((b - want).abs(), (have - want).abs());
                    if dh < db || (dh == db && have > b) {
                        have
                    } else {
                        b
                    }
                }
            });
        }
        best
    }
}

/// `fit` argument of `dl_image` (SPEC-5.2.3) and the `fit` field of the
/// `IMAGE` opcode (SPEC-C.1.1). Any other value is `E_ARG`.
pub mod fit {
    /// Scale uniformly so the image covers the rectangle; crop the overflow.
    pub const COVER: i32 = 0;
    /// Scale uniformly so the whole image fits inside the rectangle; letterbox.
    pub const CONTAIN: i32 = 1;
    /// Scale each axis independently to the rectangle.
    pub const STRETCH: i32 = 2;
    /// Highest valid fit.
    pub const MAX: i32 = 2;
}

/// Capability ids as delivered in `KIND_CAPABILITY` events (SPEC-A.4.1,
/// SPEC-A.2.1), in the order of SPEC-8.2.1.
pub mod cap {
    /// `net` {origins}.
    pub const NET: i32 = 1;
    /// `storage` {quota_kb}.
    pub const STORAGE: i32 = 2;
    /// `clipboard`.
    pub const CLIPBOARD: i32 = 3;
    /// `hires_time`.
    pub const HIRES_TIME: i32 = 4;
    /// `ime`.
    pub const IME: i32 = 5;
    /// `persist_frame`.
    pub const PERSIST_FRAME: i32 = 6;
    /// Highest valid capability id.
    pub const MAX: i32 = 6;

    /// Canonical names in id order (SPEC-8.2.1).
    pub const NAMES: [&str; 6] = [
        "net",
        "storage",
        "clipboard",
        "hires_time",
        "ime",
        "persist_frame",
    ];

    /// Canonical name of a capability id, or `None` outside 1..=6.
    pub fn name(id: i32) -> Option<&'static str> {
        if (1..=MAX).contains(&id) {
            Some(NAMES[(id - 1) as usize])
        } else {
            None
        }
    }

    /// Id of a capability name, or `None` for an unknown name.
    pub fn id(name: &str) -> Option<i32> {
        NAMES.iter().position(|n| *n == name).map(|i| i as i32 + 1)
    }
}

/// Pointer buttons (SPEC-A.2.2): the `c` argument of `POINTER_DOWN` and
/// `POINTER_UP` names one button; the `c` argument of `POINTER_MOVE` is a
/// mask with bit `n - 1` set for every held button `n`.
pub mod button {
    /// Left button, or the only one on a touch or pen device.
    pub const PRIMARY: i32 = 1;
    /// Right button.
    pub const SECONDARY: i32 = 2;
    /// Middle button or wheel press.
    pub const MIDDLE: i32 = 3;
    /// Back (the fourth button).
    pub const BACK: i32 = 4;
    /// Forward (the fifth button).
    pub const FORWARD: i32 = 5;
    /// Highest button number in 1.0.
    pub const MAX: i32 = 5;

    /// Names in button order, as the spec writes them.
    pub const NAMES: [&str; 5] = ["primär", "sekundär", "mitten", "bakåt", "framåt"];

    /// The bit of `POINTER_MOVE.c` that says `button` is held:
    /// `1 << (button - 1)`, or 0 for a number 1.0 does not define.
    pub const fn mask(button: i32) -> i32 {
        if button >= PRIMARY && button <= MAX {
            1 << (button - 1)
        } else {
            0
        }
    }

    /// Name of a button, or `None` outside 1..=5.
    pub fn name(button: i32) -> Option<&'static str> {
        if (PRIMARY..=MAX).contains(&button) {
            Some(NAMES[(button - 1) as usize])
        } else {
            None
        }
    }
}

/// Modifier bits in the `modifierare` argument of pointer, scroll and key
/// events (SPEC-A.5.1, SPEC-A.2.1).
pub mod modifier {
    /// Either Shift key.
    pub const SHIFT: i32 = 1;
    /// Either Control key.
    pub const CTRL: i32 = 2;
    /// Either Alt/Option key.
    pub const ALT: i32 = 4;
    /// Command (macOS) or Windows key.
    pub const META: i32 = 8;

    /// Every bit with its key, in bit order (SPEC-A.5.1).
    pub const ALL: [(i32, &str); 4] = [
        (SHIFT, "Skift"),
        (CTRL, "Ctrl"),
        (ALT, "Alt / Option"),
        (META, "Meta (Cmd / Win)"),
    ];
}

/// USB HID usage codes of the keys the host itself acts on (SPEC-10.3.1):
/// Tab moves focus and never reaches the module; Enter and Escape are
/// delivered as `KEY_DOWN` for the module to act on.
pub mod key {
    /// Return / Enter.
    pub const ENTER: i32 = 0x28;
    /// Escape.
    pub const ESCAPE: i32 = 0x29;
    /// Tab (with Shift: backwards).
    pub const TAB: i32 = 0x2B;
}

/// Semantic roles for `sem_node` (SPEC-A.3.1).
pub mod role {
    /// Plain text run.
    pub const TEXT: i32 = 1;
    /// Heading.
    pub const HEADING: i32 = 2;
    /// Link.
    pub const LINK: i32 = 3;
    /// Button.
    pub const BUTTON: i32 = 4;
    /// Text input.
    pub const TEXTBOX: i32 = 5;
    /// Checkbox.
    pub const CHECKBOX: i32 = 6;
    /// Radio button.
    pub const RADIO: i32 = 7;
    /// List container.
    pub const LIST: i32 = 8;
    /// List item.
    pub const LISTITEM: i32 = 9;
    /// Image with alt text.
    pub const IMAGE: i32 = 10;
    /// Table.
    pub const TABLE: i32 = 11;
    /// Table row.
    pub const ROW: i32 = 12;
    /// Table cell.
    pub const CELL: i32 = 13;
    /// Navigation region.
    pub const NAV: i32 = 14;
    /// Main content region.
    pub const MAIN: i32 = 15;
    /// Generic region.
    pub const REGION: i32 = 16;
    /// Dialog.
    pub const DIALOG: i32 = 17;
    /// Menu.
    pub const MENU: i32 = 18;
    /// Menu item.
    pub const MENUITEM: i32 = 19;
    /// Tab.
    pub const TAB: i32 = 20;
    /// Tab panel.
    pub const TABPANEL: i32 = 21;
    /// Progress indicator.
    pub const PROGRESS: i32 = 22;
    /// Slider.
    pub const SLIDER: i32 = 23;
    /// Separator.
    pub const SEPARATOR: i32 = 24;
    /// Highest valid role.
    pub const MAX: i32 = 24;

    /// Roles the host moves keyboard focus through, in document order
    /// (SPEC-10.3.2): the interactive widgets.
    pub const FOCUSABLE: [i32; 8] = [
        LINK, BUTTON, TEXTBOX, CHECKBOX, RADIO, MENUITEM, TAB, SLIDER,
    ];

    /// True when a node of this role takes focus (SPEC-10.3.2).
    pub fn is_focusable(role: i32) -> bool {
        FOCUSABLE.contains(&role)
    }

    /// Canonical lower-case name of a role, or `None` outside 1..=24 (SPEC-10.1.2).
    pub fn name(role: i32) -> Option<&'static str> {
        const NAMES: [&str; 24] = [
            "text",
            "heading",
            "link",
            "button",
            "textbox",
            "checkbox",
            "radio",
            "list",
            "listitem",
            "image",
            "table",
            "row",
            "cell",
            "nav",
            "main",
            "region",
            "dialog",
            "menu",
            "menuitem",
            "tab",
            "tabpanel",
            "progress",
            "slider",
            "separator",
        ];
        if (1..=MAX).contains(&role) {
            Some(NAMES[(role - 1) as usize])
        } else {
            None
        }
    }
}

/// Display list opcodes (SPEC appendix C, SPEC-C.1.1).
pub mod op {
    /// Rounded rectangle: x, y, w, h, radius: f32; color: u32.
    pub const RECT: u8 = 0x01;
    /// Text run: x, y, max_w: f32; style_id: u16; len: u16; utf8.
    pub const TEXT: u8 = 0x02;
    /// Image: x, y, w, h: f32; resource_id: u16; fit: u8.
    pub const IMAGE: u8 = 0x03;
    /// Path: color: u32; stroke_w: f32; n: u16; n commands.
    pub const PATH: u8 = 0x04;
    /// Push clip rectangle: x, y, w, h, radius: f32.
    pub const CLIP_PUSH: u8 = 0x05;
    /// Pop clip rectangle.
    pub const CLIP_POP: u8 = 0x06;
    // 0x07 (`LINE`) and 0x08 (`SHADOW`) were withdrawn before the freeze and
    // stay reserved: no import produced them (SPEC-C.1.1, SPEC-C.1.3).
    /// End of list.
    pub const END: u8 = 0xFF;

    /// One opcode as appendix C lists it: code, name, fields and size.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct Desc {
        /// The opcode byte.
        pub code: u8,
        /// Name as the spec writes it.
        pub name: &'static str,
        /// Fields after the opcode byte, in order, little-endian.
        pub fields: &'static str,
        /// Size in bytes including the opcode byte; `len` and `n` are the
        /// variable parts.
        pub bytes: &'static str,
    }

    /// Every opcode of 1.0 in numeric order (SPEC-C.1.1).
    pub const ALL: [Desc; 7] = [
        Desc {
            code: RECT,
            name: "RECT",
            fields: "x, y, w, h, radius: f32; color: u32",
            bytes: "25",
        },
        Desc {
            code: TEXT,
            name: "TEXT",
            fields: "x, y, max_w: f32; style_id: u16; len: u16; utf8[len]",
            bytes: "17 + len",
        },
        Desc {
            code: IMAGE,
            name: "IMAGE",
            fields: "x, y, w, h: f32; resource_id: u16; fit: u8",
            bytes: "20",
        },
        Desc {
            code: PATH,
            name: "PATH",
            fields: "color: u32; stroke_w: f32; n: u16; n × (u8 typ + 2 × f32)",
            bytes: "11 + 9n",
        },
        Desc {
            code: CLIP_PUSH,
            name: "CLIP_PUSH",
            fields: "x, y, w, h, radius: f32",
            bytes: "21",
        },
        Desc {
            code: CLIP_POP,
            name: "CLIP_POP",
            fields: "-",
            bytes: "1",
        },
        Desc {
            code: END,
            name: "END",
            fields: "-",
            bytes: "1",
        },
    ];

    /// Opcode bytes that are reserved and never appear in a 1.x list.
    pub const RESERVED: [u8; 2] = [0x07, 0x08];
}

/// Hard limits enforced by the host (SPEC-6.1.4, SPEC-6.1.6, SPEC-9.3.1).
///
/// The byte caps (`MAX_*_BYTES`) bound every copy the host makes out of module
/// memory on an import call. They are the host's own resource limits in the
/// sense of SPEC-9.3.1 ("resursgränser per flik verkställs av värden"): a call
/// whose `len` exceeds its cap returns `E_ARG`, is logged, and copies nothing,
/// so a module cannot make the host allocate more than it would for a
/// well-formed call. The per-frame caps bound what the host holds for one
/// frame. The numbers are normative since v0.2 (SPEC-9.3.2).
pub mod limits {
    /// Maximum opcodes per frame.
    pub const MAX_OPS_PER_FRAME: u32 = 65_536;
    /// Maximum clip stack depth.
    pub const MAX_CLIP_DEPTH: u32 = 32;
    /// Frame budget in milliseconds of logical time (SPEC-5.3.3).
    pub const FRAME_BUDGET_MS: u32 = 8;
    /// Maximum `memory_max` in wasm pages (SPEC-8.1.3).
    pub const MAX_MEMORY_PAGES: u32 = 4096;
    /// Largest width or height of an image resource in pixels (SPEC-6.5.1).
    pub const MAX_IMAGE_SIDE_PX: u32 = 8192;

    /// Largest `len` accepted by `log` (SPEC-5.2.1): 4 KB (SPEC-9.3.1).
    pub const MAX_LOG_BYTES: u32 = 4096;
    /// Largest text accepted by `dl_text`, `text_measure` and `text_caret`
    /// (SPEC-5.2.3, SPEC-5.2.5): 64 KB less one byte, so that every string the
    /// text imports accept also fits the `len: u16` field of a `TEXT` opcode
    /// (SPEC-C.1.1) (SPEC-9.3.1).
    pub const MAX_TEXT_BYTES: u32 = 65_535;
    /// Largest path buffer accepted by `dl_path` (SPEC-5.2.3): 65 535 commands
    /// of 9 bytes, the most the `n: u16` field of a `PATH` opcode can carry
    /// (SPEC-C.1.1, SPEC-C.1.2) (SPEC-9.3.1).
    pub const MAX_PATH_BYTES: u32 = 65_535 * 9;
    /// Largest text accepted by `sem_node` (SPEC-5.2.13): 4 KB (SPEC-9.3.1).
    pub const MAX_SEM_TEXT_BYTES: u32 = 4096;
    /// Largest URL accepted by `navigate`, `link_register` and `fetch_begin`
    /// (SPEC-5.2.7, SPEC-5.2.10): 2 KB (SPEC-9.3.1).
    pub const MAX_URL_BYTES: u32 = 2048;
    /// Largest request body accepted by `fetch_begin` (SPEC-5.2.7): 1 MB (SPEC-9.3.1).
    pub const MAX_FETCH_BODY_BYTES: u32 = 1 << 20;
    /// Largest response body the host buffers for `fetch_read` (SPEC-5.2.7):
    /// 64 MB; a longer one is `E_TOO_LARGE` in `FETCH_DONE`.
    pub const MAX_FETCH_RESPONSE_BYTES: u32 = 64 << 20;
    /// Most requests open at once per instance (SPEC-5.2.7, SPEC-9.3.1):
    /// the 65th `fetch_begin` is `E_RATE`.
    pub const MAX_OPEN_FETCHES: u32 = 64;
    /// Largest key accepted by `state_put`, `state_get` and `state_delete`
    /// (SPEC-5.2.9): 256 bytes (SPEC-9.3.1).
    pub const MAX_STATE_KEY_BYTES: u32 = 256;
    /// Largest value accepted by `state_put` (SPEC-5.2.9): 1 MB (SPEC-9.3.1).
    pub const MAX_STATE_VALUE_BYTES: u32 = 1 << 20;
    /// Largest text accepted by `clipboard_write` (SPEC-5.2.12): 1 MB (SPEC-9.3.1).
    pub const MAX_CLIPBOARD_BYTES: u32 = 1 << 20;
    /// Maximum semantic nodes per frame (SPEC-10.1.1); the 65 537th is
    /// `E_DL_FULL`, like the 65 537th opcode (SPEC-6.1.6) (SPEC-9.3.1).
    pub const MAX_SEM_NODES_PER_FRAME: u32 = 65_536;
    /// Maximum text bytes held by the display list and the semantic tree
    /// together for one frame: 16 MB; a `dl_text` or `sem_node` that would
    /// exceed it is `E_DL_FULL` (SPEC-9.3.1).
    pub const MAX_FRAME_TEXT_BYTES: u32 = 16 << 20;
}

/// Wasm value types used in ABI signatures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ty {
    /// 32-bit integer.
    I32,
    /// 64-bit integer.
    I64,
    /// 32-bit float.
    F32,
}

/// A function signature in the ABI: name, parameters, result.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sig {
    /// Function name as exported or imported.
    pub name: &'static str,
    /// Parameter types in order.
    pub params: &'static [Ty],
    /// Result type, if any.
    pub result: Option<Ty>,
    /// The parameters with the names SPEC §5 gives them, as one
    /// `name: type, ...` string; the same count and types as `params`.
    pub args: &'static str,
}

impl Ty {
    /// The wasm type's name.
    pub const fn name(self) -> &'static str {
        match self {
            Ty::I32 => "i32",
            Ty::I64 => "i64",
            Ty::F32 => "f32",
        }
    }
}

use Ty::{F32, I32, I64};

/// The six functions every module must export, and nothing else (SPEC-4.2.1, SPEC-5.1.1..5.1.6).
pub const EXPORTS: &[Sig] = &[
    Sig {
        name: "vy_init",
        params: &[I32, I32, I32, F32],
        result: Some(I32),
        args: "abi_minor: i32, width: i32, height: i32, scale: f32",
    },
    Sig {
        name: "vy_frame",
        params: &[I32],
        result: Some(I32),
        args: "dt_ms: i32",
    },
    Sig {
        name: "vy_event",
        params: &[I32, I32, I32, I32, I32],
        result: Some(I32),
        args: "kind: i32, a: i32, b: i32, c: i32, d: i32",
    },
    Sig {
        name: "vy_resize",
        params: &[I32, I32, F32],
        result: None,
        args: "width: i32, height: i32, scale: f32",
    },
    Sig {
        name: "vy_suspend",
        params: &[],
        result: Some(I32),
        args: "",
    },
    Sig {
        name: "vy_alloc",
        params: &[I32],
        result: Some(I32),
        args: "size: i32",
    },
];

/// Every import the host provides in namespace `vy` (SPEC-5.2.1..5.2.13).
pub const IMPORTS: &[Sig] = &[
    // lifecycle (SPEC-5.2.1)
    Sig {
        name: "abi_version",
        params: &[],
        result: Some(I32),
        args: "",
    },
    Sig {
        name: "log",
        params: &[I32, I32, I32],
        result: Some(I32),
        args: "level: i32, ptr: i32, len: i32",
    },
    Sig {
        name: "request_frame",
        params: &[],
        result: None,
        args: "",
    },
    Sig {
        name: "exit",
        params: &[I32],
        result: None,
        args: "code: i32",
    },
    // display list (SPEC-5.2.3)
    Sig {
        name: "dl_begin",
        params: &[],
        result: Some(I32),
        args: "",
    },
    Sig {
        name: "dl_end",
        params: &[],
        result: Some(I32),
        args: "",
    },
    Sig {
        name: "dl_rect",
        params: &[F32, F32, F32, F32, F32, I32],
        result: Some(I32),
        args: "x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32",
    },
    Sig {
        name: "dl_text",
        params: &[F32, F32, F32, I32, I32, I32],
        result: Some(I64),
        args: "x: f32, y: f32, w: f32, style_id: i32, ptr: i32, len: i32",
    },
    Sig {
        name: "dl_image",
        params: &[F32, F32, F32, F32, I32, I32],
        result: Some(I32),
        args: "x: f32, y: f32, w: f32, h: f32, resource_id: i32, fit: i32",
    },
    Sig {
        name: "dl_path",
        params: &[I32, I32, I32, F32],
        result: Some(I32),
        args: "ptr: i32, len: i32, color: i32, stroke_w: f32",
    },
    Sig {
        name: "dl_clip_push",
        params: &[F32, F32, F32, F32, F32],
        result: Some(I32),
        args: "x: f32, y: f32, w: f32, h: f32, radius: f32",
    },
    Sig {
        name: "dl_clip_pop",
        params: &[],
        result: Some(I32),
        args: "",
    },
    // text (SPEC-5.2.5)
    Sig {
        name: "text_style",
        params: &[I32, F32, I32, I32],
        result: Some(I32),
        args: "font_resource_id: i32, size_px: f32, weight: i32, color: i32",
    },
    Sig {
        name: "text_measure",
        params: &[I32, F32, I32, I32],
        result: Some(I64),
        args: "style_id: i32, max_w: f32, ptr: i32, len: i32",
    },
    Sig {
        name: "text_caret",
        params: &[I32, I32, I32, I32],
        result: Some(I64),
        args: "style_id: i32, ptr: i32, len: i32, byte_offset: i32",
    },
    // input (SPEC-5.2.6); `pointer_get` was withdrawn before the freeze:
    // the pointer arrives through `POINTER_MOVE` only.
    Sig {
        name: "key_down",
        params: &[I32],
        result: Some(I32),
        args: "code: i32",
    },
    Sig {
        name: "ime_begin",
        params: &[F32, F32, F32, F32],
        result: Some(I32),
        args: "x: f32, y: f32, w: f32, h: f32",
    },
    Sig {
        name: "ime_end",
        params: &[],
        result: Some(I32),
        args: "",
    },
    // network (SPEC-5.2.7)
    Sig {
        name: "fetch_begin",
        params: &[I32, I32, I32, I32, I32],
        result: Some(I32),
        args: "method: i32, url_ptr: i32, url_len: i32, body_ptr: i32, body_len: i32",
    },
    Sig {
        name: "fetch_poll",
        params: &[I32],
        result: Some(I32),
        args: "req_id: i32",
    },
    Sig {
        name: "fetch_read",
        params: &[I32, I32, I32],
        result: Some(I32),
        args: "req_id: i32, dst_ptr: i32, max: i32",
    },
    Sig {
        name: "fetch_cancel",
        params: &[I32],
        result: Some(I32),
        args: "req_id: i32",
    },
    // storage (SPEC-5.2.9)
    Sig {
        name: "state_put",
        params: &[I32, I32, I32, I32],
        result: Some(I32),
        args: "key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32",
    },
    Sig {
        name: "state_get",
        params: &[I32, I32, I32, I32],
        result: Some(I32),
        args: "key_ptr: i32, key_len: i32, dst_ptr: i32, max: i32",
    },
    Sig {
        name: "state_delete",
        params: &[I32, I32],
        result: Some(I32),
        args: "key_ptr: i32, key_len: i32",
    },
    // navigation (SPEC-5.2.10)
    Sig {
        name: "navigate",
        params: &[I32, I32, I32],
        result: Some(I32),
        args: "url_ptr: i32, url_len: i32, replace: i32",
    },
    Sig {
        name: "link_register",
        params: &[I32, F32, F32, F32, F32, I32, I32],
        result: Some(I32),
        args: "id: i32, x: f32, y: f32, w: f32, h: f32, url_ptr: i32, url_len: i32",
    },
    // time and randomness (SPEC-5.2.11)
    Sig {
        name: "now_ms",
        params: &[],
        result: Some(I64),
        args: "",
    },
    Sig {
        name: "random",
        params: &[I32, I32],
        result: Some(I32),
        args: "dst_ptr: i32, len: i32",
    },
    // clipboard (SPEC-5.2.12)
    Sig {
        name: "clipboard_read",
        params: &[I32, I32],
        result: Some(I32),
        args: "dst_ptr: i32, max: i32",
    },
    Sig {
        name: "clipboard_write",
        params: &[I32, I32],
        result: Some(I32),
        args: "ptr: i32, len: i32",
    },
    // semantics (SPEC-5.2.13)
    Sig {
        name: "sem_node",
        params: &[I32, I32, I32, F32, F32, F32, F32, I32, I32],
        result: Some(I32),
        args:
            "id: i32, parent_id: i32, role: i32, x: f32, y: f32, w: f32, h: f32, ptr: i32, len: i32",
    },
];

/// Look up an import signature by name.
pub fn import(name: &str) -> Option<&'static Sig> {
    IMPORTS.iter().find(|s| s.name == name)
}

/// Look up an export signature by name.
pub fn export(name: &str) -> Option<&'static Sig> {
    EXPORTS.iter().find(|s| s.name == name)
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;

    /// SPEC-2.1.3: the host API has at most 40 imports in 1.0.
    #[test]
    fn spec_2_1_3_import_count_under_40() {
        assert!(IMPORTS.len() <= 40, "{} imports", IMPORTS.len());
    }

    /// SPEC-4.2.1, SPEC-5.1.1..SPEC-5.1.6: exactly six exports with the specified names.
    #[test]
    fn spec_4_2_1_exactly_six_exports() {
        let names: [&str; 6] = [
            "vy_init",
            "vy_frame",
            "vy_event",
            "vy_resize",
            "vy_suspend",
            "vy_alloc",
        ];
        assert_eq!(EXPORTS.len(), 6);
        for n in names {
            assert!(export(n).is_some(), "missing export {n}");
        }
    }

    /// SPEC-5.1.1: vy_init signature.
    #[test]
    fn spec_5_1_1_init_signature() {
        let s = export("vy_init").unwrap();
        assert_eq!(s.params, &[I32, I32, I32, F32]);
        assert_eq!(s.result, Some(I32));
    }

    /// SPEC-5.1.2: vy_frame signature.
    #[test]
    fn spec_5_1_2_frame_signature() {
        let s = export("vy_frame").unwrap();
        assert_eq!(s.params, &[I32]);
        assert_eq!(s.result, Some(I32));
    }

    /// SPEC-5.1.3: vy_event signature.
    #[test]
    fn spec_5_1_3_event_signature() {
        let s = export("vy_event").unwrap();
        assert_eq!(s.params, &[I32, I32, I32, I32, I32]);
        assert_eq!(s.result, Some(I32));
    }

    /// SPEC-5.1.4: vy_resize signature.
    #[test]
    fn spec_5_1_4_resize_signature() {
        let s = export("vy_resize").unwrap();
        assert_eq!(s.params, &[I32, I32, F32]);
        assert_eq!(s.result, None);
    }

    /// SPEC-5.1.5: vy_suspend signature.
    #[test]
    fn spec_5_1_5_suspend_signature() {
        let s = export("vy_suspend").unwrap();
        assert!(s.params.is_empty());
        assert_eq!(s.result, Some(I32));
    }

    /// SPEC-5.1.6: vy_alloc signature.
    #[test]
    fn spec_5_1_6_alloc_signature() {
        let s = export("vy_alloc").unwrap();
        assert_eq!(s.params, &[I32]);
        assert_eq!(s.result, Some(I32));
    }

    /// SPEC-5.2.1: abi_version encodes MAJOR*100+MINOR.
    #[test]
    fn spec_5_2_1_abi_version_encoding() {
        assert_eq!(ABI_VERSION, 100);
        assert!(import("abi_version").is_some());
        assert!(import("log").is_some());
        assert!(import("request_frame").is_some());
        assert!(import("exit").is_some());
    }

    /// SPEC-5.2.1: `log.level` is 0 debug, 1 info, 2 warn, 3 error, and a
    /// host records any other value as 3.
    #[test]
    fn spec_5_2_1_log_levels() {
        assert_eq!(
            (level::DEBUG, level::INFO, level::WARN, level::ERROR),
            (0, 1, 2, 3)
        );
        assert_eq!(level::MAX, 3);
        assert_eq!(level::NAMES, ["debug", "info", "warn", "error"]);
        for l in 0..=3 {
            assert_eq!(level::clamp(l), l);
        }
        for bad in [-1, 4, 99, i32::MIN, i32::MAX] {
            assert_eq!(level::clamp(bad), level::ERROR, "{bad}");
            assert_eq!(level::name(bad), "error");
        }
        assert_eq!(level::name(level::WARN), "warn");
    }

    /// SPEC-5.2.7: `fetch_begin.method` is 0 GET, 1 POST, 2 PUT, 3 DELETE
    /// and nothing else.
    #[test]
    fn spec_5_2_7_fetch_methods() {
        assert_eq!(
            (method::GET, method::POST, method::PUT, method::DELETE),
            (0, 1, 2, 3)
        );
        assert_eq!(method::MAX, 3);
        assert_eq!(method::name(method::GET), Some("GET"));
        assert_eq!(method::name(method::DELETE), Some("DELETE"));
        assert_eq!(
            method::name(4),
            None,
            "HEAD, PATCH and the rest are not in 1.0"
        );
        assert_eq!(method::name(-1), None);
        assert_eq!(method::NAMES.len(), method::MAX as usize + 1);
    }

    /// SPEC-5.2.3, SPEC-5.2.5, SPEC-5.2.6, SPEC-5.2.7, SPEC-5.2.9, SPEC-5.2.10,
    /// SPEC-5.2.11, SPEC-5.2.12, SPEC-5.2.13: every import group is present with
    /// the stated arity.
    #[test]
    fn spec_5_2_import_groups_present() {
        let expect: &[(&str, usize)] = &[
            ("dl_rect", 6),
            ("dl_text", 6),
            ("dl_image", 6),
            ("dl_clip_push", 5),
            ("text_style", 4),
            ("key_down", 1),
            ("fetch_begin", 5),
            ("state_put", 4),
            ("navigate", 3),
            ("link_register", 7),
            ("now_ms", 0),
            ("random", 2),
            ("clipboard_read", 2),
            ("sem_node", 9),
        ];
        for (name, arity) in expect {
            let s = import(name).unwrap_or_else(|| panic!("missing import {name}"));
            assert_eq!(s.params.len(), *arity, "{name} arity");
        }
        assert!(
            import("pointer_get").is_none(),
            "withdrawn: the pointer is delivered by POINTER_MOVE only (SPEC-5.2.6)"
        );
        assert_eq!(IMPORTS.len(), 32);
        // No duplicate names (SPEC-2.1.1: one canonical expression per operation).
        for (i, a) in IMPORTS.iter().enumerate() {
            for b in &IMPORTS[i + 1..] {
                assert_ne!(a.name, b.name, "duplicate import");
            }
        }
    }

    /// SPEC-5.3.1, SPEC-5.3.2, SPEC-6.1.4, SPEC-A.1.1: every import that has
    /// an error path in appendix A returns a status (`i32`, or `i64` for the
    /// text measurements); only `request_frame` and `exit`, which cannot
    /// fail, return nothing.
    #[test]
    fn spec_5_3_1_fallible_imports_return_status() {
        for s in IMPORTS {
            match s.name {
                "request_frame" | "exit" => assert_eq!(s.result, None, "{}", s.name),
                "text_measure" | "text_caret" | "dl_text" | "now_ms" => {
                    assert_eq!(s.result, Some(I64), "{}", s.name)
                }
                _ => assert_eq!(s.result, Some(I32), "{} returns a status", s.name),
            }
        }
    }

    /// SPEC-A.1.1: error code values are fixed.
    #[test]
    fn spec_a_1_1_error_codes_fixed() {
        assert_eq!(err::OK, 0);
        assert_eq!(err::E_DENIED, -1);
        assert_eq!(err::E_DL_STATE, -10);
        assert_eq!(err::E_DL_FULL, -11);
        assert_eq!(err::E_DL_CLIP, -12);
        assert_eq!(err::E_ORIGIN, -30);
        assert_eq!(err::E_GESTURE, -40);
    }

    /// SPEC-5.2.3, SPEC-C.1.1: `dl_image` takes `fit` as its last argument and
    /// the three fit modes are 0 cover, 1 contain, 2 stretch.
    #[test]
    fn spec_5_2_3_image_fit_modes() {
        let s = import("dl_image").unwrap();
        assert_eq!(s.params, &[F32, F32, F32, F32, I32, I32]);
        assert_eq!(s.result, Some(I32));
        assert_eq!(fit::COVER, 0);
        assert_eq!(fit::CONTAIN, 1);
        assert_eq!(fit::STRETCH, 2);
        assert_eq!(fit::MAX, 2);
        assert_eq!(limits::MAX_IMAGE_SIDE_PX, 8192, "SPEC-6.5.1");
    }

    /// SPEC-A.2.1: event kinds are fixed.
    #[test]
    fn spec_a_2_1_event_kinds_fixed() {
        assert_eq!(event::POINTER_MOVE, 1);
        assert_eq!(event::TEXT_INPUT, 7);
        assert_eq!(event::IME_COMPOSE, 8);
        assert_eq!(event::POINTER_LEAVE, 9, "new kind, new number");
        assert_eq!(event::FOCUS, 10);
        assert_eq!(event::FETCH_DONE, 20);
        assert_eq!(event::VISIBILITY, 30);
        assert_eq!(event::BUDGET, 31);
        assert_eq!(event::CAPABILITY, 32);
    }

    /// SPEC-A.4.1, SPEC-8.2.1, SPEC-A.2.1: the six capabilities of 1.0 have
    /// the ids 1..=6 of appendix A, in the order the spec lists them, and
    /// names round-trip.
    #[test]
    fn spec_a_4_1_capability_ids() {
        assert_eq!(cap::NAMES.len(), 6);
        assert_eq!(cap::MAX, 6);
        assert_eq!(
            (
                cap::NET,
                cap::STORAGE,
                cap::CLIPBOARD,
                cap::HIRES_TIME,
                cap::IME,
                cap::PERSIST_FRAME
            ),
            (1, 2, 3, 4, 5, 6)
        );
        assert_eq!(cap::name(cap::NET), Some("net"));
        assert_eq!(cap::name(cap::IME), Some("ime"));
        assert_eq!(cap::name(cap::PERSIST_FRAME), Some("persist_frame"));
        assert_eq!(cap::name(0), None);
        assert_eq!(cap::name(7), None);
        for (i, n) in cap::NAMES.iter().enumerate() {
            assert_eq!(cap::id(n), Some(i as i32 + 1));
            assert_eq!(cap::name(i as i32 + 1), Some(*n));
        }
        assert_eq!(cap::id("file"), None, "1.1 capabilities are not in 1.0");
    }

    /// SPEC-10.3.1, SPEC-10.3.2: the host acts on Tab itself and leaves Enter
    /// and Escape to the module; exactly the eight interactive roles take focus.
    #[test]
    fn spec_10_3_2_focusable_roles() {
        assert_eq!((key::TAB, key::ENTER, key::ESCAPE), (0x2B, 0x28, 0x29));
        for r in 1..=role::MAX {
            let expect = matches!(
                r,
                role::LINK
                    | role::BUTTON
                    | role::TEXTBOX
                    | role::CHECKBOX
                    | role::RADIO
                    | role::MENUITEM
                    | role::TAB
                    | role::SLIDER
            );
            assert_eq!(role::is_focusable(r), expect, "{}", role::name(r).unwrap());
        }
        assert!(!role::is_focusable(0) && !role::is_focusable(25));
        assert_eq!(role::FOCUSABLE.len(), 8);
    }

    /// SPEC-6.4.2, SPEC-5.2.5: the five built-in roles are the resource
    /// ids −1 sans, −2 serif, −3 mono, −4 ui, −5 emoji; nothing else is a
    /// role, and 0 (the module) is never a font.
    #[test]
    fn spec_6_4_2_font_roles() {
        assert_eq!(
            (font::SANS, font::SERIF, font::MONO, font::UI, font::EMOJI),
            (-1, -2, -3, -4, -5)
        );
        assert_eq!(font::MIN, -5);
        assert_eq!(font::NAMES, ["sans", "serif", "mono", "ui", "emoji"]);
        for (i, n) in font::NAMES.iter().enumerate() {
            assert_eq!(font::name(-(i as i32) - 1), Some(*n));
        }
        assert_eq!(font::name(0), None, "the module is not a font");
        assert_eq!(font::name(1), None, "a package resource, not a role");
        assert_eq!(font::name(-6), None);
    }

    /// SPEC-5.2.5: `text_style.weight` is 1..=1000 as in CSS; a host clamps
    /// the argument and picks the nearest face it has, the heavier on a tie.
    #[test]
    fn spec_5_2_5_weight_range_and_nearest_face() {
        assert_eq!((weight::MIN, weight::MAX), (1, 1000));
        assert_eq!((weight::NORMAL, weight::BOLD), (400, 700));
        assert_eq!(weight::clamp(0), 1);
        assert_eq!(weight::clamp(-5), 1);
        assert_eq!(weight::clamp(1001), 1000);
        assert_eq!(weight::clamp(i32::MAX), 1000);
        assert_eq!(weight::clamp(i32::MIN), 1);
        assert_eq!(weight::clamp(550), 550);
        // The reference client's sans family: 400 and 700.
        let faces = [400, 700];
        assert_eq!(weight::nearest(400, &faces), Some(400));
        assert_eq!(weight::nearest(549, &faces), Some(400), "nearer to 400");
        assert_eq!(
            weight::nearest(550, &faces),
            Some(700),
            "a tie goes heavier"
        );
        assert_eq!(weight::nearest(600, &faces), Some(700));
        assert_eq!(weight::nearest(1000, &faces), Some(700));
        assert_eq!(weight::nearest(0, &faces), Some(400), "clamped first");
        assert_eq!(weight::nearest(5000, &faces), Some(700));
        // A single face takes every weight; the order of the list is irrelevant.
        assert_eq!(weight::nearest(900, &[400]), Some(400));
        assert_eq!(weight::nearest(550, &[700, 400]), Some(700));
        assert_eq!(weight::nearest(300, &[100, 500, 300]), Some(300));
        assert_eq!(weight::nearest(400, &[]), None);
    }

    /// SPEC-A.2.2, SPEC-A.2.1: the buttons are 1 primary, 2 secondary,
    /// 3 middle, 4 back, 5 forward and nothing else; `POINTER_MOVE` carries
    /// them as a mask with bit `n - 1` per button `n`.
    #[test]
    fn spec_a_2_2_pointer_buttons() {
        assert_eq!(
            (
                button::PRIMARY,
                button::SECONDARY,
                button::MIDDLE,
                button::BACK,
                button::FORWARD
            ),
            (1, 2, 3, 4, 5)
        );
        assert_eq!(button::MAX, 5);
        assert_eq!(
            [1, 2, 3, 4, 5].map(button::mask),
            [1, 2, 4, 8, 16],
            "bit n - 1 for button n"
        );
        assert_eq!(button::mask(0), 0);
        assert_eq!(button::mask(6), 0);
        assert_eq!(button::mask(-1), 0);
        let held = button::mask(button::PRIMARY) | button::mask(button::BACK);
        assert_eq!(held, 0b1001);
        assert_eq!(button::name(button::PRIMARY), Some("primär"));
        assert_eq!(button::name(button::FORWARD), Some("framåt"));
        assert_eq!(button::name(0), None);
        assert_eq!(button::name(6), None);
        assert_eq!(button::NAMES.len(), button::MAX as usize);
    }

    /// SPEC-A.5.1, SPEC-A.2.1: the modifier bits are Shift 1, Ctrl 2, Alt 4,
    /// Meta 8 and nothing else; they combine as a mask.
    #[test]
    fn spec_a_5_1_modifier_bits() {
        assert_eq!(
            (
                modifier::SHIFT,
                modifier::CTRL,
                modifier::ALT,
                modifier::META
            ),
            (1, 2, 4, 8)
        );
        let all = modifier::SHIFT | modifier::CTRL | modifier::ALT | modifier::META;
        assert_eq!(all, 15, "four distinct bits, no overlap");
    }

    /// SPEC-A.3.1, SPEC-10.1.2: roles 1..24 are the only roles and each has one name.
    #[test]
    fn spec_a_3_1_roles_range() {
        assert_eq!(role::TEXT, 1);
        assert_eq!(role::SEPARATOR, 24);
        assert_eq!(role::MAX, 24);
        assert_eq!(role::name(0), None);
        assert_eq!(role::name(25), None);
        assert_eq!(role::name(role::TEXT), Some("text"));
        assert_eq!(role::name(role::MAIN), Some("main"));
        assert_eq!(role::name(role::SEPARATOR), Some("separator"));
    }

    /// SPEC-9.3.1, SPEC-9.3.2: the host's byte caps are the normative values and
    /// agree with the wire format they feed (SPEC-C.1.1, SPEC-C.1.2) and with each other.
    #[test]
    fn spec_9_3_1_byte_caps_consistent() {
        assert_eq!(limits::MAX_TEXT_BYTES, u32::from(u16::MAX), "TEXT len: u16");
        assert_eq!(
            limits::MAX_PATH_BYTES,
            u32::from(u16::MAX) * 9,
            "PATH n: u16"
        );
        assert_eq!(limits::MAX_SEM_NODES_PER_FRAME, limits::MAX_OPS_PER_FRAME);
        for cap in [
            limits::MAX_LOG_BYTES,
            limits::MAX_TEXT_BYTES,
            limits::MAX_PATH_BYTES,
            limits::MAX_SEM_TEXT_BYTES,
            limits::MAX_URL_BYTES,
            limits::MAX_FETCH_BODY_BYTES,
            limits::MAX_STATE_KEY_BYTES,
            limits::MAX_STATE_VALUE_BYTES,
            limits::MAX_CLIPBOARD_BYTES,
        ] {
            assert!(cap > 0 && cap <= limits::MAX_FRAME_TEXT_BYTES, "{cap}");
            assert!(i32::try_from(cap).is_ok(), "caps compare with i32 len");
        }
        const _: () = assert!(limits::MAX_SEM_TEXT_BYTES <= limits::MAX_TEXT_BYTES);
        const _: () = assert!(limits::MAX_STATE_KEY_BYTES < limits::MAX_STATE_VALUE_BYTES);
    }

    /// SPEC-5.3.5, SPEC-A.1.1, SPEC-A.2.1, SPEC-A.5.1, SPEC-C.1.1: the
    /// tables the frozen listing (`docs/ABI-1.0.md`) is generated from agree
    /// with the constants: every named argument list has the arity and types
    /// of its signature, every status code, event kind, modifier bit and
    /// opcode has exactly one entry, and the reserved opcodes are nobody's.
    #[test]
    fn spec_5_3_5_listing_tables_agree_with_constants() {
        for s in EXPORTS.iter().chain(IMPORTS) {
            let named: std::vec::Vec<&str> = s
                .args
                .split(',')
                .map(str::trim)
                .filter(|a| !a.is_empty())
                .collect();
            assert_eq!(named.len(), s.params.len(), "{}: {}", s.name, s.args);
            for (arg, ty) in named.iter().zip(s.params) {
                let (name, t) = arg
                    .split_once(": ")
                    .unwrap_or_else(|| panic!("{}: {arg}", s.name));
                assert!(!name.is_empty());
                assert_eq!(t, ty.name(), "{}: {arg}", s.name);
            }
        }
        assert_eq!(err::ALL.len(), 15);
        for (code, name) in err::ALL {
            assert_eq!(err::name(code), Some(name));
            assert_eq!(err::ALL.iter().filter(|(c, _)| *c == code).count(), 1);
        }
        assert_eq!(err::name(-99), None);
        assert_eq!(event::ALL.len(), 14);
        for k in event::ALL {
            assert_eq!(event::name(k.kind), Some(k.name));
            assert_eq!(event::ALL.iter().filter(|o| o.kind == k.kind).count(), 1);
        }
        assert_eq!(event::name(99), None);
        assert_eq!(
            event::ALL.map(|k| k.kind),
            [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 20, 30, 31, 32]
        );
        assert_eq!(modifier::ALL.map(|(b, _)| b), [1, 2, 4, 8]);
        assert_eq!(
            op::ALL.map(|o| o.code),
            [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0xFF]
        );
        for o in op::ALL {
            assert!(!op::RESERVED.contains(&o.code), "{}", o.name);
        }
        assert_eq!(op::RESERVED, [0x07, 0x08]);
    }

    /// SPEC-C.1.1, SPEC-C.1.3: opcode values are fixed; 0x07 and 0x08 are
    /// reserved (the withdrawn `LINE` and `SHADOW`) and no opcode uses them.
    #[test]
    fn spec_c_1_1_opcodes_fixed() {
        assert_eq!(op::RECT, 0x01);
        assert_eq!(op::TEXT, 0x02);
        assert_eq!(op::IMAGE, 0x03);
        assert_eq!(op::PATH, 0x04);
        assert_eq!(op::CLIP_PUSH, 0x05);
        assert_eq!(op::CLIP_POP, 0x06);
        assert_eq!(op::END, 0xFF);
        let defined = [
            op::RECT,
            op::TEXT,
            op::IMAGE,
            op::PATH,
            op::CLIP_PUSH,
            op::CLIP_POP,
            op::END,
        ];
        assert!(!defined.contains(&0x07) && !defined.contains(&0x08));
    }
}
