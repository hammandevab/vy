# M4 – rättningar i vy-shell efter granskning 0006, 0007 och 0009

Byggagentens noteringar för grenen `m4/fix-shell` (från `m4/integration` f986c0e). Bara
`crates/vy-shell` och SPEC.md (ett `spec:`-commit) är ändrade. Granskningssidan skrivs av en
annan agent.

## Fynd, commit och test

| Fynd | Commit | Vad | Test |
| --- | --- | --- | --- |
| 0007:1 (och 0006:3) | `c62b711` | Inspelning och uppspelning görs av huvudprocessen. Skrivartråden för en inspelad flik skriver varje ram den skickar fliken även till filen (efter att ramen gått ut på röret, och inget efter `Shutdown`); för en uppspelning matar skrivartråden fliken med filens ramar, och skalets egna meddelanden släpps. Barnet är alltid samma låsta `--tab`: flaggorna `--record`/`--replay` finns inte längre (ger användningsfel, exit 2), och strict-läget på Windows gäller nu även dessa flikar. Filformatet är oförändrat, med `ToTab::StopRecording` sist när användaren stoppar. | `spec_9_1_3_recording_and_replay_are_sandboxed` (ny, `tests/sandbox.rs`): sonden med `--record`/`--replay` avvisas utan att någon fil skapas; en inspelande flik har `Seccomp: 2` och `NoNewPrivs: 1` i `/proc/<pid>/status`; filen är exakt ramarna fliken fick; uppspelningen ger samma svar. Föll före rättningen (sonden: allt tillåtet; `Seccomp: 0`). `spec_11_4_1_devtools_record_and_replay` och `spec_11_4_1_record_replay` gröna. |
| 0007:4 | `291ddd1` | `prlimit64` borttaget ur seccomp-listan, inte bara argumentkontrollerat: std och glibc läser `RLIMIT_STACK`/`RLIMIT_NOFILE` vid uppstart, före låsningen. Spårat med `strace -f` över hela vy-shells lib-svit (24 låsta flikprocesser) och åtta referenssidor via `--screenshot`: inget `prlimit64` och ingen `clone` efter `seccomp`. | `sandbox::platform::tests::filter_layout`, omskrivet: en liten tolk för klassisk BPF kör filtret som kärnan gör. `prlimit64` nekas för varje pid (0, egen, 1, annan); `tgkill` bara till egen pid; `clone` bara med `CLONE_THREAD`; `clone3` `ENOSYS`; `openat`, `socket`, `connect`, `execve`, `ptrace`, `kill`, `process_vm_writev`, `setrlimit` `EPERM`; främmande arkitektur och x32 dödas. Föll med `prlimit64` kvar. |
| 0007:5 | `6a998ec` | Ny IPC-ram `ToShell::Sandbox(Report)` (tagg 78): flikprocessen skickar vad låsningen gjorde och inte gjorde precis innan den hanterar sin första `Load`, alltså innan någon sidkod körts i processen. Huvudprocessen tar bara den första rapporten medan fliken laddar; en senare loggas som ignorerad, och en laddning som besvaras utan rapport räknas som ingen sandlåda. Skalet lägger till sina egna lager och deras fel (Windows: `job object`, i strict `AppContainer`). Är sandlådan inte hel visas `sandlåda: delvis` (något lager saknas) eller `sandlåda: av` (inget lager) i adressfältet, och detaljen loggas som `[host warn] sandbox: …` i flikens logg (devtools "Logg"). | `spec_9_1_3_sandbox_off_is_shown` (egen testbinär `tests/sandbox_off.rs`, `VY_SANDBOX=off`, riktig flikprocess genom det huvudlösa skalet: `sandlåda: av` i chromen, detaljen i loggen), `spec_9_1_3_sandbox_state_in_the_chrome`, `spec_9_1_3_sandbox_marker_in_the_chrome`, `spec_9_1_3_sandbox_reported_before_the_first_load`, `sandbox::tests::report_marker`. Inspelnings- och uppspelningsflikarna rapporterar sondens låsning (testet för 0007:1). |
| 0006:1 | `7d8b7ff` | En släppt flikprocess (navigering, F5, Ctrl+W, fönstret stängs) får `Shutdown` direkt och lämnas till en `process::Reaper`, som avslutar den senast vid sin tidsgräns (`SHUTDOWN_GRACE`, 500 ms) på en egen tråd. UI-tråden väntar aldrig. Läsartråden levererar som förut, så lagringssvaret sparas när det kommer. `Shell::shutdown` släpper alla flikar med en gemensam tidsgräns och sparar svaren medan de kommer: högst en tidsgräns i allt, och kortare om alla svarar. En sida vars origin har en släppt process som ännu kan svara parkeras tills svaret kommit eller processen avslutats, så att en omladdning läser det som just sparats. | `spec_5_2_9_letting_tabs_go_waits_once`: åtta ersättningsprocesser (riktiga rör, ingen process, svarar aldrig). Ingen stängning väntar; alla dödas först efter hela tidsgränsen och inom mindre än en tidsgräns från varandra. Föll med den blockerande nedstängningen. `spec_5_2_9_a_page_waits_for_its_origins_storage` föll utan parkeringen. `spec_5_2_9_storage_saved_on_reload_and_close` grönt: det väntar nu på `Shell::retiring()` i stället för en ensam `drain()`, med samma påståenden. |
| 0006:2 | `dbbb1c8` (`spec:`) | En mening i SPEC-8.2.4, ordagrant enligt uppdraget men med kodformat på `TEXT_INPUT` och `clipboard` som i resten av specen. Ingen kod ändrad. | `spec_8_2_4_paste_is_user_input_even_restricted`: en lokal fil i begränsat läge utan capabilities. Den inklistrade texten når sidan som `TEXT_INPUT`, Ctrl+V når den inte som tangent, och sidans `clipboard_read` direkt efter är `E_DENIED`. Beteendet fanns redan, så testet föll inte före; det låser beslutet. |

Ändrade befintliga tester: `filter_layout` (omskrivet), `spec_11_2_1_messages_round_trip` (15
`ToShell`-varianter), `spec_9_3_2_main_process_holds_tab_messages_to_the_caps` (rapportens tak:
32 rader à 1 024 byte per lista), `spec_5_2_9_storage_saved_on_reload_and_close` (väntan, se
ovan), `probe_text_lists_attempts` (`sandbox::applied`). Inga guldbilder ändrade.

## Grindar vid sista kodcommit (`dbbb1c8`)

fmt rent; clippy `-D warnings` rent (även `--target x86_64-pc-windows-msvc` och
`aarch64-apple-darwin` för vy-shell); `cargo test --workspace` **361 gröna, 0 röda, 4
ignorerade** (före: 352/4); `spec_trace.py` 139/139; modulgrinden (12 moduler, ABI 1.0 OK,
12/12) före första konformanskörningen; konformans **59/59 à 0,000 %**; `check-cross
--workspace --all-targets` rent. Varje commit passerade samma grindar. vy-shells svit kördes
fyra gånger i rad efter 0006:1 utan fel.

## Att veta vid sammanslagningen

- **`tab::main(record, replay)` behåller sina två parametrar** bara för att `vy-cli --tab`
  (`vy_shell::tab::main(None, None)`) ska bygga orört. En fil där avvisas med
  `ErrorKind::Unsupported`. Uppföljning i vy-cli: ta bort argumenten, sedan parametrarna.
- **Rapporten skickas vid första `Load`, inte först i strömmen**, eftersom vy-clis
  `tab_mode_serves_the_protocol` väntar sig `Idle` som första ram efter en ensam `Tick`.
- **Fuzzfrö för `ToShell::Sandbox` saknas.** `fuzz/` ligger utanför spåret. Lägg
  `("sandbox.bin", ToShell::Sandbox(..).encode())` i fröskrivaren
  (`tests/fuzz_regressions.rs`) och kör `VY_WRITE_FUZZ_SEEDS=1` i ett commit som också får
  röra `fuzz/corpus/ipc/`.
- **Konformansen kan nu visa sandlådan.** Skalfallen ritar chromen. På en CI-maskin där ett
  lager inte går (t.ex. Landlock avstängd i kärnan eller en container, eller `sandbox_init`
  på macOS) syns `sandlåda: delvis` i `shell-*.png.new` och skalfallen faller. Det är
  avsiktligt: det är just fynd 5. Orsaken står i flikens logg och i
  `vy-shell --tab --sandbox-probe`.
- Andra spårs noteringar (m4-security steg 4, m4-ux steg 2 och 6, m4-integration öppen punkt 3)
  beskriver det gamla läget: `--record`/`--replay` i barnet och väntan i `retire_process`. De
  är inte ändrade här.
- Ett skrivfel i inspelningsfilen (t.ex. full disk) stoppar inspelningen med en rad på
  stderr. Fliken körs vidare, och filen är avkortad.

## Windows (och macOS) – att kontrollera för hand

Allt plattformsspecifikt här är bara typkontrollerat och clippy-kontrollerat.

1. **Inspelning i sandlådan.** F12 → "Inspelning" → "Spela in…" på `form`, rör sidan,
   "Stoppa", spela upp filen. Det ska fungera som förut. I Aktivitetshanteraren (kolumnen
   Integritetsnivå) är både den inspelande och den uppspelande fliken **Låg**. Med
   `set VY_SANDBOX=strict` och binären under `C:\Program Files\Vy\` är de **AppContainer**
   (det gick inte förut, eftersom barnet öppnade filen).
2. **Ingen markör i standardläget.** Visas `sandlåda: delvis` utan miljövariabel:
   rapportera loggraden (F12 → "Logg", `[host warn] sandbox: …`) och Windows-versionen.
   Raden säger vilket lager som föll (jobbobjekt, låg integritet eller en av de tre
   processpolicyerna).
3. **`set VY_SANDBOX=off`**: chromen visar `sandlåda: delvis` (jobbobjektet står kvar), och
   loggen `[host warn] sandbox: job object; not applied: VY_SANDBOX=off`.
4. **Strict som faller tillbaka.** `set VY_SANDBOX=strict` med binären i Hämtade filer:
   chromen visar `sandlåda: delvis` och loggen `AppContainer: …`. Under Program Files syns
   ingen markör.
5. **Stängning.** Öppna `settings` i tio flikar, slå om ett val i några och stäng fönstret.
   Det ska stängas inom ungefär en halv sekund, och valen ska finnas kvar vid nästa start.
   Ctrl+W och F5 ska aldrig frysa fönstret.
6. **macOS.** Kör `vy-shell --tab --sandbox-probe`. Om `sandbox_init` inte tar profilen
   visar chromen `sandlåda: delvis`, inte `av`, eftersom "inherited descriptors closed"
   räknas som ett lager. Loggraden säger `sandbox_init: …; running unsandboxed`.

## Frågor till ägaren

- **Windows standardläge räknas som helt** (ingen markör), fast det inte hindrar läsning och
  nät (spec-fråga 62). Ska chromen visa `sandlåda: delvis` där tills strict är standard?
- **`prlimit64` spårades bara på Linux x86_64 med glibc 2.39.** På aarch64 och med en annan
  libc kan anropet förekomma efter låsningen. I så fall nekas det med `EPERM`; vad anroparen
  gör då är inte provat. En körning av vy-shells tester under `strace` på aarch64 vore bra.
