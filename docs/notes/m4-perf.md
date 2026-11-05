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
