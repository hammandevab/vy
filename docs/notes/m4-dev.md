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
- **Testprojekt byggs i `target/vy-dev-tests`** (`CARGO_TARGET_DIR`), så upprepade körningar delar
  ett bygge av `vy-sdk` i stället för att fylla disken med en katalog per test.

## Hotbild

Steg 1 kör inget och lyssnar inte; se kommande steg.

## Nya beroenden

Inga nya crates i trädet. `vy-dev` beror på `anyhow` (fanns); i tester `vy-manifest` och
`vy-runtime` (fanns).

## Öppna frågor

Inga ännu.

## Berör andra spår

Inget utanför `crates/vy-dev` utöver medlemsraden i `Cargo.toml` och `Cargo.lock`.

## Release

Kommer med steg 5.
