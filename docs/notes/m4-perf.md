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

## Steg 2b: rasteriseringens snabbvägar, med samma byte och samma budget (SPEC-13.2.1)

Regeln: guldbilderna är det tiny-skia ritade genom en klippmask, och budgeten är det M3 debiterade;
ingenting får ändra en byte eller en enhet. Varje ändring nedan bygger på en egenskap i tiny-skias
aritmetik som ett test bevisar, och `raster_reference.rs` (M3:s rasterisering ordagrant, bara i
tester) jämförs byte för byte och rapport för rapport med den nya.

| Ändring | Varför det är samma byte | Bevis |
| --- | --- | --- |
| **Raka klipp utan mask.** Under en stack av raka klipp (ingen radie) skärs op:en till klippets pixlar och ritas utan mask; maskens kostnad debiteras ändå, en gång per nivå, där den förr byggdes. Rundade klipp har mask som förr. | En rak nivås mask är 255 i klippets pixlar, och `MaskU8` med 255 är identiteten i båda pipelinerna (`(255x + 255) >> 8 = x`, `x × 1.0 = x`). Men tiny-skia blandar en *opak* färg utan mask som "source" (lerp) och med mask som "source over" – de skiljer en enhet vid AA-kanter (spec-fråga 41). Opaka fyllningar under raka klipp ritas därför med alfa `ALPHA_BELOW_ONE` (största `f32` under 1): inte opak för tiny-skia, så source over behålls, men `(v·a·255 + 0,5)` trunkerar till samma byte. Rundade rektanglar och fyllda banor tar snabbvägen när hela deras täckning ligger inom klippet; strykningar (fogar når utanför) alltid masken. | `alpha_below_one_stores_the_same_colour` (alla 256 värden), `fast_paths_draw_what_m3_drew`, `fast_paths_spend_the_budget_m3_spent` |
| **Hela pixlar fylls direkt.** En rektangel med alla kanter på pixelgränser fylls här: opak som en memcpy per rad, genomskinlig med lowp:s `s + (d·(255 − sa) + 255) >> 8`. | En sådan rektangel täcker sina pixlar helt (täckning 255) och inga andra, och då är både "source" och "source over" färgen respektive den formeln. | samma differentialtest (heltalskoordinater vid skala 1 och 2 är en fjärdedel av slumpens rektanglar) |
| **Glyfer och bilder kompositeras direkt** (`over_highp`), utan en `Pixmap` och tre kompilerade pipelines per glyf. | `draw_pixmap` kör alltid högprecisionspipelinen (mönsterskuggningens `Gather` finns inte i lowp): kanal som `v·(1/255)`, mask som `m/255`, `d·(1 − sa) + s`, klämt, ×255, avrundat halvt-till-jämnt (`cvtps2dq`/`vcvtnq`). Samma operationer i samma ordning i skalär `f32` ger samma bitar; avrundningen görs med 2²³-tricket (`round_ties_even` är ett libm-anrop på x86_64 utan SSE4.1 och stod för 15 % av instruktionerna). Glyfens färgning (M3:s heltalsformel) räknas en gång per textrad i en tabell över 256 täckningar; bildens källkolumn en gång per kolumn. | `over_highp_is_what_draw_pixmap_does`: varje källalfa, varje premultiplicerad kanal ≤ alfa och varje destinationsvärde (release; debug var sjunde destinationsvärde), opak och varierande destinationsalfa, och tio maskvärden; `round_even_u8_is_round_ties_even` |
| **Maskpool.** Rundade nivåers masker tas ur en pool och nollas vid pop över det område de skrev (plus en pixel). | Masken är densamma som en ny nollställd; poolen ändrar bara allokeringen (en 1 MB-mask per push vid 1280 × 800 förut). | differentialtestets nästlade rundade klipp, `spec_6_1_4_clip_levels_restore_on_pop` |
| **Duken** är en vektor med 0xFF (premultiplicerad opak vit) och tas ut utan avmultiplicering. | Duken börjar opak och varje op blandar source over (eller opak source), så alfa förblir 255 och avmultiplicering är identiteten; `debug_assert` i debug. | alla jämförelser ovan, konformans |
| **Banor återanvänds** (`PathBuilder` ur förra banan); fyllda banor skalas med `Path::transform` före `fill_path`. | `fill_path` gör själv `clone().transform(t)` och fyller med identitet; samma funktion på samma punkter. | konformans, differentialtest |

**Profil** (callgrind, `frame_2000` tre gånger, release med radtabeller): 1 133 M instruktioner före
steg 2b:s sista två ändringar, 893 M efter avrundningstricket, 713 M efter färgtabellen och
radvisa glyfslingor. Det som återstår är till ⅓ tiny-skias kantutjämnade banfyllning av de rundade
rektanglarna (`path_aa`, med tre pipelines som kompileras per anrop i `RasterPipelineBlitter::new`) –
inte nåbart utan att ändra tiny-skia eller byta rasterisering, vilket ändrar guldbilderna.

**Testerna med väggklocka.** `spec_5_3_3_bench_65000_fullscreen_rects` och
`spec_5_3_3_bench_32_rounded_clips_2000_rects` (M2.7) föll då och då i debug vid load ≈ 5 (1,0–1,6 s mot
1 s), också på andra spår med orörd `vy-render`. De deterministiska kontrollerna (avbrott, antal
ops, enheter) gäller i alla byggen; väggklockan hävdas nu bara i release (`time_bound`) och skrivs
ut i debug. Mina tunga tester och väggklocktesterna delar dessutom ett lås (`tests::serial`), så en
tidtagning aldrig delar binärens två CPU:er med en jämförelse.

**Tal, hela steg 2** (2a + 2b), interfolierad A/B under load 4–5: binären före (steg 1) som A,
efter som B, fyra block à 20 körningar per rad växelvis, median av blockens P50 och P95.

```
id                              A p50    A p95    B p50    B p95  B/A p50  budget
raster.ops-2000                144.47   179.04    56.59    61.26     0.39  2.0
raster.ops-2000-cold           140.37   157.24    56.11    74.48     0.40  None
raster.rects-2000               25.93    39.24    24.25    32.09     0.94  2.0
raster.text-200                 26.27    28.81     8.87    11.83     0.34  2.0
raster.mix-1800-200             42.62    52.28    26.45    29.59     0.62  2.0
raster.cards-2000              160.62   169.58    35.45    39.45     0.22  None
raster.page.hello                1.26     5.96     0.32     2.68     0.25  2.0
raster.page.text                 9.02    11.05     1.36     5.50     0.15  2.0
raster.page.form                 1.56     5.86     0.50     2.86     0.32  2.0
raster.page.list                 4.41     8.46     0.64     4.84     0.15  2.0
raster.page.gallery             20.05    24.19    10.16    12.67     0.51  2.0
raster.page.game                 5.50     7.65     3.28     7.23     0.60  2.0
raster.page.dashboard            9.58    11.59     3.63     7.64     0.38  2.0
raster.page.chat                 5.32     8.64     1.37     5.63     0.26  2.0
raster.page.shop                32.23    35.90    14.42    18.87     0.45  2.0
raster.page.document            15.42    21.75     2.44     6.59     0.16  2.0
raster.page.settings             8.86    10.42     2.48     6.72     0.28  2.0
scroll.text.frame               14.13    14.79     2.45     6.37     0.17  16.67
input.dashboard                 13.73    16.01    10.02    13.52     0.73  16.0
first-frame.hello               43.37    56.77    38.15    44.85     0.88  100.0
first-frame.text                79.14   130.99    70.13   102.83     0.89  100.0
```

Varje referenssida rasteriseras 2–7 gånger snabbare; textsidans rullning tar 2,5 ms (var 14).
Ramen i sidornas proportioner (2 000 opkoder) tar 57 ms P50 här, 28 gånger SPEC-13.2.1:s 2 ms;
lika många kantutjämnade rundade rektanglar och glyfpixlar ryms inte på den här maskinen med en
CPU-rasterisering som ger guldbildernas byte (se *Kvar*). De patologiska genomskinliga rundade
rektanglarna (`rects-2000`) flyttar sig knappt: allt där är tiny-skias `path_aa`.

## Nätgrinden i `vy-runtime`: tom eller okänd origin-lista (SPEC-5.2.8)

Påpekat av m4/dev via koordinatorn; äldre än M4. `Host` sparar manifestets `net`-origins genom
`origin_of`, som bara känner `https://`. En tom lista betydde "inget manifest, ingen grind": med
`net` beviljat nådde `fetch_begin` *varje* https-origin. Tom blev listan också när manifestet bara
angav `vy://`- eller `http://`-origins (som `vy-manifest` godtar, SPEC-8.2.1), och skalets
standardpolicy beviljar `net` automatiskt när alla listade origins är sidans egna – `all()` över en
tom lista är sann. En signerad sida med `"net": {"origins": []}` fick alltså `net` utan fråga och
fritt nät i flikprocessen. Huvudprocessen stoppade det (`Shell::request_verdict` släpper bara
listade origins, granskning 0004 fynd 2), men grinden i fliken är den första och i
huvudlöst läge den enda.

Nu: en origin som inte står i listan är `E_ORIGIN`, också när listan är tom eller saknar
`https://`-origins – en lista som inte namnger något når inget (SPEC-5.2.8 ordagrant). Spec-fråga 24
(huvudlöst utan manifest) avgörs därmed åt det strängare hållet: `vy-cli render --grant net` utan
manifest når ingen origin, vilket ändå bara gällde inspelade svar. Testerna som hämtar namnger nu
sin origin; `spec_5_2_8_origin_gate` visar tom lista och listor med bara `vy://`, `http://` och
värd utan schema. Ingen guldbild berörs (chat-fallen har `https://chat.example`).

**Berör andra spår.** Skalets standardpolicy (`vy-manifest`, m4/security: `default_grants` med
`all()` över tom lista) beviljar fortfarande `net` för en tom lista; det ger nu inget nät men
visas som beviljat i adressfältet. Förslag: `net` med tom origin-lista beviljas inte (eller avvisas
av valideringen). Spec-fråga 53 nedan.

## Spec-frågor

53. **`net` med tom origin-lista.** SPEC-8.2.1 sätter ett tak (≤ 16) men inget golv, och
    SPEC-8.2.3 beviljar `net` "för egen origin" automatiskt. En tom lista namnger ingen origin;
    `vy-runtime` släpper nu ingenting igenom för den, men policyn beviljar ändå `net` och
    adressfältet visar det. Ska valideringen kräva minst en origin, eller policyn låta bli?
