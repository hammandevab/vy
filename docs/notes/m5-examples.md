# M5 examples – sex större exempelsidor

Byggagentens noteringar för grenen `m5/examples` (från `m5/sdk` 1de409e). Granskningssidan skrivs av en
annan agent. Ägda delar: `modules/{kanban,kalkyl,rita,skriv,block,rymd}`, deras utkast i
`conformance/manifests/`, deras fall, guldbilder och träd i `conformance/`, `docs/EXEMPEL.md` och den
här filen. Det som rör andra spår står under *Berör andra spår*.

Varför: ägaren vill ha mer avancerade sidor att köra i klienten än referenssidorna. De visar vad
formatet klarar, prövar att komponenterna i `vy_sdk::ui` räcker för riktiga appar, och publiceras på
den publika sajten (`tools/publish_site.sh` tar varje utkast i `conformance/manifests/`).

## Läget (uppdateras efter varje sida)

| # | Sida | Modul | Läge | Commits |
| --- | --- | --- | --- | --- |
| 1 | Tavlan | `kanban` | klar: 54 267 byte, 0,11 M bränsle per frame (1,1 M med 60 kort), 5 fall | 2188796, 94e6aa2 |
| 2 | Kalkylark | `kalkyl` | klar: 61 149 byte, 0,33 M bränsle per frame (1,6 M med 1 040 celler vid 1600 × 1000), 4 fall | c252b0f, 01c49db |
| 3 | Ritbord | `rita` | klar: 37 376 byte, 0,08 M bränsle per frame tom (3,0 M full, 4,1 M när banorna byggs om), 5 fall | dae13cb, 121a185 |
| 4 | Anteckningar | `skriv` | klar: 60 895 byte, 0,45 M bränsle per frame (1,6 M med 6 000 byte där varje ord byter stil, 3,9 M när den bryts om), 4 fall | 8742960, 686aefd |
| 5 | Block | `block` | pågår | – |
| 6 | Rymd | `rymd` | ej påbörjad | – |
| 7 | `docs/EXEMPEL.md` | – | ej påbörjad | – |

Ordningen är uppdragets. Varje sida är två commits: `modules/<namn>: …` (modul, utkast, tester; röd
på `spec_12_1_2_conformance_defines_client` tills fallen finns, som `widgets` och `studio` landade)
och `golden: …` (fall, guldbilder, träd).

## Så arbetar spåret

- Grindarna körs från roten före varje commit (se CLAUDE.md); `CARGO_INCREMENTAL=0` i varje skal.
- Bränsle per frame mäts med ett litet verktyg utanför repot (ett ocommittat exempel i `vy-cli` som
  kör ett fall som `vy-cli conformance` gör och skriver ut `Runtime::last_frame().fuel` per frame);
  `vy-publish build` körs för varje sida (SPEC-13.4.1).
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

## SDK-luckor (fylls på per sida)

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

## Berör andra spår

Inget ännu.

## Kvar och öppet
