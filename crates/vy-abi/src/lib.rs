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
}

/// Hard limits enforced by the host (SPEC-6.1.4, SPEC-6.1.6, SPEC-9.3.1).
///
/// The byte caps (`MAX_*_BYTES`) bound every copy the host makes out of module
/// memory on an import call. They are the host's own resource limits in the
/// sense of SPEC-9.3.1 ("resursgränser per flik verkställs av värden"): a call
/// whose `len` exceeds its cap returns `E_ARG`, is logged, and copies nothing,
/// so a module cannot make the host allocate more than it would for a
/// well-formed call. The per-frame caps bound what the host holds for one
/// frame. SPEC 1.0 does not list the numbers; `docs/BUILD-NOTES.md` proposes
/// them as a new normative sentence in §9.3.
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
}

use Ty::{F32, I32, I64};

/// The six functions every module must export, and nothing else (SPEC-4.2.1, SPEC-5.1.1..5.1.6).
pub const EXPORTS: &[Sig] = &[
    Sig {
        name: "vy_init",
        params: &[I32, I32, I32, F32],
        result: Some(I32),
    },
    Sig {
        name: "vy_frame",
        params: &[I32],
        result: Some(I32),
    },
    Sig {
        name: "vy_event",
        params: &[I32, I32, I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "vy_resize",
        params: &[I32, I32, F32],
        result: None,
    },
    Sig {
        name: "vy_suspend",
        params: &[],
        result: Some(I32),
    },
    Sig {
        name: "vy_alloc",
        params: &[I32],
        result: Some(I32),
    },
];

/// Every import the host provides in namespace `vy` (SPEC-5.2.1..5.2.13).
pub const IMPORTS: &[Sig] = &[
    // lifecycle (SPEC-5.2.1)
    Sig {
        name: "abi_version",
        params: &[],
        result: Some(I32),
    },
    Sig {
        name: "log",
        params: &[I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "request_frame",
        params: &[],
        result: None,
    },
    Sig {
        name: "exit",
        params: &[I32],
        result: None,
    },
    // display list (SPEC-5.2.3)
    Sig {
        name: "dl_begin",
        params: &[],
        result: Some(I32),
    },
    Sig {
        name: "dl_end",
        params: &[],
        result: Some(I32),
    },
    Sig {
        name: "dl_rect",
        params: &[F32, F32, F32, F32, F32, I32],
        result: Some(I32),
    },
    Sig {
        name: "dl_text",
        params: &[F32, F32, F32, I32, I32, I32],
        result: Some(I64),
    },
    Sig {
        name: "dl_image",
        params: &[F32, F32, F32, F32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "dl_path",
        params: &[I32, I32, I32, F32],
        result: Some(I32),
    },
    Sig {
        name: "dl_clip_push",
        params: &[F32, F32, F32, F32, F32],
        result: Some(I32),
    },
    Sig {
        name: "dl_clip_pop",
        params: &[],
        result: Some(I32),
    },
    // text (SPEC-5.2.5)
    Sig {
        name: "text_style",
        params: &[I32, F32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "text_measure",
        params: &[I32, F32, I32, I32],
        result: Some(I64),
    },
    Sig {
        name: "text_caret",
        params: &[I32, I32, I32, I32],
        result: Some(I64),
    },
    // input (SPEC-5.2.6); `pointer_get` was withdrawn before the freeze:
    // the pointer arrives through `POINTER_MOVE` only.
    Sig {
        name: "key_down",
        params: &[I32],
        result: Some(I32),
    },
    Sig {
        name: "ime_begin",
        params: &[F32, F32, F32, F32],
        result: Some(I32),
    },
    Sig {
        name: "ime_end",
        params: &[],
        result: Some(I32),
    },
    // network (SPEC-5.2.7)
    Sig {
        name: "fetch_begin",
        params: &[I32, I32, I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "fetch_poll",
        params: &[I32],
        result: Some(I32),
    },
    Sig {
        name: "fetch_read",
        params: &[I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "fetch_cancel",
        params: &[I32],
        result: Some(I32),
    },
    // storage (SPEC-5.2.9)
    Sig {
        name: "state_put",
        params: &[I32, I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "state_get",
        params: &[I32, I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "state_delete",
        params: &[I32, I32],
        result: Some(I32),
    },
    // navigation (SPEC-5.2.10)
    Sig {
        name: "navigate",
        params: &[I32, I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "link_register",
        params: &[I32, F32, F32, F32, F32, I32, I32],
        result: Some(I32),
    },
    // time and randomness (SPEC-5.2.11)
    Sig {
        name: "now_ms",
        params: &[],
        result: Some(I64),
    },
    Sig {
        name: "random",
        params: &[I32, I32],
        result: Some(I32),
    },
    // clipboard (SPEC-5.2.12)
    Sig {
        name: "clipboard_read",
        params: &[I32, I32],
        result: Some(I32),
    },
    Sig {
        name: "clipboard_write",
        params: &[I32, I32],
        result: Some(I32),
    },
    // semantics (SPEC-5.2.13)
    Sig {
        name: "sem_node",
        params: &[I32, I32, I32, F32, F32, F32, F32, I32, I32],
        result: Some(I32),
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
