# Vy ABI 1.0 – fryst förteckning

Genererad ur `crates/vy-abi` av `vy-cli abi-dump`; testet `spec_5_3_5_abi_listing_is_frozen` \
i `vy-cli` fallerar när filen avviker från konstanterna. Varje signatur, nummer och kod här \
ändras aldrig inom 1.x (SPEC-5.3.5, SPEC-A.1.1, SPEC-A.2.1, SPEC-A.3.1, SPEC-A.4.1, SPEC-A.5.1, \
SPEC-C.1.1). Normativ text och motiveringar står i `SPEC.md`; detta är registret.

- ABI-version: 1.0 (`abi_version()` returnerar 100).
- Importnamnrymd: `vy` (SPEC-4.2.2). Exporterat minne: `memory` (SPEC-4.2.1).
- Typer: wasm-primitiver `i32`, `i64`, `f32`. Strängar och buffertar är `(ptr: i32, len: i32)` i \
modulens linjära minne, UTF-8 utan nollterminering. Statuskod: 0 = ok, negativ = fel enligt \
tabellen nedan; en `i64` som packar två mått (× 100) är aldrig negativ, så ett negativt \
värde är alltid en felkod (SPEC-5.2.5).

## Exporter (6)

| Funktion | Wasm-typ | Spec |
| --- | --- | --- |
| `vy_init(abi_minor: i32, width: i32, height: i32, scale: f32) -> i32` | `(i32, i32, i32, f32) -> i32` | SPEC-5.1.1 |
| `vy_frame(dt_ms: i32) -> i32` | `(i32) -> i32` | SPEC-5.1.2 |
| `vy_event(kind: i32, a: i32, b: i32, c: i32, d: i32) -> i32` | `(i32, i32, i32, i32, i32) -> i32` | SPEC-5.1.3 |
| `vy_resize(width: i32, height: i32, scale: f32)` | `(i32, i32, f32)` | SPEC-5.1.4 |
| `vy_suspend() -> i32` | `() -> i32` | SPEC-5.1.5 |
| `vy_alloc(size: i32) -> i32` | `(i32) -> i32` | SPEC-5.1.6 |

## Importer (32)

| Funktion | Wasm-typ | Grupp |
| --- | --- | --- |
| `abi_version() -> i32` | `() -> i32` | livscykel (SPEC-5.2.1) |
| `log(level: i32, ptr: i32, len: i32) -> i32` | `(i32, i32, i32) -> i32` | livscykel (SPEC-5.2.1) |
| `request_frame()` | `()` | livscykel (SPEC-5.2.1) |
| `exit(code: i32)` | `(i32)` | livscykel (SPEC-5.2.1) |
| `dl_begin() -> i32` | `() -> i32` | display list (SPEC-5.2.3) |
| `dl_end() -> i32` | `() -> i32` | display list (SPEC-5.2.3) |
| `dl_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32) -> i32` | `(f32, f32, f32, f32, f32, i32) -> i32` | display list (SPEC-5.2.3) |
| `dl_text(x: f32, y: f32, w: f32, style_id: i32, ptr: i32, len: i32) -> i64` | `(f32, f32, f32, i32, i32, i32) -> i64` | display list (SPEC-5.2.3) |
| `dl_image(x: f32, y: f32, w: f32, h: f32, resource_id: i32, fit: i32) -> i32` | `(f32, f32, f32, f32, i32, i32) -> i32` | display list (SPEC-5.2.3) |
| `dl_path(ptr: i32, len: i32, color: i32, stroke_w: f32) -> i32` | `(i32, i32, i32, f32) -> i32` | display list (SPEC-5.2.3) |
| `dl_clip_push(x: f32, y: f32, w: f32, h: f32, radius: f32) -> i32` | `(f32, f32, f32, f32, f32) -> i32` | display list (SPEC-5.2.3) |
| `dl_clip_pop() -> i32` | `() -> i32` | display list (SPEC-5.2.3) |
| `text_style(font_resource_id: i32, size_px: f32, weight: i32, color: i32) -> i32` | `(i32, f32, i32, i32) -> i32` | text (SPEC-5.2.5) |
| `text_measure(style_id: i32, max_w: f32, ptr: i32, len: i32) -> i64` | `(i32, f32, i32, i32) -> i64` | text (SPEC-5.2.5) |
| `text_caret(style_id: i32, ptr: i32, len: i32, byte_offset: i32) -> i64` | `(i32, i32, i32, i32) -> i64` | text (SPEC-5.2.5) |
| `key_down(code: i32) -> i32` | `(i32) -> i32` | input (SPEC-5.2.6) |
| `ime_begin(x: f32, y: f32, w: f32, h: f32) -> i32` | `(f32, f32, f32, f32) -> i32` | input (SPEC-5.2.6) |
| `ime_end() -> i32` | `() -> i32` | input (SPEC-5.2.6) |
| `fetch_begin(method: i32, url_ptr: i32, url_len: i32, body_ptr: i32, body_len: i32) -> i32` | `(i32, i32, i32, i32, i32) -> i32` | nätverk, capability `net` (SPEC-5.2.7) |
| `fetch_poll(req_id: i32) -> i32` | `(i32) -> i32` | nätverk, capability `net` (SPEC-5.2.7) |
| `fetch_read(req_id: i32, dst_ptr: i32, max: i32) -> i32` | `(i32, i32, i32) -> i32` | nätverk, capability `net` (SPEC-5.2.7) |
| `fetch_cancel(req_id: i32) -> i32` | `(i32) -> i32` | nätverk, capability `net` (SPEC-5.2.7) |
| `state_put(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32` | `(i32, i32, i32, i32) -> i32` | lagring, capability `storage` (SPEC-5.2.9) |
| `state_get(key_ptr: i32, key_len: i32, dst_ptr: i32, max: i32) -> i32` | `(i32, i32, i32, i32) -> i32` | lagring, capability `storage` (SPEC-5.2.9) |
| `state_delete(key_ptr: i32, key_len: i32) -> i32` | `(i32, i32) -> i32` | lagring, capability `storage` (SPEC-5.2.9) |
| `navigate(url_ptr: i32, url_len: i32, replace: i32) -> i32` | `(i32, i32, i32) -> i32` | navigering (SPEC-5.2.10) |
| `link_register(id: i32, x: f32, y: f32, w: f32, h: f32, url_ptr: i32, url_len: i32) -> i32` | `(i32, f32, f32, f32, f32, i32, i32) -> i32` | navigering (SPEC-5.2.10) |
| `now_ms() -> i64` | `() -> i64` | tid och slump (SPEC-5.2.11) |
| `random(dst_ptr: i32, len: i32) -> i32` | `(i32, i32) -> i32` | tid och slump (SPEC-5.2.11) |
| `clipboard_read(dst_ptr: i32, max: i32) -> i32` | `(i32, i32) -> i32` | urklipp, capability `clipboard` (SPEC-5.2.12) |
| `clipboard_write(ptr: i32, len: i32) -> i32` | `(i32, i32) -> i32` | urklipp, capability `clipboard` (SPEC-5.2.12) |
| `sem_node(id: i32, parent_id: i32, role: i32, x: f32, y: f32, w: f32, h: f32, ptr: i32, len: i32) -> i32` | `(i32, i32, i32, f32, f32, f32, f32, i32, i32) -> i32` | semantik (SPEC-5.2.13) |

## Statuskoder

| Kod | Namn |
| --- | --- |
| 0 | `OK` |
| -1 | `E_DENIED` |
| -2 | `E_ARG` |
| -3 | `E_OOM` |
| -4 | `E_QUOTA` |
| -5 | `E_RATE` |
| -10 | `E_DL_STATE` |
| -11 | `E_DL_FULL` |
| -12 | `E_DL_CLIP` |
| -20 | `E_RESOURCE` |
| -21 | `E_STYLE` |
| -30 | `E_ORIGIN` |
| -31 | `E_NET` |
| -32 | `E_TOO_LARGE` |
| -40 | `E_GESTURE` |

## Argumentvärden

`log.level` (`vy_abi::level`, SPEC-5.2.1); ett annat värde loggas som 3 (`error`):

| Nivå | Namn |
| --- | --- |
| 0 | `debug` |
| 1 | `info` |
| 2 | `warn` |
| 3 | `error` |

`fetch_begin.method` (`vy_abi::method`, SPEC-5.2.7); ett annat värde ger `E_ARG` utan \
nätverksanrop:

| Värde | Metod |
| --- | --- |
| 0 | `GET` |
| 1 | `POST` |
| 2 | `PUT` |
| 3 | `DELETE` |

## Händelsetyper (`kind` i `vy_event`)

| Kind | Namn | a | b | c | d |
| --- | --- | --- | --- | --- | --- |
| 1 | `POINTER_MOVE` | x × 100 | y × 100 | knappar | modifierare |
| 2 | `POINTER_DOWN` | x × 100 | y × 100 | knapp | modifierare |
| 3 | `POINTER_UP` | x × 100 | y × 100 | knapp | modifierare |
| 4 | `SCROLL` | dx × 100 | dy × 100 | 0 pixlar / 1 rader | modifierare |
| 5 | `KEY_DOWN` | keycode (USB HID) | modifierare | repeat | - |
| 6 | `KEY_UP` | keycode | modifierare | - | - |
| 7 | `TEXT_INPUT` | ptr | len | - | - |
| 8 | `IME_COMPOSE` | ptr | len | markör | - |
| 9 | `POINTER_LEAVE` | - | - | - | - |
| 10 | `FOCUS` | nod-id | 1 in / 0 ut | 0 mus / 1 tangentbord | - |
| 20 | `FETCH_DONE` | req_id | HTTP-status eller negativ felkod | bytes | - |
| 30 | `VISIBILITY` | 1 synlig / 0 gömd | - | - | - |
| 31 | `BUDGET` | ms använt | ms tillåtet | 1 = sista varningen | - |
| 32 | `CAPABILITY` | capability-id | 1 beviljad / 0 återkallad | - | - |

Tangenter värden själv agerar på (USB HID): Tab 0x2B (blir `FOCUS`, når aldrig modulen), \
Enter 0x28 och Escape 0x29 (levereras som `KEY_DOWN`) (SPEC-10.3.1).

## Semantiska roller

| Roll | Namn | Fokuserbar |
| --- | --- | --- |
| 1 | `text` | nej |
| 2 | `heading` | nej |
| 3 | `link` | ja |
| 4 | `button` | ja |
| 5 | `textbox` | ja |
| 6 | `checkbox` | ja |
| 7 | `radio` | ja |
| 8 | `list` | nej |
| 9 | `listitem` | nej |
| 10 | `image` | nej |
| 11 | `table` | nej |
| 12 | `row` | nej |
| 13 | `cell` | nej |
| 14 | `nav` | nej |
| 15 | `main` | nej |
| 16 | `region` | nej |
| 17 | `dialog` | nej |
| 18 | `menu` | nej |
| 19 | `menuitem` | ja |
| 20 | `tab` | ja |
| 21 | `tabpanel` | nej |
| 22 | `progress` | nej |
| 23 | `slider` | ja |
| 24 | `separator` | nej |

## Capability-id (`a` i `CAPABILITY`)

| Id | Capability |
| --- | --- |
| 1 | `net` |
| 2 | `storage` |
| 3 | `clipboard` |
| 4 | `hires_time` |
| 5 | `ime` |
| 6 | `persist_frame` |

## Modifierarbitar

| Bit | Tangent |
| --- | --- |
| 1 | Skift |
| 2 | Ctrl |
| 4 | Alt / Option |
| 8 | Meta (Cmd / Win) |

## Display list-opkoder

Varje post är en `u8`-opkod följd av fasta fält, little-endian; koordinater `f32` i logiska \
pixlar, färger `u32` som `0xRRGGBBAA`; `fit` är 0 cover, 1 contain, 2 stretch; \
path-kommandon är 9 byte (`u8` typ: 0 moveTo, 1 lineTo, 2 quadTo, 3 close; två `f32`) \
(SPEC-C.1.1, SPEC-C.1.2).

| Op | Namn | Fält | Byte |
| --- | --- | --- | --- |
| 0x01 | `RECT` | x, y, w, h, radius: f32; color: u32 | 25 |
| 0x02 | `TEXT` | x, y, max_w: f32; style_id: u16; len: u16; utf8[len] | 17 + len |
| 0x03 | `IMAGE` | x, y, w, h: f32; resource_id: u16; fit: u8 | 20 |
| 0x04 | `PATH` | color: u32; stroke_w: f32; n: u16; n × (u8 typ + 2 × f32) | 11 + 9n |
| 0x05 | `CLIP_PUSH` | x, y, w, h, radius: f32 | 21 |
| 0x06 | `CLIP_POP` | - | 1 |
| 0xFF | `END` | - | 1 |

Reserverade och aldrig använda i 1.x: 0x07, 0x08.

## Gränser (`vy_abi::limits`)

| Konstant | Värde | Betydelse |
| --- | --- | --- |
| `MAX_OPS_PER_FRAME` | 65536 | opkoder per frame (SPEC-6.1.6) |
| `MAX_CLIP_DEPTH` | 32 | klippstackens djup (SPEC-6.1.4) |
| `FRAME_BUDGET_MS` | 8 | logisk tid per `vy_frame` (SPEC-5.3.3) |
| `MAX_MEMORY_PAGES` | 4096 | största `memory_max` i wasm-sidor (SPEC-8.1.3) |
| `MAX_IMAGE_SIDE_PX` | 8192 | bildsida i pixlar (SPEC-6.5.1) |
| `MAX_LOG_BYTES` | 4096 | `log`-meddelande (SPEC-9.3.2) |
| `MAX_TEXT_BYTES` | 65535 | `dl_text`, `text_measure`, `text_caret` (SPEC-9.3.2) |
| `MAX_PATH_BYTES` | 589815 | `dl_path` (SPEC-9.3.2) |
| `MAX_SEM_TEXT_BYTES` | 4096 | `sem_node`-text (SPEC-9.3.2) |
| `MAX_URL_BYTES` | 2048 | `navigate`, `link_register`, `fetch_begin`-URL (SPEC-9.3.2) |
| `MAX_FETCH_BODY_BYTES` | 1048576 | `fetch_begin`-kropp (SPEC-9.3.2) |
| `MAX_FETCH_RESPONSE_BYTES` | 67108864 | buffrad svarskropp, annars `E_TOO_LARGE` (SPEC-5.2.7) |
| `MAX_OPEN_FETCHES` | 64 | öppna nätverksanrop, därefter `E_RATE` (SPEC-5.2.7) |
| `MAX_STATE_KEY_BYTES` | 256 | `state_*`-nyckel (SPEC-9.3.2) |
| `MAX_STATE_VALUE_BYTES` | 1048576 | `state_put`-värde (SPEC-9.3.2) |
| `MAX_CLIPBOARD_BYTES` | 1048576 | `clipboard_write` (SPEC-9.3.2) |
| `MAX_SEM_NODES_PER_FRAME` | 65536 | semantiska noder per frame (SPEC-9.3.2) |
| `MAX_FRAME_TEXT_BYTES` | 16777216 | text i träd och display list per frame (SPEC-9.3.2) |
| `fit::MAX` | 2 | högsta `fit` (SPEC-5.2.3) |
| `cap::MAX` | 6 | högsta capability-id (SPEC-A.4.1) |
| `level::MAX` | 3 | högsta `log.level`; högre klampas hit (SPEC-5.2.1) |
| `method::MAX` | 3 | högsta `fetch_begin.method` (SPEC-5.2.7) |
