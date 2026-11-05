# M4: bygg direkt i verktyget – `vy-dev` och Studio

Byggagentens noteringar för grenen `m4/dev` (från main `3bc0bcb`). Granskningssidan skrivs av en
annan agent. Spåret: en användare startar `vy-dev` i en projektkatalog, öppnar Studio i Vy, skriver
"gör en sida som …", och Claude Code bygger sidan, som visas i Vy.

Rörda kataloger: `crates/vy-dev` (ny), `tools/fake_agent.sh` och `tools/fake_agent.ps1` (nya),
`docs/notes/m4-dev.md` (den här sidan), `Cargo.toml` (en medlemsrad), `Cargo.lock`.

## Vad som landat

| Steg | Innehåll | Test |
| --- | --- | --- |
| 1 `vy-dev new` | `vy-dev new <dir> [--name <n>] [--sdk <vy-checkout>]` skriver ett sidprojekt: `Cargo.toml` på `vy-sdk` (sökväg till checkoutens `crates/vy-sdk` med `--sdk`, annars git-beroendet `https://github.com/hammandevab/vy`), `src/lib.rs` (text, räknare och en knapp med fokusring, `layout()`-mönstret, ett nativt test), `manifest.json` (utkast i formen av `conformance/manifests/*.json`, `published` = dagens datum), release-profilen från `modules/hello` (inget `.cargo/config.toml` finns där), `.gitignore` och en `CLAUDE.md` för en sidskrivande agent: loopen, reglerna, *Klart när*, ett API-sammandrag och SDK:ts README inbäddad (`include_str!`), så agenten inte behöver nätet. | `scaffold_builds_to_wasm_and_passes_check_abi` (bygger till wasm32 här utan varningar, `check_abi_bytes` OK, utkastet validerar mot modulen, första framen har fem semantiska noder, mallen är rustfmt-ren, projektets eget `cargo test` grönt), `api_summary_covers_the_sdk` (varje import, event och roll i SDK:t nämns i sammandraget), `writes_a_project`, `names`, `civil_dates` |
| 2a `vy-dev build` | Byggkedjan som ett steg: `cargo build --release --target wasm32-unknown-unknown --message-format=json-render-diagnostics` (diagnostiken renderad på stderr = loggen, JSON på stdout namnger modulen var målkatalogen än ligger), ABI-kontrollen (`vy_runtime::check_abi_bytes`, samma kod som `vy-cli check-abi`), utkastet mot modulen och resurserna (`vy_publish::load_resources` + `vy_publish::build`: `memory_max`, bildstorlekar, signatur) och paketet skrivet atomärt (temporär fil + `rename`). Loggen strömmas rad för rad till den som anropar och hålls till de sista 64 KB. `vy-dev build [<dir>] [--key] [--out]` kör den från kommandoraden. Nyckeln: `--key`, annars `VY_DEV_KEY`, annars `<datakatalog>/dev.key` (samma katalog som `vy-shell`: `%APPDATA%\vy`, `~/Library/Application Support/vy`, `~/.config/vy`), skapad med `vy_publish::keygen`/`write_key` första gången. | `build_pipeline_signs_a_valid_package_and_reports_failures` (giltigt signerat paket; trasig källa och för litet `memory_max` ger fel med orsak i loggen och inget paket), `finds_the_module_among_cargo_messages`, `writes_atomically`, `log_keeps_the_tail`, `keys_are_made_once`, `opens_a_scaffold_and_fingerprints_it` |
| 2 `vy-dev serve` | `vy-dev serve [<dir>] [--port 8080] [--studio-port 8081] [--key <dev.key>] [--agent-cmd <cmd>] [--quiet]`. Två `tiny_http`-servrar, båda på `127.0.0.1`: origin A `vy://localhost:8080/<namn>` är en `vy_serve::Site` över `target/vy-dev/site/` som öppnas på nytt efter varje lyckat bygge (blobbar från gamla byggen släpps), origin B `vy://localhost:8081/` är Studio (steg 5) och dev-API:t under `/_dev/`. Nyckeln och dess `.pub` (`/.well-known/vy-key` på båda) som i 2a; skapad nyckel skrivs ut. En byggtråd med kondvariabel kör kedjan vid start och på begäran; en bevakare läser källornas mtime och längd var 500 ms (`src/`, `res/`, `Cargo.toml`, `manifest.json`, utkastets resurser; inte `Cargo.lock`, som cargo själv skriver) och begär ett bygge när filerna stått still ett intervall och skiljer sig från vad förra bygget utgick från. Byggfel hamnar i byggloggen och statusen, sidan från senaste lyckade bygge serveras vidare. Manifest och paket får `Cache-Control: no-cache`. | `serve_builds_and_serves_two_origins` (start → bygge → signerat paket på A, nyckeln på båda, sidan finns inte på B och API:t inte på A, vakten nekar `Origin`, `Sec-Fetch-Site`, `Referer`, fel `Host` och `X-Forwarded-Host`-förfalskning, `POST /_dev/build` ger samma paket, en trasig källa ger `error` i tillståndet medan sidan finns kvar, en rättad källa byggs av bevakaren, `vy-dev build` från kommandoraden ger samma paket), `fresh_rewrites_cache_control_for_pages_only`, `loopback_hosts`, `guard_refuses_browsers_and_foreign_hosts` |
| 3 dev-API och agent | På origin B: `GET /_dev/state` → `{name, address, build: {status, log, builds}, agent: {running, last_exit}, seq}`; `POST /_dev/build` → 202; `POST /_dev/agent` (kroppen = uppdraget, UTF-8, högst 64 KB) → 202, 409 när en agent redan kör, 400/413 för tomt, ogiltigt eller för långt, 500 med orsak när programmet inte startar; `POST /_dev/agent/stop` → `{stopped}`; `GET /_dev/agent/events?after=<seq>&wait=<ms>&raw=0&limit=<n>` → `{next, entries: [{seq, kind, text, status?}]}`, long-poll upp till 20 s. Agenten startas som `<cmd> -p --output-format stream-json --verbose --permission-mode acceptEdits --allowedTools "Read,Edit,Write,Glob,Grep,Bash(cargo *),Bash(vy-cli *),Bash(vy-dev build*),Bash(ls *)"` i projektkatalogen med uppdraget på stdin, `vy-dev`s egen katalog först på `PATH` (där `vy-cli` ligger i en release) och `VY_DEV_KEY` satt. Kommandot: `--agent-cmd`, annars `VY_AGENT_CMD`, annars `claude`; på Windows slås ett namn utan filändelse upp med `PATHEXT` (på `PATH`, eller bredvid sig själv för en sökväg), så `claude.cmd` hittas. Utdata läses rad för rad in i en ringlogg (4 000 poster, 8 KB per post): varje rå rad (`raw`), assistentens text (`assistant`), verktygets namn och vad det rörde (`tool`, t.ex. `Edit src/lib.rs`), slutresultatet (`result`), start/stopp/exit (`agent`) och byggen (`build` med `status`). En agent i taget; när den avslutas begärs ett bygge; under tiden begär bevakaren inget. Stopp: `taskkill /T /F` på Windows, annars processträdet ur `ps` med `SIGTERM` och efter 1,5 s `SIGKILL`. | `agent_runs_streams_stops_and_triggers_a_build` med `tools/fake_agent.sh` (uppdraget kommer fram på stdin med exakt de fasta flaggorna, händelserna kommer som text och verktygsnamn utan råa rader, 409 under en körning, stopp avslutar agenten och dess barnprocess, exit bygger och agentens ändring syns i modulen på A, råa rader med `raw=1`, tom long-poll efter `wait`, `limit` ger de nyaste, exitkod 3 rapporteras), `an_agent_that_does_not_start_is_reported`, `commands_parse`, `finds_scripts_with_pathext`, `process_trees`, `stream_json_lines_become_entries`, `select_filters_raw_and_keeps_the_newest`, `sequence_and_cap` |

## Beslut

- **Namnet är adressens sista segment och craten.** `[a-z][a-z0-9_-]*`, högst 64 tecken, inte ett
  namn cargo reserverar (`test`, `std`, `core`, `alloc`, `proc-macro`), inte `vy-sdk`/`vy-abi`
  (beroendet självt) och inte Windows enhetsnamn (`con`, `nul`, `com1` …), som cargo nekar där.
  Utan `--name` härleds det ur katalognamnet.
- **Sökvägen i `Cargo.toml` skrivs med `/`** även på Windows, så den behöver ingen escapning.
- **Projektet är sin egen workspace** (`[workspace]` i mallen), som modulerna i `modules/`: det
  bygger var det än ligger.
- **API-sammandraget i `CLAUDE.md` är handskrivet men vaktat**: testet läser `vy-sdk/src/api.rs`,
  `event.rs` och `sem.rs` och fäller bygget om en import, ett event eller en roll saknas i texten.
- **Modulen hittas genom cargos JSON-meddelanden**, inte en härledd sökväg: `CARGO_TARGET_DIR`,
  `.cargo/config.toml` och en delad målkatalog fungerar utan att `vy-dev` behöver veta om dem.
  `cargo` är `CARGO` när `vy-dev` själv startats av cargo (samma verktygskedja), annars `cargo` på
  `PATH` (`cargo.exe` på Windows hittas av `CreateProcess`).
- **Ett misslyckat bygge är ett utfall, inte ett fel**: `BuildOutcome { log, package, wasm_path }`,
  där `first_error()` är loggens första `error`-rad. Inget paket skrivs då, och ett tidigare paket
  rörs inte.
- **Uppdraget går på stdin, inte som argument.** `claude -p` läser prompten från stdin när inget
  promptargument ges (provat mot Claude Code 2.1.318: utan stdin svarar den "Input must be provided
  either through stdin or as a prompt argument"). På Windows är `claude` från npm en `claude.cmd`, och
  ett argument till en batchfil går genom `cmd.exe`: Rust vägrar radbrytningar i det och raden får
  vara högst ~8 000 tecken. Stdin har ingen av gränserna. `--allowedTools` tar dessutom flera värden
  (`<tools...>`), så ett promptargument efter det hade lästs som ett verktyg.
- **`Bash(vy-dev build*)` står i verktygslistan** utöver de uppräknade: *Klart när* i projektets
  `CLAUDE.md` kräver att manifestet validerar, och `vy-cli` har inget kommando för det.
- **Ingen egen processgrupp för agenten.** Med en egen grupp hade Ctrl+C på `vy-dev serve` lämnat
  agenten (och dess byggen) kvar i bakgrunden med användarens inloggning. Utan den når Ctrl+C hela
  förgrundsgruppen, och ett stopp från Studio går i stället processträdet ur `ps -A -o pid= -o ppid=`
  (POSIX, samma på Linux och macOS).
- **Bevakaren jämför mot vad bygget utgick från.** Byggtråden tar källornas fingeravtryck under
  låset innan cargo läser dem; bevakaren begär ett bygge bara när filerna stått still ett intervall
  och skiljer sig från det. Så ger agentens sista ändring, som exit-bygget redan täcker, inget extra
  bygge, och en ändring medan ett bygge pågår ger ett till efteråt.
- **Byggloggen strömmas** in i tillståndet medan bygget pågår (ett första bygge med git-beroendet
  klonar och kompilerar i minuter); `log` i `/_dev/state` är alltid det senaste byggets.
- **Händelseformatet** är ett objekt `{next, entries}` och inte en ren lista: med `raw=0` hoppar
  klienten över råa rader och behöver ändå veta var den ska fortsätta.
- **Testprojekt byggs i `target/vy-dev-tests`** (`CARGO_TARGET_DIR`), så upprepade körningar delar
  ett bygge av `vy-sdk` i stället för att fylla disken med en katalog per test.

## Hotbild

**Vad står på spel.** `POST /_dev/agent` startar Claude Code med användarens inloggning i
projektkatalogen, med `Bash(cargo *)` tillåtet. cargo kör byggskript, så den som kan anropa API:t kan
i praktiken köra godtycklig kod som användaren och spendera användarens krediter. API:t skyddas
därför som ett skal, inte som en webbsida.

**Originseparationen.** Sidan under utveckling serveras på origin A (`localhost:8080`), Studio och
API:t på origin B (`localhost:8081`). Origin är schema + värd + port (SPEC-9.5.1), och
standardpolicyn beviljar `net` automatiskt bara mot sidans egen origin (SPEC-8.2.3). En sida på A som
begär `net` mot B får alltså en *fråga* i adressfältet, aldrig ett tyst ja: säg nej till den. Agenten
skriver sidans manifest, så en lurad agent kan begära det – frågan är skyddet. API:t finns inte på A
(404) och sidan inte på B, vilket `serve_builds_and_serves_two_origins` visar.

**Vad mer skyddar API:t.**

1. *Bara loopback.* Båda servrarna binder `127.0.0.1`; inget på nätverket når dem.
2. *Host-kontroll mot DNS-rebinding.* `/_dev/` svarar bara när `Host` är `localhost`, `127.0.0.1`
   eller `[::1]` (med valfri port). En webbsida som fått sitt namn att peka på 127.0.0.1 skickar sitt
   eget namn i `Host` och får 403. `X-Forwarded-Host` läses inte för API:t (vem som helst kan sätta
   det), bara den rå `Host`.
3. *Inga webbläsare.* En begäran med `Origin`, `Referer` eller något `Sec-Fetch-*`-huvud nekas (403).
   Webbläsare sätter `Origin` på varje POST mellan origins och `Sec-Fetch-*` på allt, och en sida kan
   inte ta bort dem; Vy-klientens `vy-proto` skickar inget av dem (SPEC-7.5.1). Det stänger CSRF mot
   localhost: en "enkel" `fetch("http://127.0.0.1:8081/_dev/agent", {method: "POST", mode:
   "no-cors"})` från en webbsida når aldrig agenten. Inga CORS-huvuden skickas, så svaren går inte
   att läsa från en webbsida heller.
4. *Fjärrsidor i Vy* kan inte nå `http://localhost` alls: `vy_proto::transport_allowed` släpper bara
   igenom `http://` till loopback för en sida som själv ligger på loopback.
5. *En agent i taget*, och uppdraget är högst 64 KB.

**Ingen nyckel per körning.** Övervägd och avstådd: klienten kan inte sätta huvuden i `fetch_begin`,
så en nyckel måste ligga i Studios adress eller modul. Studio serveras utan inloggning på samma origin
som API:t, så den som kan nå API:t (en annan lokal användare, en sida på A som fått `net` mot B) kan
också läsa nyckeln ur Studio-paketet; och m4/ux öppnar Studio på den fasta adressen
`vy://localhost:8081/`. Nyckeln hade alltså inte stängt något som inte redan är stängt.

**Kvarstående risk.**

- *Andra användare på samma maskin* når `127.0.0.1:8081` och kan starta agenten. Kör inte
  `vy-dev serve` på en delad fleranvändarmaskin. En riktig lösning kräver att klienten säger vilken
  origin en begäran kommer ifrån (ett huvud som huvudprocessen sätter och en sida inte kan förfalska)
  – förslag till m4/security, se *Öppna frågor*.
- *Agenten själv* kan genom cargo göra allt användaren kan. Uppdraget är användarens eget och skrivs i
  Studio; prompt injection via filer i projektet är samma risk som när användaren kör Claude Code för
  hand i katalogen.

## Nya beroenden

Inga nya crates i trädet (`Cargo.lock` får bara posten för `vy-dev`). `vy-dev` beror på crates som
redan fanns i arbetsytan: `vy-manifest`, `vy-publish`, `vy-serve`, `vy-runtime` (för
`check_abi_bytes`), `tiny_http`, `serde_json` och `anyhow`.

## Öppna frågor

1. **Vem ringer?** Ett localhost-API kan inte skilja Studio från en annan Vy-sida som fått `net` mot
   samma origin, eller från en annan lokal process. Förslag (m4/security): huvudprocessen sätter ett
   huvud med sidans origin på varje `fetch_begin`-anrop, som `Origin` i webbläsare; då kan `vy-dev`
   kräva `vy://localhost:8081` och den tredje punkten i hotbilden blir en positiv kontroll.
2. **Windows-tvillingen av den falska agenten** (`tools/fake_agent.ps1`) och `taskkill`-vägen är
   typkontrollerade men inte körda här (ingen Windows i sandlådan); CI:s Windows-jobb kör
   `agent_end_to_end` med den.
3. **Byggtestet för tidsgränser** `vy_render::tests::spec_5_3_3_bench_32_rounded_clips_2000_rects`
   (1 s i debug) faller ibland här när fyra agenter bygger på två kärnor (1,1–1,3 s) och går igenom
   ensam. Oberoende av den här grenen.

## Berör andra spår

Inget utanför `crates/vy-dev` och `tools/` (nya `fake_agent.sh`, `fake_agent.ps1`) utöver
medlemsraden i `Cargo.toml` och `Cargo.lock`. `vy-publish`
och `vy-serve` används som bibliotek med sina befintliga publika funktioner; inget i dem är ändrat.

## Release

Kommer med steg 5.
