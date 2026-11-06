# Byggnoteringar

Skrivna av byggagenten. Granskningssidorna skrivs av en annan agent (`docs/reviews/`).
M3:s andra del (vy-shell) har egna noteringar i [`docs/notes/m3-shell.md`](notes/m3-shell.md).
M2.7 (luckorna före frysen, efter granskning 0003) står först, sedan M3:s första del, M2.6, M2.5,
M2 och M1; äldre delar är oförändrade utom där senare milstolpar ändrat läget (markerat).
M4:s fyra spår har egna noteringar i `docs/notes/m4-*.md` och står överst.

# M4 – fyra spår, sammanslagna i `m4/integration`

Spåren byggdes parallellt från `3bc0bcb` och slogs ihop i ordningen m4/dev (`vy-dev` och Studio,
[`notes/m4-dev.md`](notes/m4-dev.md)), m4/ux (urklipp, markering, sök, AccessKit, devtools,
[`notes/m4-ux.md`](notes/m4-ux.md)), m4/security (DNS TXT, rotation, transparenslogg, sandlåda,
fuzzning, [`notes/m4-security.md`](notes/m4-security.md)) och m4/perf (bench, rasterisering,
kompileringscache, [`notes/m4-perf.md`](notes/m4-perf.md)). Sammanslagningen, konflikterna och
uppföljningarna står i [`notes/m4-integration.md`](notes/m4-integration.md).

## Spec-frågor (M4)

Spåren numrerade sina frågor från 53 var för sig; här är de en följd, i sammanslagningsordning,
med spåret och dess egen numrering inom parentes. Spårens noteringar använder de nya numren.

53. **SPEC-7.1.3 och en sidas egna anrop** (m4/dev, öppen fråga 7). Meningen gäller `vy://`; att en
    sidas `fetch_begin` på loopback följer samma undantag, och bara mot sidans egen loopback-origin,
    står bara i koden (`vy_runtime::origin_of`, `plain_http_allowed`) och noteringarna. En mening i
    §7 vore tydligare.
54. **Vem ringer?** (m4/dev, öppen fråga 1). Ett localhost-API kan inte skilja Studio från en annan
    Vy-sida som fått `net` mot samma origin, eller från en annan lokal process. Förslag: huvudprocessen
    sätter ett huvud med sidans origin på varje `fetch_begin`, som `Origin` i webbläsare (en sida kan
    inte förfalska det); då kan `vy-dev` kräva `vy://localhost:8081`. Rör SPEC-7.5.1.
55. **SPEC-8.2.4 mot SPEC-5.2.12/5.3.1** (m4/ux, 53). Skrivning under kopieringsgesten lyckas utan
    capability; 5.2.12 rubricerar urklippet "(capability `clipboard`)" och 5.3.1 säger att ett anrop
    utan beviljad capability ger `E_DENIED`. Läsningen är att 8.2.4 är en policyregel om *när*
    capabilityn anses beviljad, så §5 är orört. Bör 5.3.1 få en hänvisning i 1.1?
56. **SPEC-12.3.1 "högst 25 MB" mot SPEC-10.2.1 på Linux** (m4/ux, 54). AccessKit med AT-SPI
    (`accesskit_unix`, zbus) tar Linux-klienten över 25 miljoner byte (under 25 MiB); därför är
    funktionen `a11y-linux` frivillig och Linux-klienten exponerar inget träd som standard. Avses MB
    eller MiB? Ska taket höjas, Linux-tillgänglighet vara frivillig, eller något annat trimmas?
    Talen efter sammanslagningen står i `notes/m4-integration.md`.
57. **SPEC-10.1.1 "värden klipper trädet mot fönstret"** (m4/ux, 55). AccessKit klipper genom
    `clips_children` på sidnoden med nodernas rektanglar oklippta, så plattformen vet att noder
    utanför vyn finns. Avses att noder utanför vyn inte ska exponeras alls?
58. **SPEC-7.4.5 – fler än en nyckel** (m4/security, 53). Första besöket pinnar alla nycklar som står
    i både nyckelfilen och DNS, inte bara den som signerat sidan; en utgivare med två aktiva nycklar
    behöver två TXT-poster. Avsett?
59. **SPEC-7.4.5 – väntetid** (m4/security, 54). Ett uppslag som inte svarar tar upp till 6 s. Efter
    sammanslagningen håller det inte längre upp någon sidas egna förfrågningar (en tråd per flik),
    men väl andra flikars sidladdningar, som är seriella. Bör specen ge en tidsgräns, och bör sidan
    visas i begränsat läge medan uppslaget pågår?
60. **SPEC-7.4.5 – IP-literaler** (m4/security, 55). `vy://192.0.2.1/` frågar `_vy.192.0.2.1` och
    faller till begränsat läge. Ska IP-adresser (utom loopback) vara otillåtna i `vy://`, eller
    alltid begränsade?
61. **§7.6 – split view** (m4/security, 56). En server kan visa olika loggar för olika klienter; bara
    speglar som jämför huvuden upptäcker det. Ska 1.0 ha en andra källa för huvudet (t.ex. i DNS),
    eller räcker det att formatet är speglingsbart?
62. **SPEC-9.1.3 på Windows** (m4/security, 57). Standardläget hindrar skrivning och barnprocesser men
    inte läsning och nätverk; AppContainern (`VY_SANDBOX=strict`) hindrar allt men är inte provad på
    en riktig maskin. Bör meningen gälla strict, och standard bli strict när en människa kört det?
63. **`net` med tom origin-lista** (m4/perf, 53). SPEC-8.2.1 sätter ett tak (≤ 16) men inget golv.
    Efter sammanslagningen släpper varken fliken (`vy-runtime`) eller huvudprocessen igenom något för
    en tom lista (m4/dev:s öppna fråga 5 är därmed stängd), men standardpolicyn beviljar ändå `net`
    och adressfältet visar det. Ska valideringen kräva minst en origin, eller policyn låta bli?
64. **SPEC-13.1.3 utan 300 KB-modul och referenslänk** (m4/perf, 54). Testet mäter den största
    referensmodulen (31 KB) över loopback. Ska sviten ha en syntetisk 300 KB-modul och en simulerad
    länk, eller räcker uppskattningen tills referensmaskinen finns?
65. **SPEC-13.2.1 på vilken maskin, med vilken rasterisering** (m4/perf, 55). 2 ms för 2 000 opkoder
    hålls inte med CPU-rasteriseringen som ger dagens guldbilder (34 ms på byggmaskinen). Gäller
    budgeten GPU-vägen, ska ramen räknas om, eller budgeten höjas för CPU-vägen? Testet
    `spec_13_2_1_raster_budget` är ignorerat med det uppmätta värdet.

# M3 del 2 – vy-shell

Se [`docs/notes/m3-shell.md`](notes/m3-shell.md): fönstret (winit + softbuffer), en process per
flik över rör med `IpcTransport`, chromen som display list, laddning genom vy-proto med policy
och grants, `--record`/`--replay`, `vy-shell --screenshot` och fyra skalfall i sviten,
`vy-cli bench`, Windows i CI. Backloggen 29 → 19. Spec-frågor 45–52 där. Nya beroenden: winit,
softbuffer (motiverade där).

# M2.7 – sista luckorna före frys

Granskning 0003 (`docs/reviews/0003-m3-del1-abi-frys.md`) är genomförd i de delar som pekade på
byggagenten: avsnittet *ABI-frysning: inte klar* (sju tal och tre returvärden), fynd 4
(rasterbudget), fynd 6 (`HttpCache`-tak, typsäkra beviljanden) och fynd 7 (manifestets schema).
Tio commits (denna inräknad), varje `spec:`-commit med sina tester. Specens version- och statusrad är orörd –
specägaren fryser. `docs/ABI-1.0.md` är omgenererad och är grind som förr.

## De sju talen och tre returvärdena (SPEC-5.3.5)

| | Spec | `vy-abi` | Värden |
| --- | --- | --- | --- |
| (i) `fetch_begin.method` | SPEC-5.2.7: 0 GET, 1 POST, 2 PUT, 3 DELETE; annat `E_ARG` utan anrop | `method` | kontrolleras efter capability-grinden, före bytetaken |
| (ii) `log.level` | SPEC-5.2.1: 0 debug, 1 info, 2 warn, 3 error; annat loggas som 3 | `level`, `level::clamp` | klampar; anropet lyckas |
| (iii) pekarknappar | **ny mening SPEC-A.2.2** med tabell: `POINTER_DOWN/UP.c` 1 primär, 2 sekundär, 3 mitten, 4 bakåt, 5 framåt; `POINTER_MOVE.c` mask med bit n − 1 | `button`, `button::mask` | skalets sak; runnern tar `PRIMARY` som standard |
| (iv) `text_style.weight` | SPEC-5.2.5: 1–1000 som i CSS; klampas; närmaste snitt, tyngre vid lika | `weight::{MIN, MAX, NORMAL, BOLD, clamp, nearest}` | `vy-render` väljer snittet själv (tabellen nedan); vikt 0 ger inte längre `E_ARG` |
| (v) typsnittsroller | SPEC-6.4.2 namnger id: −1 sans … −5 emoji | `font` | `vy-render` och SDK:ts `Font` läser konstanterna |
| (vi) länkar per frame | SPEC-9.3.2 (oförändrad: 4 096) | `limits::MAX_LINKS_PER_FRAME` | värd och mock läser samma konstant |
| (vii) tangenter | SPEC-10.3.1: Tab 43, Skift+Tab = Tab med bit 1 (skifttangenterna 225/229), Enter 40, Escape 41 | `key::SHIFT_LEFT`, `key::SHIFT_RIGHT` | – |
| `clipboard_read` | SPEC-5.2.12: gest inom 1 s annars `E_GESTURE`; `max` = 0 ger längden; annars hela urklippet och längden, eller `E_ARG` utan kopia | – | `Options::clipboard` (None = nekas som otillgängligt); gestfönstret räknas på värdens klocka från pekartryck/-släpp, tangent och textinmatning |
| `fetch_poll` | SPEC-5.2.7: 0 på väg, 1 klar, annars negativ kod | – | som förr |
| `vy_event` ≠ 0/1 | SPEC-5.1.3: behandlas som 1, loggas en gång per instans på debug | – | `Runtime::deliver` |

Förteckningen fick avsnitten *Argumentvärden* (nivå, metod, typsnittsroller, vikt) och
*Pekarknappar*, tangentmeningen med decimala koder och Skift, och sex rader i *Gränser*
(`MAX_LINKS_PER_FRAME`, `level::MAX`, `method::MAX`, `button::MAX`, `weight::MAX`). Importer: 32.
Spec-id: 132 (nytt: A.2.2), alla spårade.

### Typsnitt (ändrat i M2.7; ersätter tabellen under M1)

Avbildningen från `weight` till snitt görs nu i `vy_render::text::face_weight`, inte av
cosmic-texts reservkedja, som vid exakt 700 gav serif och mono DejaVu Sans Bold:

| Roll | Resurs-id | Snitt |
| --- | --- | --- |
| `sans`, `ui`, `emoji` | −1, −4, −5 | DejaVu Sans för 1–549, DejaVu Sans Bold för 550–1000 |
| `serif` | −2 | DejaVu Serif för alla vikter |
| `mono` | −3 | DejaVu Sans Mono för alla vikter |

Referenssidorna använder 400, 600 och 700; inga pixlar ändrades (50/50 à 0,000 %).

## Fynd 7 – manifestets schema (`spec:`)

SPEC-8.1.1, 8.1.2 och 7.4.4 säger nu det `vy-manifest` kodar och signerar: `module`
{hash, size, url?}, `resources` {hash, size, mime, url?, w?, h?} (inte {path, type, hash, size}),
`published` som ISO 8601-datum i text, `sig` {key_id, bytes}, hash som 32 byte i CBOR och
`sha256:<64 hex>` i JSON-formen, nyckel-id som de 16 första hextecknen av SHA-256 över nyckeln,
`icon`/`skeleton` som resursindex, `chunks` {hash, size, url?}, `next_key_id` som valfritt fält
(16 hextecken) för rotation, utelämnat `url` = `/_vy/<64 hex>` relativt manifestet, tomma listor
och mappar utelämnade i den kanoniska kodningen. Ingen kodändring; testerna som refererar
meningarna utökades med deras nya delar. Spec-frågorna 28, 29, 32, 33 och 35 är därmed avgjorda.

## Fynd 4 – rasteriseringen har en budget (`spec:`)

SPEC-5.3.3 fick en mening: värdens rasterisering av framens display list är bunden av samma
budget, räknad i ritade pixlar; överskrids den ritas resten inte, och överskridandet räknas och
levereras som `KIND_BUDGET` som ett i `vy_frame`.

| Frame, 1280 × 800 | Före, release | Före, debug | Efter, debug | Efter, release |
| --- | --- | --- | --- | --- |
| 65 000 helskärmsrektanglar | 13,2 s | ≈ 340 s | **0,39 s** (avbruten efter 65 ops) | 19 ms |
| 32 nästlade rundade klipp + 2 000 rektanglar | 8,6 s | 348 s | **0,31 s** | 14 ms |
| 32 nästlade rundade klipp + 65 000 rektanglar | 312 s (granskningen) | – | 0,27 s | 15 ms |
| 32 nästlade raka klipp + 2 000 rektanglar | 8,0 s | 333 s | 0,80 s | 21 ms |

Mätt på byggmaskinen i `vy-render` ensam (ingen modul). Testerna `spec_5_3_3_bench_*` kör de två
första i debug med taken 2 s respektive 1 s.

**Klippstacken** (`raster.rs::Clip`) är nivåer: varje `CLIP_PUSH` håller det analytiska snittet av
rektanglarna hittills, de pixlar ett rakt klipp täcker (`exact_pixels`, tiny-skias egen
26.6-avrundning i den icke-antialiasade fyllaren, prövad mot tiny-skias fyllning på 200 000
slumprektanglar i `exact_pixels_match_tiny_skia`) och en mask som byggs först när en op behöver
den: ett rakt klipps mask är en memset av de pixlarna, en rundad nivås mask är formens AA-fyllning
över dess egna gränser multiplicerad med föräldern (samma `premultiply_u8` som
`Mask::intersect_path`, så byten är desamma). Masken behålls tills nivån poppas: en push kostar
högst formens yta, en pop inget, och stacken byggs aldrig om. Ops skärs till klippets pixelgränser
innan de fylls (masken är noll utanför), men ritas fortfarande *genom* masken: tiny-skia blandar
en maskad fyllning (source-over med förskalad täckning) och en omaskad (source som lerp) längs
olika heltalsvägar som skiljer sig med en enhet vid AA-kanter, och guldbilderna är de maskade
byten. Den geometriska snabbvägen utan mask gav 13 avvikande guldbilder och är borttagen.

**Budgeten** (`RASTER_BUDGET_UNITS` = 64 × 2²⁰ enheter per frame; på dukar över en megapixel
64 × ytan, `raster_budget_units`): en enhet per pixel i en opak fyllning utan klipp (memset);
`PIPELINE_WEIGHT` = 16 per pixel som går genom rasterpipelinen (mask, genomskinlig färg, glyf,
bild, bana – mätt till 15–20 × memset i både release och debug); `MASK_WEIGHT` = 4 per pixel av
en rundad nivås gränser när masken byggs; `OP_UNITS` = 64 per op. Text debiteras byte × 16 i
förväg och glyfernas bitmappsyta × 16 efteråt; en strykning längd × bredd, en fylld bana sin
bounding box. När nästa op inte ryms avbryts listan, inget mer ritas, och `render` returnerar
`Rendered { image, report: RasterReport { truncated, ops_done, units, budget } }`.
`Runtime::raster_report` loggar och räknar ett avbrutet raster som ett budgetöverskridande – samma
`KIND_BUDGET` (`a` ≥ 9), streak och tvingande `vy_suspend`; en frame som redan bröt i bränsle
räknas en gång; streaken nollställs först vid nästa frame när både bränsle och raster höll.
`vy-cli render` anropar den efter sista framen och skriver enheterna. Guldbildernas tyngsta fall
ligger på 61 % av budgeten (`shop-400x700@2`: 43 M av 72 M, åtta bildkort med rundade klipp under
ett rullklipp vid skala 2), de flesta under 15 %; i release tar de tyngsta 16–38 ms, så enheterna
motsvarar ungefär 1,3 M per ms på byggmaskinen.

## Fynd 6 – `HttpCache`-tak och typsäkra beviljanden

- `HttpCache` (svar per URL, SPEC-7.3.2) har nu ett index `http/index` med logiskt löpnummer och
  storlek som blobcachen, räknar en träff som användning, kastar ut den äldsta posten när summan
  passerar `Options::http_cache_cap_bytes` (standard `DEFAULT_HTTP_CACHE_CAP_BYTES` = 64 MB),
  överlever omstart och släpper en skadad fil som miss. Test
  `http_cache_evicts_least_recently_used_at_cap`.
- `Manifest::verify` returnerar `Option<Trust>`, en typ bara `vy-manifest` kan skapa;
  `default_grant` är crate-privat och `CapabilityRequests::default_grants(trust, net_is_own)` är
  enda vägen från begäran till `Grant` – utan `Trust` är allt `Denied`. `Fetched` håller sin
  `Trust` privat och exponerar `trusted()` och `grants()`; `Fetched.trusted` som fält är borta.
  Capabilities-mappen förblir läsbar (värden behöver `net`-origins och `quota_kb`) men ger inga
  `Grant`. (`docs/notes/m3-proto-serve.md` beskriver det gamla läget med `trusted == false`.)

## Grindar och tal

207 gröna tester i workspace (var 189), 28 ignorerade (oförändrat). Spec-id 132/132. 50
konformansfall à 0,000 %, 26 semantiska träd lika (27 fall jämför), inga guldbyten. Importer 32. Inga nya
beroenden. Modulerna: 11/11 bygger, `test_modules` 11/11, `check-abi` 11/11. `abi-dump` =
`docs/ABI-1.0.md`.

## Spec-frågor (M2.7)

39. **Rasterbudgetens konstanter** (`RASTER_BUDGET_UNITS`, vikterna 16 och 4) är värdpolicy i
    `vy-render`, inte ABI, och står därför inte i förteckningen. SPEC-5.3.3 säger "samma budget"
    men enheterna växer med dukens yta över en megapixel (ett 4K-fönster får 64 lager, inte 16).
    Bör §13 nämna rasterenheter, eller räcker meningen?
40. **Tyngsta lagliga sidan ligger på 61 %** av budgeten (`shop-400x700@2`). Marginalen är 1,6 ×
    för den; 4 × eller mer för alla andra fall. Om specägaren vill ha mer marginal är
    `PIPELINE_WEIGHT` = 8 (hälften av mätt kostnad) eller 128 lager alternativen; båda gör de
    patologiska ramarna i debug 2 × långsammare (fortfarande under taken på byggmaskinen).
41. **Maskad mot omaskad fyllning** skiljer sig med en enhet vid AA-kanter i tiny-skia, så den
    geometriska snabbvägen för rektanglar under raka klipp (ingen mask alls) är borttagen för att
    hålla guldbilderna. GPU-vägen (M3) får egen avrundning och kräver ändå guldbyte; då kan
    snabbvägen tas tillbaka med nya guldbilder.
42. **Gestfönstret för `clipboard_read`**: 1 s på värdens klocka från `POINTER_DOWN`,
    `POINTER_UP`, `KEY_DOWN`, `KEY_UP` och `TEXT_INPUT`; pekarrörelse och rullning räknas inte.
    Headless går klockan 16 ms per frame, så fönstret är 62 frames. Bekräftas.
43. **Viktavbildningen** går till närmaste snitt med det tyngre vid lika avstånd: 501–549 blir
    DejaVu Sans där CSS hade valt Bold. Ingen referenssida använder vikter mellan 400 och 600.
    Bekräftas, eller byt till CSS-regeln (> 500 uppåt) före frysen.
44. **`vy_event` utanför 0/1** loggas på nivå debug, en gång per instans, och behandlas som 1.
    Alternativet (0, ingen omritning) hade dolt buggar i SDK-lösa moduler. Bekräftas.

# M3 del 1 – manifest, publish, proto, serve

Grenen `m3/proto-serve` (byggd parallellt med M2.6, egna noteringar i
[`docs/notes/m3-proto-serve.md`](notes/m3-proto-serve.md)) är inmergad: `vy-manifest` omskriven
som moduler med kanonisk CBOR (SPEC-4.1.3), identitet (SPEC-4.1.2), Ed25519-signatur (SPEC-7.4.1),
paketformat (SPEC-4.4.1), validering och policy (SPEC-8.2.3); nya crates `vy-publish` (keygen,
build, inspect), `vy-proto` (adresser, hämtning, cache, nyckelförtroende) och `vy-serve` (statisk
server) med ett end-to-end-test publish → serve → fetch → runtime över localhost (SPEC-7.2.1);
`Dockerfile` och `railway.toml`. Konflikten i `vy-manifest` löstes med grenens version som bas och
main:s tillägg (M2.5/M2.6: typad `Manifest`, `vy-cli` som läser manifest, bildstorlekar, capabilities
som map, `memory_max ≥ initialt + 1`) förda över som tillägg; main:s egen JSON-läsare utgick till
förmån för serde. De elva manifesten i `conformance/manifests/` är nu utkast (`Draft`) som
`vy-publish build` läser oförändrade. Beroenden, spec-frågor 27–38 och vad som stubbats står i
notesfilen, liksom avsnittet *Vid merge mot main* med allt som ändrades vid sammanslagningen.
Grindar och tal: 50 konformansfall vid 0,000 %, 184 gröna tester i workspace (var 146), 28
ignorerade (var 41), spec-id 131/131. Crates i `cargo tree --workspace`: 213 unika (var 169);
`vy-cli` 182 (var 164) eftersom den nu drar in grenens `vy-manifest` (serde_json, minicbor,
ed25519-dalek, zstd, wasmparser).

# M2.6 – SDK, spec-frågor 17–21, verktyg och frysförberedelse

Beslutslistan i `docs/reviews/0002-m2-sdk-referenssidor.md` är nu genomförd i sin helhet:
23–28 (SDK) och 32–34 (verktyg) i den här delen, 1–22 och 29–31 i M2.5. Spec-frågorna 17–21
är avgjorda av specägaren och införda. `docs/ABI-1.0.md` är den frysta förteckningen; specens
version- och statusrad är orörd – specägaren fryser.

## SDK (beslut 23–28)

| Beslut | Ändring |
| --- | --- |
| 23 | `Event::Capability { cap: Capability, granted }` med enum `Capability` (sex varianter, id ur SPEC-A.4.1, `from_id`/`id`/`name`); ett okänt id blir `Event::Unknown`. `chat` beror inte längre på `vy-abi`. |
| 24 | `Sem` har `node`, `node_with_id`, `begin`, `end` och `group`; rollhjälparna är borta. Elva sidor skriver `sem.node(Role::X, …)`; pixlar och träd oförändrade. |
| 25 | `Column`/`Row` utgår (ingen sida använde dem). README visar `layout()`-mönstret: rektanglarna räknas ur fönstret på ett ställe, `event` träfftestar och `frame` ritar samma rektanglar. README-exemplet är kompilerat mot wasm32 och dess testsnutt körd. |
| 26 | `vy_sdk::math`: `floor`, `round`, `ceil` via cast och jämförelse (exakta under 2²³, NaN/∞ oförändrade) och `snap(v, scale)` = `round(v × scale) / scale`. text, document och dashboard ritar 1 px-linjer med `snap`; tio guldbilder bytta (*pixelsnappning*). Vid skala under 0,5 rundas en hårlinje till 0 – `.max(1.0 / scale)` om det någonsin behövs. |
| 27 | `App::init -> Result<Self, VyError>`; `Err` loggas på nivå Error och blir `vy_init`:s returvärde, varpå värden avvisar sidan (`InitFailed`). De nio `unwrap_or(StyleId(0))` är borta. |
| 28 | `Frame::begin`/`end` är `pub(crate)`: bara exportlimmet skapar en `Frame`; nativa modultester kör `mock::run_frame(&mut sida, dt)`. `arena::bytes` ersätts av `arena::Dispatch` som limmet skapar per `vy_event`; `Event<'_>` med `TEXT_INPUT` lånar texten ur den och kan inte överleva utskicket. |

## Spec-frågor 17–21 (avgjorda)

- **17 Programmatisk fokus: avslås i 1.0.** SPEC-10.3.1: ingen `focus_request`-import; modulen
  följer värdens fokus, och en sida som vill flytta fokus ändrar trädet (noden som ska fokuseras
  registreras först i dokumentordning, dit Tab går från inget fokus). Sidorna form, chat,
  document och settings håller inte längre egen fokus: ingen fokusflytt vid fel (form), vid
  Escape, eller i egen pekar-träfftest; chat autofokuserar inte kompositören (fallen börjar
  med Tab); settings träfftestar vippknappsraden mot samma rektangel som dess nod. Fem
  guldbilder bytta (*modulen följer värdens fokus*).
- **18 Pekarfokus:** bekräftat och normativt i SPEC-10.3.1 – `FOCUS` levereras före `POINTER_DOWN`.
- **19 Fokuserbara roller:** bekräftade som implementerat (SPEC-10.3.2).
- **20 Försvunnen nod:** SPEC-10.3.2 – registreras den fokuserade noden inte i ett `vy_frame`
  levererar värden `FOCUS` ut (`b = 0`, `c = 0`) efter framen och inget har fokus.
  `Runtime::drop_focus_if_gone`, test `spec_10_3_2_focus_lost_when_node_disappears`.
- **21 `fetch_read` = 0 som slut:** SPEC-5.2.7 – värden buffrar hela kroppen (högst 64 MB, annars
  `E_TOO_LARGE` och ingen kropp) innan `FETCH_DONE`; `fetch_read` kopierar i ordning och
  returnerar 0 först vid slutet, som frigör kroppen. Provat på riktigt mot headless-transporten
  (nedan), inte bara nekad väg: `spec_5_2_7_fetch_read_zero_only_at_end`.

## Värden headless (beslut 32, 34)

- **Lagring i minnet** när `storage` beviljas (SPEC-5.2.9): `state_get` på saknad nyckel är
  `E_RESOURCE`, hela längden rapporteras, högst `max` kopieras; `state_delete`; kvot
  `Options::storage_quota_bytes` (manifestets `quota_kb` × 1024 i sviten, annars 1 MB enligt
  SPEC-8.2.3) ger `E_QUOTA`. `Runtime::take_storage` flyttar lagringen till nästa instans.
- **Inspelad hämtning** (`Options::fetch`, SPEC-5.2.7): `fetch_begin` till en URL med inspelat
  svar får ett `req_id`; `Runtime::fetch_done` levererar `FETCH_DONE` för den äldsta väntande.
  `fetch_poll` är 0 tills dess och 1 därefter; `fetch_read` före `FETCH_DONE` eller mot en stängd
  begäran är `E_ARG`; 65:e öppna begäran `E_RATE`. Utan inspelat svar nekas som förr.
- **Origin-grind** (SPEC-5.2.8): `Options::net_origins` ur manifestet; främmande origin ger
  `E_ORIGIN` utan anrop, icke-https ger `E_ARG`; tom lista (inget manifest) tillämpar ingen grind.
  Testet `spec_5_2_8_origin_gate` är un-ignorerat.
- **Loggnivåer**: nekad capability är nivå 2 (SPEC-5.3.1); "nekad för att headless saknar
  implementation" (navigate, urklipp, fetch utan inspelat svar) och "nyckel saknas" är nivå 0.
- **Budgetsimulering**: `Runtime::force_budget_breach` rapporterar nästa frame över budget
  (`a = 9`) med samma svit, streak och tvingande suspend som en riktig.

## Konformansrunnern (beslut 32)

`events` tar nu även värdhändelser, var och en med valfritt `after_frames = N` (högst N frames
mellan föregående händelse och denna): `visibility` (inga frames gömd), `budget`, `resize`
(sista storleken är bildens), `suspend`, `reinit` (ny instans med den gamlas lagring; loggarna
behålls), `fetch_done`. `fetch = [{url, status, body}]` är fallets inspelade svar. Manifestets
`memory_max`, `net.origins` och `storage.quota_kb` tillämpas på körningen. Efter ett tvingande
suspend körs inga fler frames; en händelse därefter är ett fel i fallet. `vy-cli render` tar
`--resource <fil>` (resurs-id 1, 2, …) och `--resource-size WxH`.

Fem nya fall: `settings-800x600@1-reloaded` (persistens över `vy_suspend` och ny `vy_init`),
`game-800x600@1-paused` (gömd 20 frames efter serven, visad igen: pausöverlägg vid 0,3 s),
`game-800x600@1-budget` (BUDGET efter tio frames: stjärnfält och svans borta),
`document-1280x800@1-to-400x700` (omflöde: bild och träd identiska med `document-400x700@1`),
`chat-800x600@1-delivered` (inspelat svar och `FETCH_DONE`: "levererat"). Runnertester i
`vy-cli` för varje steg.

## Verktyg och grindar (beslut 33) och förteckningen

- `tools/test_modules.sh`: varje moduls `cargo fmt --check`, nativa `cargo test` och
  wasm32-clippy; i CI i både `gates` (Ubuntu) och `platform` (macOS, som fick rustfmt och clippy).
- `docs/ABI-1.0.md` genereras av `vy-cli abi-dump` ur `vy-abi` (nya tabeller: `err::ALL`,
  `event::ALL`, `modifier::ALL`, `op::ALL`/`RESERVED`, `Sig::args`, `Ty::name`) och är en grind:
  `spec_5_3_5_abi_listing_is_frozen` fallerar när filen avviker.

## Grindar och tal

50 konformansfall (var 45), 26 semantiska träd (var 23). Guldbyten: 10 (pixelsnappning), 5
(fokus), 2 bilder + 2 träd (headless lagring), 5 nya. Tester: 146 gröna i workspace (var 133),
41 ignorerade (var 42: 5.2.8 un-ignorerat). Spec-id: 131, alla spårade. Importer: 32. Inga nya
beroenden. 16 commits sedan M2.5.

## Spec-frågor (M2.6)

22. **`fetch_poll`:s returvärde** är ospecificerat i SPEC-5.2.7. Headless: 0 tills `FETCH_DONE`
    levererats, 1 därefter, `E_ARG` för en begäran som inte är öppen. Bör in i specen (eller
    importen strykas: `FETCH_DONE` bär redan allt).
23. **`fetch_read` före `FETCH_DONE`** ger `E_ARG` headless ("inte läsbar än"); `E_RATE` eller
    0-utan-stängning vore alternativen. Specen säger inget. Samma för `fetch_begin` med en
    adress som inte är `https://`: `E_ARG`.
24. **Origin-grind utan manifest.** Headless tillämpar SPEC-5.2.8 bara när en origin-lista finns
    (`vy-cli render --grant net` har inget manifest). Skalet har alltid ett manifest, så frågan
    är bara om `render` ska kräva `--manifest`.
25. **Fokus-ut utan orsak.** `FOCUS` för en försvunnen nod bär `c = 0` (som pekare). En tredje
    orsakskod (2 = trädet) vore tydligare men SPEC-A.2.1 låser argumenten; `c = 0` valdes.
26. **`E_RESOURCE` från `state_get` loggas** på nivå 0. SPEC-5.3.1 kräver logg bara för nekade
    capabilities; en saknad nyckel är sidans normala väg. Bör nivåerna stå i specen?

# M2.5 – ABI-ändringar före frysning

Beslutslistan i `docs/reviews/0002-m2-sdk-referenssidor.md` genomförd för beslut 1–22 och
29–31 (ABI och spec-text); 23–28 och 32–34 (SDK och verktyg) är kvar. Specen är v0.3,
*ABI-kandidat inför frysning*. Varje spec-ändring är ett eget `spec:`-commit med sina tester;
guldbyten är egna `golden:`-commits.

## Vad som ändrades i ABI:t

| Beslut | Ändring | Var |
| --- | --- | --- |
| 1 | 17 importer som kunde misslyckas returnerar `i32`: `log`, `dl_begin`, `dl_end`, `dl_rect`, `dl_image`, `dl_path`, `dl_clip_push`, `dl_clip_pop`, `ime_begin`, `ime_end`, `fetch_cancel`, `state_delete`, `navigate`, `link_register`, `random`, `clipboard_write`, `sem_node`. `request_frame` och `exit` kan inte misslyckas och förblir utan returvärde. `dl_begin` i en öppen lista ger `E_DL_STATE`. | SPEC-5.2.*, `vy_abi::IMPORTS`, `host.rs`, SDK |
| 2, 30 | `dl_text -> i64`: bredd × 100 högt, höjd × 100 lågt, samma packning som `text_measure`; varje halva ≤ 2³¹ − 1 så en negativ `i64` alltid är ett fel. `Frame::text` ger `Size`. | SPEC-5.2.4, 5.2.5, `pack_x100` |
| 3 | `dl_clip_push(x, y, w, h, radius)`; `CLIP_PUSH` är 21 byte. Rasteriseraren bygger masken ur rundade former så snart en radie finns på stacken (anti-aliasad), annars den gamla kantskarpa rektangelsnittningen. gallery och shop klipper bildkort rundat (8 guldbilder bytta). | SPEC-5.2.3, C.1.1, `raster.rs::Clip` |
| 4 | `pointer_get` utgår (33 → 32 importer). | SPEC-5.2.6 |
| 5 | `LINE` (0x07) och `SHADOW` (0x08) utgår; numren reserverade; `MAX_SHADOWS_PER_FRAME` borta; SPEC-C.1.3 *utgått*. | bilaga C |
| 6 | `POINTER_LEAVE` = 9 utan argument; `Event::PointerLeave`; `pointer_leave` i `cases.toml`; dashboard släcker verktygstipset (nytt fall `dashboard-1280x800@1-leave`). | bilaga A |

## Vad som ändrades i värden (spec-text med test)

- **`dt_ms`** (SPEC-5.1.2, beslut 8): `Runtime::frame()` tar inget argument utan räknar `dt_ms` ur
  sin klocka: 0 i första framen efter `init` och efter `VISIBILITY` synlig, annars klockans
  framsteg sedan föregående frame (headless 16 ms). `vy-cli`:s `HEADLESS_DT_MS` utgår. Tester i
  `vy-runtime` som använde `dt_ms` som kanal in i modulen kör `frame_with_dt` (bara `cfg(test)`).
- **Tangentläge** (SPEC-5.2.6, beslut 9): `key_down` hålls till `KEY_UP` och nollställs vid
  `VISIBILITY` dold. Fokusförlust är skalets sak (M3); enklast genom att skalet skickar `KEY_UP`
  för hållna tangenter – då stämmer modulens egen bokföring också.
- **Dold flik** (SPEC-6.2.1, beslut 10): `wants_frame()` är falsk dold utan `persist_frame`,
  begäran kvarstår; `frame()` svarar `Error::Hidden` om värden ändå försöker.
- **`vy_alloc`** (SPEC-5.1.6, beslut 12): 0 eller en pekare utan plats loggas
  (`TEXT_INPUT dropped: …`) och händelsen släpps; `Error::Alloc` utgår.
- **Fokus** (SPEC-10.3.1–10.3.3, 10.1.1, beslut 15, 16, 19): värden äger fokus. `Runtime::event`
  konsumerar Tab/Skift+Tab (även `KEY_UP`), går genom trädets fokuserbara noder (`link`,
  `button`, `textbox`, `checkbox`, `radio`, `menuitem`, `tab`, `slider`) i dokumentordning med
  omlopp och levererar `FOCUS` ut/in med `c = 1`. Ett `POINTER_DOWN` hit-testar trädet och
  flyttar fokus (`c = 0`) *före* trycket – samma ordning som webbläsare (mousedown → focus →
  click) och den ordning som gör att sidornas egen klickfokus och värdens sammanfaller, så inget
  guldbyte behövdes. Ett scriptat `focus` i `cases.toml` sätter värdens fokus. Modulerna form,
  document och settings tappade sin Tab-hantering (nås aldrig). Enter/Escape är `KEY_DOWN`;
  `KIND_ACTIVATE` avslås. Modulen ritar fokusringen.
- **Länkar per frame** (SPEC-5.2.10, 9.3.2, beslut 17): `dl_begin` tömmer länklistan; samma id
  ersätter; högst 4 096 per frame (`E_DL_FULL`). 4 096-taket per instans är borta.
- **Dubbla nod-id** (SPEC-5.2.13, beslut 18): `E_ARG` i värden och mocken. SDK:t gör fasta id
  negativa (`node_with_id` kräver `id < 0`) och automatiska positiva; sju semantiska träd bytta.
- **Avkodning utanför budgeten** (SPEC-5.3.3, 6.5.1, beslut 20): `dl_image` debiterar bara
  opkodens byte. `IMAGE_READY` avslås; M3 avkodar utanför UI-tråden.
- **Bilaga A** (beslut 21): capability-id 1–6 (SPEC-A.4.1) och modifierarbitar Skift 1, Ctrl 2,
  Alt 4, Meta 8 (SPEC-A.5.1).
- **Spec-text utan kodändring**: SPEC-6.4.3 `w ≤ 0` = ingen radbrytning (beslut 11); SPEC-5.2.9
  `state_get` på saknad nyckel = `E_RESOURCE` (13, mocken gör det; headless har ingen lagring);
  SPEC-5.2.7/A `FETCH_DONE.status`, 64 öppna → `E_RATE`, kroppen frigörs av `fetch_cancel` eller
  `fetch_read` = 0 (14, transporten är M3).

## Manifestet (beslut 7, 22, 29, 31)

- `vy-manifest` har nu en typad `Manifest` ur JSON-formen (`conformance/manifests/*.json`), en
  egen JSON-läsare (`json.rs`, RFC 8259, under 200 rader, ingen ny beroende) och validering av
  §4 och §8: obligatoriska fält med gränser, okända fält avvisas, `abi` major = 1,
  `memory_max` 1–4096, resurstyper enligt SPEC-4.1.5, capabilities som map namn → objekt med
  gränser. Sex M1-tester är un-ignorerade (4.1.1, 4.3.1, 4.3.2, 4.5.1, 8.1.1, 8.1.2).
- **Bildstorlekar** (SPEC-8.1.2, 6.5.1): `w`/`h` är obligatoriska för `image/*`-resurser och
  förbjudna för andra. `Manifest::check_resource_sizes` jämför med PNG-huvudet (test mot
  gallery- och shop-filerna). Värden (`vy_render::Resource { bytes, pixels }`) avvisar en bild
  vars avkodade storlek avviker: `E_RESOURCE` med båda storlekarna i loggen. `cases.toml` tar
  `resources = [{path, w, h}]` (strängformen utan kontroll finns kvar); `resource_info` avslås –
  modulen har storlekarna vid byggtid (gallery: `PICTURES`).
- **`memory_max ≥ initialt + 1`** (SPEC-8.1.3): `Manifest::check_memory(initial_pages)`,
  `Runtime::load` vägrar annars (`Error::MemoryMax`), `check-abi` varnar (`warnings` i
  `AbiReport`) när initialt minne är ≥ 4095 sidor. Alla elva moduler: 18 sidor, manifest 32–64.
- **Capabilities som map** (SPEC-8.2.1): alla elva manifest i formen
  `{"ime": {}, "net": {"origins": [...]}}`; top-level `net`/`storage` avvisas. Fyra manifest som
  saknades är tillagda (gallery, list, game, dashboard).
- **Sviten läser manifestet** (SPEC-10.4.2, 8.2.2): varje fall laddar `manifests/<modul>.json`,
  kontrollerar `memory_max` mot modulens initiala sidor, att fallet bara beviljar vad manifestet
  begär, och att en sida med `textbox`-noder begär `ime`. Fallens `resources`-listor och
  manifestets `resources` är ännu två källor; en förenkling (läs sökvägar ur manifestet) är
  möjlig i M3 när paketladdaren finns.

## Grindar och tal

44 → 45 konformansfall (nytt: `dashboard-1280x800@1-leave`); 23 semantiska träd. Guldbyten:
8 bilder (rundade klipp), 1 ny bild (POINTER_LEAVE), 7 träd (negativa fasta id). Importer: 32.
Tester: 133 gröna i workspace (var 104), 42 ignorerade (var 48). Spec-id: 131 (var 127; nya
10.3.2, 10.3.3, A.4.1, A.5.1), alla spårade. Inga nya beroenden.

## Spec-frågor (M2.5)

17. **Programmatisk fokus saknas.** Värden äger fokus (SPEC-10.3.1), men en modul kan inte be om
    det: `form` vill fokusera namnfältet vid fel och `set_focus(Name)` är då bara sidans egen
    bokföring – värdens fokus ligger kvar på knappen tills nästa Tab eller klick. Det syns inte
    i någon guldbild (modulen ritar ringen där den själv tror fokus är) men Tab-ordningen utgår
    från värdens nod. Antingen en import `focus_request(id) -> i32` (33 importer, kräver
    hotanalys: en sida kan inte stjäla fokus från adressfältet eftersom trädet bara täcker
    sidans yta) eller en regel att modulen följer värden. Bör avgöras före frysen.
18. **Pekarfokus i värden** (SPEC-10.3.1) är byggagentens tillägg till beslut 15: utan det hade
    Tab efter ett klick utgått från ingenting. Ordningen FOCUS före POINTER_DOWN är vald så att
    aktivering på `POINTER_DOWN` sker med fokus redan satt. Specägaren bör bekräfta ordningen.
19. **Fokuserbara roller** (SPEC-10.3.2) är listan ovan; `listitem`, `cell` och `tab`-paneler
    är inte fokuserbara i 1.0. En sida som vill ha fokus på en rad gör den till `button` eller
    `link`. Bekräftas.
20. **Fokus över frames.** Värden håller ett nod-id; försvinner noden ur trädet startar Tab om
    från början. Ett `FOCUS` ut skickas inte när noden försvinner (modulen vet redan). Bekräftas.
21. **`fetch_read` = 0 som slut** (SPEC-5.2.7) antar att en tom läsning bara sker vid slutet;
    M3:s transport måste blockera eller returnera `E_RATE`/`0` konsekvent. Avgörs med M3.

# M2 (första halvan) – SDK, referenssidor, bilder

## Nya beroenden

Inga nya crates. `vy-sdk` beror bara på `vy-abi`; modulerna bara på `vy-sdk`. Bildavkodningen
använder `png` som redan fanns (M1). Den begränsade TOML-läsaren i `vy-cli` fick arrayer och
inline-tabeller i stället för ett TOML-beroende (fortfarande under 200 rader).

## Vad som byggdes

| Del | Innehåll |
| --- | --- |
| Spec v0.2-ikapp | `dl_image` tar `fit` (SPEC-5.2.3); IMAGE ritas från avkodade PNG-resurser (SPEC-6.5.1); `vy_abi::cap` (capability-id) och `vy_abi::modifier`; `Capabilities::from_names`. |
| `crates/vy-sdk` | `no_std` + `alloc`; typade bindningar för alla 33 importer; `Frame` med lokal validering; `Sem`; `Column`/`Row`; `Event`; `App` + `export!`; `arena` (vy_alloc), `heap` (global allokator), `panic_handler`; `MockHost` nativt. README med exempel. |
| `modules/hello` | Porterad till SDK:t, samma pixlar (0,000 %) och samma semantiska träd. |
| `modules/text` | 3 277 ord fyllnadssvenska i 99 block; cachade blockhöjder, klipp, rullning via SCROLL och tangenter, bara synliga block i trädet. |
| `modules/form` | Två textfält (fokus, `ime_begin`/`ime_end`, TEXT_INPUT, Backspace), kryssruta (bocken är en `dl_path`), knapp, validering, tackpanel med `format!` på SDK-heapen. |
| `vy-cli` | `capabilities = [..]` och `events = [..]` i `cases.toml`; `render --grant`; SPEC-10.4.2-kontrollen; SPEC-12.1.2-täckningstest. |
| CI | `platform`-jobb på macOS: bygge, test, konformans; artefakt med `.png.new` vid avvikelse. |

Storlekar: `hello` 5 061 byte (var 1 147), `text` 30 561, `form` 16 365. Ökningen i `hello` är
SDK-glue, allokator och den 64 KB stora `vy_alloc`-arenan (i bss, kostar inte bytes men en sida).

## Bilder (SPEC-6.5.1)

- Resurser kommer in via `Options.resources: Vec<Vec<u8>>` tills manifestläsaren finns (M3);
  `resources[i]` är resurs-id `i + 1`, id 0 är modulen (SPEC-4.5.1).
- Avkodning sker lat vid första `dl_image` som namnger resursen, i värdimporten, och cachas per
  instans (`vy_render::ImageCache`). Även ett misslyckande cachas så en trasig resurs kostar ett
  försök, inte ett per frame.
- Sidor över 8 192 px avvisas på IHDR innan någon pixelbuffert allokeras; id 0, okänt id,
  bytes som inte är PNG och en full cache är alla `E_RESOURCE` (loggat med orsak). Opkoden
  skrivs inte, så rasteriseraren ser aldrig en okänd resurs från en runtime; en display list som
  renderas utanför en runtime (tester) får grå platshållare som förr.
- **WebP och AVIF** (SPEC-4.1.5) avkodas inte i M2 och ger `E_RESOURCE`; de kräver nya
  beroenden (`image-webp`, en AVIF-avkodare) som får motiveras när en referenssida behöver dem.
- **Cachetak**: `MAX_IMAGE_CACHE_BYTES` = 256 MB avkodat per instans (= en maximal bild) så ett
  paket med 32 MB PNG (SPEC-4.1.5) inte kan få värden att hålla gigabyte (SPEC-9.3.1). Värdpolicy,
  inte ABI.
- **Bränsle**: resursbyte som lästes vid avkodning debiterades i M2 (specfråga 11); sedan M2.5
  debiteras avkodningen inte (beslut 20, SPEC-5.3.3).
- **Sampling** är närmaste granne i heltals- och IEEE-754-aritmetik: `cover` skalar likformigt
  och beskär, `contain` skalar likformigt och lämnar rest, `stretch` skalar per axel; destina-
  tionspixlar vars centrum ligger i snittet av rektangel, ritad bild och duk fylls. Bilinjär och
  mipmappad sampling kommer med GPU-vägen (M3).

## SDK-beslut

- **Ett sätt per sak**: `dl_*` finns bara via `Frame`, inte som fria funktioner; `Frame` finns
  bara mellan `dl_begin` och `dl_end`, så SPEC-5.3.2 kan inte brytas från Rust.
- **Lokal validering före värden**: opkodsräkning, klippdjup, text-/sökvägs-/url-/nyckeltak och
  nodantal speglar `vy-dl` och `vy_abi::limits`. Ett nekat anrop når aldrig värden; värden förblir
  auktoriteten (testet `spec_9_3_2_byte_caps_checked_locally` visar tom värdlogg).
- **Semantik medan man ritar**: `Frame::sem` numrerar noder i anropsordning från 1; `node_with_id`
  för fasta id (formulärets 10–13) så `KIND_FOCUS` kan peka på dem. `Frame::group` nästlar
  föräldern och ger hela framen till closuren (en `Sem::group`-closure kan inte rita – lånet).
- **`vy_alloc`** är en 64 KB bump-arena som börjar om när den är full; en buffert lever tills
  arenan slagit runt, vilket aldrig sker inom den händelse som bär den. Nativt är "pekaren" ett
  offset + 65 536 så ett lyckat anrop aldrig returnerar 0 (SPEC-5.1.6).
- **Global allokator** (`heap`): första passning i en adresssorterad frilista över sidor från
  `memory.grow`, sammanslagning vid frigöring, högst 16 bytes justering. Modulens initiala
  minne rörs aldrig; vid `memory_max` (SPEC-4.2.4) returneras null och sidan trappar i stället
  för att skriva vilt. Testad nativt över en arena.
- **Mocken** formar text med fast modell (tecken = storlek × 0,6, rad = storlek × 1,25) och
  nekar som värden; pixlar bara från `vy-cli`.
- `Options.capabilities` var redan en `Capabilities`-struct med bool-fält; i stället för en
  parallell `Vec<String>` fick den `from_names`/`grant` så att `cases.toml` och `--grant` talar
  namn och runtimen en representation (princip 1).

## Händelser i konformansfall

`events = [{kind = "pointer_down", x = 300, y = 183}, …]` med `pointer_move/down/up`,
`key_down/up`, `text_input`, `scroll`. `render` levererar dem efter första framen och kör en
frame efter varje som svarar 1, inom taket `frames` (standard 1 + antal händelser). Klockan
(`now_ms`) går 16 ms per frame som förr. Värdens egen fokusordning (SPEC-10.3.1, `KIND_FOCUS`)
finns inte headless än; formuläret hanterar eventet men inget fall skickar det.

## Approximationer och öppet i M2:s andra halva

- Enbart DejaVu Sans har fet vikt; serif/mono med vikt ≥ 600 matchas av fontdb till Sans Bold
  (sedan M2.7 väljer `vy-render` snittet själv: serif och mono är alltid sitt enda snitt).
  `text` begär därför sans för titeln. Fet serif/mono: fler typsnittsfiler eller syntetisk
  fetstil (M3).
- `text_caret` layoutar utan radbrytning (M1); formulärets fält är enradiga så det räcker.
- `spec_13_4_1_sdk_refuses_slow_module` (`vy publish --allow-slow`) väntar på publiceringsverktyget.
- Återstående referenssidor (tio i M2) och ABI-frys (SPEC-5.3.5) är andra halvan.
- Guldbilderna är tagna på Linux x86_64. macOS-jobbet rapporterar; se specfråga 13.

## Spec-frågor (M2)

11. **SPEC-6.5.1 / 5.3.3 – avkodning i framebudgeten.** Lat avkodning vid första `dl_image`
    lägger värdarbetet i den frame som ritar bilden. Debiteras lästa byte som övrigt värdarbete
    (32 enheter/byte) bryter varje PNG över 250 KB 8 ms-budgeten och varje över 2 MB den hårda
    gränsen, fast specen tillåter 8192 × 8192 px och 32 MB resurser. Förslag: avkoda vid
    paketladdning (före `vy_init`) eller asynkront med en `IMAGE_READY`-händelse; tills dess
    gäller debiteringen ovan.
12. **SPEC-A.2.1 – odefinierade tal.** `capability-id` i `KIND_CAPABILITY` och bitarna i
    `modifierare` saknar värden i specen. Värden använder `vy_abi::cap` (1–6 i SPEC-8.2.1:s
    ordning) och `vy_abi::modifier` (Shift 1, Ctrl 2, Alt 4, Meta 8). Bör in i bilaga A.
13. **SPEC-2.1.5 / 12.2.2 – guldbilder över plattformar.** Rasteriseraren (tiny-skia med `simd`)
    och textmotorn är flyttalsbaserade; macOS-jobbet avgör om samma guldbilder håller på ARM.
    Avviker de rapporteras diffkvoten här och tröskeln lättas inte.
14. **SPEC-C.1.1 / 5.2.3 – LINE och SHADOW utan import.** Display listen har opkoderna `LINE`
    (0x07) och `SHADOW` (0x08) men §5.2 har inga `dl_line`/`dl_shadow`. En modul kan alltså inte
    producera dem (linjer går via `dl_path` med strykning). Antingen tillkommer två importer
    (ryms: 35 av 40) eller så utgår opkoderna.
15. **SPEC-10.4.2 – vad "utan ime-capability" syftar på.** Manifestet läses inte förrän M3;
    sviten kontrollerar i stället att ett fall med `textbox`-noder beviljar `ime`. När manifestet
    finns bör kontrollen flyttas dit (begärd, inte beviljad).
16. **SPEC-5.1.6 – buffertens livslängd.** Specen säger inte hur länge en `vy_alloc`-buffert
    måste vara giltig. SDK:t antar "tills händelsen som bär den är levererad"; bör stå i specen.

---

# M1 – headless rendering

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

`anyhow` och `wasmparser` fanns sedan M0. `vy-runtime` använder en egen `Error`-enum, inte anyhow
(beroendet togs bort ur `vy-runtime/Cargo.toml` efter granskning 0001, fynd 6).

`rust-version` i workspace höjdes från 1.80 till 1.97: wasmtime 50 kräver 1.97 och cosmic-text 0.19
kräver 1.89. `rust-toolchain.toml` pekar på `stable`.

## Typsnitt

`crates/vy-render/fonts/` innehåller DejaVu Sans, DejaVu Sans Bold, DejaVu Serif och DejaVu Sans Mono
(Bitstream Vera-licens, kopierad till `fonts/LICENSE`). De bäddas in med `include_bytes!` (2,2 MB) och
är de enda typsnitt klienten någonsin laddar; systemtypsnitt rörs aldrig (SPEC-2.1.5, SPEC-6.4.2).

| Roll | Resurs-id | Typsnitt |
| --- | --- | --- |
| `sans` | −1 | DejaVu Sans (+ Bold för vikt ≥ 600; sedan M2.7 ≥ 550, se *Typsnitt* under M2.7) |
| `serif` | −2 | DejaVu Serif |
| `mono` | −3 | DejaVu Sans Mono |
| `ui` | −4 | DejaVu Sans |
| `emoji` | −5 | DejaVu Sans – riktig färgemoji kommer i 1.x |

Radhöjd är `size_px × 1.25` (`vy_render::text::LINE_HEIGHT_FACTOR`). Locale i textmotorn är fast
(`sv-SE`) och fallback-listan är de tre inbyggda familjerna i fast ordning, oberoende av plattform.

## Approximationer i M1

- **SHADOW** ritades i M1 som 8 koncentriska rundade rektanglar; opkoden utgick i M2.5 (beslut 5).
- **IMAGE** ritades i M1 som en grå platshållare; sedan M2 avkodas PNG (se M2 ovan).
  Platshållaren finns kvar för resurser som inte avkodats.
- **PATH** med `stroke_w = 0` fylls med nonzero-regeln; `> 0` strukes med butt-ändar.
- **Klippstacken** är rektanglar, sedan M2.5 med hörnradie; masken byggs om när stacken ändras.
- `dl_text` layoutar texten en gång för höjden och rasteriseraren en gång till; ingen glyfcache
  utöver cosmic-texts egen. Prestandabudgeten (SPEC §13) mäts från M3.
- `text_caret` layoutar utan radbrytning.
- `max_w ≤ 0` i `dl_text`/`text_measure` betyder ingen radbrytning (normativt sedan v0.3, SPEC-6.4.3).

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

### Värdarbete debiteras (granskning 0001, fynd 1)

Bränslet mätte bara wasm-instruktioner: ett `text_measure` på 64 KB kostade några enheter men
hundratals millisekunder värd-CPU, och 65 536 sådana per frame var tillåtna. Nu debiteras varje byte
som värden formar, kodar, kopierar eller genererar åt modulen med `vy_runtime::FUEL_PER_HOST_BYTE`
= 32 ur samma bränsle som instruktionerna:

| Import | Debiteras |
| --- | --- |
| `dl_text` | textens byte (formning) + opkodens byte i display listen |
| `text_measure`, `text_caret` | textens byte |
| övriga `dl_*` | byte som skrevs till display listen (25 för `RECT`, 1 för `END`, …) |
| `log`, `sem_node`, `link_register`, `navigate` | byte som kopierades |
| `random` | byte som genererades |

Ett nekat eller avvisat anrop kostar inget. Värdet är en konvention som `FUEL_PER_MS`: lågt nog att
en full display list (65 536 × 25 byte ≈ 52 M enheter) ryms under den hårda gränsen (64 M), högt
nog att fyra 64 KB-formningar (4 × 65 535 × 32 > 8 M) bryter 8 ms-budgeten. Det underskattar
verklig formningskostnad; textcachen (SPEC-13.2.3, M3) gör räkningen rättvisare.

wasmtime kontrollerar bränslet bara vid funktionsingång och loophuvud, så en import som tömmer det
utlöser själv `Trap::OutOfFuel`; modulen avbryts då exakt som vid en wasm-instruktion utan bränsle
och `frame()` svarar `Error::OverBudget`. `FrameStats::host_fuel` visar värdens andel.

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
   `vy-cli render --frames N` följer samma regel (granskning 0001, fynd 3): frame 1 körs alltid,
   därefter bara så länge modulen begärt en ny; `N` är ett tak, inte ett antal, och utskriften
   anger hur många som kördes.
4. **SPEC-5.3.3 / varning vid första överskridandet.** Meningen nämner bara det andra; eventtabellen
   har "1 = sista varningen" vilket förutsätter en tidigare. Tolkning som ovan: varning (`c = 0`)
   vid första, sista varning (`c = 1`) plus suspend vid andra.
5. **SPEC-5.2.6 / `ime` som capability.** Importerna `ime_begin`/`ime_end` saknar
   capability-markering i §5.2 men `ime` är en capability i SPEC-8.2.1. Tolkning: de kräver `ime`
   och nekas med `E_DENIED` i loggen annars.
6. **SPEC-5.2.10 / `navigate` headless.** Ingen capability i specen, men en headless-värd kan inte
   navigera. Tolkning: loggas som nekad i headless-läge; `vy-shell` (M3) kopplar in den.
7. **SPEC-5.2.3 / `dl_image` och `fit`.** Avgjord i v0.2: `dl_image` har `fit` (M2).
8. **SPEC-5.2.11 / CSPRNG headless.** Se ovan: headless använder seedad PRNG för determinism.
9. **SPEC-5.2.6 / `pointer_get`.** Utgick i M2.5 (beslut 4).
10. **SPEC-9.3.1 / bytetak – föreslagen SPEC-9.3.2.** Meningen räknar upp minne, CPU, nätverk och
    lagring men inte hur mycket värden får kopiera ur modulminnet per anrop; utan tak kunde en modul
    få värden att allokera hela sitt minne per `sem_node` (65 536 × 16 MB per frame) och display
    listen växa till 4 GB (granskning 0001, fynd 2). Värden sätter därför följande tak, namngivna i
    `vy_abi::limits`. Ett `len` över taket ger `E_ARG`, loggas med taket och kopierar ingenting;
    ett anrop som skulle fylla trädet eller frame-budgeten ger `E_DL_FULL`. Förslag till ny
    normativ mening, icke-normativ tills specägaren tar in den:

    > **[SPEC-9.3.2]** Värden kopierar högst dessa byte ur modulminnet per anrop: `log` 4 KB;
    > `dl_text`, `text_measure`, `text_caret` 65 535; `dl_path` 65 535 × 9; `sem_node` 4 KB;
    > `navigate`, `link_register` och `fetch_begin`-URL 2 KB; `fetch_begin`-kropp 1 MB;
    > `state_*`-nyckel 256 byte; `state_put`-värde 1 MB; `clipboard_write` 1 MB. Ett `len` över
    > taket ger `E_ARG` utan kopia. Per frame håller värden högst 65 536 semantiska noder och
    > högst 16 MB text i semantiskt träd och display list tillsammans; överskott ger `E_DL_FULL`.

    | Konstant | Värde | Varför just det |
    | --- | --- | --- |
    | `MAX_LOG_BYTES` | 4 096 | en loggrad |
    | `MAX_TEXT_BYTES` | 65 535 | `TEXT`-opkodens `len: u16` (SPEC-C.1.1); samma tak för mätning så att allt som mäts kan ritas |
    | `MAX_PATH_BYTES` | 65 535 × 9 | `PATH`-opkodens `n: u16` (SPEC-C.1.1, SPEC-C.1.2) |
    | `MAX_SEM_TEXT_BYTES` | 4 096 | ett tillgängligt namn eller textstycke |
    | `MAX_URL_BYTES` | 2 048 | vanlig URL-gräns |
    | `MAX_FETCH_BODY_BYTES` | 1 MB | större kroppar strömmas i 1.x |
    | `MAX_STATE_KEY_BYTES` / `MAX_STATE_VALUE_BYTES` | 256 / 1 MB | kvoten är högst 16 MB (SPEC-5.2.9) |
    | `MAX_CLIPBOARD_BYTES` | 1 MB | |
    | `MAX_SEM_NODES_PER_FRAME` | 65 536 | samma som opkoder (SPEC-6.1.6) |
    | `MAX_FRAME_TEXT_BYTES` | 16 MB | träd + display list; trädet lever till nästa `vy_frame`, listan till nästa `dl_begin` |

    Kapabilitetsgrinden (SPEC-5.3.1) kommer före taket: ett anrop utan `net`/`storage`/`clipboard`
    får `E_DENIED` oavsett längd. Länkar (`link_register`) är dessutom högst 4 096 per instans och
    loggen högst 10 000 rader (`vy_runtime::host`).
