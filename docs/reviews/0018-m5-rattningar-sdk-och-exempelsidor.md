# Granskning 0018 – rättningarna i SDK:t och exempelsidorna efter 0010 och 0013

Gren `m5/gate-fixes`, HEAD `b7fb243`. Granskat: rättningscommitten efter `1eda75b` som hänvisar
till granskning 0010 och 0013, `docs/notes/m5-fix-a.md` och 0013-delen av `docs/notes/m5-fix-b.md`.
Granskaren skrev ingen av koden. Klienten, releaseverktygen och lagringsändringen (0016, 0017)
ingår inte.

**Rekommendation.** **Släpp v0.3.0 efter att fynd 1 är rättat.** Skäl i en mening: exempelsidan
`kalkyl` kan fortfarande stängas av för gott av ett blad den själv sparat, efter en enda vanlig
inmatning, och det är just den klass av fel (0013:2) som releasen och `docs/EXEMPEL.md` säger är
borta. Rättningen är liten (en reservation och ett test); fynd 2–5 hindrar inte releasen.

**Det som blockerar.** Bara fynd 1. Det kräver ett stort blad: avstängningen är visad med 2 599
formler, en frame över budgeten med 999 och ingen med 599 (värden längre än omkring tio tecken).
Användaren har en väg ut i klienten ("Rensa sidans lagring"), men den kastar bladet.

**Vad ändrades.** Rättningar av fynden i 0010 (SDK:ts `ui::TextBox`, konformanssviten, paketet)
och 0013 (`skriv`, `kalkyl`, `rita` och sparad data i alla sex exempelsidorna), plus guidetext i
`docs/SDK.md` och `docs/EXEMPEL.md`. Det tyngsta är `kalkyl`: omräkningen går nu i steg om 20 000
arbetsenheter per frame i stället för i ett anrop.

**Spec-id som berörs.** SPEC-12.2.4 är ny (sviten håller varje frame till budgeten). Rättningarna
bär SPEC-5.3.3, 5.2.9, 5.2.6, 5.1.6, 6.4.3, 10.1.1 och 9.7.1. Inget i §5, bilaga A eller C är ändrat.

**Risk.** En sida som läser sin lagring i `vy_init` och sedan går över budgeten två frames i rad
går inte att öppna igen. Den som drabbas är användaren som byggt ett stort blad, och formatets
rykte: sidorna är det utomstående kör först och det andra agenter kopierar.

**Bevis.** Koordinatorns grindar (466 tester, spårning 143/143, konformans 101/101, CI 43557039351)
är inte omkörda. Eget: `sheet.rs` och radbrytningen i `textbox.rs` lästa i sin helhet, övriga
rättningar lästa som diff; `conformance --stats kalkyl kanban rita` (dyrast i sviten 0,66 M); 29
egna fall mot `kalkyl` i en skrotrot (tabellen i fynd 1); guidens exempelfall kört ordagrant mot
`kalkyl` (går: bild och träd skrivs, 8 frames, 0,65 M). Inget i repot är ändrat utom den här sidan.

**Nya beroenden.** Inga.

**Hotbild.** Oförändrad: inget nytt i ABI:t. Sparad data kommer från samma origin, så fynd 1 är
robusthet och inte intrång: en sida kan bara låsa sig själv.

## Tidigare fynd

| Fynd | Commit | Bedömning |
| --- | --- | --- |
| 0010:1 långt ord i flerradsfält | `f211181` | **Rättat.** `fit_prefix` mäter högst omkring två rader per rad; `budget_tests` (tre fall, tak 8 M) faller utan rättningen, och ett test jämför brytpunkterna mot den gamla sökningen. |
| 0010:2 ingen mätning efter händelser | `0700141`, `2902727` | **Rättat.** Prövat: mina egna fall över budgeten underkändes, också med `--update`, och fick ingen guldbild. Se fynd 3 för en felräknad rad. |
| 0010:3 paketet och umask | `96c70f6` | Inte läst (releaseverktyg); testet `spec_9_7_1_conformance_package_is_the_same_under_any_umask` finns. |
| 0010:4 textfält utan namn | `def14b2`, `e1b6a99` | **Rättat.** Testet går igenom varje träd i sviten och kräver en namngiven `text`-nod före varje `textbox`. Gräns: ett fält som inte finns i något träd i sviten ses inte. |
| 0010:5 komposition, U+2028/2029 | `f1902e2` | **Rättat delvis.** SDK-sidan är rättad och testad mot mocken. Med en riktig IME är det oprövat, och enligt noteringen skickar fönstret ännu inte `IME_COMPOSE` vidare. |
| 0013:1 `skriv` på sparad text | `906cfad` | Rättat enligt testet och den förra granskarens 61 texter; inte omprövat här. |
| 0013:2 `kalkyl` på sparat blad | `0f9cdc1`, `ea08bb9` | **Rättat delvis.** Fyndets två blad (2 500 summor, 200 summor av hela bladet) går nu, och testet faller utan rättningen. Klassen är kvar: se fynd 1. |
| 0013:3 långa texter i `kalkyl` | `d2c3a13` | **Rättat för texter** (tak per frame för ritade, mätta och trädets byte). Tal har inget sådant tak: sista raden i fynd 1. |
| 0013:4 fall utan `storage`, sparat värde | `77de40f`, `b86c3e8` | **Rättat.** `state = [{key, hex}]` använt i alla egna fall; tio nya fall i sviten. |
| 0013:5 noteringens "4,1 M, 49 % marginal" | – | **Inte rättat.** `docs/notes/m5-examples.md` rad 26 står kvar utan förbehåll; `m5-fix-a.md` har punkten som ogjord. Mätverktyget finns nu (`--stats`). |
| 0013, regeln om sparad data | `08b104c`, `66b1bc3`, `31bead8` | Avkodningen i alla sex prövas på 300 slumpade värden, avklippta värden och hela kvoten. `rita`-ändringen inte läst i detalj. Testbladen för `kalkyl` har bara korta värden: se fynd 1. |

## Egna fynd

1. **Hög – `kalkyl` kan fortfarande stoppas för gott av ett blad den själv sparat: formler med långa
   värden.** *Var:* `modules/kalkyl/src/sheet.rs` rad 898 (`Sheet::set`: etiketten får 16 byte, "any value it
   usually gets") och rad 205 (`Cell::relabel`); roten är av allt att döma SDK:ts allokator
   (`crates/vy-sdk/src/heap.rs`), som noteringen själv listar som öppen. *Scenario:* ett blad med A1 = `7` och `=A1/7` i övriga
   celler öppnas utan problem (dyraste frame 4,05 M). Användaren skriver `1234567,891234` i A1 och
   trycker Enter. Bladet sparas i händelsen; omräkningen ger sedan några hundra formler per frame
   ett nytt värde (`176366,841605`), etiketten `B1: 176366,841605` är 17 byte och ryms inte i de 16
   som reserverats, strängen flyttas, och varje flytt kostar mer ju fler hål högen har. Två frames
   i rad går över budgeten och klienten stänger av sidan. Vid nästa besök läser sidan samma blad och
   stängs av igen: enda vägen ut är "Rensa sidans lagring", som kastar bladet. Det är samma klass
   som 0013:2. *Verifierat (reproducerat)* med egna fall i en skrotrot utanför repot (`/tmp/review18/root`;
   bladen i formatet från `sheet_blob` i `budget_tests`, givna med `state = [{key, hex}]`),
   `vy-cli conformance --root … --update --stats`, budget 8 M per frame):

   | Fall (sparat blad, alla inom kvoten 256 KB) | Resultat |
   | --- | --- |
   | A1 = `7`, 2 599 × `=A1/7`; klick på A1, `1234567,891234`, Enter (800 × 600, 1024 × 720, 1600 × 1000) | sidan avstängd: "the module is suspended after two budget breaches" i alla tre storlekarna |
   | samma blad som det sparades, öppnat på nytt (800 × 600, 1024 × 720) | avstängd igen före första händelsen |
   | samma blad utan ändringen (1024 × 720) | går: dyraste frame 4,05 M |
   | A1 = `1234567,891234` och 999 × `=A1/7` | frame 4: 10,7 M (1024 × 720), 12,8 M (1600 × 1000) |
   | … 599 × `=A1/7` | går: 5,7 M / 6,6 M |
   | 1 560 formler **bara under bild** (rad 41–100), långt värde | frame 5: 11,8 M – alltså inte ritningen eller mätningen |
   | samma med kort värde (A1 = `14`) | går: 3,1 M |
   | 220 formler bara i bild, långt värde | går: 4,0 M |
   | 2 600 tal `12345678901,123456` (inga formler, 54,6 KB) | första framen 7,25 M (1024 × 720), **12,8 M** (1600 × 1000), **25,6 M** (2560 × 1440); en frame, alltså varning och inte stopp |

   Gränsen ligger mellan 600 och 1 000 formler vars värde blir längre än ungefär tio tecken (namn +
   `: ` + värde > 16 byte); en division ger sex decimaler, så `=B2/7` med ett sexsiffrigt
   tal räcker (femsiffrigt från rad 10).
   `budget_tests` ser det inte: dess blad har korta värden (`1,5`, `150`, `{col*row},{row}`) och
   största fönstret är 1600 × 1000. Sista raden i tabellen är samma rot från ett annat håll:
   `draw_cell` (`lib.rs` rad 735–746) gör upp till sju `format_decimals` (en ny `String` var) per tal
   som är bredare än cellen, utan tak per frame (texter har `MEASURE_POOL`, tal inget).
   *Förslag:* (a) reservera etiketten för det längsta värdet ett tal kan få (namn 4 + 2 + värde 19 byte)
   för varje cell som är tal eller formel, och formatera i `draw_cell` till en buffert på stacken;
   (b) ett tak per frame för mätta tal, som `MEASURE_POOL`; (c) i `budget_tests`: bladet ovan
   (2 599 × `=A1/7`, A1 ändrad till ett långt tal, sedan `reinit`) och 2 600 breda tal i
   800 × 600 till 2560 × 1440; (d) på sikt allokatorn (storleksklasser eller O(1)-frigörelse), annars
   kommer felet tillbaka i nästa sida som allokerar i en frame.

2. **Medel – `docs/EXEMPEL.md` rad 25 lovar mer än koden håller.** "det sidan visar håller budgeten
   vad värdet än är" stämmer inte för `kalkyl` (fynd 1). *Verifierat:* följer av fynd 1.
   *Förslag:* rätta fynd 1; annars skriv gränsen i stället för löftet.
3. **Låg – `conformance --update` räknar guldbilder som inte skrevs.** `crates/vy-cli/src/main.rs`
   rad 314 skriver `results.len()`. *Scenario:* fem fall varav fyra över budgeten: "5 golden
   image(s) written", en fil på disk. *Verifierat:* reproducerat. *Förslag:* räkna de fall som gick.
4. **Låg – medelvärdet i statusraden kan slå runt.** `modules/kalkyl/src/lib.rs` rad 433:
   `sum + n / 2 * sum.signum()` efter en mättad summa. *Scenario:* två markerade celler med
   `92233720368` ger en summa vid `i64::MAX`, additionen slår runt och "Medel" blir ett stort
   negativt tal. *Verifierat:* läst, inte kört; raden är äldre än rättningarna. *Förslag:* räkna i
   `i128` som `div_round` i `sheet.rs`.
5. **Låg – 0013:5 är obesvarat** (se tabellen). *Förslag:* en rad i `m5-examples.md` som säger att
   siffran gäller de fall som mättes.

Determinism: `kalkyl` räknar i heltal och går genom cellerna i indexordning, inga hashtabeller;
`fit_prefix` gissar med `f32` inne i modulen, vilket är samma på alla plattformar. Inget fynd.
Åtkomlighet: i trädet från mitt eget fall har `kalkyl`s tre fält var sitt namn, och sviten kräver
det i varje träd; inte prövat därutöver. Inget fynd.
Dokumentationen: lagringsexemplet i avsnitt 4 stämmer mot `state_get`/`state_put` i
`crates/vy-sdk/src/api.rs` (hela längden tillbaka, `Resource` utan värde, `Denied` utan `storage`),
och exempelfallet i "Händelser utan fönster" är giltigt i fallformatet.

## Inte kontrollerat

- `budget_tests` och modulernas egna tester är inte körda här; jag litar på koordinatorns körning.
- `kanban` och `rita` är bara lästa ytligt (gränserna 60 kort à 80 byte, 2 500 punkter) och körda
  i svitens egna fall; inga egna fientliga händelseföljder mot dem, och inga mot `skriv`, `block`
  eller `rymd`. Kopiering av hela bladet (Ctrl+C) och ett enskilt `vy_event` nära den hårda
  gränsen på 64 M är inte mätta: runtimen redovisar bara bränsle för frames.
- Fynd 1: jag har inte sökt den minsta utlösaren, och roten (allokatorn) är en slutsats av tre
  jämförande fall, inte av en mätning i allokatorn.
- Radbrytningens cache i `textbox.rs` nycklas på FNV-1a och längd; två olika stycken med samma
  nyckel skulle dela brytpunkter. Koden är äldre än rättningarna och saken är inte prövad.
- `md.rs` (0013:1), `96c70f6` (paketet), fynden om klienten i `m5-fix-b.md`, riktig IME, macOS och
  Windows.
