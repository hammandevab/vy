# M4: bygg direkt i verktyget – `vy-dev` och Studio

Byggagentens noteringar för grenen `m4/dev` (från main `3bc0bcb`). Granskningssidan skrivs av en
annan agent. Spåret: en användare startar `vy-dev` i en projektkatalog, öppnar Studio i Vy, skriver
"gör en sida som …", och Claude Code bygger sidan, som visas i Vy.

Rörda kataloger: `crates/vy-dev` (ny), `modules/studio` (ny), `docs/DEVELOPING.md` (ny), ett
icke-normativt stycke i SPEC §17, `conformance/manifests/studio.json`,
fyra fall i `conformance/cases.toml` med guldbilder och två semantiska träd, `tools/fake_agent.sh`,
`tools/fake_agent.ps1` och `tools/build_studio.sh` (nya),
`docs/notes/m4-dev.md` (den här sidan), `Cargo.toml` (en medlemsrad), `Cargo.lock`, och utanför
spåret `crates/vy-runtime/src/{host,lib,tests}.rs` och `crates/vy-cli/src/conformance.rs` med
formatraden i `conformance/cases.toml` och `conformance/README.md` (se *Berör andra spår*).

## Vad som landat

| Steg | Innehåll | Test |
| --- | --- | --- |
| 1 `vy-dev new` | `vy-dev new <dir> [--name <n>] [--sdk <vy-checkout>]` skriver ett sidprojekt: `Cargo.toml` på `vy-sdk` (sökväg till checkoutens `crates/vy-sdk` med `--sdk`, annars git-beroendet `https://github.com/hammandevab/vy`), `src/lib.rs` (text, räknare och en knapp med fokusring, `layout()`-mönstret, ett nativt test), `manifest.json` (utkast i formen av `conformance/manifests/*.json`, `published` = dagens datum), release-profilen från `modules/hello` (inget `.cargo/config.toml` finns där), `.gitignore` och en `CLAUDE.md` för en sidskrivande agent: loopen, reglerna, *Klart när*, ett API-sammandrag och SDK:ts README inbäddad (`include_str!`), så agenten inte behöver nätet. | `scaffold_builds_to_wasm_and_passes_check_abi` (bygger till wasm32 här utan varningar, `check_abi_bytes` OK, utkastet validerar mot modulen, första framen har fem semantiska noder, mallen är rustfmt-ren, projektets eget `cargo test` grönt), `api_summary_covers_the_sdk` (varje import, event och roll i SDK:t nämns i sammandraget), `writes_a_project`, `names`, `civil_dates` |
| 2a `vy-dev build` | Byggkedjan som ett steg: `cargo build --release --target wasm32-unknown-unknown --message-format=json-render-diagnostics` (diagnostiken renderad på stderr = loggen, JSON på stdout namnger modulen var målkatalogen än ligger), ABI-kontrollen (`vy_runtime::check_abi_bytes`, samma kod som `vy-cli check-abi`), utkastet mot modulen och resurserna (`vy_publish::load_resources` + `vy_publish::build`: `memory_max`, bildstorlekar, signatur) och paketet skrivet atomärt (temporär fil + `rename`). Loggen strömmas rad för rad till den som anropar och hålls till de sista 64 KB. `vy-dev build [<dir>] [--key] [--out]` kör den från kommandoraden. Nyckeln: `--key`, annars `VY_DEV_KEY`, annars `<datakatalog>/dev.key` (samma katalog som `vy-shell`: `%APPDATA%\vy`, `~/Library/Application Support/vy`, `~/.config/vy`), skapad med `vy_publish::keygen`/`write_key` första gången. | `build_pipeline_signs_a_valid_package_and_reports_failures` (giltigt signerat paket; trasig källa och för litet `memory_max` ger fel med orsak i loggen och inget paket), `finds_the_module_among_cargo_messages`, `writes_atomically`, `log_keeps_the_tail`, `keys_are_made_once`, `opens_a_scaffold_and_fingerprints_it` |
| 2 `vy-dev serve` | `vy-dev serve [<dir>] [--port 8080] [--studio-port 8081] [--key <dev.key>] [--agent-cmd <cmd>] [--studio <studio.vyp>] [--quiet]`. Två `tiny_http`-servrar, båda på `127.0.0.1`: origin A `vy://localhost:8080/<namn>` är en `vy_serve::Site` över `target/vy-dev/site/` som öppnas på nytt efter varje lyckat bygge (blobbar från gamla byggen släpps), origin B `vy://localhost:8081/` är Studio (steg 5) och dev-API:t under `/_dev/`. Nyckeln och dess `.pub` (`/.well-known/vy-key` på båda) som i 2a; skapad nyckel skrivs ut. En byggtråd med kondvariabel kör kedjan vid start och på begäran; en bevakare läser källornas mtime och längd var 500 ms (`src/`, `res/`, `Cargo.toml`, `manifest.json`, utkastets resurser; inte `Cargo.lock`, som cargo själv skriver) och begär ett bygge när filerna stått still ett intervall och skiljer sig från vad förra bygget utgick från. Byggfel hamnar i byggloggen och statusen, sidan från senaste lyckade bygge serveras vidare. Manifest och paket får `Cache-Control: no-cache`. | `serve_builds_and_serves_two_origins` (start → bygge → signerat paket på A, nyckeln på båda, sidan finns inte på B och API:t inte på A, vakten nekar `Origin`, `Sec-Fetch-Site`, `Referer`, fel `Host` och `X-Forwarded-Host`-förfalskning, `POST /_dev/build` ger samma paket, en trasig källa ger `error` i tillståndet medan sidan finns kvar, en rättad källa byggs av bevakaren, `vy-dev build` från kommandoraden ger samma paket), `fresh_rewrites_cache_control_for_pages_only`, `hosts_must_name_the_api`, `guard_refuses_browsers_foreign_hosts_and_missing_tokens` |
| 3 dev-API och agent | På origin B, varje väg med `token=<körningens nyckel>`: `GET /_dev/state` → `{name, address, build: {status, log, builds}, agent: {running, last_exit}, seq}`; `POST /_dev/build` → 202; `POST /_dev/agent` (kroppen = uppdraget, UTF-8, högst 64 KB) → 202, 409 när en agent redan kör, 400/413 för tomt, ogiltigt eller för långt, 500 med orsak när programmet inte startar; `POST /_dev/agent/stop` → `{stopped}`; `GET /_dev/agent/events?after=<seq>&wait=<ms>&raw=0&limit=<n>` → `{next, entries: [{seq, kind, text, status?}]}`, long-poll upp till 20 s. Agenten startas som `<cmd> -p --output-format stream-json --verbose --permission-mode acceptEdits --allowedTools "Read,Edit,Write,Glob,Grep,Bash(cargo *),Bash(vy-cli *),Bash(vy-dev build*),Bash(ls *)"` i projektkatalogen med uppdraget på stdin, `vy-dev`s egen katalog först på `PATH` (där `vy-cli` ligger i en release) och `VY_DEV_KEY` satt. Kommandot: `--agent-cmd`, annars `VY_AGENT_CMD`, annars `claude`; på Windows slås ett namn utan filändelse upp med `PATHEXT` (på `PATH`, eller bredvid sig själv för en sökväg), så `claude.cmd` hittas. Utdata läses rad för rad in i en ringlogg (4 000 poster, 8 KB per post): varje rå rad (`raw`), assistentens text (`assistant`), verktygets namn och vad det rörde (`tool`, t.ex. `Edit src/lib.rs`), slutresultatet (`result`), start/stopp/exit (`agent`) och byggen (`build` med `status`). En agent i taget; när den avslutas begärs ett bygge; under tiden begär bevakaren inget. Stopp: `taskkill /T /F` på Windows, annars processträdet ur `ps` med `SIGTERM` och efter 1,5 s `SIGKILL`. | `agent_runs_streams_stops_and_triggers_a_build` med `tools/fake_agent.sh` (uppdraget kommer fram på stdin med exakt de fasta flaggorna, händelserna kommer som text och verktygsnamn utan råa rader, 409 under en körning, stopp avslutar agenten och dess barnprocess, exit bygger och agentens ändring syns i modulen på A, råa rader med `raw=1`, tom long-poll efter `wait`, `limit` ger de nyaste, exitkod 3 rapporteras), `an_agent_that_does_not_start_is_reported`, `commands_parse`, `finds_scripts_with_pathext`, `process_trees`, `stream_json_lines_become_entries`, `select_filters_raw_and_keeps_the_newest`, `sequence_and_cap` |
| 4 Studio (`modules/studio`) | En vanlig ABI 1.0-sida på `vy-sdk` (60 KB wasm): rubrik med projektets namn, statuspiller (väntar / bygger / klar / fel), adressen till sidan under utveckling som länk (`link_register` + nod `link`), uppdragsfält (`textbox`, `ime_begin`/`ime_end` efter värdens fokus, en rad som rullar så slutet syns), **Kör** (blir **Stoppa** medan agenten arbetar) och **Bygg om**, en rad för vad senaste knappen gjorde, agentens logg (lista med assistentens text, verktygsnamn, resultat och bygghändelser; följer nyaste posten, `SCROLL` och Page Up/Down rullar) och byggloggen (svansen som får plats, felrader röda). Nätet: `GET /_dev/state` och `GET /_dev/agent/events` som long-poll (`wait=1000`, `raw=0`, `limit=200`) som ställs om så fort den svarar, `POST` för knapparna, alla mot sin egen origin med `fetch_begin` → `FETCH_DONE` → `fetch_poll` = 1 → `fetch_read` till 0. `storage` håller senaste uppdraget. Var API:t finns och körningens nyckel står i en platshållare (`VYDEV-STUDIO-CONFIG http://localhost:8081 000…`, 128 byte i en `static` som läses med volatila läsningar) som `vy-dev` skriver över före signering (steg 5). Utan `net` säger sidan det och väntar på `CAPABILITY`; tappad förbindelse visas och prövas igen efter 2 s. Egen JSON-läsare (`json.rs`, RFC 8259, djup ≤ 32). Manifestutkast `conformance/manifests/studio.json` (`ime`, `net` mot `http://localhost:8081`, `storage` 16 KB, `memory_max` 256). | Nativt: `reads_the_dev_api_answers`, `strings_unescape`, `refuses_what_is_not_json`, `config_placeholder_and_patched`, `spec_5_3_1_without_net_studio_says_so`, `answers_become_state_and_log`, `spec_10_1_1_the_tree_names_everything`, `spec_5_2_9_typing_and_the_kept_mission`, `layout_fits`. Konformans: `studio-800x600@1` (träd `studio`), `studio-800x600@1-running` (fokus, uppdrag, Kör, agentens poster, 202; träd `studio-running`), `studio-400x700@2`, `studio-800x600@1-error` – inspelade svar med platshållarnyckeln, `fetch_done` i begärd ordning. |
| 5 Studio i `vy-dev serve` | `vy-dev serve` letar `studio.vyp` (`--studio <fil>`, annars bredvid binären, annars en katalog upp: `target/studio.vyp` i en checkout, som `tools/build_studio.sh` skriver). Paketet anpassas till körningen innan det serveras som `index` på origin B: modulens konfigurationsblock får `http://localhost:<studio-port> <nyckel>` (exakt en förekomst av markören, samma längd, utfyllt med mellanslag), manifestets `net` blir den origin ensam, modulens hash och storlek följer de nya byten, och utvecklarnyckeln signerar. Finns inget paket svarar API:t ändå och terminalen säger hur man bygger ett. `tools/build_studio.sh [ut]` bygger modulen och packar den med en slängnyckel (`VY_PUBLISH` kan peka på en färdig `vy-publish`). | `studio_runs_a_mission_through_the_client_path`: paketet som `build_studio.sh` gör, `vy-dev serve --studio`, `vy-proto` hämtar Studio från `vy://localhost:<b>/` och litar på den, `net`/`ime`/`storage` beviljas utan fråga, modulen bär körningens origin och nyckel och inte platshållaren, `vy-runtime` kör den riktiga modulen med `Transport::External`, varje begäran prövas mot `plain_http_allowed` och utförs med `Fetcher::request` som skalets laddare gör; adressen blir en länk, ett uppdrag skrivs och körs med Kör, den falska agenten får det, och agentens text, verktygsnamn och "agenten är klar" syns i Studios träd. `studio_config_block_is_patched_in_place`, `studio_lookup_order`. |
| 6 Dokumentation | `docs/DEVELOPING.md`: de två vägarna (för hand och med Studio), förutsättningar, portar och origins, felsökning. Ett icke-normativt stycke i SPEC §17 om `vy-dev` och Studio (två meningar, inga id). | – |

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
- **Studio får sin konfiguration genom modulen, inte genom klienten.** En sida kan inte fråga efter
  sin egen adress, och `fetch_begin` tar bara absoluta adresser. I stället för en ny import eller
  en specialväg i klienten har modulen ett 128 byte stort block med en unik markör som `vy-dev`
  skriver över innan det signerar (samma längd, så inget i modulen flyttas). Paketet i en release är
  därmed generiskt och kan signeras med en slängnyckel.
- **Studios long-poll väntar högst 1 s** (`wait=1000`), inte 20: skalets laddare utför en sidas
  begäranden en i taget (se *Öppna frågor*), och en lång väntan hade hållit uppe både Studios egna
  knappar och andra flikars laddning.
- **Studio drar slutsatser om agenten ur händelserna** (`uppdrag:` → arbetar, `agenten …` → klar) och
  läser om `/_dev/state` bara när ett bygge blivit klart eller fel, så att tillståndet och historiken
  aldrig motsäger varandra mitt i en körning.
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
skriver sidans manifest, så en lurad agent kan begära det; frågan är första skyddet och nyckeln per
körning (punkt 6) det andra. API:t finns inte på A
(404) och sidan inte på B, vilket `serve_builds_and_serves_two_origins` visar.

**Vad mer skyddar API:t.**

1. *Bara loopback.* Båda servrarna binder `127.0.0.1`; inget på nätverket når dem.
2. *Host-kontroll mot DNS-rebinding.* Studios origin svarar på *varje* väg (`/`, `/index`,
   `/_vy/<hash>`, nyckelfilen och `/_dev/`) bara när `Host` är `localhost:<studio-port>` eller
   `127.0.0.1:<studio-port>` (punkt 7); före granskning 0005 fynd 1 gällde det bara `/_dev/`, och
   paketet med modulen, där nyckeln står, gick att läsa under ett främmande namn. En webbsida som
   fått sitt namn att peka på 127.0.0.1 skickar sitt eget namn i `Host` och får 403.
   `X-Forwarded-Host` läses inte (vem som helst kan sätta det), bara den rå `Host`. Sidans origin
   (A) har samma `Host`-kontroll för sin egen port: den bär ingen nyckel och gör inget, men utan
   kontrollen kunde en webbsida genom rebinding läsa sidan under utveckling (opublicerat arbete).
   Webbläsarhuvudena nekas inte på A, eftersom HTML-reserven är till för webbläsare.
3. *Inga webbläsare.* En begäran med `Origin`, `Referer` eller något `Sec-Fetch-*`-huvud nekas (403)
   på varje väg på Studios origin, inte bara under `/_dev/`.
   Webbläsare sätter `Origin` på varje POST mellan origins och `Sec-Fetch-*` på allt, och en sida kan
   inte ta bort dem; Vy-klientens `vy-proto` skickar inget av dem (SPEC-7.5.1). Det stänger CSRF mot
   localhost: en "enkel" `fetch("http://127.0.0.1:8081/_dev/agent", {method: "POST", mode:
   "no-cors"})` från en webbsida når aldrig agenten. Inga CORS-huvuden skickas, så svaren går inte
   att läsa från en webbsida heller.
4. *Fjärrsidor i Vy* kan inte nå `http://localhost` alls: `vy_proto::transport_allowed` släpper bara
   igenom `http://` till loopback för en sida som själv ligger på loopback.
5. *En agent i taget*, och uppdraget är högst 64 KB.
6. *En nyckel per körning.* `vy-dev serve` drar 16 byte ur OS:ets entropi (`getrandom`) till en
   nyckel på 32 hextecken. Varje anrop till `/_dev/` måste ha `token=<nyckel>` i frågan, annars 403;
   jämförelsen tar lika lång tid var nycklarna än skiljer sig. Nyckeln skrivs in i Studio-modulen
   som `vy-dev` signerar och serverar (steg 5) och skrivs ut i terminalen (`api:`-raden), ingen
   annanstans; begärandeloggen (på utan `--quiet`) skriver vägen utan frågan, så nyckeln hamnar
   inte där (granskning 0005 fynd 2). En webbsida når inte paketet (punkt 2 och 3). En Vy-sida kan inte läsa den ur Studio: `fetch_begin` kan inte sätta `Accept`, så
   servern svarar HTML (som inte bär nyckeln) på `/` och `/index`, och modulen som blob kräver en hash
   sidan aldrig ser (den beror på nyckeln). Den okodade Studio-modulen i repot har platshållaren
   `000…` (konformansfallet), som aldrig är en körnings nyckel.
7. *`Host` måste vara API:ets egen port*: `localhost:<studio-port>` eller `127.0.0.1:<studio-port>`
   (porten får utelämnas bara om den är 80). En annan port eller ett annat namn ger 403.

`origin_boundary::pages_cannot_drive_the_dev_api` prövar gränsen genom klientens egen kod mot en
körande `vy-dev serve`: sidan på A som begär `net` mot B får `Ask` av `Fetched::grants`; tillåten ändå
får den 403 utan nyckel och med platshållaren; det sidan kan hämta från B innehåller inte nyckeln; en
fjärrsida får `Error::TlsRequired` från `Fetcher::request` även med rätt nyckel; agenten startar
aldrig; Studios egen origin med nyckeln får 200.

**Kvarstående risk.**

- *Andra processer på samma maskin* som talar Vy-protokollet (sätter `Accept` och hämtar
  Studio-paketet) kan läsa nyckeln ur modulen och starta agenten. För användarens egna processer är
  det ingen ny förmåga; för andra användare på en delad maskin är det en. Kör inte `vy-dev serve` på
  en delad fleranvändarmaskin. En riktig lösning kräver att klienten säger vilken
  origin en begäran kommer ifrån (ett huvud som huvudprocessen sätter och en sida inte kan förfalska)
  – förslag till m4/security, se *Öppna frågor*.
- *Agenten själv* kan genom cargo göra allt användaren kan. Uppdraget är användarens eget och skrivs i
  Studio; prompt injection via filer i projektet är samma risk som när användaren kör Claude Code för
  hand i katalogen.

**Ändringen i `vy-runtime` (plain http till loopback).** Studio på `vy://localhost:8081/` måste nå
`http://localhost:8081/_dev/…`, men `vy_runtime::origin_of` tog bara `https://`, så `fetch_begin` gav
`E_ARG` för varje `http://`-adress – även för en sida på loopback, som SPEC-7.1.3 undantar från TLS
och som `vy-proto` redan släpper igenom. Ändringen är minsta möjliga: `origin_of` ger nu också
`http://<loopback>` (exakt `localhost`, `127.0.0.1` eller `[::1]` med valfri siffer-port; inget annat
namn, ingen user info), och flikens egen grind släpper aldrig igenom plain http för att listan är tom
(sidan utan manifest): bara en loopback-origin som manifestets `net` namnger. Varför det inte öppnar
loopback för andra sidor, lager för lager:

1. *Fjärrsida (vilken origin som helst utanför loopback) som listar `http://localhost:8081`.*
   Standardpolicyn frågar (inte egen origin), och även om användaren säger ja eller slår på `net` i
   adressfältet stoppar huvudprocessens laddare anropet: `vy_proto::transport_allowed` släpper bara
   igenom `http://` till loopback för en sida som själv ligger på loopback, och svaret blir `E_NET`
   utan att någon förbindelse öppnas. Det var så före ändringen och är oförändrat; det prövas mot en
   körande `vy-dev` i `vy-dev`s tester.
2. *Sida på `localhost:8080` (sidan under utveckling) som listar `http://localhost:8081`.* Policyn
   frågar, aldrig tyst ja. Skalet kan i dag inte skilja `localhost:8080` från `localhost:8081` vid
   anropet (laddaren får bara *sidan ligger på loopback*), så den hårda regeln *bara sin egen
   loopback-origin* ligger färdig som `vy_runtime::plain_http_allowed(origin, sidans_auktoritet)` för
   m4/ux att anropa i `Shell::request_verdict` (en rad, se *Berör andra spår*). Till dess skyddar
   dev-API:t sig självt: utan körningens nyckel, som en sida inte kan läsa ut, svarar det 403.
3. *Sidan utan manifest* (headless, `vy-cli render` utan utkast) har ingen grind för `https://` som
   förut, men plain http kräver alltid att origin står i listan.
4. *Studio på `vy://localhost:8081/`* listar `http://localhost:8081`: egen origin, `net` beviljas
   automatiskt (SPEC-8.2.3), grinden och laddaren släpper igenom.

## Nya beroenden

Inga nya crates i trädet (`Cargo.lock` får bara posten för `vy-dev`). `vy-dev` beror på crates som
redan fanns i arbetsytan: `vy-manifest`, `vy-publish`, `vy-serve`, `vy-runtime` (för
`check_abi_bytes` och `plain_http_allowed`), `tiny_http`, `serde_json` och `anyhow`, samt:

| Crate | Vad | Varför inte själv | Drar in |
| --- | --- | --- | --- |
| `getrandom` 0.2 (fanns via `ring` och `vy-publish`) | 16 byte OS-entropi till dev-API:ts nyckel per körning | OS-anrop per plattform; `std` har ingen säker slump | 0 nya (samma version som redan är låst) |
| `vy-abi` (bara `dev-dependencies`) | händelsetyper och metodnamn när Studio-testet driver modulen | konstanterna finns där och ska inte kopieras | 0 nya |
| `vy-proto` (bara `dev-dependencies`) | klientens egen begäranväg i `origin_boundary` | testet ska pröva klientens kod, inte en kopia | 0 nya (ureq, rustls fanns) |

## Grindar och tal

Sista körningen (22:17–22:27, i arbetskopians egen målkatalog efter samordnarens byte, moduler
byggda på nytt): `cargo fmt --all --check` och `cargo clippy --workspace --all-targets -D warnings`
rena; `cargo test --workspace` 282 gröna och 19 ignorerade (var 257 och 19 på `3bc0bcb`), ett rött:
tidsgränstestet i *Öppna frågor* 3; `spec_trace.py` 132/132 (inga nya id); 12 moduler bygger,
`check-abi` OK, `test_modules.sh` 12/12; konformans 59/59 à 0,000 % (var 55). Guldbilderna för
Studio renderades två gånger, den andra med en privat kopia av `vy-cli` medan målkatalogen var
delad, med samma byte. `vy-dev` typkontrollerad för `x86_64-pc-windows-msvc` och
`aarch64-apple-darwin` (byggskripten för C-beroenden fick en låtsaskompilator som skriver tomma
objektfiler, eftersom sandlådan saknar Windows- och macOS-SDK:er). Testprojekt byggs i
`target/vy-dev-tests`.

Commitet `0aba99f` (Studio utan fall) är avsiktligt rött i `spec_12_1_2_conformance_defines_client`
tills `0c46014` lägger till fallen med guldbilderna, efter mönstret `5c3adca`/`4d929b4`.

## Öppna frågor

1. **Vem ringer?** (spec-fråga 54 i `docs/BUILD-NOTES.md`, M4) Ett localhost-API kan inte skilja Studio från en annan Vy-sida som fått `net` mot
   samma origin, eller från en annan lokal process. Förslag (m4/security): huvudprocessen sätter ett
   huvud med sidans origin på varje `fetch_begin`-anrop, som `Origin` i webbläsare; då kan `vy-dev`
   kräva `vy://localhost:8081` och den tredje punkten i hotbilden blir en positiv kontroll.
2. **Windows-tvillingen av den falska agenten** (`tools/fake_agent.ps1`) och `taskkill`-vägen är
   typkontrollerade men inte körda här (ingen Windows i sandlådan); CI:s Windows-jobb kör
   `agent_end_to_end` med den.
3. **Byggtestet för tidsgränser** `vy_render::tests::spec_5_3_3_bench_32_rounded_clips_2000_rects`
   (1 s i debug) beror på lasten: när fyra agenter bygger på två kärnor (load 5–6) tar det
   1,0–1,5 s och faller, också ensamt; vid lägre last tidigare under kvällen gick det igenom ensamt.
   `vy-render`, `vy-dl` och `vy-abi` är oförändrade mot `3bc0bcb`, och testet kör ingen modul.
4. **Skalets laddare är en tråd.** `vy_shell::page::start_loader` utför sidors `fetch_begin` en i
   taget; en long-poll från Studio (≤ 1 s) skjuter upp andra flikars laddning lika länge. En liten
   pool eller en tråd per begäran (med samma gränser) vore bättre – m4/ux.
5. **Flikens grind är öppen när listan är tom** (före den här grenen): `Options::net_origins` tom
   betyder "inget manifest", men skalet skickar manifestets lista, som är tom också när `net` begärs
   med enbart origins som `origin_of` inte känner (`vy://…`) eller utan origins. Huvudprocessens
   `request_verdict` är sträng och räddar läget i skalet; plain http täcks av den här grenens regel.
   Bör stängas i `vy-runtime` (m4/perf, m4/security), t.ex. `Option<Vec<String>>`.
6. **Konformansfallet för Studio** svarar med platshållarnyckeln och `http://localhost:8081`, eftersom
   modulen i `target/wasm/` är den okodade. Det prövar sidan, inte anpassningen; anpassningen prövas
   av `studio_end_to_end`.
7. **SPEC-7.1.3 och en sidas egna anrop.** (spec-fråga 53 i `docs/BUILD-NOTES.md`, M4) Meningen gäller `vy://`; att en sidas `fetch_begin` på
   loopback följer samma undantag (och bara mot sin egen origin) står bara i koden och den här sidan.
   En mening i §7 vore tydligare; lämnad till specägaren.

## Berör andra spår

- **`crates/vy-runtime` (m4/perf).** Eget commit, bara tillägg i beteende:
  `host.rs`: `origin_of` ger även `http://` för en exakt loopback-auktoritet (ny privat
  `is_loopback_authority`), grinden i `fetch_begin` kräver att en plain-http-origin står i listan
  även när listan är tom, felmeddelandet för `E_ARG` säger "neither https:// nor http:// to
  loopback", dokumentationen av `FetchRequest::url`; ny publik ren funktion `plain_http_allowed`.
  `lib.rs`: exporten av `plain_http_allowed`. `tests.rs`: nytt test
  `spec_7_1_3_plain_http_only_to_a_listed_loopback_origin`. Inga signaturer ändrade, inga befintliga
  tester ändrade. `vy-shell` får beteendet utan kodändring, eftersom huvudprocessen normaliserar
  manifestets origins och prövar anropens origin med samma `origin_of`.
- **`crates/vy-cli/src/conformance.rs` (konformanssviten).** Eget commit, bara tillägg:
  `cases.toml` får TOML-literalsträngar `'…'` (inga escapes, får innehålla `"`, inte `'`), så att
  ett inspelat JSON-svar kan stå i `fetch`-postens `body`. De tre hjälparna som letar citattecken
  (`split_top`, `strip_comment`, `depth`) delar nu en skanner `scan` som känner båda sorterna.
  Inga befintliga fall ändrade (inget innehöll `'`). Nytt test `literal_strings_hold_json_bodies`.
  Formatraden överst i `conformance/cases.toml` och en mening i `conformance/README.md`.
- **`crates/vy-manifest/src/lib.rs` och `crates/vy-cli/src/conformance.rs`, ett tal var i två
  tester:** `conformance_manifests_validate` och `spec_10_4_2_ime_required_for_input` räknade elva
  referensmoduler; med `modules/studio` och `conformance/manifests/studio.json` är de tolv.
- **`crates/vy-shell` (m4/ux), inte ändrat, men en rad behövs:** i `Shell::request_verdict`, efter
  att origin hittats i listan, även `vy_runtime::plain_http_allowed(&origin, <sidans auktoritet ur
  tab.origin>)`, annars `E_ORIGIN`. Då når en sida bara sin egen loopback-origin över plain http,
  vad användaren än beviljat.
- `vy-publish` och `vy-serve` används som bibliotek med sina befintliga publika funktioner; inget i
  dem är ändrat. Utöver det bara medlemsraden i `Cargo.toml`, `Cargo.lock`, nya filer i `tools/`
  (`fake_agent.sh`, `fake_agent.ps1`, `build_studio.sh`) och ett stycke i SPEC §17 (eget
  `spec:`-commit).

## För m4/ux: krokarna

- **"Öppna Studio"**: öppna `vy://localhost:8081/` (eller den `--studio-port` användaren valt) som en
  vanlig sida. Studio behöver inget av klienten utöver vanlig laddning och `net` mot sin egen origin,
  som standardpolicyn beviljar utan fråga.
- **Live-reload i devtools**: anropa inte `/_dev/` (det kräver körningens nyckel, som bara Studio och
  terminalen har). Hämta i stället sidans egen adress på nytt: origin A serverar manifest och paket
  med `Cache-Control: no-cache`, och `ETag` och identitet ändras efter varje lyckat bygge. Ladda om
  fliken när identiteten ändrats.
- **Länken i Studio** är `vy://localhost:8080/<namn>` via `link_register`; ett klick ska öppna den
  i en flik som andra `vy://`-länkar.
- **Den hårda regeln för plain http** (se *Berör andra spår*): `vy_runtime::plain_http_allowed` i
  `Shell::request_verdict`.

## Vid merge

- `Cargo.toml` (medlemsrad) och `Cargo.lock` (posten för `vy-dev`) är tillägg.
- `vy-runtime` (`182477a`): ändringen ligger i `origin_of` och `fetch_begin`s origingrind; behåll
  båda sidornas ändringar där om m4/perf rört samma rader.
- `vy-cli` (`7562b78`, `0c46014`): parserns skanner och talet tolv. Lägger ett annat spår till en
  modul blir talen i `conformance_manifests_validate` och `spec_10_4_2_ime_required_for_input`
  tretton.
- `conformance/cases.toml`: Studios fyra fall står före skalfallen; fall från andra spår kan läggas
  var som helst.
- `tools/publish_site.sh` publicerar Studio tills deploy-spåret hoppar över den (se *Release*).
- Den otrackade `.cargo/config.toml` i arbetskatalogen (samordnarens, isolerar byggen) ska inte med.

## Release

**Releasezippen** (`.github/workflows/release.yml`, `vy-<version>-<target>.zip`, inte ändrad här)
behöver, per mål:

1. `vy-dev` (`vy-dev.exe` på Windows): `cargo build --release --locked -p vy-dev`, i samma katalog som
   `vy-cli`. `vy-dev serve` lägger sin egen katalog först på agentens `PATH`, så loopen i projektets
   `CLAUDE.md` (`vy-cli check-abi`, `vy-cli render`, `vy-dev build`) hittar dem utan att användaren
   rör `PATH`.
2. `studio.vyp` bredvid `vy-dev`: `VY_PUBLISH=target/release/vy-publish tools/build_studio.sh
   <staging>/studio.vyp` (kräver `wasm32-unknown-unknown`, som jobbet redan har för modulerna).
   Paketet signeras med en slängnyckel; ingen hemlighet behövs, eftersom `vy-dev` skriver in origin
   och nyckel och signerar om med användarens utvecklarnyckel vid varje start.
3. Inget mer: Claude Code installeras av användaren (`claude` på `PATH`), Rust med
   `wasm32-unknown-unknown` likaså (`docs/DEVELOPING.md`).

**Dockerfile/Railway** behöver inget: den offentliga sajten serverar inte Studio och kör inte
`vy-dev`. Men `tools/publish_site.sh` (orörd enligt uppdraget) går igenom
`conformance/manifests/*.json` och publicerar därför också `studio.vyp` på den offentliga sajten.
Det är ofarligt (från en fjärrorigin når Studio aldrig `http://localhost`: transporten nekar och
sidan visar "ingen kontakt"), men Studio ska inte vara offentlig: deploy-spåret bör hoppa över
`studio.json` (eller utkast vars `net` pekar på loopback) i skriptet.
