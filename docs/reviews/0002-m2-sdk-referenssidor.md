# PR #2 – M2: SDK och referenssidor (`a419760..34410d0`)

**Vad ändrades.** `vy-sdk` (3 900 rader, `no_std`): typade bindningar för alla 33 importer, `Frame`
med lokal validering, `Sem`, `Event`, `export!`, bump-arena för `vy_alloc`, egen allokator och en
nativ mock. `dl_image` fick `fit` och riktig PNG-avkodning med cache per instans; `vy-cli` fick
händelser, capabilities, resurser och fokus i `cases.toml`; elva moduler (hello porterad + tio
referenssidor), 44 fall och 23 semantiska träd; macOS i CI. 14 600 rader, inga nya crates.

**Spec-id som berörs.** SPEC-4.2.1, 4.2.4–4.2.5, 5.1.*, 5.2.3, 5.2.6, 5.2.9–5.2.11, 5.3.1–5.3.3,
6.1.4, 6.1.6, 6.5.1, 8.2.1, 9.3.2, 10.1.1, 10.4.2, 12.1.2, A.2.1, C.1.2. Sex nya spec-frågor i
BUILD-NOTES (11–16) och 34 SDK-luckor i `docs/notes/m2-pages-{a,b}.md`, konsoliderade nedan.

**Grindar (körda av granskaren).** Modulbygge 11/11, fmt, clippy, `cargo test --workspace`
(103 gröna, 48 ignorerade; 104 efter fixen nedan), spec_trace 127/127, check-abi 11/11, konformans
44/44 à 0,000 % (23 träd lika), modul-clippy 11/11, `ci.yml` parsar och bygger modulerna före
testerna i båda jobben. Exporterna kontrollerade direkt ur wasm-binärerna, oberoende av check-abi:
exakt sex funktioner + `memory`, ingen start-sektion, 18 initiala sidor utan max (SPEC-4.2.1,
4.2.5) i alla elva. **Fuzz** (tillfällig harness, raderad): PNG 111 000 inmatningar (slump,
bitflippade gallery/shop-bilder, omskrivna IHDR med giltig CRC) – **1 processavbrott, fixat**
(`ebe9396`), därefter 0; `parse_cases` 65 000 muterade filer, 0 panics, långsammast 46 ms; alla
elva moduler genom `Runtime` med slumpade händelser (pekare ±2³¹, NaN/∞-skala, 0×0-fönster, 70 KB
text, okända kinds, bitflippade resurser), 322 körningar à 200 frames, 54 000 händelser:
**0 värdpanics**. Modultrappar uppstod bara vid `memory_max` = initialt minne (fynd 4); med
växtutrymme 0 trappar på 23 000 frames.

**Risk.** ABI:t fryses efter M2 (SPEC-5.3.5): varje signatur som är fel nu är fel i hela 1.x.
Det största fyndet är därför inte en bugg utan en motsägelse: 16 importer kan misslyckas men
returnerar ingenting (fynd 1). Sandboxen höll mot fuzz; ingen läcka förbi importerna.

**Bevis.** Un-ignorerade: 6.5.1 (riktigt: header-gräns, cache, fit-pixlar), 10.4.2, 12.1.2. 23 nya
`spec_*`-tester, varav SDK:ts 13 går mot mocken, inte mot värden – de bevisar SDK:ts spegling av
reglerna; värdens egna tester finns sedan M1. Fem moduler har 18 nativa tester (gröna här, körs
inte av CI). `spec_10_4_2_ime_required_for_input` (`conformance.rs:811`)
är verkligt men svagt: det kontrollerar att ett fall med `textbox`-noder i *sista* framen listar
`ime` i `cases.toml`, alltså svitens egen konsekvens, inte manifestets begäran (spec-fråga 15).
`spec_12_1_2_conformance_defines_client` (`conformance.rs:612`) är ett metatest (varje modul ≥ 2
fall + träd, inga föräldralösa guldbilder) – bra spårankare, men inget bevis för att sviten
räcker. Guldbilder: 40 nya, 4 oförändrade (hello efter portering: 0,000 %). Fix i eget commit med
test som avbryter processen utan fixen: `spec_6_5_1_decode_png_header_checked_before_alloc`.

**Fynd (ej fixade, rapporteras).**
1. *Importer utan returvärde kan inte returnera felkoder.* `dl_rect`, `dl_image`, `dl_path`,
   `dl_clip_push/pop`, `ime_begin/end`, `state_delete`, `navigate`, `link_register`, `random`,
   `clipboard_write`, `sem_node`, `log`, `fetch_cancel`, `dl_end` (`vy-abi/src/lib.rs:388–552`)
   är `-> ()`, men SPEC-5.3.1, 5.3.2 och 6.1.4 säger "returnerar `E_DENIED`/`E_DL_STATE`/
   `E_DL_CLIP`". En modul ser aldrig att `dl_image` gav `E_RESOURCE`; SDK:ts `Result` på dessa
   speglar bara lokala kontroller (`api.rs:133–247`). Måste avgöras före frysen.
2. *SDK:ts opkodsräkning kan avvika från värdens.* `Frame` räknar `ops += 1` för `dl_image`/`dl_rect`
   även när värden avvisade anropet (`frame.rs:149,184`), så `vy_frame` rapporterar fler opkoder
   än listan har. Följd av fynd 1. `Frame::begin` är dessutom `pub` (`frame.rs:105`), så
   "SPEC-5.3.2 kan inte brytas från Rust" (BUILD-NOTES) är överdrivet; `arena::bytes` ger
   `&'static [u8]` till minne värden skriver om (`arena.rs:71`) – osund signatur, ofarlig i praktiken.
3. *`vy_alloc` över 64 KB ger 0* (`arena.rs:55`); värden svarar rent med `Error::Alloc`
   (`vy-runtime/src/lib.rs:487`) men specen säger inte vad som händer med texten (spec-fråga 16).
4. *`memory_max` = modulens initiala sidor dödar varje SDK-sida i första framen:* första
   `memory.grow` nekas (`heap.rs:140`), `handle_alloc_error` blir `unreachable` (`app.rs:200`).
   Rätt enligt SPEC-4.2.4 men inget kontrollerar manifestet mot modulens minimum, och
   `Error::Trap` tappar orsaken (`lib.rs:349` använder `to_string()`, inte `Trap`-koden).
5. *Värden accepterar dubbla nod-id och länkar lever över frames.* `sem_node` saknar
   dubblettkontroll (`host.rs:1026`), och `node_with_id` flyttar `next_id` så fasta id kolliderar
   (`sem.rs:144`, notes b §5); `link_register` byts per id men nollställs aldrig (`host.rs:939`), så
   4 096-taket nås av en sida som rullar länkar i och ur vyn.
6. *Avkodning i framebudgeten.* En laglig 8192² RGBA-PNG (1,36 MB) kostar 43 M bränsle vid första
   `dl_image` (`host.rs:569`) – `KIND_BUDGET`-varning för varje bild över 250 KB, avbruten frame över
   2 MB (spec-fråga 11). Mätt: 1,9 s avkodning i debug.
7. *Småsaker.* Fyra moduler saknar manifest i `conformance/manifests/` (gallery, list, game,
   dashboard); `text_style(..).unwrap_or(StyleId(0))` är kopierad i nio moduler; modulernas nativa
   tester körs inte av CI (notes a §13); macOS-jobbet kör varken modul-clippy eller dessa tester;
   `pack_x100` ger negativ `i64` för bredd × 100 ≥ 2³¹ som SDK:t läser som fel (`text.rs:340`,
   `error.rs:131`); `Column`/`Row` används av en sida av elva.

**Nya beroenden.** Inga. `vy-sdk` → `vy-abi`; modulerna → `vy-sdk` (chat även `vy-abi`, fynd i
listan nr 23). Lockfiler per modul.

**Hotbild.** Inga nya ABI-importer (`dl_image` bytte signatur enligt v0.2). Nya ytor: PNG-avkodaren
tar modulstyrda byte – header-gräns före allokering, cache på 256 MB per instans, misslyckande
cachas; fuzzad. `vy_alloc`-pekaren bounds-kontrolleras av värden. Allokatorn rör inte initialt
minne. Inga nya hotanalyser krävs.

**Rekommendation.** **Merge efter** att specägaren avgjort beslutslistans punkter 1–4 och de
som blir *ändra ABI* är genomförda med tester i samma M2 – efter SPEC-5.3.5 går det inte. Fynd 2
och 5, spec-texterna (5–22, 29–31) och verktygen kan följa som M2.5. Fixen `ebe9396` ingår redan
på `main` i den här granskningen.

## Beslutslista inför ABI-frysning

Konsoliderad ur BUILD-NOTES 11–16, notes a §1–15, notes b §1–19 och fynden ovan; dubbletter ihopslagna.
*ABI* = signatur/nummer som inte kan ändras efter frysen; *spec-text* = normativ mening utan ABI-ändring.

| Nr | Punkt (källa) | Beslut | Motivering |
| --- | --- | --- | --- |
| 1 | Fallerande importer returnerar `()` (fynd 1) | **ändra ABI**: alla importer som kan ge en kod i bilaga A returnerar `i32` | SPEC-5.3.1/5.3.2/6.1.4 lovar redan returkoder; omöjligt efter frysen |
| 2 | Högerställd text kostar två formningar (b10, a9) | **ändra ABI**: `dl_text -> i64`, bredd × 100 hög / höjd × 100 låg | samma packning som `text_measure`; halverar värdarbete i tre av fyra sidor; ingen `align`-parameter (princip 3) |
| 3 | `dl_clip_push` utan radie (b12) | **ändra ABI**: lägg till `radius: f32` (CLIP_PUSH 21 byte) | syns på varje kort med bild; en parameter nu eller aldrig i 1.x |
| 4 | `pointer_get` oanvänd (a6) | **ändra ABI**: stryk (33 → 32) | POINTER_MOVE levereras alltid, sidan kan hålla positionen själv; `key_down` behålls för hållna tangenter |
| 5 | `LINE`/`SHADOW` utan import (BN14) | **ändra spec-text**: stryk opkoderna; **avslå** `dl_line`/`dl_shadow` | ingen modul kan producera dem; linjer via `dl_path`; nya opkoder får nya nummer i 1.1 |
| 6 | Pekaren lämnar sidan (a1) | **ändra spec-text**: ny kind `POINTER_LEAVE` = 9 utan argument | SPEC-A.2.1 tillåter nya nummer; billigare än sentinelkoordinater |
| 7 | Bildstorlek okänd för modulen (a2) | **ändra spec-text**: `resources[i].w/h` obligatoriska för bilder; **avslå** `resource_info` | manifestet är signerat (princip 2), ingen import (princip 3) |
| 8 | `dt_ms` efter paus (a3) | **ändra spec-text** 5.1.2: tid sedan föregående `vy_frame`, 0 efter `vy_init`/VISIBILITY synlig | annars klampar varje spel själv |
| 9 | `key_down`-tillstånd (a4) | **ändra spec-text** 5.2.6: hålls till KEY_UP, nollställs vid VISIBILITY dold och fokusförlust | headless gör det första, skalet måste göra det andra |
| 10 | Omritning dold (a5) | **ändra spec-text** 6.2.1: inget `vy_frame` dold utan `persist_frame`, oavsett svar | följer redan av 9.3.1 |
| 11 | `max_w ≤ 0` = ingen radbrytning (a8) | **ändra spec-text** 6.4.3 | alla sidor bygger på M1-tolkningen |
| 12 | `vy_alloc`: livslängd och 0 (BN16, fynd 3) | **ändra spec-text** 5.1.6: giltig tills `vy_event` returnerat; vid 0 loggar värden och släpper händelsen | 64 KB räcker för IME; stora inklistringar är `clipboard` |
| 13 | `state_get` på saknad nyckel (b1) | **ändra spec-text** 5.2.9: `E_RESOURCE` | koden betyder redan "okänd resurs" |
| 14 | `FETCH_DONE.status` och kroppens liv (b8) | **ändra spec-text** A/5.2.7: HTTP-status eller negativ kod; kroppen frigörs vid `fetch_cancel`; 64 öppna, sedan `E_RATE` | idag bara i SDK-doc |
| 15 | Tab/Enter/Escape (b3) | **ändra spec-text** 10.3.1: Tab/Skift+Tab konsumeras av värden och blir FOCUS; Enter/Escape är KEY_DOWN; **avslå** `KIND_ACTIVATE` | FOCUS bär ingen handling; ACTIVATE dubblerar KEY_DOWN (princip 1) |
| 16 | Träd utanför fönstret (b4) | **ändra spec-text** 10.1.1: hela trädet, värden klipper | fokusordningen måste nå länkar nedanför |
| 17 | Länkars livslängd (b11, fynd 5) | **ändra spec-text** 5.2.10: per frame, nollställs vid `dl_begin` | samma regel som trädet; tar bort 4 096-taket som fälla |
| 18 | Dubbla nod-id (b5, fynd 5) | **ändra spec-text** 5.2.13: dubblett i samma frame ger `E_ARG`; **ändra SDK**: fasta id är negativa, automatiska positiva | två id-rum utan `reserve` |
| 19 | Fokusring (b13) | **ändra spec-text**: modulen ritar | värden har ingen stil; spegel av 9.6.1 |
| 20 | Avkodning i budgeten (BN11, fynd 6) | **ändra spec-text** 5.3.3/6.5.1: resursavkodning debiteras inte; M3 avkodar utanför UI-tråden; **avslå** `IMAGE_READY` | engångskostnad bunden av manifestet, inte av modulen per frame |
| 21 | Capability-id och modifierarbitar (BN12) | **ändra spec-text** bilaga A: `vy_abi::cap` 1–6, Shift 1/Ctrl 2/Alt 4/Meta 8 | utan tal kan ingen annan klient vara konform |
| 22 | SPEC-10.4.2 "utan ime" (BN15) | **ändra spec-text**: "begär inte `ime` i manifestet"; sviten läser manifestet från M3 | dagens kontroll mäter svitens egen fil |
| 23 | `Event::Capability { id: i32 }` (b6) | **ändra SDK**: enum `Capability`, ingen `vy-abi`-import i sidor | chat beror på `vy-abi` bara för detta |
| 24 | `Sem`-hjälpare för sju roller (a10) | **ändra SDK**: behåll `node`/`node_with_id`/`begin`/`group`, stryk rollhjälparna | ett sätt (princip 1) |
| 25 | `Column`/`Row` oanvända (a11, b14) | **ändra SDK**: stryk; dokumentera `layout()`-mönstret | tio av elva sidor räknar rektanglar själva; lägg inte till `Layout` förrän en sida behöver |
| 26 | `floor`/`round`/`snap` i `no_std` (a12, b15) | **ändra SDK**: `vy_sdk::math` via cast, ingen `libm` | 1 px-linjer vid skala 1,5 |
| 27 | `text_style(..).unwrap_or(StyleId(0))` × 9 (fynd 7) | **ändra SDK**: `App::init -> Result<Self, VyError>`, koden blir `vy_init`:s returvärde | SPEC-5.1.1 tillåter redan icke-noll |
| 28 | `Frame::begin` pub, `arena::bytes` `'static` (fynd 2) | **ändra SDK**: `pub(crate)` + livstid bunden till utskicket | sundhet, ingen ABI-påverkan |
| 29 | `memory_max` < initialt minne + 1 (fynd 4) | **ändra spec-text** 8.1.3: `memory_max` ≥ modulens initiala sidor + 1; validering i `vy-manifest` | annars trappar sidan deterministiskt i frame 1 |
| 30 | `i64`-packning vs negativa fel (fynd 7) | **ändra spec-text** 5.2.5: varje halva ≤ 2³¹ − 1; fel är negativ `i64` | annars läses 21 M px bredd som fel |
| 31 | Manifestform för `net.origins`/`quota_kb` (b17) | **ändra spec-text** 8.2.1: `capabilities` är en map namn → objekt | kanonisk form före `vy-manifest` |
| 32 | Runner: scriptad visibility/budget/resize/suspend/reinit/fetch_done, `after_frames`, minneslagring headless, `render --resource` (a14, a15, b2, b7, b16) | **ändra verktyg** (M2.5) | inga ABI-följder; ger guldbilder för paus, persistens, omflöde |
| 33 | Modultester och clippy i CI på båda plattformar (a13, fynd 7) | **ändra verktyg**: `tools/test_modules.sh` i båda jobben | fem nativa testsviter (18 tester) står utanför grindarna |
| 34 | Loggbrus headless (b18) | **ändra verktyg**: nivå Debug för "nekad i headless" | kosmetiskt |
| 35 | `now_ms` utan epok (b9) | **avslå** för 1.0 | fingeravtryck; `prefs` är en 1.1-möjliggörare (§16) |
| 36 | `text_caret` utan radbrytning (b19) | **avslå** som ABI-fråga; fixas i `vy-render` (M3) | M1-approximation, inte signatur |
| 37 | `dl_rect`-radie per hörn (a7) | **avslå** | två opkoder är acceptabelt |
