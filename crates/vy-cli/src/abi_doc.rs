//! The frozen, human-readable listing of ABI 1.0 (`docs/ABI-1.0.md`),
//! generated from `vy-abi` so that it can never drift from the constants
//! the host and the modules are built against (SPEC-5.3.5).
//!
//! `vy-cli abi-dump` prints it; `tests::spec_5_3_5_abi_listing_is_frozen`
//! regenerates it and fails when the committed file differs, which makes
//! the file a gate: a change to `vy-abi` is a change to the listing, in the
//! same commit, and after the freeze no such commit is allowed within 1.x.

use std::fmt::Write;
use vy_abi::{
    button, cap, err, event, font, key, level, limits, method, modifier, op, role, weight, Sig,
};

/// Path of the listing relative to the repository root.
pub const PATH: &str = "docs/ABI-1.0.md";

fn signature(s: &Sig) -> String {
    match s.result {
        Some(r) => format!("`{}({}) -> {}`", s.name, s.args, r.name()),
        None => format!("`{}({})`", s.name, s.args),
    }
}

fn wasm_types(s: &Sig) -> String {
    let params: Vec<&str> = s.params.iter().map(|t| t.name()).collect();
    match s.result {
        Some(r) => format!("({}) -> {}", params.join(", "), r.name()),
        None => format!("({})", params.join(", ")),
    }
}

/// The listing as Markdown, byte for byte what `docs/ABI-1.0.md` holds.
pub fn abi_listing() -> String {
    let mut out = String::new();
    let w = &mut out;
    let _ = writeln!(w, "# Vy ABI 1.0 – fryst förteckning");
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "Genererad ur `crates/vy-abi` av `vy-cli abi-dump`; testet `spec_5_3_5_abi_listing_is_frozen` \\\n\
         i `vy-cli` fallerar när filen avviker från konstanterna. Varje signatur, nummer och kod här \\\n\
         ändras aldrig inom 1.x (SPEC-5.3.5, SPEC-A.1.1, SPEC-A.2.1, SPEC-A.3.1, SPEC-A.4.1, SPEC-A.5.1, \\\n\
         SPEC-C.1.1). Normativ text och motiveringar står i `SPEC.md`; detta är registret."
    );
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "- ABI-version: {}.{} (`abi_version()` returnerar {}).",
        vy_abi::ABI_MAJOR,
        vy_abi::ABI_MINOR,
        vy_abi::ABI_VERSION
    );
    let _ = writeln!(
        w,
        "- Importnamnrymd: `{}` (SPEC-4.2.2). Exporterat minne: `memory` (SPEC-4.2.1).",
        vy_abi::IMPORT_MODULE
    );
    let _ = writeln!(
        w,
        "- Typer: wasm-primitiver `i32`, `i64`, `f32`. Strängar och buffertar är `(ptr: i32, len: i32)` i \\\n\
         modulens linjära minne, UTF-8 utan nollterminering. Statuskod: 0 = ok, negativ = fel enligt \\\n\
         tabellen nedan; en `i64` som packar två mått (× 100) är aldrig negativ, så ett negativt \\\n\
         värde är alltid en felkod (SPEC-5.2.5)."
    );
    let _ = writeln!(w);

    let _ = writeln!(w, "## Exporter ({})", vy_abi::EXPORTS.len());
    let _ = writeln!(w);
    let _ = writeln!(w, "| Funktion | Wasm-typ | Spec |");
    let _ = writeln!(w, "| --- | --- | --- |");
    for (i, s) in vy_abi::EXPORTS.iter().enumerate() {
        let _ = writeln!(
            w,
            "| {} | `{}` | SPEC-5.1.{} |",
            signature(s),
            wasm_types(s),
            i + 1
        );
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Importer ({})", vy_abi::IMPORTS.len());
    let _ = writeln!(w);
    let _ = writeln!(w, "| Funktion | Wasm-typ | Grupp |");
    let _ = writeln!(w, "| --- | --- | --- |");
    for s in vy_abi::IMPORTS {
        let group = match s.name {
            "abi_version" | "log" | "request_frame" | "exit" => "livscykel (SPEC-5.2.1)",
            n if n.starts_with("dl_") => "display list (SPEC-5.2.3)",
            n if n.starts_with("text_") => "text (SPEC-5.2.5)",
            "key_down" | "ime_begin" | "ime_end" => "input (SPEC-5.2.6)",
            n if n.starts_with("fetch_") => "nätverk, capability `net` (SPEC-5.2.7)",
            n if n.starts_with("state_") => "lagring, capability `storage` (SPEC-5.2.9)",
            "navigate" | "link_register" => "navigering (SPEC-5.2.10)",
            "now_ms" | "random" => "tid och slump (SPEC-5.2.11)",
            n if n.starts_with("clipboard_") => "urklipp, capability `clipboard` (SPEC-5.2.12)",
            "sem_node" => "semantik (SPEC-5.2.13)",
            _ => "-",
        };
        let _ = writeln!(w, "| {} | `{}` | {group} |", signature(s), wasm_types(s));
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Statuskoder");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Kod | Namn |");
    let _ = writeln!(w, "| --- | --- |");
    for (code, name) in err::ALL {
        let _ = writeln!(w, "| {code} | `{name}` |");
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Argumentvärden");
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "`log.level` (`vy_abi::level`, SPEC-5.2.1); ett annat värde loggas som {} (`{}`):",
        level::MAX,
        level::name(level::MAX)
    );
    let _ = writeln!(w);
    let _ = writeln!(w, "| Nivå | Namn |");
    let _ = writeln!(w, "| --- | --- |");
    for (i, name) in level::NAMES.iter().enumerate() {
        let _ = writeln!(w, "| {i} | `{name}` |");
    }
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "`fetch_begin.method` (`vy_abi::method`, SPEC-5.2.7); ett annat värde ger `E_ARG` utan \\\n\
         nätverksanrop:"
    );
    let _ = writeln!(w);
    let _ = writeln!(w, "| Värde | Metod |");
    let _ = writeln!(w, "| --- | --- |");
    for (i, name) in method::NAMES.iter().enumerate() {
        let _ = writeln!(w, "| {i} | `{name}` |");
    }
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "`text_style.font_resource_id` under 0 är en inbyggd typsnittsroll (`vy_abi::font`, \\\n\
         SPEC-6.4.2); 0 är modulen och ett positivt id en typsnittsresurs i paketet (SPEC-4.5.1):"
    );
    let _ = writeln!(w);
    let _ = writeln!(w, "| Resurs-id | Roll |");
    let _ = writeln!(w, "| --- | --- |");
    for (i, name) in font::NAMES.iter().enumerate() {
        let _ = writeln!(w, "| {} | `{name}` |", -(i as i32) - 1);
    }
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "`text_style.weight` är {}–{} som i CSS ({} normal, {} fet; `vy_abi::weight`); ett värde \\\n\
         utanför klampas, och värden ritar med det snitt vars vikt ligger närmast, vid lika avstånd \\\n\
         det tyngre (SPEC-5.2.5).",
        weight::MIN,
        weight::MAX,
        weight::NORMAL,
        weight::BOLD
    );
    let _ = writeln!(w);

    let _ = writeln!(w, "## Händelsetyper (`kind` i `vy_event`)");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Kind | Namn | a | b | c | d |");
    let _ = writeln!(w, "| --- | --- | --- | --- | --- | --- |");
    for k in event::ALL {
        let _ = writeln!(
            w,
            "| {} | `{}` | {} | {} | {} | {} |",
            k.kind, k.name, k.args[0], k.args[1], k.args[2], k.args[3]
        );
    }
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "Tangenter värden själv agerar på (USB HID): Tab {:#04X} (blir `FOCUS`, når aldrig modulen), \\\n\
         Enter {:#04X} och Escape {:#04X} (levereras som `KEY_DOWN`) (SPEC-10.3.1).",
        key::TAB,
        key::ENTER,
        key::ESCAPE
    );
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "## Pekarknappar (`c` i `POINTER_DOWN` och `POINTER_UP`; bitar i `POINTER_MOVE`)"
    );
    let _ = writeln!(w);
    let _ = writeln!(w, "| Knapp | Namn | Bit i `POINTER_MOVE.c` |");
    let _ = writeln!(w, "| --- | --- | --- |");
    for b in 1..=button::MAX {
        let _ = writeln!(
            w,
            "| {b} | {} | {} |",
            button::name(b).unwrap_or("?"),
            button::mask(b)
        );
    }
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "`POINTER_DOWN` och `POINTER_UP` bär knappen i `c`; `POINTER_MOVE` bär en mask där bit \\\n\
         `n − 1` är satt för varje nedtryckt knapp `n` (`vy_abi::button`, SPEC-A.2.2)."
    );
    let _ = writeln!(w);

    let _ = writeln!(w, "## Semantiska roller");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Roll | Namn | Fokuserbar |");
    let _ = writeln!(w, "| --- | --- | --- |");
    for r in 1..=role::MAX {
        let _ = writeln!(
            w,
            "| {r} | `{}` | {} |",
            role::name(r).unwrap_or("?"),
            if role::is_focusable(r) { "ja" } else { "nej" }
        );
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Capability-id (`a` i `CAPABILITY`)");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Id | Capability |");
    let _ = writeln!(w, "| --- | --- |");
    for (i, name) in cap::NAMES.iter().enumerate() {
        let _ = writeln!(w, "| {} | `{name}` |", i + 1);
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Modifierarbitar");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Bit | Tangent |");
    let _ = writeln!(w, "| --- | --- |");
    for (bit, name) in modifier::ALL {
        let _ = writeln!(w, "| {bit} | {name} |");
    }
    let _ = writeln!(w);

    let _ = writeln!(w, "## Display list-opkoder");
    let _ = writeln!(w);
    let _ = writeln!(
        w,
        "Varje post är en `u8`-opkod följd av fasta fält, little-endian; koordinater `f32` i logiska \\\n\
         pixlar, färger `u32` som `0xRRGGBBAA`; `fit` är 0 cover, 1 contain, 2 stretch; \\\n\
         path-kommandon är 9 byte (`u8` typ: 0 moveTo, 1 lineTo, 2 quadTo, 3 close; två `f32`) \\\n\
         (SPEC-C.1.1, SPEC-C.1.2)."
    );
    let _ = writeln!(w);
    let _ = writeln!(w, "| Op | Namn | Fält | Byte |");
    let _ = writeln!(w, "| --- | --- | --- | --- |");
    for o in op::ALL {
        let _ = writeln!(
            w,
            "| {:#04X} | `{}` | {} | {} |",
            o.code, o.name, o.fields, o.bytes
        );
    }
    let reserved: Vec<String> = op::RESERVED.iter().map(|c| format!("{c:#04X}")).collect();
    let _ = writeln!(
        w,
        "\nReserverade och aldrig använda i 1.x: {}.",
        reserved.join(", ")
    );
    let _ = writeln!(w);

    let _ = writeln!(w, "## Gränser (`vy_abi::limits`)");
    let _ = writeln!(w);
    let _ = writeln!(w, "| Konstant | Värde | Betydelse |");
    let _ = writeln!(w, "| --- | --- | --- |");
    let rows: [(&str, u64, &str); 24] = [
        (
            "MAX_OPS_PER_FRAME",
            limits::MAX_OPS_PER_FRAME.into(),
            "opkoder per frame (SPEC-6.1.6)",
        ),
        (
            "MAX_CLIP_DEPTH",
            limits::MAX_CLIP_DEPTH.into(),
            "klippstackens djup (SPEC-6.1.4)",
        ),
        (
            "FRAME_BUDGET_MS",
            limits::FRAME_BUDGET_MS.into(),
            "logisk tid per `vy_frame` (SPEC-5.3.3)",
        ),
        (
            "MAX_MEMORY_PAGES",
            limits::MAX_MEMORY_PAGES.into(),
            "största `memory_max` i wasm-sidor (SPEC-8.1.3)",
        ),
        (
            "MAX_IMAGE_SIDE_PX",
            limits::MAX_IMAGE_SIDE_PX.into(),
            "bildsida i pixlar (SPEC-6.5.1)",
        ),
        (
            "MAX_LOG_BYTES",
            limits::MAX_LOG_BYTES.into(),
            "`log`-meddelande (SPEC-9.3.2)",
        ),
        (
            "MAX_TEXT_BYTES",
            limits::MAX_TEXT_BYTES.into(),
            "`dl_text`, `text_measure`, `text_caret` (SPEC-9.3.2)",
        ),
        (
            "MAX_PATH_BYTES",
            limits::MAX_PATH_BYTES.into(),
            "`dl_path` (SPEC-9.3.2)",
        ),
        (
            "MAX_SEM_TEXT_BYTES",
            limits::MAX_SEM_TEXT_BYTES.into(),
            "`sem_node`-text (SPEC-9.3.2)",
        ),
        (
            "MAX_URL_BYTES",
            limits::MAX_URL_BYTES.into(),
            "`navigate`, `link_register`, `fetch_begin`-URL (SPEC-9.3.2)",
        ),
        (
            "MAX_FETCH_BODY_BYTES",
            limits::MAX_FETCH_BODY_BYTES.into(),
            "`fetch_begin`-kropp (SPEC-9.3.2)",
        ),
        (
            "MAX_FETCH_RESPONSE_BYTES",
            limits::MAX_FETCH_RESPONSE_BYTES.into(),
            "buffrad svarskropp, annars `E_TOO_LARGE` (SPEC-5.2.7)",
        ),
        (
            "MAX_OPEN_FETCHES",
            limits::MAX_OPEN_FETCHES.into(),
            "öppna nätverksanrop, därefter `E_RATE` (SPEC-5.2.7)",
        ),
        (
            "MAX_STATE_KEY_BYTES",
            limits::MAX_STATE_KEY_BYTES.into(),
            "`state_*`-nyckel (SPEC-9.3.2)",
        ),
        (
            "MAX_STATE_VALUE_BYTES",
            limits::MAX_STATE_VALUE_BYTES.into(),
            "`state_put`-värde (SPEC-9.3.2)",
        ),
        (
            "MAX_CLIPBOARD_BYTES",
            limits::MAX_CLIPBOARD_BYTES.into(),
            "`clipboard_write` (SPEC-9.3.2)",
        ),
        (
            "MAX_SEM_NODES_PER_FRAME",
            limits::MAX_SEM_NODES_PER_FRAME.into(),
            "semantiska noder per frame (SPEC-9.3.2)",
        ),
        (
            "MAX_FRAME_TEXT_BYTES",
            limits::MAX_FRAME_TEXT_BYTES.into(),
            "text i träd och display list per frame (SPEC-9.3.2)",
        ),
        (
            "fit::MAX",
            vy_abi::fit::MAX as u64,
            "högsta `fit` (SPEC-5.2.3)",
        ),
        (
            "cap::MAX",
            cap::MAX as u64,
            "högsta capability-id (SPEC-A.4.1)",
        ),
        (
            "level::MAX",
            level::MAX as u64,
            "högsta `log.level`; högre klampas hit (SPEC-5.2.1)",
        ),
        (
            "method::MAX",
            method::MAX as u64,
            "högsta `fetch_begin.method` (SPEC-5.2.7)",
        ),
        (
            "button::MAX",
            button::MAX as u64,
            "högsta pekarknapp (SPEC-A.2.2)",
        ),
        (
            "weight::MAX",
            weight::MAX as u64,
            "tyngsta `text_style.weight`; `weight::MIN` är 1 (SPEC-5.2.5)",
        ),
    ];
    for (name, value, meaning) in rows {
        let _ = writeln!(w, "| `{name}` | {value} | {meaning} |");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::Path;

    /// SPEC-5.3.5, SPEC-4.2.1, SPEC-2.1.3: the committed listing is exactly
    /// what `vy-abi` generates: six exports, 32 imports, every table of
    /// appendix A and C. A difference means `vy-abi` changed without the
    /// listing, or the other way round; regenerate with
    /// `cargo run -p vy-cli -- abi-dump > docs/ABI-1.0.md`, which after the
    /// freeze is a 1.x-breaking change nobody may make.
    #[test]
    fn spec_5_3_5_abi_listing_is_frozen() {
        let listing = abi_listing();
        assert!(listing.contains("## Exporter (6)"));
        assert!(listing.contains("## Importer (32)"));
        assert!(listing.contains("`vy_event(kind: i32, a: i32, b: i32, c: i32, d: i32) -> i32`"));
        assert!(listing.contains("| -32 | `E_TOO_LARGE` |"));
        assert!(listing.contains("| 9 | `POINTER_LEAVE` |"));
        assert!(listing.contains("| 23 | `slider` | ja |"));
        assert!(listing.contains("| 0x05 | `CLIP_PUSH` | x, y, w, h, radius: f32 | 21 |"));
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(PATH);
        let committed = fs::read_to_string(&path).unwrap_or_else(|e| {
            panic!(
                "{}: {e}; run `cargo run -p vy-cli -- abi-dump > {PATH}`",
                path.display()
            )
        });
        assert!(
            committed == listing,
            "{PATH} differs from what vy-abi generates; regenerate it with \\\n\
             `cargo run -p vy-cli -- abi-dump > {PATH}` and review the diff (SPEC-5.3.5)"
        );
    }
}
