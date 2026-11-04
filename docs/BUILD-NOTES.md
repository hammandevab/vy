# Byggnoteringar M1 – headless rendering

Skrivna av byggagenten för M1. Granskningssidan skrivs av en annan agent (`docs/reviews/`).

## Nya beroenden

Alla versioner är låsta i `Cargo.lock`. Antal unika crates i `vy-cli`:s normala beroendeträd: 164,
varav 100 kommer från wasmtime.

| Crate | Version | Varför | Varför inte 200 rader själv | Drar in (unika crates) |
| --- | --- | --- | --- | --- |
| `wasmtime` | 50 | Kör modulerna: cranelift-kompilering, bränsle (`consume_fuel`), minnestak via `StoreLimits`. Utan standardfunktioner: ingen komponentmodell, ingen async, ingen GC, inga trådar (SPEC-4.2.3), ingen cache, ingen parallell kompilering. | En wasm-kompilator är inte 200 rader; en tolk är förbjuden (SPEC-2.1.6). | 100 (cranelift, wasmparser, object, gimli m.fl.) |
| `sha2` | 0.11 | SHA-256 av modulen före laddning (SPEC-9.4.1). | Kryptoprimitiver skrivs inte själv; `digest`-ekosystemet är granskat. | 9 |
| `tiny-skia` | 0.12 | Rasterisering: fyllda/strukna banor, rundade rektanglar, antialias, klippmask. Utan `png-format`. | En korrekt AA-scanline-rasterizer med strokning är flera tusen rader. | 16 |
| `cosmic-text` | 0.19 | Textformning (harfrust), radbrytning, bidi, glyfrasterisering (swash). Utan `fontconfig` och standardfunktioner; endast `std` + `swash`. | OpenType-formning och radbrytning enligt UAX #14 är inte 200 rader. | 48 |
| `unicode-script` | 0.5 | Endast för att namnge `Script` i cosmic-texts `Fallback`-trait så att vi kan ge en plattformsoberoende fallback-lista. | Används inte själv; samma version som cosmic-text. | 0 extra |
| `png` | 0.18 | PNG-kodning av renderingar och avkodning av guldbilder. | zlib/deflate plus PNG-filter är mer än 200 rader; ren Rust. | 11 |
| `wat` | 1 (dev) | Testmoduler skrivs som WAT i `vy-runtime` och `vy-runtime::abi_check`. Fanns redan i `vy-cli`. | – | endast dev |

`anyhow` och `wasmparser` fanns sedan M0. `vy-runtime` använder en egen `Error`-enum, inte anyhow
(beroendet togs bort ur `vy-runtime/Cargo.toml` efter granskning 0001, fynd 6).

`rust-version` i workspace höjdes från 1.80 till 1.97: wasmtime 50 kräver 1.97 och cosmic-text 0.19
kräver 1.89. `rust-toolchain.toml` pekar på `stable`.

## Typsnitt

`crates/vy-render/fonts/` innehåller DejaVu Sans, DejaVu Sans Bold, DejaVu Serif och DejaVu Sans Mono
(Bitstream Vera-licens, kopierad till `fonts/LICENSE`). De bäddas in med `include_bytes!` (2,2 MB) och
är de enda typsnitt klienten någonsin laddar; systemtypsnitt rörs aldrig (SPEC-2.1.5, SPEC-6.4.2).

| Roll | Resurs-id | Typsnitt |
| --- | --- | --- |
| `sans` | −1 | DejaVu Sans (+ Bold för vikt ≥ 600) |
| `serif` | −2 | DejaVu Serif |
| `mono` | −3 | DejaVu Sans Mono |
| `ui` | −4 | DejaVu Sans |
| `emoji` | −5 | DejaVu Sans – riktig färgemoji kommer i 1.x |

Radhöjd är `size_px × 1.25` (`vy_render::text::LINE_HEIGHT_FACTOR`). Locale i textmotorn är fast
(`sv-SE`) och fallback-listan är de tre inbyggda familjerna i fast ordning, oberoende av plattform.

## Approximationer i M1

- **SHADOW** ritas som 8 koncentriska rundade rektanglar (`SHADOW_STEPS`) med alfa valt så att
  ackumulerad täckning växer linjärt från `+blur` till `−blur` runt kanten. Riktig gaussisk oskärpa
  kommer med GPU-vägen i M3.
- **IMAGE** ritas som en grå platshållare (`0x808080FF`). Bildavkodning (SPEC-6.5.1) är M1.5;
  testet `spec_6_5_1_image_decode_limits` är fortfarande ignorerat. `dl_image` skriver alltid
  `fit = 0` (cover) eftersom ABI-anropet saknar fit-parameter.
- **PATH** med `stroke_w = 0` fylls med nonzero-regeln; `> 0` strukes med butt-ändar.
- **Klippstacken** är rektanglar; masken byggs om när stacken ändras.
- `dl_text` layoutar texten en gång för höjden och rasteriseraren en gång till; ingen glyfcache
  utöver cosmic-texts egen. Prestandabudgeten (SPEC §13) mäts från M3.
- `text_caret` layoutar utan radbrytning.
- `max_w ≤ 0` i `dl_text`/`text_measure` betyder ingen radbrytning.

## Bränsle och logisk tid

`vy_runtime::FUEL_PER_MS = 1_000_000`. wasmtime debiterar ungefär en enhet per wasm-instruktion
och referensmaskinen (SPEC §13) klarar grovt 10⁹ enkla instruktioner per sekund, så 1 ms logisk tid
är 10⁶ enheter. Det är en konvention, inte en mätning: budgeten blir deterministisk över maskiner
(SPEC-2.1.5) på bekostnad av att vara ungefärlig på varje enskild.

- Mjuk budget per `vy_frame`: `FRAME_BUDGET_FUEL` = 8 × 10⁶ (SPEC-5.3.3).
- Hård gräns per anrop in i modulen: `CALL_HARD_LIMIT_FUEL` = 8 × budgeten = 64 ms logisk tid.
  Nås den avbryts anropet med `Error::OverBudget`.
- Första överskridandet i rad ger `KIND_BUDGET` med `c = 0`, det andra ger `KIND_BUDGET` med
  `c = 1` (sista varningen) följt av ett tvingande `vy_suspend`; därefter svarar `frame()` med
  `Error::Suspended`. En frame inom budget nollställer räknaren.

### Värdarbete debiteras (granskning 0001, fynd 1)

Bränslet mätte bara wasm-instruktioner: ett `text_measure` på 64 KB kostade några enheter men
hundratals millisekunder värd-CPU, och 65 536 sådana per frame var tillåtna. Nu debiteras varje byte
som värden formar, kodar, kopierar eller genererar åt modulen med `vy_runtime::FUEL_PER_HOST_BYTE`
= 32 ur samma bränsle som instruktionerna:

| Import | Debiteras |
| --- | --- |
| `dl_text` | textens byte (formning) + opkodens byte i display listen |
| `text_measure`, `text_caret` | textens byte |
| övriga `dl_*` | byte som skrevs till display listen (25 för `RECT`, 1 för `END`, …) |
| `log`, `sem_node`, `link_register`, `navigate` | byte som kopierades |
| `random` | byte som genererades |

Ett nekat eller avvisat anrop kostar inget. Värdet är en konvention som `FUEL_PER_MS`: lågt nog att
en full display list (65 536 × 25 byte ≈ 52 M enheter) ryms under den hårda gränsen (64 M), högt
nog att fyra 64 KB-formningar (4 × 65 535 × 32 > 8 M) bryter 8 ms-budgeten. Det underskattar
verklig formningskostnad; textcachen (SPEC-13.2.3, M3) gör räkningen rättvisare.

wasmtime kontrollerar bränslet bara vid funktionsingång och loophuvud, så en import som tömmer det
utlöser själv `Trap::OutOfFuel`; modulen avbryts då exakt som vid en wasm-instruktion utan bränsle
och `frame()` svarar `Error::OverBudget`. `FrameStats::host_fuel` visar värdens andel.

## Headless-klocka och slump

- `now_ms` returnerar `antal färdiga frames × 16` (`HEADLESS_FRAME_MS`); samma värde under hela
  framen, aldrig väggklocka.
- `random` fylls av xorshift64* med fast frö `HEADLESS_RANDOM_SEED`. Det är inte en CSPRNG
  (SPEC-5.2.11 säger CSPRNG); i headless-läge är determinism (SPEC-2.1.5) viktigare, och
  `vy-shell` (M3) byter källa.

## Spec-frågor

Inga normativa meningar har ändrats. Följande tolkningar gjordes och bör avgöras av specägaren:

1. **SPEC-4.2.1 / minnesexport.** "Exporterar exakt de sex funktionerna … och inget annat" – men
   värden måste läsa `(ptr, len)` ur linjärt minne, och SPEC-4.2.2 tillåter bara funktionsimporter,
   så minnet kan inte importeras. Tolkning: exakt sex *funktions*exporter plus en minnesexport med
   namnet `memory` (krävs av ABI-kontrollen). Specen bör säga det uttryckligen.
2. **SPEC-5.3.4 / startfunktion.** En wasm-startfunktion är en ingång utanför de sex exporterna och
   kör vid instansiering innan värden är redo. ABI-kontrollen avvisar moduler med `start`.
3. **SPEC-6.2.1 / första framen.** Meningen listar `request_frame`, event som returnerar 1 och
   fönsterändring, men inte den första framen efter `vy_init`. Tolkning: initieringen räknas som
   fönsterändring (från inget till `width × height`), så `wants_frame()` är sann direkt efter `init`.
   `vy-cli render --frames N` följer samma regel (granskning 0001, fynd 3): frame 1 körs alltid,
   därefter bara så länge modulen begärt en ny; `N` är ett tak, inte ett antal, och utskriften
   anger hur många som kördes.
4. **SPEC-5.3.3 / varning vid första överskridandet.** Meningen nämner bara det andra; eventtabellen
   har "1 = sista varningen" vilket förutsätter en tidigare. Tolkning som ovan: varning (`c = 0`)
   vid första, sista varning (`c = 1`) plus suspend vid andra.
5. **SPEC-5.2.6 / `ime` som capability.** Importerna `ime_begin`/`ime_end` saknar
   capability-markering i §5.2 men `ime` är en capability i SPEC-8.2.1. Tolkning: de kräver `ime`
   och nekas med `E_DENIED` i loggen annars.
6. **SPEC-5.2.10 / `navigate` headless.** Ingen capability i specen, men en headless-värd kan inte
   navigera. Tolkning: loggas som nekad i headless-läge; `vy-shell` (M3) kopplar in den.
7. **SPEC-5.2.3 / `dl_image` och `fit`.** Opkoden `IMAGE` har ett `fit`-fält som ABI-anropet inte kan
   sätta. Värden skriver 0. Antingen bör `dl_image` få en `fit`-parameter eller fältet utgå.
8. **SPEC-5.2.11 / CSPRNG headless.** Se ovan: headless använder seedad PRNG för determinism.
9. **SPEC-5.2.6 / `pointer_get`.** Packningen är inte specificerad; tolkning som `text_measure`:
   x × 100 i höga 32 bitarna, y × 100 i låga.
10. **SPEC-9.3.1 / bytetak – föreslagen SPEC-9.3.2.** Meningen räknar upp minne, CPU, nätverk och
    lagring men inte hur mycket värden får kopiera ur modulminnet per anrop; utan tak kunde en modul
    få värden att allokera hela sitt minne per `sem_node` (65 536 × 16 MB per frame) och display
    listen växa till 4 GB (granskning 0001, fynd 2). Värden sätter därför följande tak, namngivna i
    `vy_abi::limits`. Ett `len` över taket ger `E_ARG`, loggas med taket och kopierar ingenting;
    ett anrop som skulle fylla trädet eller frame-budgeten ger `E_DL_FULL`. Förslag till ny
    normativ mening, icke-normativ tills specägaren tar in den:

    > **[SPEC-9.3.2]** Värden kopierar högst dessa byte ur modulminnet per anrop: `log` 4 KB;
    > `dl_text`, `text_measure`, `text_caret` 65 535; `dl_path` 65 535 × 9; `sem_node` 4 KB;
    > `navigate`, `link_register` och `fetch_begin`-URL 2 KB; `fetch_begin`-kropp 1 MB;
    > `state_*`-nyckel 256 byte; `state_put`-värde 1 MB; `clipboard_write` 1 MB. Ett `len` över
    > taket ger `E_ARG` utan kopia. Per frame håller värden högst 65 536 semantiska noder och
    > högst 16 MB text i semantiskt träd och display list tillsammans; överskott ger `E_DL_FULL`.

    | Konstant | Värde | Varför just det |
    | --- | --- | --- |
    | `MAX_LOG_BYTES` | 4 096 | en loggrad |
    | `MAX_TEXT_BYTES` | 65 535 | `TEXT`-opkodens `len: u16` (SPEC-C.1.1); samma tak för mätning så att allt som mäts kan ritas |
    | `MAX_PATH_BYTES` | 65 535 × 9 | `PATH`-opkodens `n: u16` (SPEC-C.1.1, SPEC-C.1.2) |
    | `MAX_SEM_TEXT_BYTES` | 4 096 | ett tillgängligt namn eller textstycke |
    | `MAX_URL_BYTES` | 2 048 | vanlig URL-gräns |
    | `MAX_FETCH_BODY_BYTES` | 1 MB | större kroppar strömmas i 1.x |
    | `MAX_STATE_KEY_BYTES` / `MAX_STATE_VALUE_BYTES` | 256 / 1 MB | kvoten är högst 16 MB (SPEC-5.2.9) |
    | `MAX_CLIPBOARD_BYTES` | 1 MB | |
    | `MAX_SEM_NODES_PER_FRAME` | 65 536 | samma som opkoder (SPEC-6.1.6) |
    | `MAX_FRAME_TEXT_BYTES` | 16 MB | träd + display list; trädet lever till nästa `vy_frame`, listan till nästa `dl_begin` |

    Kapabilitetsgrinden (SPEC-5.3.1) kommer före taket: ett anrop utan `net`/`storage`/`clipboard`
    får `E_DENIED` oavsett längd. Länkar (`link_register`) är dessutom högst 4 096 per instans och
    loggen högst 10 000 rader (`vy_runtime::host`).
