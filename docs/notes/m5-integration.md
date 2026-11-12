# M5 – sammanslagningen av spåren (`m5/integration`)

Integrationsagentens noteringar. Grenen `m5/integration` utgår från main `0fc0453` (release v0.2.0).
Spåren slås ihop som riktiga merge-commits (`--no-ff`) i ordningen m5/sdk, m5/gpu, m5/release, med
grindarna efter varje sammanslagning och uppföljningarna som egna små commits. m5/examples (från
m5/sdk) slås ihop sist, när koordinatorn säger till. Granskningssidan skrivs av en annan agent.
Sessionen kan startas om: läget står här och i `git log`.

"Grindar" är alltid alla, från arbetskopians rot: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `tools/build_modules.sh` + `check-abi target/wasm/hello.wasm` +
`tools/test_modules.sh`, `cargo test --workspace`, `python3 tools/spec_trace.py`, `vy-cli conformance`,
`check-cross --workspace --all-targets` (Windows och macOS) och arbetsflödenas YAML parsad med
`python3 yaml`. Modulbygget måste gå före `cargo test` (vy-cli:s tester läser `target/wasm/`).

## Sammanslagningar

| Commit | Spår | Konflikter | Grindar efteråt |
| --- | --- | --- | --- |
| `843bce2` Slå ihop m5/sdk | `1de409e`, 15 commits | inga | 390 gröna / 4 ignorerade, spec 140/140, 13 moduler, 64/64 à 0,000 % |
| `dc40f39` Slå ihop m5/gpu | `bcd26f8`, 10 commits | inga | 397 / 4, spec 140/140, 13 moduler, 64/64 à 0,000 %; GL-testet under Xvfb med llvmpipe grönt (sidorna nu 99 frames med `widgets`, värsta 0,254 %) |
| `a69efe4` Slå ihop m5/release | `877c774`, 11 commits | två, se nedan | 401 / 3 (`spec_9_7_1_reproducible_build` och `spec_12_3_1_size_budget` av-ignorerade av spåret), spec 140/140, 13 moduler, 64/64 à 0,000 %, GL grönt |

### Konflikter och hur de löstes

- **m5/release – `crates/vy-shell/Cargo.toml`, `default`.** m5/gpu `["window", "gl"]`, m5/release
  `["window", "a11y-linux"]`. Båda: `["window", "gl", "a11y-linux"]`.
- **m5/release – `.github/workflows/release.yml`, jobbet `site`.** m5/sdk lade steget *Package the
  conformance suite* före *Zip*, m5/release en kommentar om Ed25519 på samma ställe. Båda, steget
  först. Dessutom (samma commit, utan konflikt men följden av att båda finns): filhuvudet säger "de
  tolv paketen" i stället för "de elva" och listar `vy-conformance-<version>.zip`; `SHA256SUMS` får
  konformanszippens summa bredvid `vy-site` (SPEC-9.7.1: varje fil releasen består av), och den
  sammansatta åtgärden `.github/actions/conformance` pinnar `actions/upload-artifact` på samma commit
  som `release.yml` (granskning 0004 punkt 11).
- **`modules/widgets/Cargo.lock`** (ingen textkonflikt): m5/sdk skrev den mot `vy-abi`/`vy-sdk` 0.1.0,
  m5/release höjde arbetsytan till 0.2.0 och bygger modulerna med `--locked`, så `widgets` föll i
  modulbygget. `cargo update --workspace --offline` i modulen: 0.1.0 → 0.2.0, inget annat.
- Automatiskt och kontrollerat: `ci.yml` har m5/gpu:s Mesa/Xvfb-steg och SPEC-13.2.1-stegen och
  m5/release:s jobb `repro` och `size`; `crates/vy-render/src/text.rs` har m5/release:s
  typsnittsladdning och m5/gpu:s `Glyph::key`; spec-frågorna 66 (m5/gpu) och 67 (m5/release) krockar
  inte, och m5/sdk numrerade inga.

## Uppföljningar

| Commit | Vad | Grindar efteråt |
| --- | --- | --- |
| `b261ff1` version 0.3.0 | `[workspace.package]` 0.2.0 → 0.3.0; `Cargo.lock` och de tretton `modules/*/Cargo.lock` (`cargo update --workspace --offline`). `scaffold::SDK_TAG` är `concat!("v", env!("CARGO_PKG_VERSION"))`, så `vy-dev new` pinnar nu `tag = "v0.3.0"`, som ska innehålla `vy_sdk::ui`; `sdk_tag_is_the_release_version` läser versionen ur rotens `Cargo.toml` och behövde ingen ändring. `release.yml`:s versionskontroll provkörd lokalt med steget ur YAML:en: `v0.3.0` går igenom (`tag=v0.3.0`, `version=0.3.0`), `v0.2.0` stoppar | 401 / 3, spec 140/140, 13 moduler, 64/64 à 0,000 % |
| `ac2bd80` `spec:` SPEC-12.3.1 per arbetsenhet | Spec-fråga 67, avgjord av koordinatorn på ägarens uppdrag: "varje crate under `crates/` tillsammans med `SPEC.md` och `CLAUDE.md` ryms under 400 000 tokens (byte/4 över git-spårade textfiler); hela kodbasen rapporteras men begränsas inte". `spec_12_3_1_codebase_tokens` av-ignorerat: skriver varje enhet, den största och hela kodbasen och hävdar taket för varje crate. Största enheten **vy-shell 203 259 tokens** (marginal 196 741), sedan vy-render 92 891 och vy-runtime 88 981; hela kodbasen **995 791 tokens** (3 983 161 textbyte), rapporterad | 402 / 2 |
| (denna) `golden:` widgets-typed | Konformansens värd har ett tomt urklipp i minnet (`RenderRequest::clipboard`, `Some(&[])` i `conformance.rs` och i `bench.rs`, som kör samma fall; `vy-cli render` har fortfarande inget), och ett skriptat `key_down` C (6) eller X (27) med Ctrl utan Alt, eller Meta, levereras med `Runtime::event_as_copy_gesture` (SPEC-8.2.4; `vy_cli::is_copy_gesture`, skalets regel). Urklippet följer med över `reinit`; `RenderOutput::clipboard` visar det. `widgets-800x600@1-typed`: bilden oförändrad (statusraden ligger under fönstret), trädets statusrad "Ctrl+C: urklippet nekades (E_DENIED)." → "Ctrl+C: markeringen är kopierad."; inget annat fall ändrat. Nytt test `spec_8_2_4_scripted_copy_gesture` (gesten släpper igenom skrivningen utan capabilityn, Ctrl+Alt+C gör det inte, utan urklipp nekas den). `conformance/README.md` (engelska avsnittet om den deterministiska värden) och kommentarerna i `cases.toml` säger det nya | 403 / 2, 64/64 à 0,000 % |
