# M5 examples – sex större exempelsidor

Byggagentens noteringar för grenen `m5/examples` (från `m5/sdk` 1de409e). Granskningssidan skrivs av en
annan agent. Ägda delar: `modules/{kanban,kalkyl,rita,skriv,block,rymd}`, deras utkast i
`conformance/manifests/`, deras fall, guldbilder och träd i `conformance/`, `docs/EXEMPEL.md` och den
här filen. Det som rör andra spår står under *Berör andra spår*.

Varför: ägaren vill ha mer avancerade sidor att köra i klienten än referenssidorna. De visar vad
formatet klarar, prövar att komponenterna i `vy_sdk::ui` räcker för riktiga appar, och publiceras på
den publika sajten (`tools/publish_site.sh` tar varje utkast i `conformance/manifests/`).

## Läget

Allt i uppdraget är klart: sex sidor, `docs/EXEMPEL.md` och den här filen.

| # | Sida | Modul | wasm | Bränsle per frame (av 8 M) | Nativa tester | Fall | Commits |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | Tavlan | `kanban` | 54 267 byte | 0,11 M; 1,1 M med 60 kort | 10 | 5 | 2188796, 94e6aa2 |
| 2 | Kalkylark | `kalkyl` | 61 149 byte | 0,33 M; 1,6 M med 1 040 celler i bild vid 1600 × 1000 | 14 | 4 | c252b0f, 01c49db |
| 3 | Ritbord | `rita` | 37 376 byte | 0,08 M tom; 3,0 M full (2 500 punkter), 4,1 M den frame banorna byggs om | 9 | 5 | dae13cb, 121a185 |
| 4 | Anteckningar | `skriv` | 60 895 byte | 0,45 M; 1,6 M med 6 000 byte där varje ord byter stil, 3,9 M den frame texten bryts om | 9 | 4 | 8742960, 686aefd |
| 5 | Block | `block` | 21 746 byte | 0,10–0,15 M i spel | 12 | 4 | 4df6c82, 79f213d |
| 6 | Rymd | `rymd` | 38 963 byte | 0,44 M med förvalen; 1,87 M som mest (torus, 800 partiklar) | 18 | 5 | fd5a5c1, 8169a92 |
| 7 | `docs/EXEMPEL.md` | – | – | – | – | – | e31e311 |

Sämsta uppmätta frame i hela spåret är 4,1 M (`rita`, full ritning, fönstret ändrat), alltså
49 % marginal; med förvalen ligger varje sida under 0,5 M. Alla sidor är under 64 KB.
`vy-publish build` godtar alla sex paketen (SPEC-13.4.1).

Sviten har 27 nya fall (91 totalt, alla 0,000 %) och 19 nya träd i `conformance/semantic/`. Inga
befintliga guldbilder eller träd är ändrade: efter varje `conformance --update` visade `git status`
bara nya filer.

Ordningen är uppdragets. Varje sida är två commits: `modules/<namn>: …` (modul, utkast, tester) och
`golden: …` (fall, guldbilder, träd). **Mellan de två är två tester röda, inte ett:**
`spec_12_1_2_conformance_defines_client` och `spec_10_4_2_ime_required_for_input` i `vy-cli` kräver
båda ett fall per modul. Det står i varje modulcommit. Den som delar historiken (bisect) ska räkna
paret som en enhet.

## Så arbetar spåret

- Grindarna körs från roten före varje commit (se CLAUDE.md); `CARGO_INCREMENTAL=0` i varje skal.
- Bränsle per frame mättes med ett eget litet verktyg: ett ocommittat exempel i `vy-cli` som körde ett
  fall som `vy-cli conformance` gör och skrev ut `Runtime::last_frame().fuel`, rasterenheter och
  minnessidor per frame och sparade sista bilden. Det är **raderat** och har aldrig committats (se
  *Verktygsluckor*); siffrorna ovan kommer därifrån. `vy-publish build` kördes för varje sida.
- Varje sida ses som PNG vid 1280 × 800, 800 × 600 och 400 × 700 @2 före commit.

## Beslut (gemensamma)

- **Varje sida är en egen crate** under `modules/` med bara `vy-sdk` som beroende, som
  referenssidorna. Ingen delad hjälpcrate: det lilla som upprepas (lagringsomslag, en växlingsknapp)
  är några tiotal rader per sida och står i *SDK-luckor* som underlag för nästa SDK-runda.
- **Utan `storage`** (begränsat läge, eller nekad): sidan kör med tillståndet i minnet och visar
  "sparas inte" diskret i sin statusrad. `state_get` vid start avgör läget; ett `state_put` som nekas
  senare (kvot) visar samma sak.
- **Fokus är värdens** (SPEC-10.3.1): allt som kan få fokus har fasta negativa nod-id, och sidorna
  ritar ringen själva (SPEC-10.3.3).

## Beslut i `rymd`

- **Vinklar är heltal** (65 536 på varvet) och allt annat är `f32`-addition, -multiplikation och
  -division i fast ordning, som wasm och varje plattform räknar lika. Sinus är ett polynom på en
  kvarts varv (fel under 4 × 10⁻⁶), kvadratroten Newton och används bara när kropparna byggs.
- **Ritordningen sorteras på heltal** (djup × 65 536, ytans index som särskiljare), så ordningen är
  total och densamma vilken sorteringsalgoritm `core` än har.
- **Ljuset är fast mot ögat**, inte mot kroppen: en yta ljusnar när den vänds uppåt till vänster.
- **Fyllda ytor stryks också i sin egen färg** (1 px). Två ytor som möts täcker annars kantpixlarna
  bara delvis var, och himlen lyser igenom som en hårfin fog. Det dubblar banornas byte.
- **Trådmodellen är två banor**, inte en per yta: de bortvända ytornas kanter först och svagare. En
  kant som två ytor delar blir då en linje, och 200 ytor kostar två opkoder i stället för 200.
- **Scenen klipps inte.** Klippta fyllningar kostar 16 rasterenheter per pixel mot 1; i stället ritas
  panelen efter scenen och täcker det som sticker ut. En inzoomad kropp kan därför gå över rubriken.
- **Stjärnorna kommer ur en fast talföljd**, inte ur värdens `random`: himlen är densamma överallt
  och guldbilderna beror inte på slumpkällan. De ritas på hela enhetspixlar (`math::snap`).
- **Första framen efter en vila vrider inget** (dess `dt_ms` är tiden sidan stod still), och ett
  stopp längre än 100 ms räknas som 100 ms.
- **Rotationen är på från start**, så sidan begär frames redan från den första. I `cases.toml`
  måste därför *varje* händelse ha `after_frames`, också den första; annars körs alla frames upp
  till taket innan händelsen levereras.

## SDK-luckor

Från `kanban`:

- **Knapp med annat namn än sin etikett.** `ui::Button::draw` registrerar etiketten som nodens text, så
  en ikonknapp ("+", "×") får noden "+". Sidan ritar därför sina egna små knappar med noden "Lägg till
  kort i Pågår". Förslag: `Button::draw_named(f, r, &theme, etikett, namn)` eller en `IconButton`.
- **Fokusringen är inte publik.** `ui::ring` är `pub(crate)`; varje sida som ritar en egen kontroll
  skriver om den (3 px utanför, `palette.ring`). Förslag: `ui::focus_ring(f, r, radie, &theme)`.
- **Säga till en komponent att den har fokus.** Ett kort blir ett textfält på samma nod-id (fokus följer
  id:t, SPEC-10.3.2), men `TextBox` lär sig fokus bara ur en `FOCUS`-händelse, så sidan räcker den en
  egen `Event::Focus` genom `Input::new`. Förslag: `TextBox::adopt_focus(r)` för en nod som redan har
  värdens fokus.
- **Ingen hjälp för dragning.** Tröskeln innan ett tryck blir ett drag, dubbelklick (händelserna bär
  inget klickantal; sidan jämför `now_ms`) och "släppt utanför fönstret" (`POINTER_MOVE` utan knapp)
  skrivs för hand. Förslag: en liten `ui::Drag` som ger `Started/Moved/Dropped/Clicked/DoubleClicked`.
- **Centrerad text.** `text_measure` + `f.text` med halva skillnaden skrivs om i varje sida;
  `Frame::text_centred(r, style, text)` vore en rad.

Från `kalkyl`:

- **Tab kan inte gå mellan celler.** `cell` är ingen fokuserbar roll (SPEC-10.3.2) och Tab når aldrig
  sidan (SPEC-10.3.1). "Tab bekräftar och går höger" görs därför med trädet: formelfältet registreras
  mellan två `textbox`-noder till, fälten för cellen till vänster och höger, med tre nod-id i en ring.
  När Tab ger grannen fokus väljer sidan den cellen och låter fältet ta över den fokuserade noden.
  Det följer reglerna men är mycket kod för en vanlig sak; samma spec-fråga som m5/sdk:s nr 1 (en
  fokuserbar roll för en markeringsbar lista), här för ett rutnät.
- **`TextBox` kan inte byta nod-id** (det sätts i `new`), så ringen bygger ett nytt fält vid varje
  Tab. Förslag: `TextBox::set_id`.
- **`ui::scroll::Activity` är inte publik.** Ett eget tangentbordsområde utan fokuserbar roll
  (rutnätet) skriver om regeln "aktiv efter ett tryck som inte gav någon nod fokus, tills en nod får
  fokus". Förslag: gör `Activity` publik, som `List` och `ScrollView` redan använder den.
- **`ScrollView` rullar bara i höjdled** och ritar alltid sin ring när den är aktiv. Bladet behövde
  två axlar och fasta rubriker, så sidan använder `ScrollView` för raderna (hjul, list) utan dess
  `draw` – listen ritas ur `thumb()` – och har en egen list för kolumnerna. Förslag: en
  `ScrollView` för två axlar, och `draw_bar` publik.
- **Ett enradsfält tar Upp/Ned utan att göra något** (noterat i m5-sdk), så sidan måste hinna före
  fältet för att låta pilarna bekräfta och flytta markeringen.
- **`core::fmt` kostar runt 4 KB i en modul.** `kalkyl` bygger sina strängar för hand och gick från
  65,2 till 61,0 KB. En `push_int` i SDK:t vore nog för de flesta sidor som bara skriver heltal.

Från `rita`:

- **`dl_path` debiteras två gånger per byte** (kopian ur modulminnet och kodningen i listan, 64
  bränsle per byte), och varje frame måste skicka hela ritningen igen: det finns inget lager eller
  någon bild en sida kan rita i och behålla. En kurva kostar 18 byte per punkt, så en ritning får
  plats med omkring 2 500 punkter innan en frame når halva budgeten. Inget för SDK:t att lösa; en
  1.x-fråga om ett behållet lager (eller en lägre taxa för banor).
- **`dl_path` har ingen ändstil eller hörnstil.** Runda ändar ritas som en prick i var ände (två
  opkoder till per streck); hörn blir värdens gering.
- **`Path` kan inte återanvändas** (ingen `clear`, ingen `with_capacity`) och kostar runt 470
  instruktioner per punkt att bygga; sidan behåller därför varje strecks `Path` mellan frames.
  Förslag: `Path::clear` och `Path::with_capacity`.
- **Ingen kvadratrot, sinus eller cosinus** i `vy_sdk::math` (`core` har inga). `rita` klarar sig med
  en uppskattning av avståndet; `rymd` skriver egna. Förslag: `math::{sqrt, sin, cos}`.
- **Ingen växlingsknapp.** `ui::Button` har inget "vald"-läge och kan inte byta utseende efter
  `new`/`secondary`, så verktygsraden (färger, pennor, sudd) är sidans egna knappar. Förslag:
  `Button::draw_toggled(…, on)` eller en `ToggleButton`.
- **Ingen dialog.** Skymning, kort och att hålla Tab inne i dialogen (verktygen lämnar trädet medan
  den är öppen) skrivs för hand. Förslag: `ui::Dialog` som ritar skymningen och säger vilka noder
  som får registreras.

Från `skriv`:

- **`text_style` har ingen lutning** (typsnittsroll, storlek, vikt, färg; SPEC-5.2.5), och värden har
  bara DejaVu Sans, Sans Bold, Serif och Sans Mono. Betoning sätts därför i serifrollen i stället för
  kursivt. En 1.x-fråga: en kursiv roll eller ett lutningsargument.
- **Ingen hjälp för text i flera stilar.** En rad med fet, kod och länk bryts för hand: en
  `text_measure` per ord, bredden av ett mellanslag mätt per stil, ett fragment per stil och rad.
  `text_measure` ger bredd och höjd men ingen baslinje, så olika storlekar på samma rad centreras
  på radhöjden. Förslag: `ui::RichText` (stilade stycken in, fragment ut) – det är samma kod som
  `document`, `text` och `skriv` nu har var sin variant av.
- **Ingen länkkomponent.** `link_register` + `link`-nod med fast id + ring + Enter som anropar
  `navigate` skrivs om i varje sida med länkar. Förslag: `ui::Link`.
- **Inga flikar.** `kanban` och `skriv` ritar var sin flikrad med `tab`-noder. Förslag: `ui::Tabs`.
- **`TextBox` säger inte när texten ändrats annat än genom svaret** (ingen revisionsräknare att
  jämföra), och det går inte att fråga var markören står i bild. Sidan räknar själv revisioner och
  följer markörens byteposition.

Från `block`:

- **Inga fasta steg.** Varje sida som animerar skriver om saldot av `dt_ms` som betalar steg, taket
  för hur många steg en frame hinner ikapp, och regeln att första framen efter en vila inte räknas
  (dess `dt_ms` är tiden sidan stod still; `kanban` har samma regel för sin glidning). Förslag:
  `vy_sdk::time::Steps::new(16)` med `advance(dt_ms) -> antal` och `rest()`.
- **Mellanslag hör till knappen.** `ui::Button` aktiveras av mellanslag med fokus, men i ett spel är
  mellanslag en speltangent vad som än har fokus (ett pekartryck på en knapp ger den fokus), så
  spelets knappar är sidans egna och tar bara Enter. De upprepar också medan pekaren håller dem,
  vilket `Button` inte kan. Förslag: `Button::event_with(…, keys)` eller en flagga för upprepning.
- **Få tangentnamn.** `event::key` har A, C, V och X; sidorna definierar själva Z, Y, P, H och F2.
  Förslag: hela alfabetet och F-tangenterna.

Från `rymd`:

- **Egen trigonometri.** Sinus, cosinus och kvadratrot skrevs i sidan (`scene.rs`, omkring 40 rader,
  testade mot `std` till fem decimaler). De är deterministiska och kan flyttas som de är till
  `vy_sdk::math::{sin, cos, sqrt}`; samma lucka som under `rita`.
- **`Path` byggs om för varje yta och frame.** Utan `clear` och `with_capacity` är varje yta en ny
  `Path` som växer 8 → 16 → 32 → 64 byte, alltså fyra allokeringar. Uppmätt på trådmodellen med 200
  ytor: 857 k bränsle på modulsidan med en bana per yta mot 579 k med två gemensamma banor, alltså
  omkring 1 400 per yta för banan själv. De fyllda ytorna har var sin färg och kan inte slås ihop.
- **Samma bana skickas två gånger** när en yta både fylls och stryks: `Frame::path` tar ett läge, och
  varje anrop debiteras 64 bränsle per byte. En opkod för "fyll och stryk" vore en 1.x-fråga.
- **`ui::Slider::draw` använder `format!`**, så en sida med ett reglage drar in `core::fmt` hur den
  än bygger sina egna strängar (i `kalkyl` var `core::fmt` värt omkring 4 KB; här är det inte
  uppmätt). Reglaget ritar inte heller sin etikett: sidan mäter och ritar "Partiklar" själv bredvid.
- **`Color` kan inte skalas eller läsas av.** Belysningen räknas på tre `u8` vid sidan av. Förslag:
  `Color::scaled(f32)` eller `r()`, `g()`, `b()`.
- **`SCROLL` bär ingen position** (SPEC bilaga A), så sidan vet inte om pekaren är över scenen när
  hjulet rullar; `rymd` zoomar var pekaren än står. En sida med två rullbara ytor måste själv minnas
  senaste `POINTER_MOVE`. Det är ABI, inte SDK: en 1.x-fråga.
- **Växlingsknapp och fasta steg** saknas här också (formknapparna är sidans egna, och regeln om
  första framen efter en vila skrevs för tredje gången efter `kanban` och `block`).

### Verktygsluckor

- **Inget sätt att se bränsle per frame för ett fall med händelser.** `vy-cli conformance` skriver
  bara diffen, `vy-cli render` tar inga händelser, och `vy-publish build` rapporterar dyraste framen
  i en körning utan händelser. Jag mätte med ett eget exempel i `vy-cli` (omkring 230 rader som körde
  fallen som sviten gör). Det raderades i stället för att committas, eftersom det dubblerade svitens
  händelseleverans och skulle glida ifrån den. Rätt plats är sviten själv, till exempel
  `vy-cli conformance --stats [filter]` som skriver frames, högsta och sista bränsle, rasterenheter
  och minnessidor per fall.
- **Ett fall för en sida som animerar från start är lätt att skriva fel:** utan `after_frames` på
  första händelsen körs alla frames först, och fallet blir grönt på fel bild. En varning när en
  händelse utan `after_frames` följer på en frame som begärt nästa hade sparat en omgång per sida.

## Berör andra spår

Inget. Ingen fil utanför `modules/`, `conformance/` och `docs/` är ändrad på grenen, och
`crates/vy-sdk` är orörd. Mätverktyget låg ocommittat under `crates/vy-cli/examples/` medan arbetet
pågick och är borttaget; `.cargo/config.toml` i arbetskatalogen är koordinatorns och är inte
committad.

`conformance/cases.toml` är delad med andra spår: de sex avsnitten står i följd före
`# Skalet (M3 …)`, så en sammanslagning med ett spår som lagt till fall på samma ställe ger en
textkonflikt som löses genom att behålla båda.

## Kvar och öppet

- **`tools/publish_site.sh` är inte kört här.** Det bygger `vy-cli` och `vy-publish` i release, och
  det bygget kördes inte för att spara den delade disken (7 GB fritt). Varje paket är i stället
  byggt med `vy-publish build` (debug). Skriptet tar varje utkast i `conformance/manifests/`, så
  sidorna följer med utan ändring; kör det en gång före publicering.
- **Guldbilderna är skapade på Linux.** De 27 nya fallen är inte körda på macOS.
- **Prestandagrinden (`vy-cli bench`) är inte körd**; den stod inte i spårets grindar och ingen
  klientkod är ändrad.
- **Granskningssidan** i `docs/reviews/` skrivs av en annan agent.
- **README och CLAUDE.md** nämner inte de nya modulerna eller `docs/EXEMPEL.md`; de ligger utanför
  spåret.
- Kända gränser, alla avsiktliga: `rita` tar högst 2 500 punkter och säger sedan ifrån; `skriv` tar
  6 000 byte och sätter betoning i serif (ingen lutning i ABI:t); `kalkyl` klistrar in text i en cell
  i taget; `rymd` sorterar ytor på mittpunktens djup, vilket i princip kan ge en felordnad yta på torusen
  i någon vinkel (ingen sådan syntes i de vyer jag såg), och klipper inte scenen.
