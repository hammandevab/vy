# M5 sdk – komponenter, dokumentation och en publicerad svit

Byggagentens noteringar för grenen `m5/sdk` (från `main` 0fc0453 = v0.2.0). Granskningssidan skrivs av en
annan agent. Ägda delar: `crates/vy-sdk`, `modules/` (ny sida `widgets`, försiktiga portningar),
`conformance/` (fall och guldbilder för nya fall, `README.md`), `docs/SDK.md`, scaffoldens mall och
startsida i `crates/vy-dev/src/scaffold.rs`, `tools/build_docs.sh`, `tools/package_conformance.sh`. Det som
rör andra spår står under *Berör andra spår*.

Varför: M5-grinden är att en utomstående agent publicerar en sida på under en timme. I dag rullar varje
sida sina egna textfält, knappar, listor och rullning på råa frames. Spåret ger SDK:t färdiga komponenter
och dokumentation, och paketerar konformanssviten för andra klienter.

## Läge

Pågår. Uppdateras efter varje commit; `git log` är facit.

| Steg | Vad | Läge |
| --- | --- | --- |
| 1 | `vy_sdk::ui`: tema, `Input`/`Response`, `Column`/`Row`, `Button`, `Checkbox`, `RadioGroup`, `Slider`, `TextBox`, `ScrollView`, `List`; galleriet `modules/widgets` med fall och guldbilder | klart (7cd8fdb, 0eca12c, f2f03f9, 1fd44a2, d0309fc, f5b354a) |
| 2 | Portning av `form` (textfält, knapp) och `settings` om guldbilderna står på 0,000 % | pågår |
| 3 | README per komponent, `docs/SDK.md`, `cargo doc` utan varningar, `tools/build_docs.sh` | påbörjat |
| 4 | Scaffoldens `CLAUDE.md` och startsida med `ui::TextBox` och `ui::Button` | – |
| 5 | `tools/package_conformance.sh`, engelskt avsnitt i `conformance/README.md`, steg i `release.yml` | – |
| 6 | Slutnoteringar | – |

Utgångsläge på `main` (0fc0453): konformans 59/59 à 0,000 %; modulstorlekar chat 25 039, dashboard
23 321, document 27 692, form 16 694, gallery 16 638, game 15 166, hello 5 274, list 18 806, settings
23 371, shop 25 312, studio 60 692, text 31 424 byte.

## Beslut

- **Komponenterna är vanlig modulkod** i `vy_sdk::ui` (`no_std` + `alloc`, inga nya beroenden, inget nytt i
  ABI:t). De följer `layout()`-mönstret: sidan räknar ut rektangeln, komponenten ritar i den med `draw`,
  registrerar sina semantiska noder och hanterar sina händelser med `event` mot samma rektangel.
- **Händelser går genom `ui::Input`.** `App::event` gör `let mut ui = Input::new(event)` och ger `&mut ui`
  till varje komponent i tur och ordning, inre före yttre. En komponent som använder en pekar-, rullnings-,
  tangent- eller texthändelse *tar* den (`ui.taken()`), och komponenter efter den ser den inte; en
  `FOCUS`-händelse tas aldrig, eftersom varje komponent behöver veta vart värdens fokus gick. `App::event`
  returnerar `ui.redraw()`. Det som ingen komponent tog får sidan med `ui.unhandled()`. Utfallet per
  komponent är en `Response` (`redraw`, `changed`, `activated`).
- **Fasta negativa nod-id** för alla fokuserbara noder (`TextBox::new(-1)`), det kanoniska sättet i
  SDK:t (SPEC-5.2.13, SPEC-10.3.2: fokus följer nod-id över frames). En `RadioGroup` med `n` alternativ tar
  id `first, first − 1, …`.
- **Fokusmarkering ritas av komponenten** (SPEC-10.3.3): ett fokuserat textfält får 2 px ram i
  accentfärgen (så som `form` redan ritar), knapp, kryssruta, radioknapp, slider och aktiv lista en ring i
  temats `ring`-färg 3 px utanför.
- **Knappar aktiveras på `POINTER_DOWN`**, som i alla referenssidor (och fallen i sviten bygger på det),
  och med Enter eller mellanslag när de har fokus.
- **Kopiering och urklipp.** Ctrl+C och Ctrl+X (Cmd på macOS: genvägsmodifieraren är Ctrl utan Alt, eller
  Meta, samma regel som skalet) i ett fokuserat textfält skriver markeringen med `clipboard_write` medan
  sidan hanterar det `KEY_DOWN` värden levererat, vilket SPEC-8.2.4 tillåter utan capability. Ctrl+X tar
  bort markeringen bara om skrivningen lyckades, så en nekad skrivning aldrig tappar text. Ctrl+V kommer som
  `TEXT_INPUT` (SPEC-8.2.4) och klistras in som skriven text. Utfallet finns i `TextBox::last_copy()`.
  Mocken fick `MockHost::copy_gesture` och `mock::as_copy_gesture`, så att gesten kan provas nativt.
- **Listor och rullytor har inget värdfokus.** `list` och `listitem` är inte fokuserbara roller
  (SPEC-10.3.2), och en fokuserbar nod per rad vore ett tabbstopp per rad. `List` och `ScrollView` tar
  därför tangenter medan de är *aktiva*: efter ett tryck i dem som inte gav någon nod fokus, tills värden
  ger en nod fokus. Sidan kan också sätta dem aktiva från start. Se spec-fråga nedan.
- **Hela trädet varje frame** (SPEC-10.1.1): `List` ritar bara raderna i bild men registrerar en
  `listitem` för varje rad; kostnaden är en `sem_node` per rad och frame.

## Spec-frågor

1. **Fokus för en lista med markering.** Ingen fokuserbar roll passar en lista där piltangenterna flyttar
   markeringen (ARIA:s `listbox`/`option` med rullande tabindex). Med 1.0:s roller kan Tab inte nå en
   `ui::List`; den blir aktiv med pekaren eller för att sidan säger det. Förslag för 1.1: en fokuserbar
   roll för en markeringsbar lista, med fokus på listan och markeringen i texten.

## Berör andra spår

(fylls i)

## Kvar och öppet

(fylls i)
