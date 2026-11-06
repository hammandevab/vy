# M5: rasterisering inom SPEC-13.2.1 – grenen `m5/gpu`

Byggagentens noteringar. Granskningssidan skrivs av en annan agent. Spåret äger `crates/vy-render`
(utom typsnittsladdningen i `text.rs`, som m5/release får komprimera) och presentationen i
`crates/vy-shell/src/window.rs`. Målet: SPEC-13.2.1 (2 000 opkoder ≤ 2 ms på referensmaskinen) i
fönsterklienten, utan att ge upp determinismen, konformanssviten (0,000 %), SPEC-12.3.1:s 25 MB
eller Windows.

## Beslut (steg 1)

**(b): en snabbare CPU-väg i `vy-render`, byte för byte samma som referensen – bandparallell
rasterisering med `std::thread` och skadespårning mellan frames.** wgpu (a) ryms inte: den minsta
wgpu-konfigurationen lägger till 3,9 MB och den som Linux behöver 5,0 MB, mot 2,68 MB kvar under
taket utan tillgänglighetsfunktionen och −0,44 MB med den. Därmed byggs inte heller (c) med wgpu.
CPU-vägen når inte 2 ms för en hel ram på referensmaskinen (uppskattat 4–6 ms med fyra kärnor),
men den gör ramar som bara ändrar en del av sidan billiga, är den reserv varje GPU-väg ändå
behöver, och ändrar ingen guldbild. **SPEC-13.2.1 förblir ohållen** och testet ignorerat med
uppmätta värden. Den enda väg till 2 ms för en hel ram inom taket som mätningarna visar är en
GPU-väg på rå OpenGL (d, `glow` + `glutin`, +0,23 MB) – det är ett byte av den teknik CLAUDE.md
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

`vy-shell` är 22 322 112 byte (m4/integration, samma huvudcommit), så det finns **2 677 888 byte**
kvar till 25 000 000; med `a11y-linux` (25 435 064) är det redan 435 064 byte över. wgpu får
alltså inte plats i någon konfiguration, inte ens utan tillgänglighetsfunktionen.

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
| `vy_render::render` i dag (= `spec_13_2_1_raster_budget`) | 19,4–20,2 ms | 24–35 ms | load 2,7–5; granskning 0008 mätte 14,9 / 17,5 ms vid load 0,2–0,5 på förra värden |
| bara textens CPU-del: layout ur cachen och glyfbild ur cachen för varje glyf, inget ritat | 2,06 ms | 2,10 ms | 25 480 glyfer (81 ns per glyf), 1 724 427 px glyfrutor, 1 168 085 px med täckning |
| två renderingar samtidigt på två trådar | 23,9 ms per ram och tråd i genomströmning | | mot 22,2 ms med en tråd: med två andra agenter igång finns ingen ledig kärna, så parallellism kan inte mätas här |

Textens CPU-del (2,06 ms) är värd att lägga märke till: den är vad *varje* väg betalar per ram –
också en GPU-väg, som måste slå upp varje glyfs plats i atlaset – och den ensam är över 2 ms här.
En cache på körnivå (glyfernas platser och bild-id för en formad text vid ett givet bråkdelsläge
och en skala, återanvänd mellan frames) tar bort den för text som inte ändrats; det byggs i (b).

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
den ändrade ytan och de opkoder som når den, typiskt långt under 2 ms. *Storlek:* inga nya crates,
några kB. *Determinism:* byte för byte samma som referensen, se *Exakthet* nedan; konformansen kan
fortsätta mot samma guldbilder och den här vägen kan till och med köras i sviten.

**(c) Kombinationen** (wgpu där det finns, (b) som reserv). Samma storlek som (a), alltså över
taket; som avstängd funktionsflagga skulle den bara bevisa något på en maskin som ändå inte får
leverera binären. Inte byggd.

**(d) Rå OpenGL** (GL 3.3 core / GLES 3.0 med `glow`, kontext via `glutin` över EGL/GLX på Linux,
WGL på Windows, CGL på macOS). Samma pipeline som (a) med GLSL skriven för hand (ingen
översättning, ingen `naga`), samma tid, samma toleransfråga, **+0,23 MB**. Nackdelar: OpenGL är
avskrivet på macOS (4.1, fungerar på M1 genom Metal men utan framtid), Windows-maskiner utan
leverantörsdrivrutin har bara GDI:s GL 1.1 (reserven tar över), och drivrutinsfel syns direkt i
stället för bakom wgpu:s validering. Den enda väg i mätningarna som når 2 ms för en hel ram och ryms;
spec-fråga 66.

| | Hel ram, referensmaskinen | Delvis ändrad ram | Storlek | Byte som referensen | Windows |
| --- | --- | --- | --- | --- | --- |
| (a) wgpu | ≈ 0,5–1 ms (omätt) | samma | **+3,9–5,0 MB, över taket** | nej, tolerans | DX12 |
| (b) band + skada | ≈ 4–6 ms | ≪ 2 ms | ≈ 0 | **ja** | ja |
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

## Spec-frågor

66. **GPU-vägen: wgpu eller rå OpenGL, eller ett högre tak.** wgpu kostar 3,9–5,0 MB och ryms inte
    under SPEC-12.3.1 (2,68 MB kvar, −0,44 MB med `a11y-linux`). SPEC-13.2.1 hålls för en hel ram
    bara med en GPU-väg (uppskattning ovan; CPU-vägen ≈ 4–6 ms med fyra kärnor). Alternativ: (1) en
    GPU-väg på rå OpenGL (+0,23 MB, avskrivet på macOS); (2) wgpu och ett högre tak i SPEC-12.3.1;
    (3) SPEC-13.2.1 gäller ramar med begränsad ändring, eller CPU-vägen med en annan siffra. Spåret
    har byggt det som behövs i alla tre fallen (den exakta CPU-vägen och reserven) men ingen GPU-väg.
