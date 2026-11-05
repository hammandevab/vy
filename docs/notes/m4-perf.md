# M4: prestanda (SPEC §13) – grenen `m4/perf`

Byggagentens noteringar. Granskningssidan skrivs av en annan agent. Spåret äger `crates/vy-render`,
`crates/vy-runtime` (prestandavägar, kompileringscachen), `crates/vy-cli` (bench) och
`crates/vy-publish` (`--allow-slow`). Målet: §13 mätbart, hållet där maskinen tillåter, med
identiska pixlar (konformans 0,000 %).

## Mätmetod

- **`vy-cli bench`** mäter P95 och P50 över `--runs N` körningar (standard 100, som §13 säger),
  `--only <prefix>` väljer rader på id, `--json` skriver en maskinläsbar rapport (`id`, `spec`,
  `unit`, `budget`, `p50`, `p95`, `n`, profil). Tabellen markerar `ÖVER` när P95 > budget.
- **Rader** (id i hakparentes):
  - kompilering per referensmodul, `Runtime::load` med hash och ABI-kontroll [`compile.<modul>`];
  - rasterisering 1280 × 800 (SPEC-13.2.1): ramen i sidornas proportioner (`vy_render::bench::frame_2000`,
    2 000 opkoder räknade ur konformansfallens listor: 41 % text à ~30 byte, 19 % opaka
    rektanglar, 19 % opaka rundade, 8 % klipp med pop, 2 % bilder, 2 % genomskinliga, 0,4 %
    banor) varm [`raster.ops-2000`] och med ny textmotor per körning [`raster.ops-2000-cold`, ingen
    budget]; M3:s patologiska listor (granskning 0004 mätte dem) [`raster.rects-2000`,
    `raster.text-200`, `raster.mix-1800-200`]; vy-render-testets 2 000 rundade kort med push,
    rektangel och pop (6 000 opkoder, ingen budget) [`raster.cards-2000`];
  - varje referenssidas första konformansfall, kört till sista framen och rasteriserat
    [`raster.page.<modul>`];
  - rullning av `text` 1280 × 800: SCROLL 24 px, frame och rasterisering per steg med en egen
    textmotor på rasteriserande sidan som skalets huvudprocess [`scroll.text.frame`, budget 16,7 ms],
    och textcachens missar i procent på båda sidor [`scroll.text.miss`, `scroll.text.measure-miss`,
    budget 5 %, dvs. > 95 % träff];
  - flikprocessens kallstart till första display list [`tab.cold-start`], Enter till första bild
    huvudlöst för `hello` och `text` [`first-frame.*`], pekarhändelse till pixel i `dashboard`
    [`input.dashboard`].
- **Maskinen** är en delad VM med 2 CPU och 7 GB, där fyra byggagenter kompilerar samtidigt. Load
  average var 4–6 under mätningarna nedan; talen är därför uppblåsta och P95 särskilt brusig.
  Jämförelser före/efter görs som A/B: båda binärerna sparas och körs omedelbart efter varandra.
- **Indata.** `target/` delas med tre andra spår (också `target/wasm` och den upplyfta
  `target/<profil>/vy-cli`), så benchen och konformansen körs från en privat kopia av binären och
  mot en privat rot med modulerna byggda ur grenens `modules/` och symlänkar till grenens
  `conformance/` och `modules/` (`vy-cli bench --root`, `vy-cli conformance --root`).

## Mätningar före (release, 100 körningar, load 4,0–5,8)

Binären byggd från grenens första commit (mättverktyget, ingen optimering), alla åtta
arbetsytecrates kompilerade ur grenens träd. En tidigare körning är kasserad: `target/` delas av
fyra worktrees och cargos artefakthash för en sökvägscrate bortser från worktreens sökväg, så en
build kunde länka ett annat spårs `vy-render` eller `vy-runtime` (koordinatorn har sedan gett
varje worktree egna profilvärden). Id som i *Mätmetod*; tid i ms, missar i procent.

```
id                                 p50       p95     budget
compile.chat                     85.52    122.78   60.00 ms  ÖVER
compile.dashboard                84.84    110.06   60.00 ms  ÖVER
compile.document                 81.47    117.29   60.00 ms  ÖVER
compile.form                     58.35     85.30   60.00 ms  ÖVER
compile.gallery                  40.86     89.93   60.00 ms  ÖVER
compile.game                     36.09     55.90   60.00 ms
compile.hello                    14.28     15.54   60.00 ms
compile.list                     41.13     50.15   60.00 ms
compile.settings                 50.18     68.58   60.00 ms  ÖVER
compile.shop                     54.58     80.42   60.00 ms  ÖVER
compile.text                     18.61     28.06   60.00 ms
raster.ops-2000                 119.55    198.65    2.00 ms  ÖVER
raster.ops-2000-cold            117.95    152.34          -
raster.rects-2000                19.96     26.12    2.00 ms  ÖVER
raster.text-200                  19.29     23.69    2.00 ms  ÖVER
raster.mix-1800-200              36.77     42.35    2.00 ms  ÖVER
raster.cards-2000               109.03    147.82          -
raster.page.hello                 1.17      5.20    2.00 ms  ÖVER
raster.page.text                  5.05      7.65    2.00 ms  ÖVER
raster.page.form                  1.43      1.62    2.00 ms
raster.page.list                  3.38      4.44    2.00 ms  ÖVER
raster.page.gallery              13.41     17.72    2.00 ms  ÖVER
raster.page.game                  3.22      7.35    2.00 ms  ÖVER
raster.page.dashboard             5.70     10.45    2.00 ms  ÖVER
raster.page.chat                  3.19      7.00    2.00 ms  ÖVER
raster.page.shop                 21.85     33.60    2.00 ms  ÖVER
raster.page.document             11.02     18.67    2.00 ms  ÖVER
raster.page.settings              5.11     11.12    2.00 ms  ÖVER
scroll.text.frame                13.88     18.14   16.67 ms  ÖVER
scroll.text.miss                100.00    100.00     5.00 %  ÖVER
scroll.text.measure-miss        100.00    100.00     5.00 %  ÖVER
tab.cold-start                   20.91     47.17  150.00 ms
first-frame.hello                46.35     79.15  100.00 ms
first-frame.text                 76.02    108.42  100.00 ms  ÖVER
input.dashboard                   8.59      9.95   16.00 ms
```

En körning med 3 varv vid lägre last gav ungefär samma P50 för de patologiska listorna som
granskning 0004 (15,6 / 16,4 / 29,7 ms mot 16,8 / 15,8 / 29,8 ms); lasten blåser upp P95 mest.
Jämförelser före/efter görs därför A/B med båda binärerna i samma stund.

### Läsning före

- **Ramen i sidornas proportioner tar 120 ms, inte 17.** Varje `CLIP_PUSH` som en op ritas under
  bygger en mask över hela duken (`Mask::new` 1280 × 800 = 1 MB, nollställd, sedan memset av
  klippets pixlar): 160 raka och 40 rundade klipp blir 200 masker per frame. Allt under ett klipp
  går dessutom genom rasterpipelinen i stället för memset, och varje glyf blittas med en egen
  `Pixmap` och `draw_pixmap` (tre pipelines kompileras per anrop i tiny-skia).
- **Fast kostnad per frame ≈ 1,2 ms vid 800 × 600** (hello, 4 opkoder): duken (1,9 MB) allokeras,
  fylls vit och avmultipliceras varje gång.
- **Texten formas om varje gång**, både i modulens `dl_text` och i rasteriseringen: 100 % missar.
- **Kompileringen** är 14–86 ms P50 för 5–30 KB-moduler utan kompileringscache.
- Rullningen av textsidan ligger på gränsen (14 ms P50, 18 ms P95 mot 16,7).

## Steg 2a: formad-text-cache i `TextEngine` (SPEC-13.2.3)

**Vad.** `TextEngine` håller de formade texterna: nyckel (typsnittsfamilj, snittvikt, storlek,
radbrytningsbredd, textens byte) – färgen ingår inte, så stilar som bara skiljer i färg delar
post – och värde glyferna per rad med radens y och det uppmätta måttet. `text_measure`,
`dl_text` och rasteriseringen slår upp i samma cache; en träff formar inget. Alla bredder som
betyder "ingen radbrytning" (≤ 0, NaN, absurd) är en nyckel (SPEC-6.4.3). Cachen har två
generationer om `SHAPE_CACHE_UNITS` = 65 536 enheter (en per glyf, rad och textbyte): en träff i
den gamla flyttas till den unga, och när den unga är full blir den den gamla och den gamla
släpps – det en ram fortsätter använda ligger kvar, och en motor håller högst två generationer
(några MB). En text större än en generation formas varje gång. cosmic-texts `SwashCache`
(rasteriserade glyfbilder) saknar tak; den börjar om vid `GLYPH_IMAGE_CAP` = 16 384 bilder.
Font-systemet, `SwashCache` och cachen lever i motorn, som lever över frames: en per
runtime-instans (fliken), en per flik i skalets huvudprocess (`tab.text`, ny vid navigering)
och en för chromen – inget byggs om per frame (kontrollerat i `vy-shell`, ingen ändring där).

**Determinism.** Bränslet för `dl_text` och `text_measure` debiteras per byte före uppslaget
och är detsamma vid träff och miss; rasterbudgeten debiterar textens byte och glyfernas yta som
förut. Testet `cached_layout_is_what_shaping_gives` jämför en motor som format, kastat och
format om hundratals texter med en ny motor för höger-till-vänster, styckeavgränsare,
radbrytning och tom text: samma mått, samma glyfer på samma pixlar. Konformans 55/55 à 0,000 %.

**Tal.** Båda binärerna efter varandra direkt efter att maskinen startats om (load 0,2 → 3),
100 körningar:

```
id                          före p50  före p95 efter p50 efter p95
raster.text-200                16.17     25.04     12.07     17.76
raster.mix-1800-200            32.54     57.27     25.16     41.32
raster.ops-2000               100.02    140.57     71.35    109.80
raster.ops-2000-cold           99.61    149.71     71.89    109.51
raster.page.text                5.05      9.35      4.23     11.15
raster.page.document            9.98     14.25      7.51     12.84
scroll.text.frame              13.08     20.50      6.64     10.07
scroll.text.miss              100.00    100.00      2.59      2.59
scroll.text.measure-miss      100.00    100.00      0.00      0.00
first-frame.text               75.19    101.00     72.49    111.93
raster.rects-2000              18.32     31.22     16.90     25.41
raster.cards-2000             103.48    149.64     91.06    135.95
```

Rullningen håller nu budgeten (P95 10 ms mot 16,7) och träffen är 97,4 % på den rasteriserande
sidan (missar när ett nytt stycke rullar in) och 100 % i modulen (`text` mäter alla stycken i
`vy_init`, och `dl_text` slår sedan upp samma text, stil och bredd). Glyfblittningen, inte
formningen, är resten av textkostnaden (steg 2b). Kontrollraderna utan text (`rects-2000`,
`cards-2000`) flyttade sig 8–12 % åt samma håll av brus. En interfolierad A/B under last 4–6
(sex block à 20 körningar per rad, binärerna växelvis, median av blockens P50) gav samma bild:
`text-200` 0,77, `scroll.text.frame` 0,83, `ops-2000` 0,86, `page.document` 0,87, kontrollerna
0,98–0,99.

**Brus i debug.** `spec_5_3_3_bench_32_rounded_clips_2000_rects` (vy-render, M2.7) hävdar en
väggklocka på 1 s i debug och föll tre gånger i rad (1,0–1,6 s) medan de andra spåren byggde
om efter en omstart; samma test med och utan textcachen gick sedan på 1,4–1,7 s för hela
binären. Det testet har ingen text. Gränsen lämnas (steg 2b gör ramen billigare).
