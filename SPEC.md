# Vy – specifikation 1.0 (utkast, M0)

Version 0.1 · 2026-11-04 · Status: utkast under granskning

Varje normativ mening har ett id på formen `SPEC-<avsnitt>.<del>.<löpnummer>`. Ett id ändras aldrig;
en mening som utgår behåller sitt id med texten *utgått*. Varje id ska refereras av minst ett test
(`tools/spec_trace.py` kontrollerar). Text utan id är förklarande.

Den fullständiga designbeskrivningen med motiveringar finns i det ursprungliga specdokumentet;
det här är den körbara delen.

---

## 1. Sammanfattning

Vy är en klient för ett nytt innehållsformat: sidor är WebAssembly-moduler som ritar sig själva via
ett minimalt värd-API, hämtas över HTTPS med adresser på formen `vy://värd/sökväg`, och körs i en
sandbox utan tillgång till något som manifestet inte uttryckligen begär.

- **[SPEC-1.1.1]** Klienten renderar aldrig HTML, CSS eller JavaScript.
- **[SPEC-1.1.2]** Produkten består av tre oberoende delar: innehållsformat, protokoll och klient; var och en kan bytas ut utan att de andra ändras.
- **[SPEC-1.1.3]** Mål för 1.0: kallstart under 100 ms vid cache-träff, typisk modul under 300 KB, 60 fps på referensmaskinen (§13).

## 2. Designprinciper

När två principer kolliderar vinner den med lägre nummer.

- **[SPEC-2.1.1]** Maskinen är den primära författaren: varje operation i ABI och format har exakt ett kanoniskt uttryck; inga alias.
- **[SPEC-2.1.2]** Verifiering före läsbarhet: typer i ABI:t, hash på moduler, signaturer på manifest och pixeltester på rendering är obligatoriska.
- **[SPEC-2.1.3]** Minimal yta: värd-API:t har högst 40 importer i 1.0; layout, komponenter och tillstånd lever i modulen.
- **[SPEC-2.1.4]** Ingen ambient auktoritet: en modul kan inte göra något den inte begärt i manifestet och fått beviljat.
- **[SPEC-2.1.5]** Determinism: samma modul, samma indata och samma fönsterstorlek ger samma display list byte för byte; tid och slump går alltid genom värden.
- **[SPEC-2.1.6]** Kompilera ner, aldrig tolka: klienten innehåller ingen tolk för något språk; sidor är färdigkompilerad WebAssembly.
- **[SPEC-2.1.7]** En `https://`-länk öppnas i systemets webbläsare; klienten renderar den aldrig.

## 3. Systemöversikt

Utgivaren signerar ett paket, transporten är vanlig HTTPS med cache per hash, klienten delar sig i en
huvudprocess som ritar och en sandboxad flikprocess per sida. De tre delarna delar inget utom
kontraktet: ABI 1.0, manifestet, capabilities och konformanssviten.

## 4. Innehållsformat

- **[SPEC-4.1.1]** Ett paket består av ett manifest, exakt en WebAssembly-modul och noll eller flera resurser.
- **[SPEC-4.1.2]** Paketets identitet är SHA-256 av manifestets kanoniska CBOR-kodning.
- **[SPEC-4.1.3]** Manifestet är CBOR i kanonisk kodning enligt RFC 8949 §4.2 och högst 64 KB.
- **[SPEC-4.1.4]** Modulen är WebAssembly core; komponentmodellen krävs inte i 1.0; okomprimerad storlek högst 8 MB.
- **[SPEC-4.1.5]** Resurser får vara PNG, WebP, AVIF, WOFF2 eller rådata; totalt högst 32 MB.
- **[SPEC-4.2.1]** Modulen exporterar exakt de sex funktionerna i §5.1 med angivna signaturer och inget annat.
- **[SPEC-4.2.2]** Modulen importerar endast funktioner ur namnrymden `vy`.
- **[SPEC-4.2.3]** Modulen importerar inte WASI och använder inte trådar i 1.0.
- **[SPEC-4.2.4]** Modulens linjära minne får inte växa över manifestets `memory_max`.
- **[SPEC-4.3.1]** Manifestet anger ABI-version som `"MAJOR.MINOR"`; en modul med okänd major-version avvisas före laddning.
- **[SPEC-4.3.2]** Minor-versioner är bakåtkompatibla: en 1.x-klient kör varje 1.y-modul där y ≤ x.
- **[SPEC-4.4.1]** Paketet levereras som zstd-komprimerad tar-ström med medietypen `application/vy-package+zstd`.
- **[SPEC-4.5.1]** Resurser refereras med heltalsindex; index 0 är alltid modulen själv.

## 5. ABI

Alla typer är wasm-primitiver (`i32`, `i64`, `f32`). Strängar och buffertar skickas som `(ptr: i32, len: i32)`
i modulens linjära minne, UTF-8 utan nollterminering. Statuskoder: `0` = ok, negativt = fel enligt bilaga A.

### 5.1 Exporter

- **[SPEC-5.1.1]** `vy_init(abi_minor: i32, width: i32, height: i32, scale: f32) -> i32` anropas en gång efter att modulen laddats och verifierats.
- **[SPEC-5.1.2]** `vy_frame(dt_ms: i32) -> i32` anropas när värden vill ha en ny display list och returnerar antal opkoder skrivna.
- **[SPEC-5.1.3]** `vy_event(kind: i32, a: i32, b: i32, c: i32, d: i32) -> i32` anropas vid input- eller systemhändelse och returnerar 1 om omritning begärs.
- **[SPEC-5.1.4]** `vy_resize(width: i32, height: i32, scale: f32)` anropas när fönstret ändrat storlek eller DPI.
- **[SPEC-5.1.5]** `vy_suspend() -> i32` anropas när fliken göms; modulen får skriva undan tillstånd via `state_put` under anropet.
- **[SPEC-5.1.6]** `vy_alloc(size: i32) -> i32` returnerar en pekare till `size` byte som värden får skriva till, eller 0 vid fel.

### 5.2 Importer

- **[SPEC-5.2.1]** Livscykel: `abi_version() -> i32` (MAJOR × 100 + MINOR), `log(level: i32, ptr: i32, len: i32)`, `request_frame()`, `exit(code: i32)`.
- **[SPEC-5.2.2]** `request_frame()` är det enda sättet för modulen att begära ett `vy_frame`-anrop utanför händelser och fönsterändringar.
- **[SPEC-5.2.3]** Display list: `dl_begin()`, `dl_end()`, `dl_rect(x: f32, y: f32, w: f32, h: f32, radius: f32, color: i32)`, `dl_text(x: f32, y: f32, w: f32, style_id: i32, ptr: i32, len: i32) -> i32`, `dl_image(x: f32, y: f32, w: f32, h: f32, resource_id: i32)`, `dl_path(ptr: i32, len: i32, color: i32, stroke_w: f32)`, `dl_clip_push(x: f32, y: f32, w: f32, h: f32)`, `dl_clip_pop()`.
- **[SPEC-5.2.4]** `dl_text` returnerar layoutens höjd i logiska pixlar × 100.
- **[SPEC-5.2.5]** Text: `text_style(font_resource_id: i32, size_px: f32, weight: i32, color: i32) -> i32`, `text_measure(style_id: i32, max_w: f32, ptr: i32, len: i32) -> i64` (bredd × 100 i de höga 32 bitarna, höjd × 100 i de låga), `text_caret(style_id: i32, ptr: i32, len: i32, byte_offset: i32) -> i64` (x × 100 i de höga, y × 100 i de låga).
- **[SPEC-5.2.6]** Input: `pointer_get() -> i64`, `key_down(code: i32) -> i32`, `ime_begin(x: f32, y: f32, w: f32, h: f32)`, `ime_end()`.
- **[SPEC-5.2.7]** Nätverk (capability `net`): `fetch_begin(method: i32, url_ptr: i32, url_len: i32, body_ptr: i32, body_len: i32) -> i32`, `fetch_poll(req_id: i32) -> i32`, `fetch_read(req_id: i32, dst_ptr: i32, max: i32) -> i32`, `fetch_cancel(req_id: i32)`.
- **[SPEC-5.2.8]** `fetch_begin` mot en origin som inte står i manifestets `net`-lista returnerar `E_ORIGIN` utan nätverksanrop.
- **[SPEC-5.2.9]** Lagring (capability `storage`): `state_put(key_ptr: i32, key_len: i32, val_ptr: i32, val_len: i32) -> i32`, `state_get(key_ptr: i32, key_len: i32, dst_ptr: i32, max: i32) -> i32`, `state_delete(key_ptr: i32, key_len: i32)`; nyckelrymd per origin, högst 16 MB.
- **[SPEC-5.2.10]** Navigering: `navigate(url_ptr: i32, url_len: i32, replace: i32)` accepterar endast `vy://` och `https://`; `link_register(id: i32, x: f32, y: f32, w: f32, h: f32, url_ptr: i32, url_len: i32)`.
- **[SPEC-5.2.11]** Tid och slump: `now_ms() -> i64` är monoton med 1 ms upplösning utan capability `hires_time` och 10 µs med; `random(dst_ptr: i32, len: i32)` fylls av värdens CSPRNG.
- **[SPEC-5.2.12]** Urklipp (capability `clipboard`): `clipboard_read(dst_ptr: i32, max: i32) -> i32` kräver användargest inom 1 s; `clipboard_write(ptr: i32, len: i32)`.
- **[SPEC-5.2.13]** Semantik: `sem_node(id: i32, parent_id: i32, role: i32, x: f32, y: f32, w: f32, h: f32, ptr: i32, len: i32)` med roller enligt bilaga A.

### 5.3 Regler för alla anrop

- **[SPEC-5.3.1]** Ett anrop till en import utan beviljad capability returnerar `E_DENIED`, loggas, och avslutar inte modulen.
- **[SPEC-5.3.2]** `dl_*`-anrop utanför `dl_begin`/`dl_end` returnerar `E_DL_STATE` och ignoreras.
- **[SPEC-5.3.3]** `vy_frame` har en budget på 8 ms logisk tid; överskrids den två gånger i rad får modulen `KIND_BUDGET` och därefter ett tvingande `vy_suspend`.
- **[SPEC-5.3.4]** Värden anropar aldrig modulen utanför de sex exporterna; asynkrona svar levereras genom `vy_event`.
- **[SPEC-5.3.5]** ABI 1.0 fryses vid release; befintliga signaturer ändras aldrig inom 1.x.

## 6. Renderingsmodell

- **[SPEC-6.1.1]** Modulen ritar aldrig pixlar; den beskriver en display list som värden rasteriserar.
- **[SPEC-6.1.2]** Display listen är immediate mode och byggs om vid varje `vy_frame`.
- **[SPEC-6.1.3]** Opkoder ritas i ordning; senare över tidigare; ingen z-index.
- **[SPEC-6.1.4]** Klippstacken har högst djup 32; `dl_clip_pop` utan matchande push returnerar `E_DL_CLIP`.
- **[SPEC-6.1.5]** Färg är `i32` tolkad som `0xRRGGBBAA` i sRGB.
- **[SPEC-6.1.6]** Högst 65 536 opkoder per frame; överskott ignoreras och loggas med `E_DL_FULL`.
- **[SPEC-6.2.1]** Värden anropar `vy_frame` endast efter `request_frame`, efter ett `vy_event` som returnerade 1, eller efter fönsterändring.
- **[SPEC-6.2.2]** En stillastående sida förbrukar 0 CPU-tid i modulen.
- **[SPEC-6.3.1]** Alla koordinater är logiska pixlar; `scale` anger förhållandet till fysiska pixlar.
- **[SPEC-6.4.1]** Textformning och typsnittsrendering sker i värden; modulen ser aldrig glyfer.
- **[SPEC-6.4.2]** Värden tillhandahåller fem inbyggda typsnittsroller `sans`, `serif`, `mono`, `ui`, `emoji` som resurs-id −1 till −5.
- **[SPEC-6.4.3]** `dl_text` radbryter vid `w` och returnerar höjden så att modulen kan placera nästa element.
- **[SPEC-6.5.1]** Bilder avkodas lat vid första `dl_image` och cachas per paket; högst 8192 × 8192 px.

## 7. Protokoll

- **[SPEC-7.1.1]** En `vy://värd[:port]/sökväg[?fråga]`-adress översätts till `GET https://värd[:port]/sökväg[?fråga]` med `Accept: application/vy-package+zstd, application/vy-manifest+cbor`.
- **[SPEC-7.1.2]** Fragment (`#`) är inte tillåtna i `vy://`-adresser.
- **[SPEC-7.1.3]** `vy://` kräver TLS; `localhost` är enda undantaget.
- **[SPEC-7.2.1]** Ett svar är antingen hela paketet (`application/vy-package+zstd`) eller bara manifestet (`application/vy-manifest+cbor`) med adresser till modul och resurser.
- **[SPEC-7.2.2]** Ett svar med annan medietyp renderas aldrig; klienten visar en felvy med möjlighet att öppna adressen externt.
- **[SPEC-7.3.1]** Moduler och resurser cachas per SHA-256, inte per URL; en post med matchande hash hämtas aldrig om.
- **[SPEC-7.3.2]** Manifestet hämtas med vanlig HTTP-cache (`ETag`, `max-age`).
- **[SPEC-7.4.1]** Manifestet bär en Ed25519-signatur över sin kanoniska CBOR-kodning utan `sig`-fältet.
- **[SPEC-7.4.2]** Utgivarens publika nyckel publiceras i DNS TXT `_vy.<värd>` och på `https://<värd>/.well-known/vy-key`; båda måste stämma vid första besök.
- **[SPEC-7.4.3]** En sida utan giltig signatur körs i begränsat läge där inga capabilities beviljas.
- **[SPEC-7.4.4]** Nyckelrotation sker genom att ett manifest signerat med den gamla nyckeln anger det nya nyckel-id:t.
- **[SPEC-7.5.1]** Klienten skickar aldrig `Referer` och accepterar högst 3 omdirigeringar.

## 8. Manifestet

- **[SPEC-8.1.1]** Obligatoriska fält: `abi`, `name` (≤ 80 tecken), `summary` (≤ 2 KB), `lang` (BCP 47), `module` {hash, size}, `memory_max`, `published`, `sig`.
- **[SPEC-8.1.2]** Valfria fält: `resources`, `capabilities`, `links`, `fallback` (≤ 16 KB Markdown), `min_size`, `icon`, `source_url`, `skeleton`, `chunks`, `embeds`.
- **[SPEC-8.1.3]** `memory_max` anges i wasm-sidor (64 KB) och är högst 4096.
- **[SPEC-8.2.1]** Capabilities i 1.0: `net` {origins ≤ 16}, `storage` {quota_kb ≤ 16384}, `clipboard`, `hires_time`, `ime`, `persist_frame`.
- **[SPEC-8.2.2]** En capability som inte står i manifestet kan inte beviljas i efterhand.
- **[SPEC-8.2.3]** Standardpolicy: `net` för egen origin, `storage` upp till 1 MB och `ime` beviljas automatiskt; `clipboard` och `persist_frame` efter fråga; `hires_time` nekas som standard.
- **[SPEC-8.3.1]** `fallback` visas när modulen avvisas eller fönstret är mindre än `min_size`.

## 9. Säkerhetsmodell

- **[SPEC-9.1.1]** Varje flik körs i en egen OS-process med en egen wasmtime-instans.
- **[SPEC-9.1.2]** Rasterisering och GPU-kontext finns endast i huvudprocessen; sandboxad kod delar aldrig GPU-kontext.
- **[SPEC-9.2.1]** Beviljade capabilities visas i adressfältet och kan återkallas var för sig av användaren.
- **[SPEC-9.3.1]** Resursgränser per flik verkställs av värden: minne enligt `memory_max`, 8 ms CPU per frame, 0 CPU i bakgrund utan `persist_frame`, 64 samtidiga nätverksanrop, 256 MB/min nätverk, `quota_kb` lagring.
- **[SPEC-9.4.1]** En modul vars hash inte matchar manifestet körs aldrig.
- **[SPEC-9.5.1]** Origin är schema + värd + port; lagring, capabilities och cache-beslut är per origin.
- **[SPEC-9.5.2]** Ingen mekanism låter en origin läsa en annan origins lagring eller tillstånd.
- **[SPEC-9.6.1]** Adressfältet ritas alltid av klienten; en modul kan inte rita utanför sin yta.
- **[SPEC-9.7.1]** Klienten byggs reproducerbart; varje release publicerar hash och byggrecept.

## 10. Tillgänglighet och semantik

- **[SPEC-10.1.1]** Modulen bygger ett semantiskt träd via `sem_node` parallellt med display listen, med samma determinism.
- **[SPEC-10.1.2]** Roller är den fasta listan i bilaga A; noder utan roll ignoreras av hjälpmedel.
- **[SPEC-10.2.1]** Värden exponerar trädet till plattformens tillgänglighets-API via AccessKit.
- **[SPEC-10.2.2]** Textmarkering kopierar den logiska strängen ur textnoder, aldrig pixlar.
- **[SPEC-10.2.3]** `vy-cli --dump-semantic` skriver trädet som JSON.
- **[SPEC-10.3.1]** Fokusordning hålls av värden ur trädet; Tab, Skift+Tab, Enter och Escape levereras som `KIND_FOCUS`-event.
- **[SPEC-10.4.1]** En sida utan semantiskt träd får publiceras men markeras som *otillgänglig* i adressfältet.
- **[SPEC-10.4.2]** En sida med inmatningsfält utan `ime`-capability avvisas av konformanssviten.

## 11. Klientarkitektur

- **[SPEC-11.1.1]** Klienten är uppdelad i crates `vy-abi`, `vy-manifest`, `vy-proto`, `vy-runtime`, `vy-dl`, `vy-render`, `vy-shell`, `vy-cli` med de ansvar som anges i `CLAUDE.md`.
- **[SPEC-11.2.1]** Display lists skickas från flikprocess till huvudprocess över delat minne (ringbuffert per flik).
- **[SPEC-11.2.2]** En krasch i en flikprocess påverkar inte andra flikar eller huvudprocessen.
- **[SPEC-11.3.1]** `vy-cli render <adress eller fil> --out <png> --size WxH [--scale S]` ger en deterministisk PNG utan fönster.
- **[SPEC-11.3.2]** Headless-rendering och fönsterrendering använder samma kodväg i `vy-render`.
- **[SPEC-11.4.1]** Devtools kan spela in alla events och värd-svar för en session och spela upp dem mot samma modul för exakt reproduktion.
- **[SPEC-11.5.1]** Klienten har inget tilläggssystem.

## 12. Utvecklingsflöde

- **[SPEC-12.1.1]** Varje normativ mening i denna spec refereras av minst ett test; `tools/spec_trace.py` blockerar CI annars.
- **[SPEC-12.1.2]** En klient som passerar konformanssviten är en Vy-klient.
- **[SPEC-12.2.1]** Varje PR passerar: fmt, clippy utan varningar, `cargo test --workspace`, spec-spårning, modulbygge med ABI-kontroll.
- **[SPEC-12.2.2]** Från M1: pixeldiff högst 0,1 % mot guldbilder, annars uttryckligt guldbyte i PR:en.
- **[SPEC-12.2.3]** Från M3: ingen prestandaregression över 5 % mot §13.
- **[SPEC-12.3.1]** Klientbinären är högst 25 MB; hela kodbasen inklusive spec och tester ryms under 400 000 tokens.

## 13. Prestandabudget

Referensmaskin: bärbar dator från 2020 med integrerad grafik, 8 GB RAM, 50 Mbit/30 ms RTT. P95 över 100 körningar.

- **[SPEC-13.1.1]** Klient kallstart till tomt fönster: ≤ 150 ms.
- **[SPEC-13.1.2]** Enter till första bild vid cache-träff: ≤ 100 ms.
- **[SPEC-13.1.3]** Enter till första bild vid kall hämtning av 300 KB-modul: ≤ 400 ms.
- **[SPEC-13.1.4]** Kompilering av 300 KB-modul: ≤ 60 ms; 0 ms vid kompileringscache-träff.
- **[SPEC-13.2.1]** Rasterisering av 2 000 opkoder: ≤ 2 ms.
- **[SPEC-13.2.2]** Input till pixel: ≤ 16 ms.
- **[SPEC-13.2.3]** Kontinuerlig scroll av textsida: 60 fps utan tappade frames; textcache-träff > 95 %.
- **[SPEC-13.3.1]** Klient med 10 textflikar: ≤ 400 MB RSS.
- **[SPEC-13.3.2]** Göm flik utan `persist_frame`: 0 % CPU.
- **[SPEC-13.4.1]** En modul som bryter 8 ms-budgeten på referensmaskinen publiceras inte av SDK:t utan `--allow-slow`.

## 14. Färdplan

Se `CLAUDE.md`. M0 (spec, testskal, CI) → M1 (headless) → M2 (ABI-frys, referenssidor) → M3 (klient) → M4 (härdning) → M5 (1.0).

## 15. Öppna frågor

- [ ] `dl_path`: SVG-path-syntax eller binär kommandolista? Lutar åt binär.
- [ ] Delad typsnittscache över origins? Lutar åt nej.
- [ ] Core wasm eller component model? Avgörs av mätning i M1.
- [ ] `fallback` som hela sidan (Markdown-sida utan modul)? Lutar åt ja i 1.1.
- [ ] Transparenslogg för nycklar: vem driver den?

## 16. Möjliggörare (1.1 och framåt)

Capabilities och mekanismer som återskapar det webben har: `file`, `autofill`, `spellcheck`, `socket`, `media`,
`print`, `capture`, `geo`, `notify`, `embed`, `native_ui`, `worker`, `compute`, `rtc`; `skeleton` och
strömmad kompilering; `chunks`; `prefs_get`/`PREFS`; `vy_state_get`/`vy_state_set`; textindex vid
publicering; komponentregister. Specificeras med egna id:n när respektive version påbörjas.

---

## Bilaga A: ABI-konstanter

### Felkoder

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

- **[SPEC-A.1.1]** Felkoderna ovan är de enda statuskoderna i ABI 1.0 och deras numeriska värden ändras aldrig.

### Event-typer (`kind` i `vy_event`)

| Kind | Namn | a | b | c | d |
| --- | --- | --- | --- | --- | --- |
| 1 | `POINTER_MOVE` | x × 100 | y × 100 | knappar | modifierare |
| 2 | `POINTER_DOWN` | x × 100 | y × 100 | knapp | modifierare |
| 3 | `POINTER_UP` | x × 100 | y × 100 | knapp | modifierare |
| 4 | `SCROLL` | dx × 100 | dy × 100 | 0 pixlar / 1 rader | modifierare |
| 5 | `KEY_DOWN` | keycode (USB HID) | modifierare | repeat | – |
| 6 | `KEY_UP` | keycode | modifierare | – | – |
| 7 | `TEXT_INPUT` | ptr | len | – | – |
| 8 | `IME_COMPOSE` | ptr | len | markör | – |
| 10 | `FOCUS` | nod-id | 1 in / 0 ut | 0 mus / 1 tangentbord | – |
| 20 | `FETCH_DONE` | req_id | status | bytes | – |
| 30 | `VISIBILITY` | 1 synlig / 0 gömd | – | – | – |
| 31 | `BUDGET` | ms använt | ms tillåtet | 1 = sista varningen | – |
| 32 | `CAPABILITY` | capability-id | 1 beviljad / 0 återkallad | – | – |

- **[SPEC-A.2.1]** Event-typerna ovan och deras argumentordning ändras aldrig inom 1.x; nya typer får nya nummer.

### Semantiska roller

1 text, 2 heading, 3 link, 4 button, 5 textbox, 6 checkbox, 7 radio, 8 list, 9 listitem, 10 image, 11 table,
12 row, 13 cell, 14 nav, 15 main, 16 region, 17 dialog, 18 menu, 19 menuitem, 20 tab, 21 tabpanel,
22 progress, 23 slider, 24 separator.

- **[SPEC-A.3.1]** Rollerna 1–24 ovan är de enda i 1.0.

## Bilaga B: manifestexempel

Se `conformance/manifests/hello.json` (JSON-form av den kanoniska CBOR-kodningen).

## Bilaga C: display list-opkoder

Buffertformat mellan flikprocess och huvudprocess. Varje post börjar med en `u8`-opkod följd av fasta fält,
little-endian. Koordinater `f32` i logiska pixlar, färger `u32` som `0xRRGGBBAA`.

| Op | Namn | Fält | Byte |
| --- | --- | --- | --- |
| 0x01 | `RECT` | x, y, w, h, radius: f32; color: u32 | 25 |
| 0x02 | `TEXT` | x, y, max_w: f32; style_id: u16; len: u16; utf8[len] | 17 + len |
| 0x03 | `IMAGE` | x, y, w, h: f32; resource_id: u16; fit: u8 | 20 |
| 0x04 | `PATH` | color: u32; stroke_w: f32; n: u16; n × (u8 typ + 2 × f32) | 11 + 9n |
| 0x05 | `CLIP_PUSH` | x, y, w, h: f32 | 17 |
| 0x06 | `CLIP_POP` | – | 1 |
| 0x07 | `LINE` | x1, y1, x2, y2, w: f32; color: u32 | 25 |
| 0x08 | `SHADOW` | x, y, w, h, radius, blur: f32; color: u32 | 29 |
| 0xFF | `END` | – | 1 |

- **[SPEC-C.1.1]** Opkoderna ovan med angivna fältordningar och storlekar är display list-formatet i 1.0.
- **[SPEC-C.1.2]** Path-kommandon är 9 byte: `u8` typ (0 moveTo, 1 lineTo, 2 quadTo, 3 close) följt av två `f32`; `quadTo` följs av ett extra kommando med kontrollpunkten.
- **[SPEC-C.1.3]** Högst 64 `SHADOW` per frame.
