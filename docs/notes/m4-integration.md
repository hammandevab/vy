# M4 – sammanslagningen av de fyra spåren (`m4/integration`)

Integrationsagentens noteringar. Grenen `m4/integration` utgår från main `3bc0bcb`; de fyra spåren
slogs ihop som riktiga merge-commits i ordningen m4/dev, m4/ux, m4/security, m4/perf, med
grindarna efter varje sammanslagning och uppföljningarna som egna små commits däremellan.
Granskningssidan skrivs av en annan agent.

## Öppna punkter

Först, som uppdraget bad om när laddaren inte kunde göras klart helt:

1. **Sidladdningar är fortfarande seriella sinsemellan.** Uppföljning 12 (`3b1cbac`) flyttade en
   sidas egna förfrågningar (`fetch_begin`) till en tråd per flik, så Studios long-poll (≤ 1 s) och
   ett första besöks DNS-uppslag (≤ 6 s, m4/security) håller inte längre upp någon annan fliks
   förfrågningar. Men `Job::Page` och live-omladdningens `Job::Check` går kvar på laddartråden, som
   äger `Fetcher` med blobcache, HTTP-cache och nyckelförråd (`&mut self`): ett DNS-uppslag som inte
   svarar fördröjer andra flikars *laddningar* och live-omladdningens kontroller upp till 6 s. Att
   parallellisera dem kräver att cacherna och nyckelförrådet tål flera skrivare (lås per origin i
   `vy-proto`, eller en `Fetcher` per origin över samma kataloger) – inte en ren ändring i `vy-shell`.
   Studios egen POST från en knapp väntar fortfarande bakom Studios egen long-poll (samma flik,
   ordningen bevaras), därför står `POLL_WAIT_MS = 1000` kvar i `modules/studio`.
2. **Kompileringscachen är av i skalet** (m4/perf, koordinatorns punkt 2). `tab.rs` sätter
   `compile_cache: vy_runtime::CompileCache::Off`. Att slå på den kräver att huvudprocessen
   förkompilerar (`vy_runtime::precompile`) och att fliken bara läser – men flikprocessen kan inte
   öppna någon fil efter `sandbox::lockdown` (Linux, macOS) och inte skriva på medelnivå (Windows).
   Vägar: posten skickas över IPC i `LoadRequest`, eller ett fd öppnas före låsningen och undantas
   från `close_range`. Hotbilden i notes/m4-perf (steg 3) gäller; `ReadWrite` i fliken aldrig.
3. **Inspelning i devtools kör fliken utan OS-sandlåda.** m4/ux:s "Inspelning" startar om fliken med
   `--record`, och m4/security låser inte `--record`/`--replay` (filen öppnas i loopen). En sida som
   spelas in kör alltså med processens fulla rättigheter på Linux och macOS (Windows-jobbet står
   kvar). Rättning: huvudprocessen skriver inspelningen ur IPC-strömmen, eller filen öppnas före
   låsningen och fd:t behålls; uppspelning kan läsa hela filen före låsningen.
4. **`vy-dev build` kör inte m4/perf:s 8 ms-kontroll** (`vy_publish::check_frames`, SPEC-13.4.1):
   kedjan anropar biblioteksfunktionen `vy_publish::build`, inte kommandoradens `build`. Sidan under
   utveckling serveras bara på loopback, så det är inte en publicering, men agenten i Studio får ingen
   varning om en långsam frame. Förslag: `vy-dev build` kör `check_frames` och skriver en varning i
   byggloggen.
5. **`net` med tom origin-lista visas som beviljad** (spec-fråga 63). Fliken och huvudprocessen
   släpper inget igenom, men `default_grants` beviljar `net` och adressfältet visar plupp.
6. **Plattformar.** Windows- och macOS-vägarna (AppContainer, `sandbox_init`, `DnsQuery_UTF8`,
   AccessKit-adaptrarna, `taskkill`, `fake_agent.ps1`) är typkontrollerade här, inte körda.
   Releasejobbet och Docker-bygget är inte körda härifrån (ingen CI, ingen Docker-demon); deras
   cargo-steg är provkörda lokalt (se *Release och Docker*).
7. **Modulbygget beror på byggmiljön.** `chat.wasm` och `form.wasm` byggda i den här arbetskopian
   skiljer sig byte för byte från dem m4/security byggde sina fuzzfrön ur, utan någon källändring
   (`hello` och de andra är lika). Sannolikt cargos metadatahash (sökvägen till arbetsytan) som
   påverkar symbolordningen. Guldbilderna berörs inte (pixlar), men SPEC-9.7.1 (M5) gör det.
8. Spårens egna kvarstående punkter gäller oförändrat: notes/m4-dev *Öppna frågor* 2, 3 och 6,
   notes/m4-ux *Kvar och öppet*, notes/m4-security *Kvar*, notes/m4-perf *Kvar* 2–5.

## Sammanslagningar

| Merge | Spår (topp) | Konflikter | Grindar efter |
| --- | --- | --- | --- |
| `4292768` Slå ihop m4/dev | `47cf8b7` | inga | 283 gröna / 19 ignorerade, spec 132/132, konformans 59/59 à 0,000 %, moduler 12/12, korskontroll ok |
| `495814d` Slå ihop m4/ux | `976b3a1` | inga (textuellt); `vy-runtime` fick både loopback-origin och kopieringsgest automatiskt | 306 / 15, spec 133/133, 59/59 à 0,000 % |
| `bc99335` Slå ihop m4/security | `b3a5b7b` | tre små, se nedan | 326 / 12, spec 139/139, 59/59 à 0,000 % |
| `42bb1b7` Slå ihop m4/perf | `0623dab` | tre, se nedan | 348 / 4, spec 139/139, 59/59 à 0,000 % |

"Grindar" är alltid alla: fmt, clippy `-D warnings`, `cargo test --workspace`, `spec_trace.py`,
`build_modules.sh` + `check-abi` + `test_modules.sh`, `vy-cli conformance` och `cargo check
--workspace --all-targets` för `x86_64-pc-windows-msvc` och `aarch64-apple-darwin` (med
`check-cross`, som ersätter C-kompilatorn för ring och zstd-sys med attrapper). Modulgrinden kördes
efter varje sammanslagning och när något under `modules/`, `vy-sdk` eller `vy-abi` kunde ha
ändrats (aldrig i uppföljningarna).

### Konflikter och hur de löstes

- **m4/security – `vy-runtime/src/lib.rs`, exportraden.** m4/dev exporterade `plain_http_allowed`,
  m4/security `NET_BYTES_PER_MINUTE`. Båda.
- **m4/security – `vy-shell/src/lib.rs`, modullistan.** m4/ux `selection`, m4/security `sandbox`.
  Båda, i bokstavsordning.
- **m4/security – `vy-shell/src/shell.rs`, `Tab`-fälten.** m4/ux gjorde `logs` till
  `VecDeque<(u64, String)>` (framenummer för devtools), m4/security `net_window` till
  `netlimit::NetBudget`. Båda; inget av m4/security:s anrop skrev loggrader direkt, så inget annat
  behövde ändras.
- **m4/perf – `vy-runtime/src/host.rs`, origingrinden i `fetch_begin`.** m4/dev släppte igenom
  https med tom lista men grindade alltid plain http; m4/perf grindar allt. En regel: origin måste
  stå i manifestets (normaliserade) lista, alltid. En tom lista, eller en med bara origins som
  `origin_of` inte godtar (`vy://`, `http://` till en fjärrvärd, värd utan schema), når inget; plain
  http når bara en listad loopback-origin. Loggraden för tom lista säger nu "no https:// or loopback
  http:// origin".
- **m4/perf – `vy-runtime/src/tests.rs`.** Båda sidor lade nya tester sist i filen; båda behållna.
  Texten i `spec_5_2_8_origin_gate` och `spec_7_1_3_plain_http_only_to_a_listed_loopback_origin`
  följer den gemensamma regeln (ingen "öppen grind" utan manifest längre).
- **m4/perf – `vy-publish/src/main.rs`, usage.** `--allow-slow` (m4/perf) och `log-append`
  (m4/security). Båda.

## Uppföljningar

| Commit | Punkt | Vad | Test |
| --- | --- | --- | --- |
| `1afeae6` | 1 | `Tab::request_verdict` prövar origin mot listan och sedan `vy_runtime::plain_http_allowed` med sidans egen auktoritet; `shell::plain_http_allowed` (m4/ux:s kopia) borta | `spec_5_2_8_plain_http_only_to_own_loopback_origin` går genom grinden: egen loopback-origin släpps; en fjärrsida med loopback i listan och `net` beviljat nekas; en sida på `localhost:8080` når inte `localhost:8081`; annat namn för samma port, userinfo och lokal fil nekas |
| `c9af929` | 7 | `tools/publish_site.sh` hoppar över `studio.json`, med skälet i huvudkommentaren | provkört: elva paket, elva förhandsvisningar, ingen `studio.vyp` |
| `87ee2ab` | 8 | `release.yml`: `-p vy-dev` i byggsteget; zippen får `vy-dev` och `studio.vyp` (`VY_PUBLISH=target/<mål>/release/vy-publish<ext> tools/build_studio.sh <stage>/studio.vyp` – jobbet bygger med `--target`, så binären ligger under `target/<mål>/`, inte `target/release/` som m4/dev skrev); `vy-site-<version>.zip` orörd | YAML parsar (`yaml.safe_load`); `build_studio.sh` med release-`vy-publish` provkört |
| `b20fb47` | 2 | sandlådesonden (`--tab --sandbox-probe`) tar `process::self_exe()`; kommentar vid `tab_executable` i vy-cli | `spec_9_1_3_tab_sandbox`; sonden nekar alla fyra, `VY_SANDBOX=off` släpper alla |
| `dbe3c12` | 3 | IPC-taggarna krockar inte (m4/security lade inga ramar); fröskrivaren får `CopyGesture`, `StopRecording`, `Focus`, och `studio.cbor`/`studio.md` | `fuzz_corpora_replay`, `fuzz_mutation_smoke`, `spec_11_2_1_messages_round_trip` (14 + 14) |
| `10630b7` | 5 | ingen kodändring behövdes: `revalidate` gör bara den villkorliga GET:en; omladdningen går `navigate_tab` → `load` → `fetch` → `trust` | `spec_7_3_2_revalidate_keeps_the_trust_path`: inget DNS, ingen nyckelfil, ingen logg, ingen pin vid revalidering; utan TXT-post förblir sidan begränsad och frågar igen; byter servern nyckel blir det `UnknownKey`, inte en ny pin |
| `773b85f` | perf 1 | skalets test visar också att en tom lista nekar både egen loopback-origin och https | `spec_5_2_8_plain_http_only_to_own_loopback_origin` |
| `7ffd33e` | 6 | `LoadedPage::restriction` ur `Fetched::restriction()`; chromen visar "begränsat läge: <orsak>" så länge adressen behåller 200 px (`MIN_ADDRESS_ROOM`), annars markören ensam; detaljen i flikens logg; `page::restriction_reason` är en uttömmande match | `spec_7_4_3_restriction_reason_in_the_chrome` (huvudlöst skal: genom laddaren mot en server utan nyckelfil, och DNS utan post; lokal fil utan orsak), `spec_7_4_3_restriction_reason_beside_the_marker` (smalt fönster), `spec_7_4_3_every_restriction_has_a_reason`. Ingen guldbild ändrad (skalfallen är lokala filer eller betrodda) |
| `6e15e9a` | 9 | antalet moduler läses ur `modules/*/Cargo.toml` i `conformance_manifests_validate`, `spec_10_4_2_ime_required_for_input` och `spec_13_1_4_compile_time` | samma tester; tolv moduler |
| `3b1cbac` | 12 | `page::Loader`: `Job::Request` till en tråd per flik i flikens ordning, sidladdningar och kontroller kvar på laddartråden; `vy_proto::Requester` (agenten utan cacher) och `Fetcher::requester` | `spec_5_2_7_requests_do_not_wait_for_other_tabs` (servern håller svaren i stället för att testet mäter tid; föll med 20 s tidsgräns mot den seriella laddaren), utökat `spec_5_2_7_page_requests_through_the_main_process` |
| `141c105` | 10 | spec-frågorna 53–65 i `docs/BUILD-NOTES.md` i sammanslagningsordning med spåret angivet; spårens noteringar omnumrerade | – |
| `0c435d0` | perf 3 | `--runs` i den historiska raden i notes/m3-shell | – |

Utan commit, bara kontrollerat:

- **Punkt 4, `vy-runtime`.** `host.rs` har m4/dev:s `is_loopback_authority` och `plain_http_allowed`,
  m4/ux:s `copy_gesture`/`clipboard_written` (`event_as_copy_gesture`, `take_clipboard_write`) och
  m4/security:s `net_minute`/`net_admit`; m4/perf:s kompileringscache och grind. Testerna för alla
  är gröna (`spec_7_1_3_…`, `spec_8_2_4_copy_gesture_grants_clipboard_write`,
  `spec_5_2_12_clipboard_copy_paste_through_the_shell`, `spec_9_3_1_resource_limits`,
  `spec_5_2_8_origin_gate`). m4/security:s `limit_memory` efter spawn finns i `shell.rs`.
- **Punkt 2, `current_exe`.** `grep -rn current_exe crates/` visar reserven i `self_exe`, kopian av
  testbinären i `process.rs`-testet och uppslag av sökvägar som aldrig startas själva: vy-cli:s
  `tab_executable` (hittar testprofilens `vy-cli`), vy-shell:s `shell_exe` i testerna, Studios paket
  bredvid `vy-dev` och katalogen som läggs först på agentens `PATH`. Varje självstart (fönstret,
  `--screenshot`, `tab_executable` i båda binärerna, bench via `tab_executable`, sonden) går genom
  `self_exe`.
- **Punkt 11, SPEC.md.** 139 id, inga dubbletter, i ordning, vart och ett i sitt avsnitt (§7: 7.4.5,
  7.4.6, ny §7.6 med 7.6.1–7.6.3; §8: 8.2.4; §9: 9.1.3; §17: icke-normativt stycke om `vy-dev`).
  §5, bilaga A, bilaga C, `docs/ABI-1.0.md` och `vy-abi` är orörda mot main.

## Koordinatorns punkter för m4/perf

1. *Nätgrinden.* En regel i tre lager efter sammanslagningen: fliken (`vy-runtime`) släpper bara en
   origin som står i listan (tom lista = inget); huvudprocessen (`request_verdict`) kräver dessutom
   `net` beviljat och, för plain http, sidans egen loopback-origin; laddaren
   (`vy_proto::transport_allowed`) nekar http från en sida utanför loopback. Studio-vägen är grön
   genom klientens kod (`studio_runs_a_mission_through_the_client_path`,
   `pages_cannot_drive_the_dev_api`), liksom fjärrsidans väg till loopback (nekad i alla tre lager).
2. *Kompileringscachen* är av i skalet; öppen punkt 2.
3. *`--iterations` → `--runs`.* `ci.yml` (m4/perf) tar `--runs`; ingen annan anropare fanns utom en
   historisk rad i notes/m3-shell, som nu säger att flaggan bytt namn. `CLAUDE.md` anger inget tal.
4. *vy-render:s väggklocktester* gick igenom i varje körning efter sammanslagningen.
5. *Dockerfile.* `vy-publish` drar nu in `vy-runtime` (wasmtime), men byggsteget bygger redan
   `vy-cli`, som gjorde det förut, och musl-steget bygger bara `vy-serve`, vars träd inte har wasmtime.
   Se *Release och Docker*.
6. *Spec-frågor* omnumrerade (63–65).
7. *`spec_13_2_1_raster_budget`* är ignorerat med sitt uppmätta värde; budgeten orörd.

## Release och Docker

Storlekar i release på Linux x86_64 efter alla fyra sammanslagningarna (`3b1cbac`, samma kod som
HEAD), byggda med `--locked --config 'profile.release.strip=true'` och repots profil (`lto = "thin"`,
`codegen-units = 1`, `strip = true`); `file` säger *stripped* för alla:

| Binär | Byte | Anm. |
| --- | --- | --- |
| `vy-shell` (standard, AccessKit utan Linux-adaptern) | 22 322 112 | under SPEC-12.3.1:s 25 MB; samma byte byggd ensam (`-p vy-shell`, som `release.yml`) och tillsammans med de andra |
| `vy-shell --features a11y-linux` | 25 435 064 | **över** 25 000 000 byte, under 25 MiB (26 214 400); spec-fråga 56. Funktionen förblir av som standard |
| `vy-cli` | 20 358 944 | byggd utan `vy-shell` i samma anrop, som i `release.yml`; 20 372 616 när `vy-shell` byggs i samma anrop (funktionsunionen ger `vy-shell`-beroendet fler funktioner) |
| `vy-publish` | 16 429 696 | m4/perf: drar nu in `vy-runtime` (wasmtime) för 8 ms-kontrollen |
| `vy-dev` | 3 225 056 | |
| `vy-serve` | 1 083 448 | glibc; musl (Dockerfile) 1 209 040, statisk PIE |
| `studio.vyp` | 31 539 | `tools/build_studio.sh` med release-`vy-publish` |

Jämfört med spåren: m4/ux mätte `vy-shell` till 22 299 880 (standard) och 25 417 136
(`a11y-linux`) med samordnarens `codegen-units = 13`; m4/perf 21 905 096 utan m4/ux:s beroenden.
Efter sammanslagningen är standardbinären 22 232 byte större än m4/ux:s tal (DNS, logg,
sandlåda, nätgräns, snabbare rasterisering); jämförelsen är ungefärlig, eftersom m4/ux byggde
arbetsytans crates med `codegen-units = 13`.
Windows- och macOS-binärerna kan inte länkas här (ingen MSVC-länkare, ingen macOS-SDK); CI:s
release-jobb visar dem.

**Release (`release.yml`).** Zip-steget kört lokalt med jobbets egen kod ur YAML:en (bara
`target/$TARGET/release` → `target/release`, eftersom bygget här inte har `--target`): zippen fick
`LICENSE`, `studio.vyp`, `vy-cli`, `vy-dev`, `vy-publish`, `vy-serve` och `vy-shell`.
`tools/publish_site.sh` efter m4/perf: elva paket och elva förhandsvisningar, ingen `studio.vyp`,
och `vy-publish build` vägrade ingen referenssida (8 ms-kontrollen räknar bränsle, inte tid, så den
är densamma på CI). `vy-site-<version>.zip` får alltså samma elva paket som förut.

**Docker.** Ingen Docker-demon här, så bilden är inte byggd. Dess cargo-steg är körda lokalt med
samma verktygskedja: `cargo build --release --locked --target x86_64-unknown-linux-musl -p vy-serve`
med `CC_x86_64_unknown_linux_musl=musl-gcc` (grönt, statisk PIE; `vy-serve`:s träd har ingen
wasmtime), `cargo build --release --locked -p vy-cli -p vy-publish` (grönt; `vy-cli` drog redan in
wasmtime, så `vy-publish`:s nya beroende kräver inget nytt i byggstadiet) och `tools/publish_site.sh`
(grönt, ovan).

## Grindar på HEAD

Sista körningen på `0c435d0` (koden som `3b1cbac`, därefter bara dokumentation), load 0,8–0,9:

| Grind | Utfall |
| --- | --- |
| `cargo fmt --all --check` | rent |
| `cargo clippy --workspace --all-targets -- -D warnings` | rent |
| `cargo test --workspace` | 352 gröna, 4 ignorerade, 0 röda (main `3bc0bcb`: 257 / 19) |
| `python3 tools/spec_trace.py` | 139 id, 139 spårade |
| `tools/build_modules.sh`, `check-abi target/wasm/hello.wasm`, `tools/test_modules.sh` | 12 moduler, ABI 1.0 OK, 12/12 |
| `cargo run -p vy-cli -- conformance` | 59/59 à 0,000 % (main: 55) |
| `check-cross --workspace --all-targets` | `x86_64-pc-windows-msvc` och `aarch64-apple-darwin` rena |

De fyra ignorerade: `spec_13_2_1_raster_budget` (m4/perf, över budget på maskinen, spec-fråga 65),
`spec_13_1_1_cold_start` (kräver fönster), `spec_12_3_1_size_budget` och
`spec_9_7_1_reproducible_build` (M5, release). Det tidigare ostabila
`vy_render::tests::spec_5_3_3_bench_32_rounded_clips_2000_rects` gick igenom i varje körning efter
m4/perf (tidstaket gäller nu bara i release) och behövde aldrig köras om ensamt; före m4/perf gick
det också igenom här. Inga guldbilder ändrade någonstans i sammanslagningen.

## Byggmiljön

Egen målkatalog (`target` → `/home/claude/vy-target-merge`), `CARGO_INCREMENTAL=0`, aldrig två
cargo-kommandon samtidigt. Gamla testbinärer i `target/debug/deps` (98 filer, 4,1 GB) togs bort
under arbetet för diskens skull; cargo bygger om det som behövs. Den ospårade
`.cargo/config.toml` (samordnarens, `strip = "debuginfo"` för debug) är inte incheckad; den rör
inte release-profilen, och storlekarna ovan är byggda med `--config 'profile.release.strip=true'`.
Sessionen startades om en gång under release-bygget; bygget som redan kört fick gå klart.
