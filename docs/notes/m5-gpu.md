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
| `1913b05` | noteringarna för del 1; informativa SPEC-13.2.1-steg i CI |
| `a74b90e` | del 2: GL-pipelinen i vy-render, kontexten i vy-shell, likvärdighetstestet `tests/gl.rs` |
| `b29c815` | fönstret presenterar med GL och faller tillbaka på CPU-vägen; skalets lagerläge |
| (följande) | GL-vägens CPU-arbete halverat två gånger och mätt; `spec:` toleransen i SPEC-11.3.2; CI-steget med Mesa under Xvfb; dessa noteringar |

**Utfall, del 1.** wgpu ryms inte (+3,9–5,0 MB mot 2,63 MB kvar). Fönstret ritar med en CPU-väg
som ger referensens byte i varje frame, ritar om bara det som ändrats och delar en hel ram i band:
en hel ram 1,7 gånger snabbare här med två vCPU (P50 11,5 mot 19,5 ms), 3,4 gånger kortare kritisk
väg med fyra band räknat i instruktioner (≈ 4,4 ms på fyra kärnor som den här VM:ns), en delvis
ändrad ram ≈ 1 ms; `vy-shell` +63 208 byte.

**Utfall, del 2** (spec-fråga 66 avgjord: OpenGL). GL-vägen ritar sidan på GPU:n (Linux och
Windows; macOS stannar på CPU-vägen) inom SPEC-11.3.2:s nya tolerans – högst 3 % av en bilds
pixlar mer än 2 nivåer från referensen och ingen pixel mer än 64 nivåer; uppmätt (efter
m5/fix-b) 0,32 % i värsta sidframen, 2,0 % i värsta slumplistan, största avvikelse 41 – och
faller tillbaka på CPU-vägen vid varje fel. Per hel frame: 26 209 instanser, ett
ritanrop, 1,3 MB, ≈ 1,6 ms CPU här. **SPEC-13.2.1 mäts på ägarens dator**; här finns bara
llvmpipe. Storlek: GL +247 624 byte; **med `a11y-linux` är `vy-shell` 25 790 640 byte, 790 640 över
taket**, som m5/release måste frigöra (main med `a11y-linux` är redan ≈ 480 000 över). Konformans
59/59 à 0,000 % efter varje commit, inga guldbilder ändrade.

**Grindar del 1** (vid `fd3a0d1`, från worktreens rot): `cargo fmt --all --check`; `cargo clippy
--workspace --all-targets -- -D warnings`; `cargo test --workspace` 375 gröna, 0 röda, 4 ignorerade
(main: 369 / 4; de fyra är desamma: `spec_13_2_1_raster_budget`, `spec_13_1_1_cold_start`,
`spec_12_3_1_size_budget`, `spec_9_7_1_reproducible_build`); `spec_trace.py` 140/140;
`build_modules.sh`, `check-abi target/wasm/hello.wasm`, `test_modules.sh` (12 moduler);
`vy-cli conformance` 59/59 à 0,000 %; `check-cross --workspace --all-targets` utan varningar. I
release dessutom `cargo test --release -p vy-render -- spec_13 spec_11_3_2` grönt (det CI:s
`bench` kör).

## Beslut (steg 1, del 1)

*Ersatt i del 2: spec-fråga 66 är avgjord och GL-vägen byggd (se Del 2 nedan).*

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

## Del 2: GL-vägen (spec-fråga 66, avgjord)

Koordinatorn, med ägarens mandat, avgjorde spec-fråga 66: en GPU-väg på OpenGL ovanpå CPU-vägen,
på samma gren. Kedjan är **GL → den snabba CPU-vägen → referensen**, vald när fönstret öppnas och
lämnad vid varje kontext- eller skuggningsfel, med raden i loggen och devtools.

**Kontext.** `glutin` 0.32 + `glutin-winit` 0.5: EGL på Linux (X11 och Wayland; GLX är inte
med), WGL på Windows; OpenGL 3.3 core, annars OpenGL ES 3.0. **macOS byggs utan GL** (glutin är
inte ett beroende där) och ritar med CPU-vägen: OpenGL är avskrivet på macOS, kan inte prövas här,
och glutins CGL kräver ett fönster även för testet. Mesa llvmpipe här ger OpenGL 4.5 core.
`VY_NO_GL=1` i miljön stänger av GL-vägen.

**Pipeline** (`crates/vy-render/src/gl.rs`, funktionen `gl`; beroendet `glow`). Samma upplösning
som CPU-vägen (`Renderer::resolve`: textkörningar och den torra passen med referensens rapport och
fotavtryck) och sedan en instans per rektangel, glyf, bild och bana, i listans ordning, ritade av
en skuggning i ett anrop (ett till per byte av bildtextur):

- rektanglar: exakt yttäckning för en rak, avstånd (SDF) för en rundad;
- klipp: en tabell (RGBA32F-textur) över framens `CLIP_PUSH` med förälder; skuggningen
  multiplicerar kedjans täckning som referensen multiplicerar masker – ett rakt klipp med samma
  heltalsavrundning som referensens `exact_pixels`, ett rundat med sitt avstånd;
- text: glyfbilderna CPU-vägen redan rasteriserat (swash genom `TextEngine`, typsnittsladdningen
  orörd) i en R8-atlas 2048 × 2048, hämtade texel för texel på referensens heltalspositioner;
  färgningen med referensens heltalsformel; atlasens platser sparas per körning;
- bilder: en textur per avkodad resurs (nyckel `ImageCache::id` + resurs-id), närmaste texel med
  referensens avbildning;
- banor: rasteriserade av tiny-skia på CPU:n till täckning och packade i en R8-textur per frame.

En frame vägen inte ritar (en färgglyf, en bild eller duk större än texturerna, en full atlas)
ritas av CPU-vägen och laddas upp; fönstret märker inget.

**Fönstret.** Skalet komponerar chrome och överlägg som förut men lämnar sidytan genomskinlig
(`Shell::set_page_layer`); överläggen blandas premultiplicerat, vilket på opaka pixlar ger samma
byte som förut (testat). Fönstret ritar sidan med GL i en framebuffer när listan, storleken,
skalan eller bildcachen ändrats (`Shell::page_layer`, `PageLayer::version`), lägger den i
sidytan och skalets bild över, och byter buffert. Budgetrapporten går till fliken som förut
(`Shell::page_drawn`).

**Var vägen syns.** stderr: `vy-shell: presentation: OpenGL; raster: OpenGL 4.5 (llvmpipe …), the
fast software path for what it does not draw and if it fails`; samma rad i varje fliks logg
(devtools, *Logg*), och en ny rad om vägen lämnas (`GL failed (…); raster: fast software path …`)
eller om den snabba CPU-vägen ger upp för referensen (en panik i den: `Renderer::fell_back`).

### Tolerans (SPEC-11.3.2)

GL-bilden kan inte bli referensens byte: kantutjämningen är analytisk (yttäckning och avstånd)
där tiny-skia översamplar 4 × 4, och blandningen sker i GPU:ns flyttal med drivrutinens
avrundning. `crates/vy-shell/tests/gl.rs` jämför pixel för pixel med Mesa llvmpipe:

| Indata | Frames | Pixlar med någon avvikelse | Pixlar > 2 nivåer | Värsta frame > 2 | Störst avvikelse |
| --- | --- | --- | --- | --- | --- |
| `frame_2000`, 1280 × 800 @ 1 och 2 | 4 | 10,3 % | 0,13 % | 0,15 % | 41 |
| 120 slumplistor (nästlade raka och rundade klipp, text, bilder, banor), tre skalor | 120 | 7,9 % | 0,69 % | 2,03 % | 37 |
| 45 listor med tunna strykningar (spetsiga trianglar fyllda och strukna, polylinjer; bredd 0,25–2,2), tre skalor | 45 | 6,8 % | 0,18 % | 0,89 % | 41 |
| alla nitton moduler i `target/wasm`, 800 × 600 @ 1 och 480 × 320 @ 2, manuset av hovring, klick, rullning, tangenter | 195 | 1,3 % | 0,07 % | 0,32 % (`block` 480 × 320 @ 2) | 40 |
| fönstrets komposition (`GlContext::compose`), sidytan i åtta lägen | 8 | 1,3 % | 0,31 % | – | 22 |

Talen är från `m5/fix-b` (granskning 0011:4). Vid `3fbcf36`, som granskningen mätte, var sidraden
195 frames, 0,095 %, värsta frame 0,387 % (`rymd` 800 × 600) och **största avvikelse 133**. De
133 nivåerna var inget kantbrus utan ett fel: GL-vägen fyllde konturen av en strykning där
referensen ritar en linje på högst en pixels bredd som en hårlinje utan fogar, så geringsfogen
stack ut en pixel bortom varje spetsigt hörn (`rymd` stryker sina trianglar med 1 px). En lista
med många tunna linjer låg före rättningen på upp till 8,9 % pixlar över 2 nivåer, alltså utanför
toleransen; ingen referenssida hade nog många. Rättat i `62ceaa5` (banor stryks med referensens
eget anrop), se `notes/m5-fix-b.md`.

Avvikelserna över 2 sitter i kantpixlar (rektanglar på bråkdelspositioner, rundade hörn); de små
(±1) i blandningen. Toleransen i SPEC-11.3.2 (två `spec:`-commits) är därför: **högst 3 % av en
bilds pixlar får ha någon kanal som avviker mer än 2 nivåer från headless-bilden, och ingen pixel
får avvika mer än 64 nivåer**, och testet hävdar båda för varje frame i varje klass. Den andra
halvan finns för att andelen ensam släpper igenom ett grovt fel på några få pixlar (som de 133):
en kant ritad med yta eller avstånd i stället för översampling avviker med en del av en pixels
täckning, uppmätt högst 41 nivåer; mer än en fjärdedel av skalan är en form ritad där referensen
inte ritar någon. Marginalen mot slumplistornas 2,03 % är 1,5 gånger och mot 41 nivåer 1,6
gånger, allt på en enda mjukvarudrivrutin (llvmpipe): **kör testet på en riktig GPU** (punkt 1
under *Vad ägaren ska kontrollera*). Konformanssviten använder alltid headless-vägen.

### Mätt: GL-vägen

llvmpipe är ingen GPU: att en frame tar 95–180 ms till `glFinish` här säger bara att Mesa
rasteriserar i mjukvara på två belastade vCPU:er. Det som säger något är vad som skickas och vad
CPU:n gör (release, `frame_2000` 1280 × 800, load 2–3,5, verktyget i byggkatalogen):

| Mått | Värde |
| --- | --- |
| instanser per frame | 26 209 (en per glyf, rektangel, bild och bana) |
| ritanrop per frame | 1 (en bildresurs) |
| byte till GPU:n per frame | 1 294 508 (instanser à 44 byte, klipptabell, banor; glyfer bara nya) |
| CPU: upplösning och torr pass (`resolve`) | ≈ 0,9 ms |
| CPU: instanser, klipptabell, banor (`GlStats::build`) | P50 0,64 ms (var 2,0 ms före platserna per körning och de packade instanserna) |
| CPU till sista GL-anropet med llvmpipe | ≈ 10–11 ms, varav ≈ 8,5 ms är llvmpipes egen hörnbearbetning inne i anropet |

Vårt CPU-arbete per hel frame är alltså ≈ 1,6 ms här; på en GPU tillkommer drivrutinens kopia av
1,3 MB och anropet. Om 2 ms hålls på referensmaskinen beror på den kärnans hastighet och GPU:ns
tid för ≈ 26 000 små fyrhörningar – **mät på ägarens dator** (se nedan).
`spec_13_2_1_raster_budget` (CPU-vägen) förblir ignorerat med sina tal; GL-testet skriver
GL-vägens tal i release och hävdar 2 ms bara med `VY_GL_BUDGET` satt.

### Storlek med GL

| Binär (strippad, Linux x86_64) | Byte |
| --- | --- |
| `vy-shell`, main | 22 372 360 |
| `vy-shell`, del 1 (CPU-vägen, `1913b05`) | 22 435 568 |
| `vy-shell`, del 2 (GL som standard) | 22 683 192 |
| `vy-shell --features a11y-linux`, del 2 | **25 790 640** |

GL-vägen kostar 247 624 byte; hela grenen 310 832. **Med `a11y-linux` är binären 790 640 byte över
25 000 000** – main med `a11y-linux` är redan ≈ 480 000 över (3 107 448 för funktionen), så m5/release
måste frigöra minst 790 640 byte för att båda ska rymmas. GL-delen är mest tabeller och strängar
(`glow`:s symboler 27 kB, glutins EGL-del 22 kB i sonden); att byta ut en crate sparar tiotals kB,
inte det som saknas.

### Nya beroenden (del 2)

| Crate | Vad | Varför inte 200 rader | Drar in |
| --- | --- | --- | --- |
| `glow` 0.17 | OpenGL-funktionerna laddade ur kontexten, typade anrop, GL och GLES | ≈ 45 funktioner och deras konstanter går att skriva för hand, men laddningen per plattform, skillnaderna GL/GLES och versionstolkningen är det glow redan gör rätt; wgpu använder samma crate | bara `bitflags`, som fanns |
| `glutin` 0.32 (EGL, x11, wayland, WGL) | kontexten: EGL-display och konfiguration, X11-visual som måste väljas före fönstret, Wayland-EGL-ytan, WGL med `wglCreateContextAttribsARB` (kräver en tillfällig kontext) | tusentals rader per plattform | `glutin_egl_sys` (Linux), `glutin_wgl_sys` (Windows); vid bygget `gl_generator`, `khronos_api`, `xml-rs` (genererar bindningarna, inte i binären); `libloading`, `once_cell`, `raw-window-handle`, `x11-dl`, `wayland-sys` och på Windows `windows-sys` 0.52 fanns redan |
| `glutin-winit` 0.5 | ordningen fönster/konfiguration för winit | litet, men att göra rätt för X11 och WGL kräver glutins inre | inget nytt |

Lockfilen får nio crates (`cgl` står där för glutins macOS-gren men byggs inte).

## Nya beroenden

Inga. Banden går på `std::thread::scope` (ingen rayon: fyra trådar per frame, startade och
inväntade i samma funktion, behöver ingen arbetsstöld eller pool, och en tråd kostar ≈ 20 µs mot
en frame på millisekunder). wgpu och glow/glutin finns bara i de fristående storlekssonderna i
byggkatalogen, aldrig i arbetsytan.

## Vad ägaren ska kontrollera (Windows och macOS)

Allt nedan är kört på Linux (GL med Mesa llvmpipe under Xvfb); Windows och macOS är bara
typkontrollerade (`check-cross`). CI:s `platform`-jobb kör testerna – `tests/gl.rs` försöker där
och hoppar över med en rad på stderr om ingen GL 3.3-kontext går att få (troligt på Windows-löparna
utan GPU; macOS har ingen GL-väg) – och skriver informativt CPU-vägens SPEC-13.2.1-tal.

1. **SPEC-13.2.1 – mät på din dator.** Med en skärm och ett GPU-drivet OpenGL:
   `tools/build_modules.sh`, sedan
   `VY_GL_REQUIRED=1 VY_GL_BUDGET=1 cargo test --release -p vy-shell --test gl`. Testet skriver
   GL-vägens P50/P95 för `frame_2000` till `glFinish`, instanser, ritanrop och byte, och faller om
   P95 är över 2 ms. Raden `GL:` säger vilken kontext det blev (OpenGL-version och GPU). Under
   2 ms: `spec_13_2_1_raster_budget` kan göras om till att mäta GL-vägen där en kontext finns och
   av-ignoreras (en uppföljning; det vy-render-test som finns mäter CPU-vägen).
2. **Fönstret** på Windows: `vy-shell` ska skriva `presentation: OpenGL; raster: OpenGL …` på
   stderr och samma rad i devtools → *Logg*. Öppna `shop`, `dashboard`, `text`, `settings`; hovra,
   klicka, rulla, ändra fönstrets storlek, dra det mellan skärmar med olika skalning (DPI); inget
   ska blinka eller lämna rester. `VY_NO_GL=1` ger CPU-vägen (`presentation: softbuffer`) att
   jämföra med.
3. **Reserven**: med en drivrutin utan OpenGL 3.3 (eller `VY_NO_GL=1`) ska fönstret öppnas och rita
   med CPU-vägen och säga varför på stderr.
4. **macOS**: CPU-vägen, `presentation: softbuffer, damage only`.
5. **Storleken**: CI:s Windows- och macOS-binärer mot 25 MB.

## Berör andra spår

- `crates/vy-shell/src/shell.rs` (m4/ux-filen): del 1 – `Tab` har `raster: vy_render::Renderer`
  och `rendered: Option<(u32, u32, f32)>`, `Tab::new` tar vägen; del 2 – lagerläget
  (`set_page_layer`, `page_layer`, `page_drawn`, `LayerPlace`, `PageLayer`), `set_raster_path`
  och en loggrad när den snabba vägen ger upp. Omkring 200 rader.
- `crates/vy-shell/src/compose.rs`: `clear` och premultiplicerad blandning i `fill_blend`/`blend`
  (samma byte på opaka pixlar; test).
- `crates/vy-shell/Cargo.toml`: funktionen `gl` (standard), `glow`, och `glutin`/`glutin-winit`
  utom på macOS; testet `gl` utan harness. `crates/vy-shell/src/gl.rs` är ny (kontexten).
- `Cargo.toml` (arbetsytan): `glow`, `glutin`, `glutin-winit`; `Cargo.lock` nio crates.
- `crates/vy-cli/src/lib.rs` och `crates/vy-cli/src/raster_tests.rs` (ny): sidtestet för CPU-vägen.
- `.github/workflows/ci.yml` (m5/release:s repro- och storleksjobb rörs inte): i `gates` Mesa och
  Xvfb och GL-testet med `VY_GL_REQUIRED=1`; informativa SPEC-13.2.1-steg i `bench` och `platform`.
- `SPEC.md`: SPEC-11.3.2 två gånger (`spec:`-commits): byte för byte för CPU-vägen, toleransen för
  GPU-vägen; inget i §5, bilaga A eller C.
- `crates/vy-render/src/text.rs`: bara `Glyph::key`; typsnittsladdningen är orörd.
- **Till m5/release:** `vy-shell --features a11y-linux` är 25 790 640 byte med den här grenen; 790 640
  måste bort för att den ska rymmas (main: ≈ 480 000). GL-vägen kan stängas av med
  `--no-default-features --features window` om det behövs medan utrymmet saknas (−247 624 byte).

## Kvar

1. **SPEC-13.2.1** mäts på ägarens dator (punkt 1 ovan). Den torra passen (≈ 0,6–0,8 ms här) är nu
   GL-vägens största CPU-post; den bygger banor och flyttal som GL-vägen inte behöver och kan
   förenklas för en frame som bara ska till GPU:n.
2. **Färgglyfer och bilder större än GPU:ns texturer** ritas på CPU:n för den framen (ingen
   sådan i referenssidorna; DejaVu har inga färgglyfer).
3. **`vy-cli bench`** har inga rader för fönstrets vägar (filen är m4/perf:s).
4. **Banden startar trådar per frame**; en pool sparar ≈ 20 µs per tråd.
5. **Skalets komponering** görs hel varje frame i `Shell::compose`; i GL-läget laddas bara de
   ändrade raderna upp.
6. **DPI-byte i ett riktigt fönster** och allt på Windows och macOS är oprövat här.

## Spec-frågor

66. *Avgjord* (koordinatorn, med ägarens mandat): GPU-vägen på OpenGL ovanpå CPU-vägen, toleransen
    i SPEC-11.3.2 uppmätt. Kvar för ägaren: storleken med `a11y-linux` (ovan) och mätningen på
    referensmaskinen.
