# M5 sdk – komponenter, dokumentation och en publicerad svit

Byggagentens noteringar för grenen `m5/sdk` (från `main` 0fc0453 = v0.2.0). Granskningssidan skrivs av en
annan agent. Ägda delar: `crates/vy-sdk`, `modules/` (ny sida `widgets`, försiktiga portningar),
`conformance/` (fall och guldbilder för nya fall, `README.md`), `docs/SDK.md`, scaffoldens mall och
startsida i `crates/vy-dev/src/scaffold.rs`, `tools/build_docs.sh`, `tools/package_conformance.sh`. Det som
rör andra spår står under *Berör andra spår*.

Varför: M5-grinden är att en utomstående agent publicerar en sida på under en timme. Förr rullade varje
sida sina egna textfält, knappar, listor och rullning på råa frames. Spåret ger SDK:t färdiga
komponenter och dokumentation, och paketerar konformanssviten för andra klienter.

## Vad som landat

| Steg | Vad | Commits | Test |
| --- | --- | --- | --- |
| 1 | `vy_sdk::ui`: `Theme`/`Palette`/`Words`, `Input`/`Response`, `Column`/`Row`/`split`, `Button`, `Checkbox`, `RadioGroup`, `Slider`, `TextBox` (en rad och `TextBox<Multiline>`), `ScrollView`, `List`/`Rows`; mocken modellerar kopieringsgesten; `event::key::{A, C, V, X}`, `Modifiers::shortcut` | 7cd8fdb, 0eca12c, f2f03f9, 1fd44a2, 1a8fff5, 5df5a4b | 16 nya `spec_*` i `ui::tests` och `ui::textbox::tests`, fyra doctests |
| 1 | Galleriet `modules/widgets` (referenssida 12) med manifestutkast (begär `ime` och `clipboard`), fem fall med skriptade händelser och tre semantiska träd | d0309fc, f5b354a | modulens tre tester; fallen `widgets-*` |
| 2 | `form`: textfälten och knappen är `ui::TextBox` och `ui::Button`, pixlar och träd byte för byte lika; `settings` portas inte | c5e2013 | de fem `form`-fallen och `shell-form-400x700@2` à 0,000 % |
| 3 | `docs/SDK.md` (guiden), ett README-avsnitt per komponent, `cargo doc -p vy-sdk --no-deps` utan varningar, `tools/build_docs.sh` | e2655c0 | `sdk_guide_example_builds` (steg 4) bygger guidens sida |
| 4 | Scaffoldens startsida med `ui::TextBox` och `ui::Button`, utkastet begär `ime`, mallens `CLAUDE.md` med komponenterna, determinism och budget och vanliga misstag | ddb661b | `scaffold_builds_to_wasm_and_passes_check_abi` (utökat), `sdk_guide_example_builds` (nytt), `api_summary_covers_the_sdk` (utökat) |
| 5 | `tools/package_conformance.sh`, engelskt avsnitt i `conformance/README.md`, ett steg i `release.yml` | 2977629 | paketet byggt två gånger med samma byte; `vy-cli conformance --root <uppackat>` 64/64 |

### Modulstorlekar (byte, `tools/build_modules.sh`)

| Modul | `main` 0fc0453 | nu | Kommentar |
| --- | --- | --- | --- |
| form | 16 694 | 30 547 | textfälten och knappen är komponenter (se *Beslut*) |
| widgets | – | 64 482 | ny; alla komponenter och båda radlägena |
| scaffoldens startsida | – | 28 189 | fält och knapp (`vy-dev new`) |
| guidens sida (Att göra) | – | 33 291 | fält, två knappar, lista |
| övriga elva | chat 25 039 … text 31 424, studio 60 692 | ±0,5 % | koden orörd; skillnaden kommer av att sökvägen till `vy-sdk` hashas in i symbolerna (m5/release rättar det i `build_modules.sh`, d74827e på deras gren) |

## Beslut

- **Komponenterna är vanlig modulkod** i `vy_sdk::ui` (`no_std` + `alloc`, inga nya beroenden, inget nytt i
  ABI:t). De följer `layout()`-mönstret: sidan räknar ut rektangeln, komponenten ritar i den med `draw`,
  registrerar sina semantiska noder och hanterar sina händelser med `event` mot samma rektangel. Samma
  signatur för alla: `event(&mut ui, r, &theme, …) -> Response` och `draw(f, r, &theme, …)`.
- **Händelser går genom `ui::Input`.** `App::event` gör `let mut ui = Input::new(event)` och ger `&mut ui`
  till varje komponent i tur och ordning, inre före yttre. En komponent som använder en pekar-, rullnings-,
  tangent- eller texthändelse *tar* den (`ui.taken()`), och komponenter efter den ser den inte; en
  `FOCUS`-händelse tas aldrig, eftersom varje komponent behöver veta vart värdens fokus gick. `App::event`
  returnerar `ui.redraw()`. Det som ingen komponent tog får sidan med `ui.unhandled()`. Utfallet per
  komponent är en `Response` (`Nothing`, `Redraw`, `Changed`, `Activated`).
- **Fasta negativa nod-id** för alla fokuserbara noder (`TextBox::new(-1)`), det kanoniska sättet i
  SDK:t (SPEC-5.2.13, SPEC-10.3.2: fokus följer nod-id över frames). En `RadioGroup` med `n` alternativ tar
  id `first, first − 1, …`.
- **Fokusmarkering ritas av komponenten** (SPEC-10.3.3): ett fokuserat textfält får 2 px ram i
  accentfärgen (så som `form` redan ritade), knapp, kryssruta, radioknapp, slider och aktiv lista en ring i
  temats `ring`-färg 3 px utanför, och en aktiv `ScrollView`, som saknar bakgrund, en ram av fyra staplar.
- **Knappar aktiveras på `POINTER_DOWN`**, som i alla referenssidor (och fallen i sviten bygger på det),
  och med Enter eller mellanslag när de har fokus.
- **Kopiering och urklipp.** Ctrl+C och Ctrl+X (Cmd på macOS: genvägsmodifieraren är Ctrl utan Alt, eller
  Meta, samma regel som skalet; `Modifiers::shortcut`) i ett fokuserat textfält skriver markeringen med
  `clipboard_write` medan sidan hanterar det `KEY_DOWN` värden levererat, vilket SPEC-8.2.4 tillåter utan
  capability. Ctrl+X tar bort markeringen bara om skrivningen lyckades, så en nekad skrivning aldrig
  tappar text. Ctrl+V kommer som `TEXT_INPUT` (SPEC-8.2.4) och klistras in som skriven text. Utfallet finns
  i `TextBox::last_copy()`. Mocken fick `MockHost::copy_gesture` och `mock::as_copy_gesture`.
- **IME.** Fältet anropar `ime_begin(rektangel)` när det får fokus och `ime_end()` vid varje fokusförlust,
  även när `ime_begin` nekades (utan `ime` nekar och loggar värden båda, som referenssidorna och vy-clis
  test för SPEC-10.4.2 väntar sig); flyttas ett fokuserat fält följer kompositionen med i nästa frame.
  `IME_COMPOSE` visas understruken vid markören utan att ändra värdet.
- **Inmatning städas**: CRLF och ett ensamt CR är en radbrytning, en radbrytning (och tab) blir ett
  mellanslag i ett enradsfält, övriga styrtecken tas bort, och det som inte ryms i `max_len` (byte; 1 KB
  för en rad, 4 KB för flera) kapas vid en teckengräns. Nodens text är värdet, högst 4 KB (SPEC-9.3.2).
- **Listor och rullytor har inget värdfokus.** `list` och `listitem` är inte fokuserbara roller
  (SPEC-10.3.2), och en fokuserbar nod per rad vore ett tabbstopp per rad. `List` och `ScrollView` tar
  därför tangenter medan de är *aktiva*: efter ett tryck i dem som inte gav någon nod fokus (värdens
  `FOCUS` kommer före trycket, SPEC-10.3.1), tills värden ger en nod fokus. Sidan kan också sätta dem
  aktiva från start. Se spec-fråga 1.
- **Hela trädet varje frame** (SPEC-10.1.1): `List` ritar bara raderna i bild men registrerar en
  `listitem` för varje rad; kostnaden är en `sem_node` per rad och frame. Se spec-fråga 2.
- **Radläget är en typparameter**: `TextBox` (en rad, standard) och `TextBox<Multiline>`. Med en
  körtidsflagga kom radbrytningen med i varje sida med ett fält; som typ tar kompilatorn bort den.
  `form` med komponenterna gick från 37 till 30 KB av det. Priset är att en sida med båda sorterna får
  två uppsättningar av den gemensamma koden (galleriet 55 → 64 KB); att flytta den gemensamma koden
  till icke-generiska funktioner med radläget som konstant argument vore nästa steg, inte gjort.
- **Flerradsfältet bryter själv** vid ordgränser med `text_measure` per ord och ritar varje visuell rad
  med `dl_text` utan radbrytning, så markör, markering och träfftest använder samma rader. Ett stycke
  som inte ändrats bryts inte om (det hittas på sitt gamla index eller ett steg bort), så skrivning i ett
  stycke kostar en mätning per ord i det stycket.
- **`form` portad, `settings` inte.** `form` ritar textfälten och knappen med `ui::TextBox` och
  `ui::Button` under ett tema med sidans färger (platshållaren i ramfärgen, ringen i genomskinlig
  accent). Alla fem `form`-fall och skalfallet står på 0,000 %, och `conformance --update` skrev om
  guldbilderna och träden byte för byte lika. Kryssrutan är kvar som sidans egen kod: den markerar fokus
  med accentfärgad ram, `ui::Checkbox` med en ring, och fallet `form-800x600@1-filled` slutar med
  rutan fokuserad. Storleken 16,7 → 30,5 KB är priset för riktig redigering (markör, markering,
  urklipp, IME-komposition), som sidan inte hade. Ett enradsfält gör nu en inklistrad radbrytning till ett
  mellanslag; förr fick namnfältet två rader. `settings` portas inte: kontrollerna registreras med
  automatiska id och sviten jämför träden med id, så fasta id skulle ändra `settings*.json`;
  vippknapparna är inte kryssrutor till utseendet, och sliderns spår, värde och ring är ritade annorlunda.
- **Konformansens Ctrl+C visar nekandet.** Den huvudlösa värden har inget urklipp och levererar inte
  Ctrl+C som kopieringsgest, så `widgets-800x600@1-typed` visar statusraden "Ctrl+C: urklippet nekades
  (E_DENIED)". Det är värdens svar, och sidans hantering av det syns i trädet.
- **Guiden är provad**: `docs/SDK.md` avsnitt 8 är en hel sida (Att göra) mellan markören
  `<!-- exempel: src/lib.rs -->` och manifestet i avsnitt 4; `sdk_guide_example_builds` bygger den till
  wasm utan varningar, kontrollerar ABI:t och utkastet, lägger till en uppgift i vy-runtime och kör sidans
  eget test. README-avsnitten per komponent är korta utdrag ur samma mönster; de kompileras inte (en
  doctest av README:n skulle kräva dolda rader som en agent som läser filen rakt av skulle snubbla på),
  men varje komponent har en kompilerad exempelsida: galleriet.
- **Den publicerade sviten har repots layout** (`conformance/`, `target/wasm/`, resurserna under
  `modules/…`), så `vy-cli conformance --root <uppackat>` kör den utan ändring; SPEC.md, LICENSE och
  `SHA256SUMS` följer med. Filerna sorteras och får senaste commitens tid (eller `SOURCE_DATE_EPOCH`), och
  `zip -X` sparar inga extra attribut: två körningar ger samma byte (SPEC-9.7.1). Det engelska avsnittet i
  `conformance/README.md` säger vad en annan klient ska producera (en PNG per fall i fysisk storlek, trädet
  som JSON byte för byte i den form `vy-cli --dump-semantic` skriver), hur den deterministiska värden beter
  sig (klocka, xorshift64*-fröet, inget urklipp) och hur godkänt avgörs (0,1 %, SPEC-12.2.2; trädet exakt).
- **Ett steg i `release.yml`.** Ett Actions-steg kan inte både köra ett skript och ladda upp en artefakt,
  så steget använder en liten sammansatt åtgärd, `.github/actions/conformance/action.yml`, som gör båda;
  release-jobbet fäster redan varje artefakts zip vid releasen.

## Grindar och tal

Slutläget, i grenens egen byggkatalog: `cargo fmt --all --check`, `cargo clippy --workspace --all-targets
-- -D warnings`, `cargo test --workspace`, `python3 tools/spec_trace.py` (140/140),
`tools/build_modules.sh` + `check-abi` + `tools/test_modules.sh` (13 moduler, wasm32-clippy) och
`cargo run -p vy-cli -- conformance` 64/64 à 0,000 % (59 befintliga orörda, fem nya), gröna före varje
commit utom d0309fc: modulen kom före sina fall, så `spec_12_1_2_conformance_defines_client` var röd
mellan d0309fc och f5b354a (som studio i m4/dev). `cargo doc -p vy-sdk --no-deps` med `-D warnings` går
igenom, och `cargo clippy -p vy-sdk --target wasm32-unknown-unknown -- -D warnings` också.

## Spec-frågor

1. **Fokus för en lista med markering.** Ingen fokuserbar roll passar en lista där piltangenterna flyttar
   markeringen (ARIA:s `listbox`/`option` med rullande tabindex). Med 1.0:s roller kan Tab inte nå en
   `ui::List`; den blir aktiv med pekaren eller för att sidan säger det. Förslag för 1.1: en fokuserbar
   roll för en markeringsbar lista, med fokus på listan och markeringen i texten.
2. **Hela trädet för långa listor.** SPEC-10.1.1 kräver noder även utanför fönstret, så en lista med
   10 000 rader kostar 10 000 `sem_node` per frame (taket är 65 536, SPEC-9.3.2). Ska en lista få
   registrera raderna i bild och sin längd (som ARIA:s `aria-setsize`/`aria-posinset`)?
3. **Tillstånd i nodtexten.** En nod har roll och text och inget mer (SPEC-5.2.13), så "markerad",
   "vald" och sliderns värde står i texten ("Nyheter (markerad)", "Volym: 40"), på sidans språk
   (`Words::SV`, `Words::EN`). Ett hjälpmedel kan inte skilja tillståndet från namnet. Ett tillståndsfält
   (kryssad, vald, värde, min, max) vore en 1.1-utökning med eget nummer.

## Berör andra spår

- `crates/vy-shell/src/lib.rs` (testet `spec_5_2_12_clipboard_copy_paste_through_the_shell`, m4/ux):
  testet klistrar in `"Anna\r\nLind "` i `form`s namnfält och väntade sig radbrytningen kvar. Fältet är
  nu ett enradigt `ui::TextBox`, som gör en radbrytning till ett mellanslag (ett enradsfält som visar två
  rader var felet förr), så väntevärdet är `"Anna Lind "`. Skalets normalisering av radslut prövas av
  `clipboard::tests` (`normalize_line_endings`), som är orört. Bara väntevärdet och kommentaren ändrade.
- `.github/workflows/release.yml` (m5/release): ett nytt steg i jobbet `site`, *Package the conformance
  suite*, som använder den nya sammansatta åtgärden `.github/actions/conformance/action.yml` (kör
  `tools/package_conformance.sh` och laddar upp `vy-conformance-<version>.zip` som artefakt; release-jobbet
  fäster redan varje artefakts zip). Inget befintligt steg ändrat. Filhuvudets lista över tillgångar och
  "de elva paketen" (nu tolv: `publish_site.sh` publicerar `widgets` av sig själv) är ert att uppdatera.
  Skriptet bygger modulerna med `tools/build_modules.sh` (ert; `--locked` fungerar, `widgets` har en
  `Cargo.lock`).
- `crates/vy-dev/src/scaffold.rs` och `tests/scaffold_builds.rs`: mallen, startsidan och testet (mitt
  område enligt uppdraget); raden med `SDK_GIT` är orörd. Med en `SDK_GIT` som pekar på en tagg före
  den här grenen bygger startsidan inte (den använder `vy_sdk::ui`): taggen som m5/release pinnar måste
  innehålla grenen. `vy-dev new` skriver fortfarande ut `vy-cli render …` utan `--grant ime` i sina
  nästa-steg (`main.rs`, inte ändrat); startsidan ritas ändå, eftersom `ime_begin` bara anropas vid fokus.
- vy-dev:s tester `build_pipeline.rs` och `serve_end_to_end.rs` redigerar startsidan på fasta ställen:
  `fn press(`, ramens två första rader (`let l = self.layout()?;` / `let x = l.card.x + PAD;`) och
  rubriken "Hej från Vy". Den nya startsidan behåller alla tre, så testerna är orörda.
  `origin_boundary.rs` lade till `"capabilities": {"net": …}` efter `memory_max`; utkastet har nu redan
  `"capabilities": {"ime": {}}` (startsidans textfält, SPEC-10.4.2), så testet lägger `net` bredvid
  `ime` i stället (och kontrollerar att raden finns). Inget annat i testet ändrat.
- `tools/fake_agent.sh` och `tools/fake_agent.ps1` (m4/dev:s testagent): redigeringen de gör i
  startsidan letade efter "Inga klick", som den gamla startsidan hade; nu "Ditt namn" (etiketten i den
  nya, ASCII så att `.ps1` förblir ASCII). Inget annat ändrat.
- `vy-cli conformance` har inget urklipp och levererar inte Ctrl+C/Ctrl+X som kopieringsgest
  (`Runtime::event_as_copy_gesture`), så ett fall kan bara visa att värden nekar. Ett minnesurklipp i
  körningen skulle låta ett fall visa en lyckad kopiering; inte gjort (vy-cli är inte mitt).
- `vy-cli bench`: raderna `compile.form` och `compile-hit.form` växer med modulen (16,7 → 30,5 KB) mot
  `conformance/bench/build-vm.json`; det är mer kod att kompilera, inte en långsammare värd. Nya rader för
  `widgets` saknar baslinje och hoppas över av `--compare`. Inte uppmätt här (ingen release-build av
  vy-cli i den här byggkatalogen).
- `docs/DEVELOPING.md` (m4/dev) nämner inte guiden; en rad om `docs/SDK.md` där vore bra för den som
  börjar från releasezippen. Inte ändrad.

## Kvar och öppet

- Textfältet flyttar markören per `char`, inte per grafemkluster; ingen ångra, ingen dubbelklicksmarkering
  av ord, höger-till-vänster-text får vänster-till-höger-positioner. Upp/Ned i ett enradsfält gör inget
  (tas ändå, så att en yttre rullyta inte rullar).
- Ingen inaktiverad knapp och ingen hovring (komponenterna ritar om bara på händelser som ändrar något).
- `List` och `ScrollView` inuti varandra fungerar bara med ordningen inre före yttre; två rullytor i
  varandra får båda hjulet när den inre är vid sin ände (avsiktligt: rullningen kedjar vidare).
- En sida med båda radlägena bär två kopior av fältets kod (se *Beslut*).
- Komponenternas utseende (höjder, radier, ringen) är fasta konstanter i `ui`; temat byter färger, typsnitt
  och storlek men inte geometri.
