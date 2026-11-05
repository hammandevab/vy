# M4: bygg direkt i verktyget – `vy-dev` och Studio

Byggagentens noteringar för grenen `m4/dev` (från main `3bc0bcb`). Granskningssidan skrivs av en
annan agent. Spåret: en användare startar `vy-dev` i en projektkatalog, öppnar Studio i Vy, skriver
"gör en sida som …", och Claude Code bygger sidan, som visas i Vy.

Rörda kataloger: `crates/vy-dev` (ny), `docs/notes/m4-dev.md` (den här sidan), `Cargo.toml`
(en medlemsrad), `Cargo.lock`.

## Vad som landat

| Steg | Innehåll | Test |
| --- | --- | --- |
| 1 `vy-dev new` | `vy-dev new <dir> [--name <n>] [--sdk <vy-checkout>]` skriver ett sidprojekt: `Cargo.toml` på `vy-sdk` (sökväg till checkoutens `crates/vy-sdk` med `--sdk`, annars git-beroendet `https://github.com/hammandevab/vy`), `src/lib.rs` (text, räknare och en knapp med fokusring, `layout()`-mönstret, ett nativt test), `manifest.json` (utkast i formen av `conformance/manifests/*.json`, `published` = dagens datum), release-profilen från `modules/hello` (inget `.cargo/config.toml` finns där), `.gitignore` och en `CLAUDE.md` för en sidskrivande agent: loopen, reglerna, *Klart när*, ett API-sammandrag och SDK:ts README inbäddad (`include_str!`), så agenten inte behöver nätet. | `scaffold_builds_to_wasm_and_passes_check_abi` (bygger till wasm32 här utan varningar, `check_abi_bytes` OK, utkastet validerar mot modulen, första framen har fem semantiska noder, mallen är rustfmt-ren, projektets eget `cargo test` grönt), `api_summary_covers_the_sdk` (varje import, event och roll i SDK:t nämns i sammandraget), `writes_a_project`, `names`, `civil_dates` |
| 2a `vy-dev build` | Byggkedjan som ett steg: `cargo build --release --target wasm32-unknown-unknown --message-format=json-render-diagnostics` (diagnostiken renderad på stderr = loggen, JSON på stdout namnger modulen var målkatalogen än ligger), ABI-kontrollen (`vy_runtime::check_abi_bytes`, samma kod som `vy-cli check-abi`), utkastet mot modulen och resurserna (`vy_publish::load_resources` + `vy_publish::build`: `memory_max`, bildstorlekar, signatur) och paketet skrivet atomärt (temporär fil + `rename`). Loggen strömmas rad för rad till den som anropar och hålls till de sista 64 KB. `vy-dev build [<dir>] [--key] [--out]` kör den från kommandoraden. Nyckeln: `--key`, annars `VY_DEV_KEY`, annars `<datakatalog>/dev.key` (samma katalog som `vy-shell`: `%APPDATA%\vy`, `~/Library/Application Support/vy`, `~/.config/vy`), skapad med `vy_publish::keygen`/`write_key` första gången. | `build_pipeline_signs_a_valid_package_and_reports_failures` (giltigt signerat paket; trasig källa och för litet `memory_max` ger fel med orsak i loggen och inget paket), `finds_the_module_among_cargo_messages`, `writes_atomically`, `log_keeps_the_tail`, `keys_are_made_once`, `opens_a_scaffold_and_fingerprints_it` |

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
- **Testprojekt byggs i `target/vy-dev-tests`** (`CARGO_TARGET_DIR`), så upprepade körningar delar
  ett bygge av `vy-sdk` i stället för att fylla disken med en katalog per test.

## Hotbild

Steg 1 kör inget och lyssnar inte; se kommande steg.

## Nya beroenden

Inga nya crates i trädet (`Cargo.lock` får bara posten för `vy-dev`). `vy-dev` beror på crates som
redan fanns i arbetsytan: `vy-manifest`, `vy-publish`, `vy-serve`, `vy-runtime` (för
`check_abi_bytes`), `tiny_http`, `serde_json` och `anyhow`.

## Öppna frågor

Inga ännu.

## Berör andra spår

Inget utanför `crates/vy-dev` utöver medlemsraden i `Cargo.toml` och `Cargo.lock`. `vy-publish`
och `vy-serve` används som bibliotek med sina befintliga publika funktioner; inget i dem är ändrat.

## Release

Kommer med steg 5.
