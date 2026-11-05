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
| Ctrl+V | adressfältet | klistra in (radbrytningar tas bort, högst 8 KB) |
| Ctrl+C, Ctrl+X | adressfältet | kopiera/klipp ut markeringen till systemets urklipp |
| AltGr+tangent | överallt | tecknet (t.ex. `@`, `{`, `\`), aldrig en genväg |
| Ctrl+L, Ctrl+T, Ctrl+W, Ctrl+Tab, F5, Alt+Vänster/Höger, Ctrl+Plus/Minus/0 | överallt | som i M3 (notes/m3-shell) |

På macOS gäller Cmd i stället för Ctrl.

## Nya beroenden

| Crate | Varför | Varför inte 200 rader själv | Drar in (nytt i låsfilen) |
| --- | --- | --- | --- |
| `arboard` 3.7, `default-features = false` (ingen `image-data`, ingen `wayland-data-control`) | Systemets urklipp för text på Windows, macOS och X11. | Tre plattformars urklipp: Win32 `OpenClipboard`/`CF_UNICODETEXT` vore ~80 rader, men X11 kräver en egen tråd som äger markeringen och svarar på `SelectionRequest` (INCR för stora texter, överlämning till urklippshanteraren vid avslut), och macOS kräver `NSPasteboard` via objc2. | Linux: `parking_lot`, `parking_lot_core`, `lock_api`, `scopeguard` (x11rb och resten fanns via winit). Windows: `clipboard-win`, `error-code`. macOS: `objc2-app-kit` (objc2 fanns). |

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
- Binärstorlek, `vy-shell` release på Linux, strippad: 21 958 136 byte (20,9 MiB) med `arboard`.

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

## Spec-frågor

Numreringen fortsätter efter notes/m3-shell (45–52).

53. **SPEC-8.2.4 mot SPEC-5.2.12/5.3.1.** Den nya meningen säger att en skrivning under kopieringsgesten
    lyckas utan capability; 5.2.12 rubricerar urklippet "(capability `clipboard`)" och 5.3.1 säger att
    ett anrop utan beviljad capability ger `E_DENIED`. Läsningen här är att 8.2.4 är en policyregel om
    *när* capabilityn anses beviljad (implicit, under gesten), så §5 är orört. Bör 5.3.1 få en hänvisning
    i 1.1, eller räcker det att 8.2.4 står i §8?
