# M2, andra halvan (a): referenssidorna list, gallery, game och dashboard

Byggagentens noteringar för grenen `m2/pages-a`. Granskningssidan skrivs av en annan agent.

## Vad som byggdes

| Sida | Övar i ABI:t | wasm (release, strip) | Fall | Träd |
| --- | --- | --- | --- | --- |
| `modules/list` | SCROLL i pixlar och rader, KEY_DOWN-navigering, POINTER_DOWN-träffar, nästlade klipp, `text_measure` för högerställning, `list`/`listitem`/`dialog`/`button` med fast id | 18 018 B | 3 | 2 |
| `modules/gallery` | resurser per index, `dl_image` med cover/contain/stretch, lat avkodning, `image`-noder med alt-text, omflöde vid `vy_resize`, dialog | 15 933 B | 4 | 2 |
| `modules/game` | `request_frame` bara under spel, rörelse ur `dt_ms`, `random` för serven, `key_down`-pollning jämte KEY_DOWN, VISIBILITY och `vy_suspend` pausar, BUDGET sänker arbetet | 14 401 B | 3 | 2 |
| `modules/dashboard` | `dl_path` som strukna polylinjer, `dl_rect` för staplar och stödlinjer, POINTER_MOVE med hårkors och verktygstips, omflöde under 1000 px, `table`/`row`/`cell` intill `image` | 21 892 B | 3 | 2 |

Alla fyra är `no_std` + `alloc` på `vy-sdk` precis som `text` och `form`, ingen begär capabilities,
och alla ligger långt under 64 KB. Totalt 13 nya guldbilder (27 i sviten) och 8 nya semantiska träd (14).

Varje modul har nativa tester mot `vy_sdk::mock` (`cd modules/<namn> && cargo test`). De körs
inte av `cargo test --workspace` eftersom `modules/` står utanför arbetsytan – se luckan nedan.

### Bilder

`modules/gallery/res/*.png` är åtta procedurella palett-PNG:er (1,1–2,7 KB) ur
`modules/gallery/res/generate.py` (Pillow, deterministiska funktioner av pixelkoordinaten,
inget nedladdat). De committade filerna är sanningen; skriptet visar hur de gjordes.

### Ändring i `vy-cli` (minimal, additiv)

`cases.toml` har fått nyckeln `resources = ["sökväg", …]`: filer relativt repots rot som
läses i `run_case_inner` och skickas som `Options.resources`, så `resources[i]` blir resurs-id
`i + 1` (SPEC-4.5.1). Parsern (`parse_cases`) och dess test fick samma rad. `vy-cli render`
kan fortfarande inte ta resurser; gallery renderas bara via sviten.

Inget i `crates/vy-sdk`, `crates/vy-runtime` eller `crates/vy-render` ändrades.

## SDK-luckor och vad som skavde i ABI:t

Det här är leveransen som avgör frysningen. Ordnade efter hur mycket de påverkar ABI 1.0.

### ABI

1. **Ingen "pekaren lämnade sidan"-händelse.** `dashboard` visar ett verktygstips vid
   POINTER_MOVE och har inget sätt att få veta att pekaren lämnat fönstret; tipset står kvar.
   Bilaga A behöver antingen en ny kind (`POINTER_LEAVE`) eller en regel att värden skickar
   `POINTER_MOVE` med koordinater utanför ytan (t.ex. −1, −1) när pekaren lämnar. Nytt nummer
   är billigast och tydligast (SPEC-A.2.1 tillåter det).
2. **Modulen kan inte fråga efter en bilds storlek.** `gallery` bär en egen tabell med
   bredd × höjd för att skriva "320 × 200 px" och för att veta var `contain` lägger bilden.
   Antingen en import `resource_info(id) -> i64` (bredd/höjd packade som `text_measure`) eller
   – bättre för princip 3 – att manifestets `resources` får obligatoriska `w`/`h` för bilder
   och SDK:t genererar tabellen ur manifestet. Avgör före frysningen; ingen import behövs om
   manifestvägen väljs.
3. **`dt_ms` efter paus är ospecificerat.** SPEC-5.1.2 säger inte om `dt_ms` i första framen
   efter VISIBILITY synlig/`request_frame` är tiden sedan förra framen (kan vara minuter) eller
   0. `game` klampar till 50 ms själv. Förslag: "tid sedan föregående `vy_frame`, 0 i den
   första framen efter `vy_init`, `vy_resize` eller VISIBILITY synlig".
4. **`key_down`-pollning förutsätter att värden håller reda på nedtryckta tangenter** ur
   KEY_DOWN/KEY_UP, och att den släpper dem när fliken tappar fokus eller göms. Headless gör
   det förstnämnda (`keys_down` i runtimen) men specen säger varken det ena eller det andra;
   ett spel som bara får KEY_DOWN och sedan VISIBILITY dold skulle annars se pilen "hållen"
   för evigt. SPEC-5.2.6 bör säga: hålls tills KEY_UP, nollställs vid VISIBILITY dold och
   fokusförlust.
5. **Omritning medan fliken är dold.** Specen säger inte vad värden gör om `vy_event` svarar 1
   på VISIBILITY dold. `game` svarar 0 dold och 1 synlig; SPEC-6.2.1/6.2.2 bör säga att värden
   inte anropar `vy_frame` medan fliken är dold utan `persist_frame`, oavsett svaret.
6. **`pointer_get` användes inte av någon referenssida**; alla sex sidorna klarar sig med
   händelserna. Kandidat att stryka vid frysningen (33 → 32 importer) eller behålla med
   motiveringen "för pollande spel", som `key_down`.
7. **`dl_rect`:s radie gäller alla fyra hörnen.** Stapeln med rundad topp och rak botten mot
   baslinjen kostar två opkoder (dashboard). Acceptabelt; ingen ABI-ändring, möjligen en
   SDK-hjälpare.
8. **`max_w ≤ 0` = ingen radbrytning** är en M1-tolkning som nu bär upp högerställning,
   centrering och klippta rader i alla sidor. Bör in i SPEC-6.4.3 som normativ mening.

### SDK (`vy-sdk`)

9. **Ingen justering av text.** Högerställning (listvärden, axelmarkeringar) och centrering
   (knappar, spelets överlägg) kräver `text_measure` + aritmetik, alltså två formningar i
   värden per etikett. En `Frame::text_aligned(rect, Align, …)` i SDK:t gör det till en rad
   men inte billigare; textcachen (SPEC-13.2.3, M3) gör det billigt.
10. **`Sem` har hjälpare bara för sju roller.** `listitem`, `dialog`, `region`, `table`,
    `row`, `cell` skrivs som `sem.node(Role::X, …)`. Antingen full uppsättning eller inga
    (princip 1: ett sätt). Jag lutar åt att behålla bara `node`/`node_with_id`/`begin`.
11. **`Column`/`Row` användes inte av någon av de fyra sidorna.** Alla behöver samma
    rektanglar i `event` (träffar) som i `frame` (ritning), så varje sida har en
    `layout()`-funktion som räknar ut rektanglar ur fönstret, och ritar ur den. Det är det
    mönster SDK-dokumentationen bör visa; `Column`/`Row` passar bara statiska sidor.
12. **Flyttal i `no_std`.** `floor`, `round`, `sqrt`, `sin` finns inte i `core`; sidorna
    använder `as usize`-trunkering och `game` en riktningstabell i stället för trigonometri.
    Bör stå i SDK:ts README; ett litet `vy_sdk::math` (floor/round via cast) räcker för det
    mesta, `libm` som valfritt beroende för resten.
13. **Nativa modultester står utanför grindarna.** `modules/*` är exkluderade ur arbetsytan,
    så `cargo test` i varje modul körs bara för hand. Förslag: `tools/test_modules.sh` som
    kör `cargo test` per modul, inkallad av CI jämte `build_modules.sh`.

### Konformansrunnern (`vy-cli`)

14. **Händelser kan inte läggas vid en viss frame.** `render` kör frames till taket innan
    första händelsen levereras, så för en sida som begär frame varje frame (spelet) hamnar
    alla händelser efter taket. `game` löser det genom att vila tills mellanslag trycks, och
    hållna tangenter läggs *före* serven. Men VISIBILITY och BUDGET kan varken scriptas eller
    nås vid en frame mitt i spelet, så paus och sänkt arbete är bara nativt testade. Förslag:
    `{kind = "visibility", visible = false}`, `{kind = "budget", used = 9}` och en nyckel
    `after_frames = N` per händelse (leverera efter N frames i den omgången). Tre små tillägg i
    `ScriptedEvent`/`parse_event`; inga ändringar i runtimen.
15. **`vy-cli render` saknar `--resource`.** En utomstående agent som vill se gallery med
    `render` får grå platshållare. Flaggan är tio rader; lämnad för konsolideringen.

## Determinism

`game` är det första animerade fallet: 60 frames à 16 ms med hållen pil, serve ur värdens
seedade `random`, allt i `f32`. `conformance` utan `--update` ger 0,000 % på båda körningarna
i rad, och modulens nativa test `same_seed_same_positions` visar samma sak utan wasm-motor.
Guldbilderna är tagna på Linux x86_64 som tidigare; macOS-jobbet avgör om `f32`-fysiken och
`dl_path`-strykningen håller på ARM (specfråga 13 i BUILD-NOTES).

## Grindar

Körda i `/home/claude/vy-a` efter sista committen: `tools/build_modules.sh`,
`cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
`cargo test --workspace`, `python3 tools/spec_trace.py` (127 av 127), `check-abi` på alla
fyra modulerna (ABI 1.0: OK), `conformance` (27 fall, alla 0,000 %, 14 träd lika), och
`cargo clippy --target wasm32-unknown-unknown -- -D warnings` plus `cargo test` i varje modul.
