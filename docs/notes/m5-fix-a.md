# M5 – rättningar efter granskningen, del A (`m5/integration`)

Byggagentens noteringar för rättningarna av granskningsfynden i 0010, 0012, 0013, 0014:7 och 0015
som rör SDK:t, exempelsidorna, konformansen och releaseverktygen. Arbetet görs direkt på
`m5/integration` (från `e431a60`). `crates/vy-shell`, `vy-runtime`, `vy-render` och `vy-proto` rörs
inte: de är del B:s (`m5/fix-b`). Granskningssidorna kom in med `edfff6d` (`m5/review`).
Sessionen kan startas om: läget står här och i `git log`.

Varje fynd är ett eget commit med ett test som är rött före och grönt efter. Grindarna körs från
roten före varje commit (`target/gate/fixa.sh`, ocommittad: fmt, clippy, moduler + `check-abi` +
`test_modules.sh`, `cargo test --workspace`, `spec_trace.py`, konformans), med
`CARGO_INCREMENTAL=0`.

## Läge

| # | Fynd | Läge | Commit | Test |
| --- | --- | --- | --- | --- |
| 1 | 0010:1 långt ord i flerradsfält (hög) | klar | `f211181` | `budget_tests` i vy-cli (tre fall), `spec_6_4_3_textbox_wrap_breaks_where_the_exhaustive_search_does`, `spec_5_3_3_textbox_wrap_measures_in_the_order_of_the_text` |
| 2 | 0010:2 bränsle per frame i konformansen | klar | `0700141` (`spec:` SPEC-12.2.4), `2902727` | `spec_12_2_4_every_frame_is_held_to_the_budget`, `conformance_stats_prints_the_costliest_frame` |
| 3 | 0010:3 konformanspaketet och umask | klar | `96c70f6` | `spec_9_7_1_conformance_package_is_the_same_under_any_umask` (vy-cli `tests/package.rs`) |
| 4 | 0010:4 textfältets namn i trädet | klar | `def14b2`, `e1b6a99` (`golden:`) | `spec_10_1_1_textbox_label_is_in_the_tree` (vy-sdk), `spec_10_1_1_every_text_field_has_a_name` (vy-cli), sidornas tester |
| 5 | 0010:5 komposition, U+2028/U+2029 | klar | `f1902e2` | `spec_5_2_6_textbox_press_ends_the_composition_in_the_host`, `spec_5_2_6_textbox_composition_replaces_the_selection`, `spec_5_1_6_textbox_line_separators_are_line_breaks` |
| 6 | 0014:7 storlekstak för enradssidor | klar | `5faa507` | taket i `tools/test_modules.sh` (form 32 000) och i `scaffold_builds_to_wasm_and_passes_check_abi` (startsidan 30 000) |
| 7 | två SDK-hjälpare ur m5-examples | klar | `f1e7f32` | `spec_2_1_5_numbers_as_text_without_fmt`, `spec_5_2_4_text_centred`, doctest i `num` |
| A | 0013:1 `skriv` låser sig på sparad text (hög) | – | | |
| B | 0013:2 `kalkyl` låser sig på sparat blad (hög) | – | | |
| C | sparad data får aldrig stoppa sidan (alla sex) | – | | |
| D | 0013:3 `kalkyl` långa texter och rasterbudgeten | – | | |
| E | 0013:4 fall utan `storage`, förinställd lagring | – | | |
| F | 0012:1 `MSYS2_ENV_CONV_EXCL` i `repro_env.sh` | – | | |
| G | 0012:2 `dry_run` i `release.yml` | – | | |
| H | 0012:3 rökprov av `vy-dev new` sist i `release.yml` | – | | |
| I | 0012:4 minsta rättigheter i `release.yml` | – | | |
| J | 0012:5 `SHA256SUMS` täcker allt | – | | |
| K | 0015:4 milstolpstabellen, strukturlistan, README | – | | |
| L | 0013:5, 0015:5 mätvärdena i `m5-examples.md` | – | | |
| låg | 0012:6 container på digest; 0015:3 300 KB-modul | om tid finns | | |

Grindarna är körda hela före varje kodcommit. Undantag, sagt rakt ut: `spec:`-commiten `0700141`
(SPEC-raden och ett ignorerat teststomme) kördes med fmt, clippy, spec_trace och vy-cli:s tester;
där två oberoende fynd låg i arbetskopian samtidigt (5 och 6) kördes grindarna en gång på båda och
commiten gjordes var för sig; `def14b2` har med avsikt röd konformans tills `e1b6a99` (träden).

## 1 – långt ord utan mellanslag i ett flerradsfält (0010:1)

**Felet.** `wrap_paragraph` mätte hela resten av ordet inför varje rad, och `fit_prefix` bisekterade
sedan hela resten med ett prefix av den längden per steg. Mätt i enhetstestet: 3 369 550 byte
mätta för ett ord på 4 096 byte vid 48 px.

**Rättningen.** `fit_prefix` söker brytpunkten kring en gissning i byte: första radens ur ordets
bredd (`längd × bredd / ordbredd`), varje senare rads ur raden före. Ett tecken i taget först,
sedan dubblade steg åt det håll svaret ligger, sist bisektion mellan sista prefix som fick plats
och första som inte fick det. Inget mätt prefix är längre än ungefär två rader. Ordets rest mäts
hel bara när sökningen når dess slut, och det är samma mätning som förr gav radens bredd. Sökningen
går på byteavstånd med `floor_char_boundary`/`ceil_char_boundary`, utan lista över teckengränser.

**Samma radbrytningar.** Under antagandet att ett prefix aldrig blir smalare av att växa (det den
gamla bisektionen också vilade på) finns ett enda svar, och det är samma. Egenheten att ett tecken
som inte ryms sist i ett stycke ger en tom rad efter sig är kvar. Konformans 91/91 à 0,000 % utan
en ändrad fil, och enhetstestet jämför mot den gamla sökningen (behållen i testet som
`wrap_exhaustive`) över 133 stycken × 10 bredder × 2 breddmodeller (fast bredd som mocken; varierad
med smala, breda, breddlösa och fler-byte-tecken).

**Bränsle, dyraste frame (budget 8 M, hård gräns 64 M), mätt i `budget_tests` mot vy-runtime:**

| Sida och indata | före | efter |
| --- | --- | --- |
| `widgets` 800 × 600, 4 096 × "W" inklistrat | 63 938 856 | – |
| … efter storleksbyte till 1280 × 800 | 53 029 764, sedan avstängd (två i rad) | – |
| … dyraste frame genom hela testet (inklistring, tre storlekar, tryck, drag, tangenter) | 63 938 856 | **1 002 531** (400 × 700) |
| `widgets`, 1 024 fyrbytestecken (U+10348) | 8 109 948; 8 564 145 vid 400 × 700; avstängd | **945 718** |
| `skriv` 800 × 600, 6 000 × "W" | avbruten vid 64 000 000 | **2 280 539** (1280 × 800) |

**Samma mönster på andra ställen – genomgånget.**
- SDK:t: `offset_in_line` (träfftest) bisekterar en rad med `text_caret` över radens text:
  n log n i en rad, i en händelse, inte i en frame. Ett enradsfält med 1 KB kostar omkring 0,35 M
  per tryck. `scroll_into_view` och `paint` mäter rader i bild. Inget kvadratiskt.
- `skriv`: förhandsvisningen mäter varje ord en gång och bryter inte ord; `md.rs` har ett eget
  kvadratiskt fall (0013:1 b, punkt A).
- `kanban` (kortens rubriker) och `studio` låter värden bryta (`text_measure` med bredd).
- `kalkyl`: en cell mäts högst sju gånger (decimaler tas bort tills talet ryms); långa texter i
  bild är punkt D.
- `rita`, `block`, `rymd`: ingen radbrytning.

**Mocken** fick `MockHost::text_bytes` (byte lämnade till `text_measure`, `text_caret`, `dl_text`),
så SDK-tester kan sätta tak för vad en layout mäter; README-rad under *Testa nativt*.

**Storlekar:** `widgets` 55 493 → 55 280, `skriv` 61 502 → 61 291; övriga oförändrade (`form` 30 627).

## 2 – bränsle per frame i konformanssviten (0010:2, SPEC-12.2.4)

- `drive` i vy-cli sparar `Runtime::last_frame()` efter varje frame (`RenderOutput::frame_stats`,
  `memory_pages`). `conformance::CaseStats` håller antal frames, dyraste frame (nummer, bränsle),
  sista frame, rasteriseringens enheter och budget, om den avbröts, minnet, och om fallet skriptar
  en `budget`-händelse.
- Regeln (SPEC-12.2.4, ny i §12.2): ett modulfall där en frame går över 8 000 000 bränsle, eller
  där bildens rasterisering avbryts, underkänns – också med `--update`, som då inte skriver någon
  guldbild – om fallet inte självt har en `budget`-händelse. Skalfall mäts inte (modulen körs i en
  flikprocess).
- `vy-cli conformance --stats [fallprefix..]`: en rad per fall och sist körningens dyraste frame.
  Prefixen väljer fall också utan `--stats` (och med `--update`).
- **Inget befintligt fall går över.** Dyrast i sviten: `rymd-800x600@1-torus` frame 17 med
  1 642 244 (21 %); högsta rasterandel 61 % (`shop-400x700@2`).
- Dokumenterat i `conformance/README.md` (svenska och engelska delen), `cases.toml`-huvudet och
  som en rad i CLAUDE.md:s grindtabell.

## 3 – konformanspaketet (0010:3)

`tools/package_conformance.sh` skriver zippen med `tools/release_files.py zip` (en zip-skrivare:
sorterade poster, en tid, läge 0644, inga kataloger, inga extra fält). `touch` med GNU-flaggor är
borta och summorna tas med `shasum -a 256` där `sha256sum` saknas, så skriptet går också på macOS.
Paketet för 0.3.0: 91 fall, 19 moduler, 201 filer, 5 728 677 byte; samma SHA-256 under umask 022,
002 och 077. Testet packar också upp, jämför mot `SHA256SUMS` och kör `vy-cli conformance --root`.

## 4 – textfältets namn i trädet (0010:4)

- **API.** `TextBox::draw(f, r, &theme, label, placeholder)` med `ui::Label`:
  `Label::shown(rektangel, text)` för en etikett sidan själv ritar (noden registreras där orden
  syns) och `Label::hidden(text)` för ett namn som inte ritas (noden får fältets hörn och ingen
  storlek). Fältet registrerar namnet som en `text`-nod precis före sin `textbox`-nod. Den gamla
  signaturen finns inte kvar (ett sätt, SPEC-2.1.1).
- **Varför ingen storlek på ett dolt namn.** Skalets textmarkering börjar på den `text`-nod som
  ligger under ett tryck (`selection::node_at`, strikt innanför rektangeln). En namn-nod över hela
  fältet hade startat värdens markering vid varje tryck i fältet. En nod utan storlek träffas
  aldrig, men finns i trädet, i "markera allt" och för hjälpmedel.
- **Sidor.** `widgets` (synliga etiketter: träden oförändrade), `form` (etiketterna ritades men
  registrerades aldrig), `kanban` ("Nytt kort i Att göra/Pågår/Klart", "Kortets titel"), `kalkyl`
  ("Innehåll i <cell>", och ringens två grannar "Föregående cell"/"Nästa cell"), `skriv`
  ("Anteckningens text i Markdown"), startsidan i `vy-dev new`, guidens sida. `chat` har ett eget
  fält och registrerar "Meddelande" för hand (inte i uppdragets lista, samma fel). `studio` hade
  redan sin etikett som nod före fältet.
- **Träd.** 14 filer i `conformance/semantic/` (`e1b6a99`): form ×3, chat ×3, kanban ×3, kalkyl ×3,
  skriv ×2. Ingen guldbild ändrad (91 bilder skrivna byte för byte lika).

## 5 – komposition och radskiljare (0010:5)

- Ett tryck i fältet, eller `set_text`, under en komposition anropar `ime_end` och öppnar
  kompositionen igen i nästa frame (`draw`), inte i samma händelse: annars ser värden aldrig att
  den tog slut. Utan beviljad `ime` görs inget anrop.
- En komposition ersätter markeringen från sitt första tecken (svaret är `Changed`).
- U+2028 och U+2029 är radbrytningar: mellanslag i enradsfält, `\n` i flerradsfält.
- **Till del B (vy-shell), inte rättat här:** fönstret skickar aldrig `Ime::Preedit` vidare som
  `IME_COMPOSE`, och `set_ime_allowed` växlas bara när `ime_area()` går mellan finns och saknas
  mellan två synkningar. Om värden ska avbryta plattformens komposition på `ime_end` behöver den
  alltså se anropet självt.

## 6 – storlekstak (0014:7)

`form.wasm` ≤ 32 000 (31 088 nu) i `tools/test_modules.sh`; startsidan ≤ 30 000 (28 848) i
vy-dev:s test. Mutationsprov: med radläget dolt för kompilatorn (`black_box`) blir `form.wasm`
37 551 byte och taket fäller. Meddelandena säger vad som skiljer utebliven vikning (≈ +6 500) från
verklig tillväxt.

## 7 – två SDK-hjälpare

- `vy_sdk::num::{push_int, push_decimal}`: tal som text med heltalsaritmetik. `ui::Slider`
  använder dem i stället för `format!`: `rymd` 38 963 → 35 509 byte.
- `Frame::text_centred(r, style, text)`: de tolv rader som stod i alla sex exempelsidorna.
- Exempelsidorna använder hjälparna (`centred` i alla sex, `number` i block, rita, rymd, skriv).
  Inga bilder eller träd ändrade.
- **Resten av luckorna i `m5-examples.md` är inte gjorda** (ingen större SDK-runda): knapp med
  eget nodnamn och ikonknapp, publik fokusring, `TextBox::adopt_focus`/`set_id`, dragning
  (`ui::Drag`), publik `Activity`, tvåaxlig `ScrollView`, växlingsknapp, dialog, flikar, länk,
  `RichText`, fasta steg (`time::Steps`), `math::{sqrt, sin, cos}`, `Path::clear`/`with_capacity`,
  `Color::scaled`, fler tangentnamn, revisionsräknare i `TextBox`.

### Modulstorlekar efter punkt 1–7 (byte)

| Modul | `e431a60` | nu |
| --- | --- | --- |
| form | 30 627 | 31 088 |
| widgets | 55 493 | 56 096 |
| kanban | 54 647 | 55 158 |
| kalkyl | 61 340 | 61 905 |
| skriv | 61 502 | 61 656 |
| rymd | 38 963 | 35 509 |
| block | 21 746 | 21 922 |
| rita | 37 376 | 37 403 |
| chat | 24 875 | 24 961 |
| startsidan (`vy-dev new`) | 28 342 | 28 848 |

## Öppet

- Händelsers bränsle går inte att läsa ur `vy-runtime` (bara `last_frame()`); testerna ser en
  händelse först om den når den hårda gränsen och avbryts. En `last_event()` vore del B:s.
- `studio`: `relayout` mäter varje post på nytt när loggens bredd ändras (högst 300 poster à högst
  60 KB). Inte samma mönster och inte uppmätt här.
