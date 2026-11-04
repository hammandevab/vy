# PR #1 – M1: headless rendering (`01fc0e4..4f70a84`)

**Vad ändrades.** `vy-runtime` kör moduler i wasmtime med bränslebudget, minnestak och alla 33
importer; `vy-render` rasteriserar display listen i mjukvara med inbäddade DejaVu-typsnitt;
`vy-cli render`/`conformance` ger PNG och pixeldiff mot fyra guldbilder. 5 800 rader, 164 crates.

**Spec-id som berörs.** SPEC-2.1.5, 4.2.*, 5.1.*, 5.2.*, 5.3.1–5.3.4, 6.1.*–6.4.*, 9.4.1,
10.1.1, 10.2.3, 11.3.1–11.3.2, 12.2.2, 13.1.4, C.1.*. Nio tolkningar i `docs/BUILD-NOTES.md`.

**Grindar (körda av granskaren).** fmt, clippy, `cargo test --workspace` (74 gröna, 50 ignorerade),
spec_trace 125/125, ABI-kontroll, konformans 4/4 à 0,000 %, modul-clippy: alla gröna, före och
efter fixarna nedan. **Fuzz** (tillfällig harness, raderad): `check_abi_bytes` + `Runtime::load`
~106 000 inmatningar (slump, trunkerad och bitflippad `hello.wasm`, ~2 000 laddade instanser körda
genom alla sex exporter): 0 panics. 26 importer med slumpmässiga pekare/längder/floats:
5,8 M anrop, 0 panics. `vy_dl::decode`: 0 panics – men **render av det som avkodades gav 3 687
panics på 100 s** i fem klasser (se Bevis). Efter fix: 2,7 M display lists, 0 panics.

**Risk.** Rasteriseraren och textmotorn kraschade på modulstyrd indata: `"\u{2029}א"` i ett
`dl_text` är en hård `assert_eq!` i cosmic-text även i release – i M3 är det en dödad flik per
sida som vill. RTL-text utan radbrytning hamnade på x ≈ 3,4·10³⁸ (fel pixlar, inte bara fuzz).
Determinism: motorn körde med relaxed SIMD på och NaN-kanonisering av, så samma modul kunde ge
olika display list på x86 och ARM (SPEC-2.1.5) – verifierat: `inf − inf` gav `0xFFC00000` här.

**Bevis.** Un-ignorerade: 5.2.4, 5.3.1, 5.3.3, 5.3.4, 6.2.1, 4.2.4, 9.4.1, 10.1.1, 10.2.3, 5.2.11,
6.3.1, 6.4.1–6.4.3, 11.3.1, 12.2.2, 13.1.4, C.1.2. Testerna är verkliga (pixlar, bränsle, logg),
med två undantag: `spec_13_1_4_compile_time` (`vy-cli/src/lib.rs:244`) kontrollerar < 2 s debug
mot specens 60 ms/0 ms-cache – backlogposten försvann i stället för att förbli ignorerad;
`spec_2_1_5_determinism` kör två gånger i samma process och bevisar inget över plattformar.
Guldbilder: fyra nya, oförändrade av fixarna. Fixade i egna commits (`8cbbeba`, `73600d0`), varje
med test som föll före: relaxed SIMD av + kanonisk NaN (`vy-runtime/src/lib.rs:170`, ABI-kontrollen
samma featuremängd); bidi-separatorer → `\n`, obegränsad buffert för RTL, `MAX_COORD_PX` /
`MAX_MAGNITUDE` – opkoder med NaN/inf/>2²⁴ hoppas över (`vy-render/src/text.rs:26`, `raster.rs:30`).

**Kvarstående fynd (ej fixade, rapporteras).**
1. *Budgeten mäter bara wasm-bränsle.* Värdarbete är gratis: ett `text_measure` på 65 KB kostar
   200–400 ms CPU (debug) för några bränsleenheter; 65 536 per frame är tillåtet
   (`host.rs:306,398`, `lib.rs:302`). SPEC-5.3.3/9.3.1 kringgås. Debitera bränsle per byte.
2. *Obegränsade kopior ur modulminnet.* `string()` (`host.rs:228`) kopierar upp till hela minnet;
   `sem_node` 65 536 × 16 MB per frame (`host.rs:639`), `log` 10 000 × (`host.rs:142`), `link_register`
   4 096 ×, display listen 65 536 × 64 KB ≈ 4 GB (`vy-dl/src/lib.rs:253`). Värd-OOM från en modul.
   Sätt bytetak per anrop och per frame.
3. *`render --frames N` anropar `vy_frame` ovillkorligt* (`vy-cli/src/lib.rs:108`) trots
   `wants_frame()` – bryter SPEC-6.2.1 utanför de nio listade tolkningarna.
4. *ABI-kontrollen ignorerar icke-funktionsexporter* (`abi_check.rs:193`): tabeller/globaler/extra
   minnen passerar "inget annat" (SPEC-4.2.1). Tre `wasmparser`-versioner i trädet (0.220 i
   kontrollen, 0.261 i wasmtime) – samma modul kan valideras olika; följ wasmtime:s version.
5. *M1-grinden är inte uppfylld:* 4 guldbilder, CI bara `ubuntu-latest` (`ci.yml:15`), ingen macOS.
   `tiny-skia` med `simd` (`Cargo.toml:35`) är oprövat plattformsoberoende; 2²⁴-gränsen ovan antar
   `scale ≤ 8`, vilket bara CLI:t kräver.
6. Minnesväxt per instans: `SwashCache` utan tak (`text.rs:138`), klippmask i full storlek per
   push/pop (`raster.rs:154`); `anyhow` deklarerad men oanvänd i `vy-runtime/Cargo.toml:13`.

**Nya beroenden.** `wasmtime 50`, `sha2`, `tiny-skia`, `cosmic-text`, `unicode-script`, `png`
(+`wat` dev) – alla motiverade i BUILD-NOTES; versioner låsta. `rust-version` 1.80 → 1.97.

**Hotbild.** Inga nya ABI-importer. Sandboxen håller mot fuzz; alla fynd är tillgänglighet
(värd-CPU/minne) och determinism, inga läckor förbi de 33 importerna eller minnesgränserna.

**Rekommendation.** Merge efter fynd 1–2 (bytetak och värdkostnad i bränslet) och att
`render --frames` respekterar `wants_frame()`. Fynd 4–6 kan gå som M1.5/M3-poster. Fixarna
`8cbbeba`/`73600d0` ingår redan på `main` i den här granskningen.
