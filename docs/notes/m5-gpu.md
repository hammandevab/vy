# M5: rasterisering inom SPEC-13.2.1 – grenen `m5/gpu`

Byggagentens noteringar. Granskningssidan skrivs av en annan agent. Spåret äger `crates/vy-render`
(utom typsnittsladdningen i `text.rs`, som m5/release får komprimera) och presentationen i
`crates/vy-shell/src/window.rs`. Målet: SPEC-13.2.1 (2 000 opkoder ≤ 2 ms på referensmaskinen) i
fönsterklienten, utan att ge upp determinismen, konformanssviten (0,000 %), SPEC-12.3.1:s 25 MB
eller Windows.

## Sammanfattning

| Commit | Vad |
| --- | --- |
| `e1090cd` | designbeslutet med mätningar (storlekssonder, CPU-kostnader) |
| `19b02ca` | `vy_render::Renderer`: körningscache, torr pass, skadespårning, band på trådar; likvärdighetstester |
| `7b64370` | skalet ritar sidor med den snabba vägen i fönstret, referensen huvudlöst; `window.rs` presenterar bara det som ändrats; `spec_13_2_1_raster_budget` mäter fönstrets väg |
| `fd3a0d1` | `spec:` SPEC-11.3.2 förtydligad (byte för byte, konformansen alltid headless) och sidtestet i vy-cli |
| (detta) | noteringarna kompletta; informativa SPEC-13.2.1-steg i CI |

**Utfall.** wgpu ryms inte (+3,9–5,0 MB mot 2,63 MB kvar). Fönstret ritar nu med en CPU-väg som
ger referensens byte i varje frame, ritar om bara det som ändrats och delar en hel ram i band:
en hel ram 1,7 gånger snabbare här med två vCPU (P50 11,5 mot 19,5 ms), 3,4 gånger kortare kritisk
väg med fyra band räknat i instruktioner (≈ 4,4 ms på fyra kärnor som den här VM:ns), en delvis
ändrad ram ≈ 1 ms. **SPEC-13.2.1 hålls inte för en hel ram** och testet förblir ignorerat med talen;
den enda väg dit inom taket är en GPU-väg på rå OpenGL (+0,23 MB), spec-fråga 66. Konformans 59/59
à 0,000 % efter varje commit, inga guldbilder ändrade, `vy-shell` +63 208 byte.

**Grindar** (vid `fd3a0d1`, från worktreens rot): `cargo fmt --all --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --workspace` 375 gröna, 0 röda, 4 ignorerade
(main: 369 / 4; de fyra är desamma: `spec_13_2_1_raster_budget`, `spec_13_1_1_cold_start`,
`spec_12_3_1_size_budget`, `spec_9_7_1_reproducible_build`); `spec_trace.py` 140/140;
`build_modules.sh`, `check-abi target/wasm/hello.wasm`, `test_modules.sh` (12 moduler);
`vy-cli conformance` 59/59 à 0,000 %; `check-cross --workspace --all-targets` utan varningar. I
release dessutom `cargo test --release -p vy-render -- spec_13 spec_11_3_2` grönt (det CI:s
`bench` kör).

## Beslut (steg 1)

**(b): en snabbare CPU-väg i `vy-render`, byte för byte samma som referensen – bandparallell
rasterisering med `std::thread` och skadespårning mellan frames.** wgpu (a) ryms inte: den minsta
wgpu-konfigurationen lägger till 3,9 MB och den som Linux behöver 5,0 MB, mot 2,63 MB kvar under
taket utan tillgänglighetsfunktionen och ≈ −0,49 MB med den. Därmed byggs inte heller (c) med wgpu.
CPU-vägen når inte 2 ms för en hel ram på referensmaskinen (uppskattat 4–6 ms med fyra kärnor),
men den gör ramar som bara ändrar en del av sidan billiga, är den reserv varje GPU-väg ändå
behöver, och ändrar ingen guldbild. **SPEC-13.2.1 förblir ohållen** och testet ignorerat med
uppmätta värden. Den enda väg till 2 ms för en hel ram som ryms under taket är, enligt
storleksmätningen och tidsuppskattningen nedan, en GPU-väg på rå OpenGL (d, `glow` + `glutin`,
+0,23 MB) – det är ett byte av den teknik CLAUDE.md
och uppdraget nämner (wgpu) och därför ägarens beslut (spec-fråga 66 nedan), inte gjort här.

## Mätningar

**Maskinen.** Delad VM, 2 vCPU (Intel Xeon @ 2,1–2,3 GHz, AVX-512; värden byttes vid en omstart
mitt i arbetet), 7 GB, två andra byggagenter samtidigt (load 2–5 under mätningarna). **Ingen GPU.**
Vulkan-laddaren (`libvulkan1` 1.3.275) finns men ingen ICD (ingen lavapipe, `mesa-vulkan-drivers`
saknas); Mesa 25.2.8 med llvmpipe finns för GL/EGL (`swrast_dri.so`, `libEGL_mesa`), och Xvfb. GPU-tider
kan alltså inte mätas här; de måste komma från CI eller ägarens Windows-dator. Allt nedan är
release (`lto = "thin"`, `codegen-units = 1`), utan den ospårade `.cargo/config.toml`.

### Binärstorlek (SPEC-12.3.1)

Mätt med fristående sondbinärer utanför arbetsytan (byggkatalogen, inte incheckade), byggda med
arbetsytans releaseprofil och `strip`: `base` är winit 0.30 + softbuffer 0.4 med vy-shells
funktioner och ett fönster som presenterar en CPU-bild (så som `window.rs` i dag); de andra är
samma program plus en liten renderare med instansierade fyrhörningar, ett glyfatlas (R8-textur),
en shader med SDF för rundade hörn, saxklipp och uppläsning – det vy-render skulle behöva, så
att länkaren behåller koden.

| Sond | Byte (strippad) | Tillägg mot `base` | Crates (normala, unika) |
| --- | --- | --- | --- |
| `base` (winit + softbuffer) | 2 582 560 | – | 63 |
| + wgpu 30.0.1, Vulkan + GLES (Linux behöver båda) | 7 557 736 | **+4 975 176** | 108 (+45) |
| + wgpu, bara Vulkan | 6 902 360 | +4 319 800 | |
| + wgpu, bara GLES | 6 490 784 | +3 908 224 | |
| + `glow` 0.17 + `glutin` 0.32 (EGL, GLX) + `glutin-winit` 0.5 | 2 810 832 | **+228 272** | 68 (+5) |

`vy-shell` på main är 22 372 360 byte (mätt här, se *Storlek*; m4/integration mätte 22 322 112 vid
`3b1cbac`), så det finns **2 627 640 byte** kvar till 25 000 000; `a11y-linux` lägger till
3 112 952 byte (m4/integration), så med den är binären redan ≈ 485 000 byte över. wgpu får alltså
inte plats i någon konfiguration, inte ens utan tillgänglighetsfunktionen.

Var wgpu:s byte sitter (symbolstorlekar i den ostrippade sonden, `nm -S`, grupperade per crate;
filen växer mer än symbolerna, resten är rodata, avvecklingstabeller och relokeringar):

| Del | Byte |
| --- | --- |
| `wgpu-core` (validering, spårning, resurshantering) | 1 053 724 |
| `naga` totalt: IR och hjälp 522 987, SPIR-V ut 371 071, WGSL in 342 220, validering 286 241, GLSL ut 102 814 | 1 625 333 |
| `wgpu-hal` Vulkan + `ash` | 466 957 |
| `wgpu-hal` GLES + `glow` + `khronos-egl` | 254 227 |
| `wgpu-types`, `naga-types`, bryggan, `gpu-allocator`, `codespan-reporting`, övrigt hal | 197 249 |

Standardfunktionerna är `dx12, metal, gles, vulkan, wgsl, webgpu`; med `default-features = false`
och bara `std` + backend kvarstår ändå WGSL-frontend och `renderdoc`: wgpu 30 slår på
`wgpu-core/wgsl` och `wgpu-core/renderdoc` ovillkorligt utanför wasm, så `naga-ir` (IR byggd i
koden i stället för WGSL) sparar ingenting. Windows (DX12: `naga` HLSL ut i stället för SPIR-V och
GLSL, `windows`-bindningarna för D3D12/DXGI) och macOS (Metal: MSL ut, `objc2-metal`) kunde inte
länkas här (ingen MSVC-länkare eller macOS-SDK); med samma `wgpu-core` och `naga`-kärna
(≈ 2,6 MB symboler) är tillägget där av samma storleksordning, 3,5–4,5 MB, och skulle behöva mätas
i CI innan någon räknar med det.

### CPU-kostnader (frame_2000, 1280 × 800, varm textmotor, 100 körningar)

| Mätt | P50 | P95 | Anm. |
| --- | --- | --- | --- |
| `vy_render::render` (så som `spec_13_2_1_raster_budget` mätte före grenen) | 19,4–20,2 ms | 24–35 ms | load 2,7–5; granskning 0008 mätte 14,9 / 17,5 ms vid load 0,2–0,5 på förra värden |
| bara textens CPU-del: layout ur cachen och glyfbild ur cachen för varje glyf, inget ritat | 2,06 ms | 2,10 ms | 25 480 glyfer (81 ns per glyf), 1 724 427 px glyfrutor, 1 168 085 px med täckning |
| två renderingar samtidigt på två trådar | 23,9 ms per ram och tråd i genomströmning | | mot 22,2 ms med en tråd: med två andra agenter igång finns ingen ledig kärna, så parallellism kan inte mätas här |

Textens CPU-del (2,06 ms) är värd att lägga märke till: den är vad *varje* väg betalar per ram –
också en GPU-väg, som måste slå upp varje glyfs plats i atlaset – och den ensam är över 2 ms här.
En cache på körnivå (glyfernas platser och bild-id för en formad text vid ett givet bråkdelsläge
och en skala, återanvänd mellan frames) tar bort den för text som inte ändrats; (b) har en.

## Alternativen

Referensmaskinen (SPEC §13) är en bärbar dator från 2020 med integrerad grafik, till exempel
Core i5-1035G1/i7-1065G7 (4 kärnor, Iris Plus/UHD), Ryzen 5 4500U (6 kärnor) eller M1. Antagandet
nedan är att en sådan kärna under full last är ungefär lika snabb som en av den här VM:ns (snabbare
i turbo på en kärna, långsammare med alla fyra igång i ett tunt chassi).

**(a) wgpu med en egen liten pipeline.** Instansierade rundade rektanglar med SDF och saxklipp,
rundade klipp som SDF i fragmentshadern (eller en stencil per nivå), glyfatlas matat ur
`TextEngine`/swash, bilder som texturerade fyrhörningar, banor genom CPU-rasteriseringen till en
textur. *Tid:* CPU-sidan är körnivåcachen (≈ 0,15 ms för 820 körningar), 1 100 andra opkoder
(≈ 0,1 ms) och en överföring på ≈ 0,9 MB instansdata; GPU-sidan ≈ 3–4 miljoner fragment (rensning,
1,7 M px glyfrutor, kort och rader) vilket en Iris Plus gör på 0,3–0,6 ms. Uppskattat **0,5–1 ms**,
alltså inom budgeten – omätt här. *Storlek:* +3,9–5,0 MB. *Determinism:* GPU:ns kantutjämning och
blandning (unorm8 med avrundning per drivrutin) avviker från tiny-skia med några nivåer vid kanter
och ±1 i blandningar; kräver toleransen i SPEC-11.3.2 och en mjukvaruväg för konformansen.
**Faller på storleken.**

**(b) Snabbare CPU-väg: band per tråd och skadespårning.** *Tid för en hel ram:* idealt 20 ms / 4
kärnor ≈ 5 ms, i praktiken 4–6 ms med det arbete som dubbleras för opkoder som spänner över två band
– **över 2 ms**. *Tid för en delvis ändrad ram* (hovring, en räknare, en markör): proportionell mot
den ändrade ytan och de opkoder som når den, typiskt långt under 2 ms. *Storlek:* inga nya crates
(uppmätt efteråt: +63 208 byte). *Determinism:* byte för byte samma som referensen, se *Exakthet*
nedan; konformansen fortsätter mot samma guldbilder.

**(c) Kombinationen** (wgpu där det finns, (b) som reserv). Samma storlek som (a), alltså över
taket; som avstängd funktionsflagga skulle den bara bevisa något på en maskin som ändå inte får
leverera binären. Inte byggd.

**(d) Rå OpenGL** (GL 3.3 core / GLES 3.0 med `glow`, kontext via `glutin` över EGL/GLX på Linux,
WGL på Windows, CGL på macOS). Samma pipeline som (a) med GLSL skriven för hand (ingen
översättning, ingen `naga`), samma tid, samma toleransfråga, **+0,23 MB**. Nackdelar: OpenGL är
avskrivet på macOS (4.1, fungerar på M1 genom Metal men utan framtid), Windows-maskiner utan
leverantörsdrivrutin har bara GDI:s GL 1.1 (reserven tar över), och drivrutinsfel syns direkt i
stället för bakom wgpu:s validering. Enligt storleksmätningen och tidsuppskattningen den enda väg
som når 2 ms för en hel ram och ryms; spec-fråga 66.

| | Hel ram, referensmaskinen | Delvis ändrad ram | Storlek | Byte som referensen | Windows |
| --- | --- | --- | --- | --- | --- |
| (a) wgpu | ≈ 0,5–1 ms (omätt) | samma | **+3,9–5,0 MB, över taket** | nej, tolerans | DX12 |
| (b) band + skada | ≈ 4–6 ms | ≪ 2 ms | +63 kB | **ja** | ja |
| (c) a + b | som (a) | som (a) | som (a) | reserven ja | ja |
| (d) rå GL | ≈ 0,5–1 ms (omätt) | samma | +0,23 MB | nej, tolerans | WGL, med reserv |

## Exakthet i (b)

Guldbilderna är tiny-skias kantutjämnade fyllning, och den är inte translationsinvariant över en
klippkant: en kant som börjar ovanför klipprektangeln kapas där i flyttal och stegas därefter i
fixpunkt, så att rita samma bana i en mindre pixmap (ett band, en skaderektangel) kan ge en annan
täckning vid kanten än helbilden gav. Därför ritar varje bandtråd i en **egen pixmap av hela dukens
storlek**: varje tiny-skia-anrop görs med samma pixmapstorlek, samma bana, samma färg och samma mask
som i helbilden, så täckningen och blandningen är desamma, och tråden behåller bara sina egna rader.
Det som vy-render ritar själv (hela pixlar, glyfer, bilder, raka klipp) skärs till bandet direkt. En
opkod som inte når bandet hoppas över; en som spänner över två band ritas i båda (den dubblerade
kostnaden). Budgeten (SPEC-5.3.3) räknas en gång, i ordning, före ritningen, med exakt samma
debiteringar som `raster::render`; trådarna ritar sedan opkoderna fram till samma avbrottspunkt.
Skadespårningen är samma sak med en godtycklig rektangel i stället för ett band, ovanpå förra
framens pixlar.

## Den snabba vägen (steg 2)

`vy_render::Renderer` (`crates/vy-render/src/renderer.rs`) lever lika länge som sidan och har två
vägar: `RasterPath::Reference` (anropar `vy_render::render`, som förut) och `RasterPath::Fast`.
Skalet ger varje flik en: referensen huvudlöst (`vy-shell --screenshot`, konformanssvitens
skalfall, testerna), den snabba i fönstret. `vy-cli render` och `vy-cli conformance` anropar
`vy_render::render` direkt som förut. En frame på den snabba vägen:

1. **Textkörningar lösta en gång.** Varje `TEXT` blir placerade glyfer och index i en tabell över
   glyfbilder, sparade mellan frames under allt resultatet beror på – snitt, storlek,
   radbrytningsbredd, origo, skala och textens byte, alla jämförda exakt (flyttal på bitarna). En
   körning som ritas igen kostar ett uppslag i stället för layout och ett glyfuppslag per glyf
   (2 ms av de 20 i `frame_2000`). Cachen börjar om vid 16 384 bilder eller 262 144 glyfer, bara
   mellan frames.
2. **Torr pass.** `raster::dry_pass` kör referensens egen `Canvas` med ritningen avslagen: samma
   debiteringar i samma ordning (SPEC-5.3.3), alltså samma rapport och samma avbrottspunkt, och
   varje opkods *fotavtryck* – pixlarna den kan ändra (rektangelns täckande pixlar, banans gränser,
   för en strykning grown med 2 × bredden för geringsfogar och 2 px, glyfrutornas union, bildens
   rektangel, allt skuret med klippets gränser).
3. **Skada.** Listan jämförs med förra: opkoderna före den första och efter den sista skillnaden är
   samma opkoder (alla fält, flyttal på bitarna, textstilens innehåll) i samma klipp (kedjan av
   omslutande `CLIP_PUSH`, jämförd likadant) och i samma ordning. Utanför fotavtrycken av
   opkoderna däremellan, i båda listorna, når alltså samma opkoder varje pixel i samma ordning, och
   bara den rektangeln ritas. Ny storlek, skala, avbruten budget eller en ändring i bildcachen
   (`ImageCache::generation`, ny vid varje avkodning) ritar allt; en skada över halva duken också.
4. **Band.** Det som ritas delas i horisontella band, högst `MAX_BANDS` = 4 och högst så många som
   maskinen har trådar, minst 48 rader per band; `std::thread::scope`, en tråd per band utom det
   första. En hel ram ritar sitt första band direkt i utbilden; en skaderektangel ritas alltid i en
   skissduk, eftersom tiny-skia skriver också utanför regionen. Skissdukarna (en hel dukstorlek
   var, högst 64 MB tillsammans) delas av alla flikar i processen.

Ingen ny crate, inget `unsafe`. `raster.rs` fick tre saker som referensen kör med neutrala värden:
en region (hela duken), en torr läge (av) och en textkälla (`TextSource::Engine`, som förut);
`Clip` fick ett torrt läge som debiterar masker utan att bygga dem. `ImageCache::generation` och
`Glyph::key` (cosmic-texts `CacheKey`) är nya publika fält/metoder.

**Minne (SPEC-13.3.1).** Per flik: sidans senaste bild (fanns förut som `Tab::rendered`), förra
listans avkodade opkoder och fotavtryck (storleksordningen listan själv, ≤ 16 MB text enligt
SPEC-9.3.2) och körningscachen (högst 16 384 glyfbilder och 262 144 placerade glyfer, några MB i
värsta fall, för en vanlig sida under 1 MB). Skissdukarna är gemensamma för processen: högst fyra
dukar och högst 64 MB (vid 1280 × 800 är de 4 MB var). Ingen ändring mätt i RSS här; tio flikar
håller sig inom SPEC-13.3.1 med god marginal enligt räkningen, men det är inte uppmätt.

**Fallbacks.** Kan en tråd inte startas ritas bandet på den anropande tråden efter de andra; en
panik i en bandtråd förs vidare (som en panik i referensen). Ett fel i listan eller storleken ger
samma `RenderError` som referensen. Inget i den snabba vägen kan ge andra pixlar än referensen utan
att testerna nedan fäller det; det finns ingen GPU-väg att falla tillbaka från.

**Var den aktiva vägen syns.** Fliken loggar `[host] raster: fast software path, byte for byte
the reference: damage tracking, up to N bands on threads` (devtools, *Logg*), och fönstret skriver
`vy-shell: presentation: softbuffer, damage only; raster: …` på stderr när det öppnas.

## Likvärdighet (steg 3)

Den snabba vägen ska ge referensens byte, inte en tolerans, så testerna jämför byte för byte och
rapport för rapport (SPEC-11.3.2 förtydligad i ett `spec:`-commit: fönstret får rita om bara det
som ändrats och rita i band, men varje bild är headless-bilden byte för byte; konformanssviten
använder alltid headless-vägen).

| Test | Var | Visar |
| --- | --- | --- |
| `spec_11_3_2_bands_draw_what_the_reference_draws` | vy-render | 160 slumplistor (varje op-typ under nästlade raka och rundade klipp, fyra skalor, fyra storlekar) ritade hela i 1–5 band |
| `spec_11_3_2_damage_draws_what_the_reference_draws` | vy-render | 90 slumplistor som redigeras sju gånger var (opkoder omfärgade, flyttade, borttagna, tillagda, klipp flyttade, rundade och tillagda), ritade i följd av samma renderare i 1 eller 3 band; > 200 delvisa omritningar; samma lista igen ritar ingenting |
| `spec_13_2_1_fast_path_frame_and_budget` | vy-render | `frame_2000` i tre band; en knapp omfärgad ritar om 80 × 24 px; en lista som tar slut på budget får referensens rapport och ritas hel nästa gång; en avkodad bild ritar allt |
| `spec_11_3_2_reference_pages_through_both_paths` | vy-cli | alla tolv referenssidor i 640 × 400 @ 1 (ett band) och 420 × 300 @ 1,5 (tre band), första framen och ett manus av hovring, klick, rullning och tangenter med upp till fem animationsframes efter varje: 132 frames, 40 delvisa omritningar, 8 oförändrade, alla lika |
| `spec_11_3_2_reference_path_is_render` | vy-render | referensvägen är `render`; felen är desamma |
| `spec_13_2_2_present_converts_what_changed` | vy-shell | fönstrets omvandling och skaderektangel, buffertens ålder |

Testerna prövades mot planterade fel innan de godtogs: ett för litet fotavtryck för strykningar
fälls av tre av vy-render-testerna, en skadejämförelse som struntar i omslutande klipp fälls av
skadetestet (fall 71), och en skada som glömmer de gamla fotavtrycken fälls av sidtestet
(`document`, första klicket). Konformansen är 59/59 à 0,000 % efter varje commit och inga
guldbilder är ändrade.

## Mätt: den snabba vägen

`frame_2000` (1280 × 800, 2 000 opkoder, varm textmotor), release. Väggklocka, interfolierad A/B
(referensen och den snabba vägen varannan gång, 100 frames efter 20 uppvärmning), load 1,6–1,9:

| Väg | P50 | P95 |
| --- | --- | --- |
| referensen (`vy_render::render`) | 19,5–19,6 ms | 21,6–23,9 ms |
| snabb, hel ram, 1 band | 18,7 ms | 20,5 ms |
| snabb, hel ram, 2 band (= `spec_13_2_1_raster_budget` här) | 10,8–11,5 ms | 11,9–16,9 ms |
| snabb, hel ram, 4 band (två vCPU: översubskriberat) | 14,5 ms | 16,4 ms |
| snabb, en knapp omfärgad per frame (skada 80 × 24 px) | 1,0 ms | 1,2 ms |

Instruktioner per frame (callgrind, `--separate-threads=yes`, skillnaden mellan 5 och 1 frame,
alltså oberoende av lasten): referensen 229,8 M; snabb med 1 band 227,6 M; 2 band 125,6 M på den
längsta tråden (+6 % arbete totalt, det som dubbleras för opkoder över bandgränsen); 4 band 68,0 M
på den längsta (+17 % totalt); en omfärgad knapp 7,1 M (textuppslag, torr pass och 80 × 24 px).

**Mot referensmaskinen.** Den kritiska vägen med fyra band är 3,4 gånger kortare än referensen.
Referensen tog 14,9 ms P50 på en tyst kärna här (granskning 0008), så en hel ram blir ≈ 4,4 ms på
fyra sådana kärnor – **över 2 ms**. En ram som ändrar en knapp, en räknare eller en hovring tar
≈ 0,5–1 ms. SPEC-13.2.1 hålls alltså för delvisa ramar men inte för en hel; testet förblir
ignorerat med talen ovan och den här maskinens beskrivning, och budgeten är oförändrad.

## Fönstret (steg 5)

- **Sidan** ritas av flikens `Renderer` (snabba vägen) i `Shell::compose`; resten av
  komponeringen (chrome-cachen, överlägg, statusraden) är som förut.
- **Presentationen** (`window.rs`, `Presented`): varje frame jämförs rad för rad med den förra som
  visades; bara den ändrade rektangelns pixlar omvandlas till `0x00RRGGBB` och ges till
  `softbuffer` som skada (`present_with_damage`: Wayland, X11 med XShm, Win32). En buffert som
  kommer tillbaka äldre än en presentation (dubbelbuffring) får unionen av ändringarna sedan dess;
  en ny buffert (ålder 0, efter varje storleksändring) omvandlas hel. En frame utan ändring
  presenteras hel (för en expose).
- **Storlek och DPI.** `Resized` och `ScaleFactorChanged` går till `Shell::resize` som förut;
  den nya storleken eller skalan ger en ny buffert (ålder 0) och en hel ram i renderaren (ny
  storlek eller skala ritar allt). Inget annat ändrat.
- Skalets huvudlösa skärmbilder och alla vy-shell-tester går på referensen.

**Prövat i ett riktigt fönster** (release-`vy-shell` under Xvfb med MIT-SHM, alltså softbuffers
X11-väg med `present_with_damage`; fönstret skärmavbildat med ImageMagick, storleksändringar,
pekarrörelser och klick genom libX11/libXtst via ctypes, skript i byggkatalogen):

| Steg | Jämfört med | Avvikande pixlar |
| --- | --- | --- |
| `shop.wasm` öppnad, 1024 × 720 | `vy-shell --screenshot --size 1024x720` (referensen) | 0 |
| fönstret ändrat till 800 × 600, 1180 × 830, 640 × 480 | skärmbild i samma storlek | 0, 0, 0 |
| sju pekarrörelser över sidan, sedan ut ur fönstret | skärmbild 640 × 480 | 0 |
| `settings.wasm`, sex klick på reglage (124 968 px ändrade, ritade och presenterade som skada) | samma fönster efter en storleksändring bort och tillbaka (hel omritning och hel presentation) | 0 över hela skärmen 1400 × 1000 |

Det sista steget visar att det som presenterades bit för bit är exakt det en hel ram ger; det
första att fönstret visar referensens bild. DPI-ändringar kan inte prövas under Xvfb (skalan är 1);
den vägen är samma som en storleksändring i renderaren (ny skala ritar allt) och i presentationen
(ny buffert, ålder 0).

## Storlek (SPEC-12.3.1)

`cargo build --release -p vy-shell --locked --config 'profile.release.strip=true'` med repots
profil (`lto = "thin"`, `codegen-units = 1`), Linux x86_64, båda byggda här i samma målkatalog:

| Binär | Byte |
| --- | --- |
| `vy-shell`, main (`0fc0453`, byggd ur `git archive`) | 22 372 360 |
| `vy-shell`, grenen (`fd3a0d1`) | 22 435 568 |
| skillnad | **+63 208** |

Kvar under 25 000 000: 2 564 432 byte. `a11y-linux` (+3,1 MB, m4/ux) ryms fortfarande inte, med
eller utan den här grenen (main med funktionen var 25 435 064 i m4/integration). Windows och macOS
kunde inte länkas här.

## Nya beroenden

Inga. Banden går på `std::thread::scope` (ingen rayon: fyra trådar per frame, startade och
inväntade i samma funktion, behöver ingen arbetsstöld eller pool, och en tråd kostar ≈ 20 µs mot
en frame på millisekunder). wgpu och glow/glutin finns bara i de fristående storlekssonderna i
byggkatalogen, aldrig i arbetsytan.

## Vad ägaren ska kontrollera (Windows och macOS)

Allt nedan är kört på Linux; Windows och macOS är bara typkontrollerade (`check-cross`), och CI:s
`platform`-jobb kör testerna och, informativt, `spec_13_2_1_raster_budget` (`--ignored`, kan inte
fälla jobbet).

1. **Fönstret ritar som förut** på Windows: öppna `shop`, `dashboard` och `text`, hovra, klicka,
   rulla och ändra fönstrets storlek; dra fönstret mellan skärmar med olika skalning (DPI). Inget
   ska blinka, släpa efter eller lämna rester (skaderektangeln, `present_with_damage` på Win32).
2. **Loggen**: devtools → *Logg* ska visa `[host] raster: fast software path … up to N bands`,
   med N = min(4, logiska processorer).
3. **Tiden**: `cargo test --release -p vy-render --lib -- --ignored spec_13_2_1 --nocapture` på
   referensmaskinen skriver P95 och P50 för en hel ram (testet faller, 2 ms hålls inte; talet är
   underlaget för spec-fråga 66). `cargo run --release -p vy-cli -- bench --only raster` som förut
   för referensen.
4. **Storleken**: CI:s Windows- och macOS-binärer mot 25 MB (den här grenen lägger till ≈ 0
   byte, se nedan).

## Berör andra spår

- `crates/vy-shell/src/shell.rs` (m4/ux-filen, `Shell::compose`, `Tab`): `Tab` har
  `raster: vy_render::Renderer` och `rendered: Option<(u32, u32, f32)>` i stället för en egen
  bild; `Tab::new` tar vägen (`RasterPath::Reference` huvudlöst, `Fast` i fönstret, med en
  loggrad); i `compose` anropas `tab.raster.render` och bilden blittas ur `tab.raster.image()`.
  Ett 20-tal rader.
- `crates/vy-cli/src/lib.rs` och `crates/vy-cli/src/raster_tests.rs` (ny): sidtestet ovan; en
  `mod`-rad i `lib.rs`.
- `.github/workflows/ci.yml` (m5/release:s repro- och storleksjobb rörs inte): ett informativt
  steg i `bench` och i `platform` som skriver SPEC-13.2.1-talet för löparen.
- `SPEC.md`: SPEC-11.3.2 förtydligad (`spec:`-commit), inget i §5, bilaga A eller C.
- `crates/vy-render/src/text.rs`: bara `Glyph::key` (ett fält, satt i `rasterize`);
  typsnittsladdningen, som m5/release får komprimera, är orörd. Inget i den snabba vägen beror på
  hur typsnitten laddas.

## Kvar

1. **SPEC-13.2.1 för en hel ram** – spec-fråga 66 (GPU-väg på rå OpenGL, eller wgpu och ett högre
   tak, eller en ändrad mening). Den exakta CPU-vägen är reserven för vilken GPU-väg som helst.
2. **`vy-cli bench`** har inga rader för den snabba vägen (filen är m4/perf:s); `spec_13_2_1_raster_budget`
   och CI-stegen mäter den. Förslag: `raster.ops-2000-window` (`Renderer::render_full`) och
   `raster.ops-2000-damage` (en knapp omfärgad).
3. **Banden startar trådar per frame** (`std::thread::scope`, ≈ 20 µs per tråd). En pool skulle
   spara det, men inte märkbart mot en ram på millisekunder.
4. **Skalets egen komponering** (chrome-cachens klon av hela bilden, blittningen av sidan,
   överläggen) görs fortfarande hel varje frame i `Shell::compose` (m4/ux); några kopior av hela
   bilden per frame, omätt, och nästa ställe att spara på för delvisa ramar.
5. **DPI-byte i ett riktigt fönster** och allt på Windows och macOS är oprövat (se *Vad ägaren ska
   kontrollera*).

## Spec-frågor

66. **GPU-vägen: wgpu eller rå OpenGL, eller ett högre tak.** wgpu kostar 3,9–5,0 MB och ryms inte
    under SPEC-12.3.1 (2,63 MB kvar på main, ≈ −0,49 MB med `a11y-linux`). SPEC-13.2.1 hålls för en
    hel ram bara med en GPU-väg (uppskattning ovan; CPU-vägen ≈ 4,4 ms med fyra kärnor, räknat i
    instruktioner). Alternativ: (1) en
    GPU-väg på rå OpenGL (+0,23 MB, avskrivet på macOS); (2) wgpu och ett högre tak i SPEC-12.3.1;
    (3) SPEC-13.2.1 gäller ramar med begränsad ändring, eller CPU-vägen med en annan siffra. Spåret
    har byggt det som behövs i alla tre fallen (den exakta CPU-vägen och reserven) men ingen GPU-väg.
