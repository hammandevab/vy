# Konformanssvit

- `cases.toml` – fallen: modul, logisk storlek, skala, antal frames, beviljade `capabilities`,
  `resources`, inspelade svar (`fetch`) och `events` som spelas upp före sista framen (körs av
  `vy-cli conformance`). Strängar skrivs `"…"` utan escapes eller som TOML-literalsträngar `'…'`,
  som får innehålla `"` (ett inspelat JSON-svar i `body`) men inte `'`. Händelserna är både indata (pekare, tangenter, text, rullning, fokus)
  och värdhändelser: `visibility`, `budget`, `resize`, `suspend`, `reinit` och `fetch_done`,
  var och en med valfritt `after_frames = N` som begränsar hur många frames som körs före den.
- `golden/<modul>-<WxH>@<scale>[-<tillstånd>].png` – guldbilder, en per referensmodul, storlek,
  DPI och scriptat tillstånd (från M1).
- `golden/shell-<modul>-<WxH>@<scale>[-<tillstånd>].png` – skalfall (`shell = true`, från M3):
  modulen körs med sitt utkast i en flikprocess under det huvudlösa skalet (`vy-cli --tab`), och
  bilden är hela fönstret `width × height` med chromen – flikrad, adressfält, markörer, pluppar –
  över sidytan (SPEC-9.6.1, SPEC-11.3.2). Lokala filer körs i begränsat läge (SPEC-7.4.3). Inga
  events, inget träd, inga `capabilities`. Samma bild tas med `vy-shell --screenshot <png> --size
  WxH --manifest <utkast> <modul>`. Med `address = "vy://…"` tas sidan som hämtad därifrån:
  sviten signerar paketet med sin egen nyckel och verifierar det (SPEC-7.4.2), så standardpolicyn
  avgör grants (SPEC-8.2.3) med adressens auktoritet som sidans egen origin (SPEC-9.5.1); en
  capability policyn frågar om (`net` till annan origin, `clipboard`, `persist_frame`) ger frågan
  med Tillåt/Neka, som ligger över sidytans överkant utan att flytta sidan
  (`shell-chat-800x600@1-ask`).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifestutkast (`vy_manifest::Draft`, samma fil som
  `vy-publish build --manifest` läser; kanonisk CBOR på tråden), ett per modul: inga hashar,
  storlekar eller signatur, resurser som sökvägar relativt utkastet med `mime` och bildens
  `w`/`h`. Sviten fyller i modulen och fallets resurser och kör `Manifest::validate`:
  `memory_max` mot modulens initiala minne (SPEC-8.1.3) och som minnestak för körningen
  (SPEC-4.2.4), bildresursers `w`/`h` mot filerna (SPEC-8.1.2), gränserna i §4 och §8. Fallets
  `resources` måste namnge utkastets filer i samma ordning (SPEC-4.5.1) och en storlek fallet
  anger måste vara manifestets; värden får storleken ur manifestet. Fallet får bara bevilja
  vad manifestet begär (SPEC-8.2.2), en sida med `textbox`-noder måste begära `ime`
  (SPEC-10.4.2), `net.origins` avgör vilka adresser `fetch_begin` får nå (SPEC-5.2.8) och
  `storage.quota_kb` är lagringens kvot (SPEC-9.3.1).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2); en pixel räknas som avvikande om någon kanal skiljer sig.
Ett fall vars semantiska träd har `textbox`-noder måste höra till ett manifest som begär `ime`
(SPEC-10.4.2). Tab (`key_down` 43) når aldrig modulen: värden flyttar fokus genom trädet och
skickar `FOCUS` (SPEC-10.3.1). Varje modul
under `modules/` ska ha minst två fall och ett semantiskt träd, och inga guldbilder får sakna
fall (SPEC-12.1.2, `spec_12_1_2_conformance_defines_client`).
Vid avvikelse skrivs den faktiska bilden till `golden/<namn>.png.new` (ignoreras av git).
Ett guldbyte (`cargo run -p vy-cli -- conformance --update`) är ett eget commit med `golden:` som
prefix och en motivering i PR:en.

## Running the suite against another client

The suite defines a Vy client (SPEC-12.1.2), and it is published with every release as
`vy-conformance-<version>.zip` (`tools/package_conformance.sh`): `cases.toml`, the goldens, the
semantic trees, the manifest drafts, the built reference modules under `target/wasm/` and the image
resources, in this repository's layout, with `SHA256SUMS`. `vy-cli conformance --root <dir>` runs
it with this project's headless host. Another client passes when it produces, for every case, the
same image and (where the case names one) the same semantic tree.

**Input of a case.** Load `target/wasm/<module>.wasm` with its draft `conformance/manifests/<module>.json`:
`memory_max` caps the memory (SPEC-4.2.4), `net.origins` limits `fetch_begin` (SPEC-5.2.8), and the
case may grant only capabilities the draft requests (SPEC-8.2.2). Call `vy_init(abi_minor, width,
height, scale)` with the case's logical size and scale, grant the case's `capabilities`, and give
`resources[i]` as resource id `i + 1` (SPEC-4.5.1) with the size the case states (SPEC-8.1.2). Run
the frames the module asks for (the one after `vy_init`, then those after `request_frame`, an event
that returned 1 or a resize, SPEC-6.2.1), deliver the next event, and so on until the events are
spent; an event's `after_frames` caps the frames run before it, and `frames` caps them all. `key_down`
43 is Tab: it moves the host's focus through the focusable nodes and becomes `FOCUS` events
(SPEC-10.3.1); a `pointer_down` moves focus to the node under it before the press. `fetch` lists
canned responses for `fetch_begin`, `fetch_done` delivers the oldest pending one.

**Deterministic host.** The clock is the number of completed frames × 16 ms (`now_ms`), `dt_ms` is
16, and 0 in the first frame and the first after `VISIBILITY` visible (SPEC-5.1.2). `random` is
xorshift64* from the seed `0x5EED5EED00000001`: per 8 bytes, `x ^= x >> 12; x ^= x << 25;
x ^= x >> 27`, then `x × 0x2545F4914F6CDD1D` written little-endian (SPEC-5.2.11). The clipboard
is in memory and starts empty: `clipboard_read` and `clipboard_write` serve it under SPEC-5.2.12,
and a `key_down` of C (6) or X (27) with Ctrl and without Alt, or with Meta, is a copy gesture: a
`clipboard_write` while the module handles that event succeeds without the `clipboard` capability
(SPEC-8.2.4). There is no navigation: `navigate` answers `E_DENIED`. Storage is in memory
with the draft's quota; `reinit` starts a new instance with the old instance's storage.

**Output of a case.**

- One PNG per case, `<name>.png`, the last frame rasterized at the physical size
  `round(width × scale) × round(height × scale)`, RGBA.
- For a case with `semantic`, the tree of the last frame as JSON, byte for byte in this form: `{`,
  `  "nodes": [`, one line per node in registration order,
  `    {"id": 1, "parent_id": 0, "role": 2, "role_name": "heading", "x": 24, "y": 24, "w": 752, "h": 32.5, "text": "…"}`,
  separated by `,`, then `  ]` and `}` and a final newline. Numbers are the shortest decimal that
  reads back as the same `f32`, without exponent (`24`, `32.5`); `role_name` is the role's name in
  appendix A; in `text`, `"` `\` and line feed, carriage return and tab are escaped as `\"` `\\`
  `\n` `\r` `\t`, other characters below U+0020 as `\u00xx`, everything else as is (UTF-8).

**Pass and fail.** A case passes when its image has the golden's size and at most 0.1 % of its
pixels differ from `conformance/golden/<name>.png`, a pixel differing when any of its four channels
does (SPEC-12.2.2), and, when the case names a tree, the JSON equals
`conformance/semantic/<semantic>.json` exactly. A page whose tree has `textbox` nodes must belong to
a draft that requests `ime` (SPEC-10.4.2). Shell cases (`shell = true`) compare the whole window
with this client's chrome; another client skips them or supplies its own goldens for them.
