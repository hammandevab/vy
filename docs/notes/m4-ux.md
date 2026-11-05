# M4 ux – urklipp, lagring, markering, sök, tillgänglighet, devtools

Byggagentens noteringar för grenen `m4/ux` (från `main` 3bc0bcb). Granskningssidan skrivs av en annan
agent. Ägda delar: `crates/vy-shell` (utom `process.rs`), additivt i `crates/vy-sdk`, `vy-cli`:s
`--dump-semantic`. Det som rör andra spår står under *Berör andra spår*.

## Vad som landat

| Steg | Vad | Test |
| --- | --- | --- |
| 1 | Systemets urklipp bakom Ctrl+C/V/X (`clipboard.rs`, `ClipboardBackend`): `arboard` i fönstret, minne huvudlöst och när systemets inte kan öppnas. Ctrl+V i sidan levererar urklippet som `TEXT_INPUT` (som om det skrivits), i bitar om 4 KB, `\r\n`/`\r` → `\n`, högst 1 MB. Ctrl+C/V/X i adressfältet går mot samma urklipp (inklistring högst 8 KB). Ctrl+C/Ctrl+X i sidan är en kopieringsgest (SPEC-8.2.4): sidans `clipboard_write` under just det `KEY_DOWN` lyckas utan capability. AltGr (Ctrl+Alt) skriver tecken i stället för att trigga genvägar. | `spec_8_2_4_copy_gesture_grants_clipboard_write`, `spec_5_2_12_clipboard_copy_paste_through_the_shell`, `spec_5_2_12_memory_clipboard_and_paste_chunks`, `spec_a_5_1_altgr_is_not_a_shortcut`, `spec_a_5_1_altgr_commits_text` |
| 2 | Lagring går inte längre förlorad vid F5, navigering och Ctrl+W. Flikprocessen som släpps blir `Retired` (flik, generation, origin, grant, kvot); dess svar på `Shutdown` (`vy_suspend` + `Storage`) sparas under den origin processen hade, fast fliken bytt generation eller stängts. Ctrl+W på sista fliken kör `Shell::shutdown`, som tömmer inkorgen. | `spec_5_2_9_storage_saved_on_reload_and_close` |
| 3 | Textmarkering (SPEC-10.2.2), värdens egen (`selection.rs`): dra från en textnod markerar (sidan får pekarhändelserna ändå), Skift+klick utökar, Ctrl+A tar alla textnoder, Escape och ett tryck utanför text tömmer. Markeringen är noderna i dokumentordning mellan ändarna, med teckengranularitet i ändnoderna; Ctrl+C (och Ctrl+X) kopierar de logiska strängarna med `\n` mellan noder. Ritas av huvudprocessen som en genomskinlig överlagring i sidytan (`compose::fill_blend`, chromens accentfärg); sidans display list rörs inte. Flikprocessen rapporterar fokus (`ToShell::Focus`) så att Ctrl+A i ett fokuserat textfält går till sidan. | `spec_10_2_2_text_selection` (un-ignorerad, document- och form-sidan), `spec_10_2_2_selected_text_is_logical`, `spec_10_2_2_glyph_granularity_in_wrapped_runs` |
| 4 | Sök på sidan (Ctrl+F, `find.rs`): en sökruta över sidytans övre högra hörn (egen display list, komponerad efter sidan som frågan; under den är ytan skalets), söker i det semantiska trädets text skiftlägesokänsligt med enkel Unicode-vikning (`char::to_lowercase`), visar "3 av 7"/"Inga träffar", Enter/Skift+Enter bläddrar med omlopp, Escape stänger, knapparna ‹ › × fungerar med pekaren. Träffarna markeras genom överlagringen (aktuell starkare). En träff utanför vyn hämtas in med syntetiska `SCROLL`. | `spec_10_1_1_find_on_page`, `spec_10_1_1_find_in_the_tree` |
| 5 | AccessKit (SPEC-10.2.1, `a11y.rs`): skalets ögonblicksbild (fönster, adressfält, sidans träd och fokus) blir en hel `TreeUpdate` – fönstret som rot, adressfältet (`TextInput`), sidan (`Document`, klipper sina barn till sidytan) och modulens noder med plattformsroll, text som namn eller värde och rektangel i fysiska fönsterpixlar; fokus är värdens. Fönstret skapar `accesskit_winit::Adapter` medan fönstret är osynligt och skickar en uppdatering bara när ett hjälpmedel lyssnar och sammandraget ändrats. Åtgärder: Focus flyttar värdens fokus som Tab, Click trycker och släpper i nodens mitt (och följer en registrerad länk där), SetValue stöds inte. En sida utan träd märks "otillgänglig" även i trädet; felet att markören aldrig visades (fliken skickar bara ändrade träd, så en sida utan noder skickade inget) är rättat: en första frame utan träd betyder inget träd. | `spec_10_2_1_accesskit`, `spec_10_3_1_focus_order`, `spec_10_4_1_inaccessible_marker` (alla tre un-ignorerade), `spec_10_2_1_roles_and_ids`, `spec_10_2_1_any_tree_is_well_formed` |

## Beslut

- **Urklippet är systemets, och sidan ser det bara med capability.** Huvudprocessen äger urklippet
  (`Config::clipboard`). En flikprocess startar med tomt urklipp och får systemets innehåll (`ToTab::Clipboard`)
  först vid en färsk gest – pekartryck eller tangentnedtryckning utan upprepning – och bara när
  `clipboard` är beviljad, och bara när innehållet ändrats sedan senast. Förr skickades skalets eget
  urklipp till alla flikar vid varje skrivning; nu når ingenting en flik som inte fått capabilityn.
- **Ctrl+V i sidan är inmatning, inte `clipboard_read`.** Texten levereras som `TEXT_INPUT` och sidan
  lägger den i sitt fokuserade fält, precis som skriven text; ingen capability behövs. Ctrl+V:s
  `KEY_DOWN` konsumeras av skalet (annars skulle en sida som själv klistrar in på Ctrl+V få texten
  två gånger). `KEY_UP` för V går fram som förut.
- **Kopieringsgesten (SPEC-8.2.4)** är en engångsram `ToTab::CopyGesture` som armar exakt nästa
  `ToTab::Event`; flikloopen levererar den med `Runtime::event_as_copy_gesture`, och `clipboard_write`
  släpps igenom under just det anropet. Huvudprocessen avgör igen när `ToShell::Clipboard` kommer:
  utan beviljad `clipboard` tas en skrivning bara emot inom 1 s efter en kopieringsgest i den fliken,
  och bara en per gest (flikprocessen kör sidans kod och litas inte på).
- **Varje skrivning rapporteras** (`Runtime::take_clipboard_write`), inte bara ändrade värden: systemets
  urklipp kan ha ändrats av ett annat program mellan två likadana kopieringar.
- **AltGr.** Windows och X11 rapporterar AltGr som Ctrl+Alt. Skalets Ctrl-genvägar kräver nu Ctrl utan
  Alt (eller Meta/Cmd), och fönstret skickar text för Ctrl+Alt (och för Option på macOS). Förr gick
  `@ { [ ] } \ $ |` inte att skriva på ett svenskt tangentbord, och AltGr+0 (`}`) nollställde zoomen.
- **Lagringens tömning (steg 2) använder `Shutdown` som tömningsbegäran.** Flikloopen svarar redan på
  `Shutdown` med `vy_suspend` och `ToShell::Storage` innan processen avslutas; ingen ny ram behövdes.
  Felet var att svaret släpptes: det kom under en generation fliken inte längre hade (F5, navigering)
  eller för en flik som inte fanns (Ctrl+W). Nu minns skalet den släppta processen (`Retired`, högst 64)
  och sparar dess svar med processens origin, grant och kvot; allt annat från en gammal generation släpps
  som förut. **Gränsen** är `TabConnection::shutdown` i `process.rs` (säkerhetsspårets fil): den väntar
  högst 500 ms på att processen avslutas och dödar den sedan, och den väntar in läsartråden, så svaret
  ligger i inkorgen när navigeringen fortsätter – före det laddaren svarar för nästa sida, så att en
  omladdning av samma origin läser det som just sparats. Uppdraget sade "cirka 250 ms"; 500 ms är den
  befintliga gränsen och inte min fil att ändra. En sida som inte hinner svara (fastnat i `vy_suspend`
  tills bränslet tar slut, eller en process som dödas) förlorar ändringar sedan senaste dolda läge, som
  förut. En krasch i huvudprocessen förlorar fortfarande allt sedan senaste suspend (lagring skrivs inte
  vid varje `state_put`).
- **Markeringen (steg 3) är värdens och bara logisk.** Valbara roller: `text`, `heading`, `link`,
  `listitem`, `cell` med text; kontroller (knappar, fält, rutor) lämnas åt sidan. En ände är (nod-id,
  byteoffset). Teckengranulariteten kommer från samma `TextEngine` och stilar som rasteriserade framen,
  genom dess befintliga `measure` och `caret` – ingen ny funktion i `vy-render`: en nods text knyts till
  den `TEXT`-op som har samma text och börjar i nodens rektangel; radbrytningarna hittas med binärsökning
  på `measure` över prefix till och med ett ords slut (ombrytningen är girig, så det som kommer efter ett
  ord flyttar det aldrig), och x inom raden med binärsökning på `caret`. **Approximationer:** ett ord som
  är bredare än raden (som motorn bryter inuti) räknas som en rad; höger-till-vänster-text får
  vänster-till-höger-positioner; en nod vars text inte är en enda op (bylines, bildtexter av två körningar)
  markeras hel. Försvinner en ändes nod ur trädet töms markeringen. Ctrl+X kopierar (sidans text kan inte
  klippas ut). Escape tömmer markeringen och går ändå till sidan (SPEC-10.3.1).
- **Fokus speglas i huvudprocessen.** Fokus ägs av runtime i flikprocessen (SPEC-10.3.1); en ny ram
  `ToShell::Focus(Option<i32>)` (tagg 77) skickas när det ändras, så att skalet vet om ett textfält har
  fokus (Ctrl+A) och, i steg 5, vad AccessKit ska få som fokus.
- **Sök (steg 4) läser trädet, aldrig pixlar**, med samma intervall-till-rektangel som markeringen
  (`selection::span_rects`). Träffar per nod, icke-överlappande, högst 10 000. En ny sökning börjar vid
  första träffen vars nod når in i vyn; en ny frame av samma sida behåller aktuell träff (samma nod och
  startbyte, annars nästa i dokumentordning). **Rullning är en heuristik:** rullning ägs av modulen och
  ABI:t är fryst, så skalet skickar `SCROLL` med `dy` = nodens mitt − vyns mitt (logiska pixlar, `c` = 0)
  när den aktuella träffens nod inte ligger helt i vyn, ett event per frame som sidan svarar med, högst 8
  per träff. En sida som rullar sitt huvudområde med `SCROLL` (referenssidorna gör det) får träffen
  mitt i vyn; en sida med flera rullbara ytor eller som ignorerar `SCROLL` får högst 8 meningslösa
  rullningar och sedan inget mer. `SCROLL` bär ingen pekarposition; en sida som rullar ytan under pekaren
  rullar den yta pekaren senast var över. Sökrutan fångar tangenterna när den har fokus (redigering som i
  adressfältet, Ctrl+A/C/X/V i fältet); ett klick i sidan ger sidan tangentbordet tillbaka utan att stänga
  rutan.
- **Tillgänglighetsträdet (steg 5) är en ren funktion av en ögonblicksbild** (`Shell::a11y_snapshot` →
  `a11y::tree_update`), testad utan fönster; `window.rs` äger bara adaptern. Rollkarta: text → `Label`
  (texten som värde, som AccessKit vill), heading → `Heading`, link → `Link`, button → `Button`, textbox →
  `TextInput` (värde), checkbox → `CheckBox`, radio → `RadioButton`, list/listitem → `List`/`ListItem`,
  image → `Image`, table/row/cell → `Table`/`Row`/`Cell`, nav → `Navigation`, main → `Main`, region →
  `Region`, dialog → `Dialog`, menu/menuitem → `Menu`/`MenuItem`, tab/tabpanel → `Tab`/`TabPanel`,
  progress → `ProgressIndicator`, slider → `Slider`, separator → `Splitter`. Nod-id: modulens `i32` i de
  låga 32 bitarna med bit 32 satt, så de aldrig möter skalets egna (1–4). **Föräldraregeln:** en nod är
  barn till sin `parent_id` bara om den noden kommer tidigare i dokumentordning, annars barn till sidan –
  så kan en fientlig sida (cykler, okända föräldrar, sig själv som förälder) aldrig ge AccessKit ett träd
  där en nod saknar väg till roten (AccessKit får panik på det). Rektanglar som inte är ändliga utelämnas.
  **Klippning (SPEC-10.1.1):** sidnoden har `clips_children`; nodernas rektanglar lämnas oklippta så att
  plattformen vet var noder utanför vyn finns. **Uppdateringar:** trädet byggs bara när AccessKit begärt
  det (`InitialTreeRequested`) och tills `AccessibilityDeactivated`, och skickas bara när
  `Snapshot::digest` ändrats – utan hjälpmedel kostar det inget. **Åtgärder:** Focus på en fokuserbar nod
  skickar `FOCUS` ut (gamla noden) och in (nya), båda `c = 1` – exakt det Tab levererar; runtime tar ett
  levererat `FOCUS` som sitt eget fokus (dokumenterat i `Runtime::event`), så ingen ändring i vy-runtime
  behövdes. Click är `POINTER_DOWN` + `POINTER_UP` i nodens mitt (pressen flyttar fokus först, som för
  musen) och följer en länk sidan registrerat där; en nod utanför vyn klickas i sina egna koordinater.
  Focus/Click på adressfältet fokuserar det. Allt annat (`SetValue`, rullning) stöds inte: sidan äger sina
  värden och sin rullning.

## Tangentbordstabell

Det testguiden skrivs från. "Sidan" betyder att tangenten går till modulen som `KEY_DOWN`/`KEY_UP`.

| Tangent | Var | Verkan |
| --- | --- | --- |
| Ctrl+V | sidan | urklippets text till sidan som inskriven text (`TEXT_INPUT`); hamnar i fokuserat fält |
| Ctrl+C, Ctrl+X | sidan, med markering | kopierar markerad text (logiska strängar, `\n` mellan noder); tangenten går inte till sidan |
| Ctrl+C, Ctrl+X | sidan, utan markering | går till sidan; sidan får skriva urklippet under tangenten utan capability (SPEC-8.2.4) |
| Ctrl+A | sidan | markerar all text på sidan; i ett fokuserat textfält (och på en sida utan text) går den till sidan |
| Escape | sidan | tömmer markeringen; går också till sidan |
| Dra med vänster knapp | sidan, från text | markerar text (sidan får pekarhändelserna ändå); Skift+klick utökar; klick utanför text tömmer |
| Ctrl+F | överallt | öppnar sökrutan (eller fokuserar den och markerar frågan) |
| Enter, Skift+Enter | sökrutan | nästa respektive föregående träff, med omlopp |
| Escape | sökrutan | stänger sökrutan och träffmarkeringarna |
| ‹ › × | sökrutan (pekare) | föregående, nästa, stäng; ett klick i sidan lämnar rutan öppen men ger sidan tangentbordet |
| Ctrl+V | adressfältet | klistra in (radbrytningar tas bort, högst 8 KB) |
| Ctrl+C, Ctrl+X | adressfältet | kopiera/klipp ut markeringen till systemets urklipp |
| AltGr+tangent | överallt | tecknet (t.ex. `@`, `{`, `\`), aldrig en genväg |
| Ctrl+L, Ctrl+T, Ctrl+W, Ctrl+Tab, F5, Alt+Vänster/Höger, Ctrl+Plus/Minus/0 | överallt | som i M3 (notes/m3-shell) |

På macOS gäller Cmd i stället för Ctrl.

## Nya beroenden

| Crate | Varför | Varför inte 200 rader själv | Drar in (nytt i låsfilen) |
| --- | --- | --- | --- |
| `arboard` 3.7, `default-features = false` (ingen `image-data`, ingen `wayland-data-control`) | Systemets urklipp för text på Windows, macOS och X11. | Tre plattformars urklipp: Win32 `OpenClipboard`/`CF_UNICODETEXT` vore ~80 rader, men X11 kräver en egen tråd som äger markeringen och svarar på `SelectionRequest` (INCR för stora texter, överlämning till urklippshanteraren vid avslut), och macOS kräver `NSPasteboard` via objc2. | Linux: `parking_lot`, `parking_lot_core`, `lock_api`, `scopeguard` (x11rb och resten fanns via winit). Windows: `clipboard-win`, `error-code`. macOS: `objc2-app-kit` (objc2 fanns). |

| `accesskit` 0.25 | Schemat för tillgänglighetsträdet (`Node`, `Role`, `TreeUpdate`); `a11y.rs` bygger trädet med det, även utan fönster. | Det är kontraktet plattformsadaptrarna läser; att skriva det själv vore att skriva adaptrarna. | `uuid` (utan standardfunktioner). |
| `accesskit_winit` 0.34 (`rwh_06`; ingen `accesskit_unix` som standard) | Plattformens tillgänglighets-API (SPEC-10.2.1): UI Automation på Windows, NSAccessibility på macOS, kopplat till winit-fönstret. | UIA kräver COM-servrar för provider-gränssnitten (tusentals rader i `accesskit_windows`), NSAccessibility en Objective-C-klass per nodtyp; ingen av dem går på 200 rader. | Windows: `accesskit_windows`, `accesskit_consumer`, `windows` 0.6x m.fl. (+20 crates, 218 → 238). macOS: `accesskit_macos`, `accesskit_consumer`, objc2-delar (+8, 239 → 247). Linux: +5 (265 → 270), adaptern gör inget. |
| (frivilligt) funktionen `a11y-linux` | AT-SPI över D-Bus på Linux/BSD: `accesskit_unix` med `async-io`. | – | +80 crates (270 → 350): `zbus`, `atspi`, `async-io`, `async-executor`, `blocking`, `event-listener`, `serde_repr`, `enumflags2`, `phf` m.fl. |

Binärstorlek, `vy-shell` release på Linux, strippad (`strip`), med koordinatorns `codegen-units = 13` för
arbetsytans crates (beroendena med profilens 1), så talen jämförs sinsemellan, inte med CI:s:

| Tillstånd | Byte | Skillnad |
| --- | --- | --- |
| steg 1 (med `arboard`, profilens `codegen-units = 1`) | 21 958 136 | – |
| steg 1–4, före AccessKit | 22 272 192 | – |
| steg 5, AccessKit utan Linux-adaptern (standard) | 22 299 880 | +27 688 |
| steg 5 med `--features a11y-linux` | 25 417 136 | +3 144 944 – **över** SPEC-12.3.1:s 25 MB |

Därför är AT-SPI på Linux frivilligt: med det går Linux-klienten över 25 MB (25,4 miljoner byte; under
25 MiB, se spec-fråga 54). Windows-binärens storlek kan inte mätas här (ingen MSVC-länkare); CI:s
release-jobb visar den. `accesskit_windows` drar `windows`-craten med COM-funktioner: räkna med något hundratal
kB.

Urklipp på Wayland utan XWayland: `arboard::Clipboard::new()` misslyckas, skalet loggar
`vy-shell: no system clipboard (…); using an in-memory one` och använder minnesurklippet – aldrig panik.
Samma sak i CI utan display.

Binärstorlek (release, Linux, strippad): se tabellen under *Grindar och tal*.

## Grindar och tal

- Delad byggkatalog med fyra arbetsträd på 2 CPU:er: `vy-render`:s två väggklocktester
  (`spec_5_3_3_bench_32_rounded_clips_2000_rects`, `spec_5_3_3_bench_65000_fullscreen_rects`, gräns 1 s i
  debug) föll ibland vid belastning 4–7 (1,02–1,5 s). `vy-render` är orörd på den här grenen; de gick
  igenom vid lägre belastning. Övriga grindar gröna vid varje commit: fmt, clippy, `cargo test --workspace`,
  spec_trace, konformans 55/55 à 0,000 %, typkontroll av `vy-shell` för Windows och macOS (`check-cross`,
  C-beroenden ersatta av tomma objekt).
- Binärstorlek: se *Nya beroenden*.

## Hotbild

- **Urklippsläsning** kräver fortfarande capability `clipboard` och en gest inom 1 s (SPEC-5.2.12). Det nya
  är att flikprocessen bara får systemets urklipp när capabilityn är beviljad och användaren just gjort
  en gest i fliken; en sida utan capability ser aldrig urklippet, inte ens i sitt eget minne.
- **Urklippsskrivning utan capability** (SPEC-8.2.4) är begränsad till det `KEY_DOWN` för Ctrl+C/Ctrl+X som
  användaren tryckt i sidan: en sida kan ersätta urklippet bara när användaren bett om en kopiering,
  med vad sidan vill – samma sak som på webben (`copy`-händelsen). Den kan inte läsa det. Huvudprocessen
  kräver gesten igen (inom 1 s, en skrivning per gest), så en komprometterad flikprocess kan inte skriva
  urklippet i bakgrunden. Test: `spec_8_2_4_copy_gesture_grants_clipboard_write` (flikloopen),
  `spec_5_2_12_clipboard_copy_paste_through_the_shell` (skalet, med en förfalskad `Clipboard`-ram).
- **Inklistring** är användarens egen inmatning: texten går som `TEXT_INPUT` till sidan som har fokus.
  Begränsad till 1 MB (samma som en sidas `clipboard_write`), i bitar om 4 KB så att en liten `vy_alloc`-arena
  räcker. Adressfältet tar högst 8 KB och inga styrtecken.
- **Tillgänglighetsträdet** ger plattformens tillgänglighets-API (och därmed varje hjälpmedel användaren
  kört) sidans logiska text – det är syftet (SPEC-10.2.1). Åtgärder från ett hjälpmedel är användarens egen
  inmatning och kräver ingen capability; Click räknas som en gest (pekarsläpp) precis som ett musklick, så
  en sidas `navigate("https://…")` direkt efter får öppna webbläsaren en gång, som efter ett klick. En sida
  kan inte påverka skalets noder (fönster, adressfält, sidnod, "otillgänglig"-etiketten): de har egna id:n
  utanför modulens intervall. Ett träd med 65 536 noder och 16 MB text (taket i SPEC-9.3.2) byggs om bara
  när det ändrats och bara när ett hjälpmedel lyssnar. På Linux pratar `accesskit_unix` med AT-SPI över
  sessionens D-Bus (zbus): en ny lokal yta i huvudprocessen, aldrig i flikprocessen.

## Berör andra spår

- `crates/vy-runtime/src/host.rs`, `src/lib.rs` (m4/perf): additivt – fälten `Host::copy_gesture` och
  `Host::clipboard_written`, `Runtime::event_as_copy_gesture`, `Runtime::take_clipboard_write`; grinden i
  `clipboard_write` släpper igenom under kopieringsgesten. Inget befintligt API ändrat. (Commit 26e77b6.)
- `crates/vy-shell/src/ipc.rs`: nya ramar `ToTab::CopyGesture` (tagg 13) och `ToShell::Focus` (tagg 77).
  Om ett annat spår (m4/security fuzzar ramningen) också lagt till varianter med samma taggar måste en av
  dem numreras om vid sammanslagning; testet `ipc::tests` räknar varianterna (13 respektive 14 här).
- `SPEC.md`: ny mening SPEC-8.2.4 (eget `spec:`-commit, §8 är inte fryst).
- `Cargo.toml`, `Cargo.lock`: `arboard` i `[workspace.dependencies]`.
- `crates/vy-shell/src/process.rs` (m4/security) är **inte** ändrad; steg 2 bygger på att
  `TabConnection::shutdown` väntar in processen och läsartråden. Ändras det (t.ex. till asynkron
  nedstängning) måste `Shell::retired_message` fortfarande få svaret före laddarens nästa svar.

## Kräver ett riktigt fönster – manuell kontroll på Windows

Allt ovan är testat huvudlöst; detta går bara att se i fönstret.

1. **Urklipp.** Kopiera text i Anteckningar. I Vy: `form`, klicka i Namn, Ctrl+V: texten står i fältet. Ctrl+L,
   Ctrl+A, Ctrl+C i adressfältet och klistra in i Anteckningar: adressen. På `document`: dra över en mening,
   Ctrl+C, klistra in i Anteckningar: samma text (radbrytning mellan stycken), inga bilder.
2. **AltGr.** Skriv `@`, `{`, `}`, `\`, `$`, `|` i adressfältet och i `form`: tecknen kommer, zoomen ändras inte.
3. **Lagring.** `settings`: slå om ett val, tryck F5 direkt: valet är kvar. Ändra igen, Ctrl+W, öppna
   `settings` i en ny flik: kvar.
4. **Sök.** `document`: Ctrl+F, skriv `macOS`: "1 av 3", träffarna gula, aktuell orange; Enter bläddrar och
   rullar sidan till träffen; Escape stänger.
5. **Skärmläsare (SPEC-10.2.1).** Starta Skärmläsaren (Ctrl+Win+Enter). I Vy: `form`. Tab genom fälten:
   Skärmläsaren läser fältets namn och innehåll; Caps Lock+pil navigerar texten på sidan; aktivera en knapp
   med Skärmläsarens standardåtgärd: sidan reagerar som på ett klick. På en sida utan träd säger den
   "otillgänglig". Accessibility Insights for Windows visar trädet (fönster → Adress, sidans dokument → noder).
6. **Devtools.** F12 (se steg 6).

## Spec-frågor

Numreringen fortsätter efter notes/m3-shell (45–52).

53. **SPEC-8.2.4 mot SPEC-5.2.12/5.3.1.** Den nya meningen säger att en skrivning under kopieringsgesten
    lyckas utan capability; 5.2.12 rubricerar urklippet "(capability `clipboard`)" och 5.3.1 säger att
    ett anrop utan beviljad capability ger `E_DENIED`. Läsningen här är att 8.2.4 är en policyregel om
    *när* capabilityn anses beviljad (implicit, under gesten), så §5 är orört. Bör 5.3.1 få en hänvisning
    i 1.1, eller räcker det att 8.2.4 står i §8?
54. **SPEC-12.3.1 "högst 25 MB" mot SPEC-10.2.1 på Linux.** AccessKit med AT-SPI (`accesskit_unix`, zbus) tar
    Linux-klienten från 22,3 till 25,4 miljoner byte – över 25 MB i decimal mening, under 25 MiB. Här är AT-SPI
    därför en frivillig funktion (`a11y-linux`) och Linux-klienten exponerar inget träd som standard; Windows och
    macOS gör det. Avses MB eller MiB? Ska taket höjas, ska Linux-tillgänglighet vara frivillig, eller ska något
    annat trimmas (typsnitten 2,2 MB, `pulley_interpreter` 0,2 MB enligt granskning 0004)?
55. **SPEC-10.1.1 "värden klipper trädet mot fönstret".** Här klipps det genom AccessKits `clips_children` på
    sidnoden, med nodernas rektanglar oklippta, så att plattformen vet att noder utanför vyn finns (som en
    webbläsare). Avses att noder utanför vyn inte ska exponeras alls?
