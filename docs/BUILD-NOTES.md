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

`anyhow` och `wasmparser` fanns sedan M0. `vy-runtime` använder en egen `Error`-enum, inte anyhow.

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
