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
| `8b15ef0` `golden:` widgets-typed | Konformansens värd har ett tomt urklipp i minnet (`RenderRequest::clipboard`, `Some(&[])` i `conformance.rs` och i `bench.rs`, som kör samma fall; `vy-cli render` har fortfarande inget), och ett skriptat `key_down` C (6) eller X (27) med Ctrl utan Alt, eller Meta, levereras med `Runtime::event_as_copy_gesture` (SPEC-8.2.4; `vy_cli::is_copy_gesture`, skalets regel). Urklippet följer med över `reinit`; `RenderOutput::clipboard` visar det. `widgets-800x600@1-typed`: bilden oförändrad (statusraden ligger under fönstret), trädets statusrad "Ctrl+C: urklippet nekades (E_DENIED)." → "Ctrl+C: markeringen är kopierad."; inget annat fall ändrat. Nytt test `spec_8_2_4_scripted_copy_gesture` (gesten släpper igenom skrivningen utan capabilityn, Ctrl+Alt+C gör det inte, utan urklipp nekas den). `conformance/README.md` (engelska avsnittet om den deterministiska värden) och kommentarerna i `cases.toml` säger det nya | 403 / 2, 64/64 à 0,000 % |
| (bench, inget commit) | `spec_12_2_3_perf_regression_gate` är grönt efter alla sammanslagningar (det jämför påhittade rapporter och kontrollerar att baslinjerna parsas), så `conformance/bench/build-vm.json` lämnas orörd enligt uppdraget; `compile.form` mot baslinjen står under *Storlekar och mätningar* | – |
| `e309115` docs: DEVELOPING och CLAUDE.md | `docs/DEVELOPING.md`: `--sdk`-raden säger att `vy-sdk` kommer ur git på releasens tagg och att `--sdk` tar den från disk, ett stycke om `tag = "v0.3.0"` och en länk till `docs/SDK.md` (m5/sdk:s förslag). `CLAUDE.md`: M3 ✓ ("SPEC-13.2.1 mäts på GPU (GL-vägen), CPU-vägen över budget"), M4 ✓ ("extern sandlådegranskning kvar"), M5 öppen; strukturlistan har `crates/vy-dev`, `modules/studio` och `docs/SDK.md` | fmt, spec 140/140 |
| (denna) docs: noteringarna | detta dokument komplett; spec-frågorna 66–70 i `docs/BUILD-NOTES.md` | fmt, spec 140/140 |

## Storlekar och mätningar

**Klienten (SPEC-12.3.1).** `tools/build_release.sh x86_64-unknown-linux-gnu target/stage` på
`e309115` (releasens recept: `repro_env.sh`, `--release --locked --target`, profilens `strip`), med
den ospårade `.cargo/config.toml` flyttad åt sidan (den rör bara debug-profilen):

| Binär (release, Linux x86_64, strippad) | Byte |
| --- | --- |
| `vy-shell`, standardfunktionerna `window`, `gl`, `a11y-linux` | **24 524 312** |
| marginal mot 25 000 000 | **475 688 (1,9 %)** |
| `vy-cli` / `vy-dev` / `vy-publish` / `vy-serve` | 20 111 072 / 16 690 248 / 16 047 656 / 1 083 384 |
| `studio.vyp` | 31 714 |

Samma byte som m5/release:s provsammanslagning (24 524 312); m5/sdk ändrar inget i skalet.
`vy-shell` SHA-256 `42c21c91…845a7b`. `cargo test --release --locked --target
x86_64-unknown-linux-gnu -p vy-cli --lib spec_12_3_1` (miljön ur `repro_env.sh`): grönt, "24524312
bytes, cap 25000000, margin 475688", och tokentestet "vy-shell with 203352 tokens … the whole codebase
is 997945 tokens" (med noteringarna som växt sedan `ac2bd80`). Bygget tog 12 min 40 s på två vCPU.

**Kompilering mot baslinjen (SPEC-12.2.3, informativt).** Release-`vy-cli bench --runs 30 --only
compile --compare conformance/bench/build-vm.json`: `compile.form` P50 45,0 ms mot baslinjens 37,1
(gräns 39,5) – den enda raden som faller, och det är modulen som växt (16,7 → 30,5 KB, m5/sdk);
alla andra rader är snabbare än baslinjen här. Grinden i `cargo test` (`spec_12_2_3_perf_regression_gate`)
jämför inte mätta tal och är grön, så baslinjen är orörd (uppdraget: regenerera bara om grinden faller).
SPEC-13.1.4:s 60 ms för kompileringen överskrids av `widgets` (62 KB, P50 79 ms) och `studio`
(59 KB, 67 ms) och i P95 av `form` (71 ms); raderna är informativa, men se *Öppet*.

## Grindar på `e309115`

Från arbetskopians rot, efter varje commit ovan (dokumentcommits: fmt och spec_trace):

| Grind | Utfall |
| --- | --- |
| `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings` | rena |
| `tools/build_modules.sh` + `check-abi target/wasm/hello.wasm` + `tools/test_modules.sh` | 13 moduler, ABI 1.0 OK |
| `cargo test --workspace` | **403 gröna, 0 röda, 2 ignorerade** (`spec_13_2_1_raster_budget`, `spec_13_1_1_cold_start`); main hade 369 / 4 |
| GL-testet (`xvfb-run … VY_GL_REQUIRED=1 cargo test -p vy-shell --test gl`) | grönt med llvmpipe efter varje sammanslagning; uppföljningarna rör inte GL-vägen |
| `python3 tools/spec_trace.py` | 140/140 |
| `vy-cli conformance` | **64/64 à 0,000 %** |
| `check-cross --workspace --all-targets` | Windows och macOS rena |
| arbetsflödenas YAML (`python3 yaml`) | parsas |
| `release.yml`:s versionskontroll, lokalt | `v0.3.0` går igenom |

Modulbygget måste köras före `cargo test` (vy-cli:s tester läser `target/wasm/`). Diskutrymmet tog
slut en gång (versionshöjningen bygger om hela arbetsytan): gamla testbinärer i den egna
byggkatalogen togs bort och grindarna kördes om.

## Spec-frågor

66 (m5/gpu, GL-vägen) och 67 (m5/release, kodbasens tokens) är avgjorda. m5/sdk:s tre egna frågor
har fått nummer 68–70. Alla står i `docs/BUILD-NOTES.md`.

## Öppet (från alla spår och sammanslagningen)

1. **SPEC-13.2.1 mäts på ägarens dator** med GPU-drivet OpenGL (`VY_GL_REQUIRED=1 VY_GL_BUDGET=1
   cargo test --release -p vy-shell --test gl`, m5/gpu *Vad ägaren ska kontrollera*); här finns bara
   llvmpipe. `spec_13_2_1_raster_budget` mäter CPU-vägen och är ignorerat med sina tal.
2. **Windows och macOS** är bara typkontrollerade: GL-fönstret och reserven, DPI-byte, storleken mot
   25 MB, CI-jobben `repro` (macOS och Windows `continue-on-error`) och `size`, `release.yml` med
   `build_release.sh`, `SHA256SUMS`, `BUILD.md`, konformanszippen och kodsigneringen är okörda.
3. **Kodsigneringen** kräver ägarens köp och secrets (`docs/RELEASING.md`).
4. **Marginalen** mot 25 MB är 475 688 byte (1,9 %); nästa större tillägg i skalet äter den
   (m5/release *Öppet* 4 har kandidater). Spec-fråga 56 (MB eller MiB) står kvar.
5. **Kompileringstiden** (SPEC-13.1.4, 60 ms): `widgets` och `studio` över i P50 här, `form` i P95.
   Komponenterna gör modulerna större; m5/sdk:s förslag att flytta radlägets gemensamma kod till
   icke-generiska funktioner, och mätning på referensmaskinen, är nästa steg. Baslinjen
   `build-vm.json` bör få nya rader (`widgets`) och `compile.form` i ett `bench:`-commit när någon
   mäter på en tyst maskin; inte gjort här.
6. **`widgets`-sidans hjälptext** säger "Huvudlöst finns inget urklipp, så statusraden visar värdens
   svar" (och modulens dokumentation likaså); konformansens värd har nu ett minnesurklipp, medan
   `vy-cli render` fortfarande saknar ett. Att ändra texten ändrar modulen och guldbilderna; lämnat
   till m5/sdk:s ägare (eller m5/examples-sammanslagningen).
7. **m5/sdk:** textfältet går per `char`, inte grafemkluster; ingen ångra, ingen hovring eller
   inaktiverad knapp; `List`/`ScrollView` i varandra kräver ordningen inre före yttre; fast geometri i
   temat; `settings` är inte portad.
8. **m5/gpu:** färgglyfer och bilder större än texturerna ritas på CPU:n den framen; banden startar
   trådar per frame; skalets komponering är hel varje frame; `vy-cli bench` har inga rader för
   fönstrets vägar.
9. **m5/release:** cosmic-text läser varje snitt av annan vikt vid matchning, så fet text packar upp
   serif och mono; Windows- och macOS-delarna av `repro_env.sh` är bara lästa.
10. **M4:s externa sandlådegranskning** återstår (CLAUDE.md). **M5:s grind** (en utomstående agent
    publicerar en sida på under en timme) körs av koordinatorn.
11. **m5/examples** slås ihop sist, när koordinatorn säger till: modulernas `Cargo.lock` mot 0.3.0
    (`--locked`), filhuvudet i `release.yml` ("de tolv paketen" blir arton om alla sex har utkast),
    och kopieringsgesten om något av dess fall använder Ctrl+C/X.

## m5/examples (sist, på koordinatorns besked)

| Commit | Vad | Grindar efteråt |
| --- | --- | --- |
| `b6525bc` Slå ihop m5/examples | `1d60050`, 21 commits: sex exempelsidor (`kanban`, `kalkyl`, `rita`, `skriv`, `block`, `rymd`) med utkast, 27 nya fall, 19 nya träd, `docs/EXEMPEL.md`. **Ingen konflikt**, inte heller i `cases.toml` (spårets avsnitt låg före skalfallen, våra ändringar i widgets-avsnittet). De sex modulernas `Cargo.lock` (0.1.0 → 0.3.0, `cargo update --workspace --offline`) ligger i merge-commiten och inte i ett eget commit: utan dem faller modulbygget med `--locked`, och då vore merge-commiten röd | 403 / 2, spec 140/140, 19 moduler, **91/91 à 0,000 %**, GL grönt, cross rent |
| `dfe7d2c` docs: arton paket | `release.yml`:s filhuvud och `docs/RELEASING.md` säger arton paket (19 utkast utom Studio); RELEASING får raden för `vy-conformance-<version>.zip`. README:s statusavsnitt (M2.5, "elva referenssidor") är historiskt och lämnat | fmt, spec, YAML |

Kontrollerat utan commit:

- **Guldbilderna byte för byte.** `vy-cli conformance --update` i det sammanslagna trädet skrev alla 91
  bilder och träd, och `git status` visade ingen ändring: de 27 nya fallen är identiska med m5/examples.
- **Kopieringsgesten.** Inget nytt fall skriptar Ctrl+C eller Ctrl+X: `block` använder C (6) utan
  modifierare för "håll", vilket inte är en gest, och inget nytt fall beviljar `clipboard`. Inga nya
  träd påverkas av konformansens minnesurklipp.
- **`tools/publish_site.sh target/site-test`** (release-`vy-cli` och `vy-publish`, 12 min 53 s, ingen
  nyckel): "18 packages, 18 previews"; Studio hoppas över; de sex nya har paket och 1280 × 800-förhandsbilder.
- **8 ms-kontrollen i `vy-publish build`** (SPEC-13.4.1, i samma körning): dyraste frame `kanban`
  114 223, `kalkyl` 352 296, `rita` 82 521, `skriv` 463 523, `block` 116 998, `rymd` 440 830 bränsle
  av 8 000 000; ingen behövde `--allow-slow`. (Rättat: den första versionen av den här raden parade
  ihop varje sidas `module:`-rad med nästa sidas `frames:`-rad; `vy-publish build` skriver `frames:`
  först.)

Öppet från m5/examples (`notes/m5-examples.md`): SDK-luckorna (ikonknapp, publik fokusring,
`TextBox::adopt_focus`/`set_id`, dragning, tvåaxlig `ScrollView`, `math::{sqrt, sin, cos}`,
växlingsknapp, dialog, text i flera stilar), 1.x-frågorna om ett behållet lager för banor och en kursiv
roll, guldbilderna bara körda på Linux, och att CLAUDE.md:s strukturlista inte nämner exempelsidorna
eller `docs/EXEMPEL.md`.

## m5/polish (sist, på koordinatorns besked)

| Commit | Vad | Grindar efteråt |
| --- | --- | --- |
| `3fbcf36` Slå ihop m5/polish | `b26ea99`, 9 commits på `bc22baa`: kompileringscachen i skalet (`spec:` SPEC-9.4.2, `CompileCache::Host`, `CodeStore`/`CodeKey`, `vy-shell/src/code_cache.rs`, IPC-ramarna `ToTab::CodeCache` 15 och `ToShell::Code` 79), parallella sidladdningar (`Fetcher` med `&self`, fyra laddararbetare, ordning per origin), `widgets`-hjälptexten (tre träd) och textfältets delade `Field` i `vy-sdk`. **Ingen konflikt** | 412 / 2, spec **141/141**, 19 moduler, 91/91 à 0,000 %, GL grönt, clippy och check-cross rena |
| (denna) docs: CLAUDE.md och läget | strukturlistan får `modules/widgets`, exempelsidorna och `docs/EXEMPEL.md`; det här avsnittet och *Läget inför v0.3.0* | fmt, spec |

Kontrollerat:

- **Guldbilderna.** `conformance --update` på `3fbcf36` ändrade ingen fil under `conformance/`; de enda
  ändringarna mot `dfe7d2c` är m5/polish:s tre `widgets*.json`. Textfältets omskrivning ändrar
  modulernas byte men inga pixlar eller träd, också i `kanban`, `kalkyl` och `skriv`.
- **check-cross** efter att varje crates `lib.rs` rörts (så att allt kontrollerades om): Windows och
  macOS rena, också parallella laddningar och kodcachen.
- **GL-testet** täcker nu 195 sidframes (alla 19 sidor); värsta frame `rymd` 800 × 600 med 0,387 % över
  2 nivåer, största avvikelse 133 (en kantpixel), inom SPEC-11.3.2:s 3 %.
- **8 ms-kontrollen** (`vy-publish build` för alla arton publicerade utkast, debug, bränslet är
  oberoende av profilen): dyraste frame `skriv` 463 457, `rymd` 440 830, `document` 354 674, `kalkyl`
  352 719, `list` 294 947, `widgets` 263 175 … `hello` 19 346 av 8 000 000; ingen över, ingen behövde
  `--allow-slow`.

| Modul (byte, `tools/build_modules.sh`) | Före m5/polish (`dfe7d2c`) | Efter (`3fbcf36`) |
| --- | --- | --- |
| `form` | 30 547 | 30 627 |
| `kanban` | 54 267 | 54 647 |
| `kalkyl` | 61 149 | 61 340 |
| `skriv` | 60 895 | 61 502 |
| `widgets` | 64 482 | **55 493** |

Sidor med bara enradsfält växer med 0,3–1,0 %; `widgets`, som har båda radlägena, krymper med 14 %.

**Klienten** (`tools/build_release.sh x86_64-unknown-linux-gnu`, `.cargo/config.toml` åt sidan och
tillbaka): `vy-shell` **24 591 912 byte**, samma som m5/polish mätte på sin gren, **marginal 408 088
(1,6 %)**; `spec_12_3_1_size_budget` i release grönt. Kodcachen och laddararbetarna kostar 67 600 byte
mot `e309115`. Tokentestet i samma körning: största enheten vy-shell 216 698 tokens (marginal
183 302), hela kodbasen 1 162 441.

## Läget inför v0.3.0

**Grönt på Linux x86_64** (`3fbcf36` och senare dokumentcommits): fmt, clippy `-D warnings`,
`cargo test --workspace` 412 gröna, 0 röda, 2 ignorerade (`spec_13_2_1_raster_budget` för CPU-vägen och
`spec_13_1_1_cold_start`, som kräver fönster), `spec_trace.py` 141/141, 19 moduler med ABI 1.0 och egna
tester, konformans 91/91 à 0,000 %, GL-vägen mot referensen under Mesa llvmpipe, storleken
(24 591 912 av 25 000 000) och tokentaket per crate i release, `publish_site.sh` (arton paket, Studio
utelämnad), versionskontrollen för `v0.3.0`.

**Bara typkontrollerat eller okört:** allt på Windows och macOS (`check-cross` rent; fönstret, GL och
reserven, sandlådan, kodcachen och storleken är okörda där); CI-jobben `repro`, `size` och GL-steget,
och `release.yml` med `build_release.sh`, `SHA256SUMS`, `BUILD.md`, konformanszippen och kodsigneringen
har inte körts på GitHub; guldbilderna är bara körda på Linux.

**Ägaren:** (1) mät SPEC-13.2.1 på referensmaskinen med GPU-drivet OpenGL (`VY_GL_REQUIRED=1
VY_GL_BUDGET=1 cargo test --release -p vy-shell --test gl`); (2) köp och lägg in secrets för
kodsigneringen (`docs/RELEASING.md`), eller släpp osignerat; (3) beställ den externa
sandlådegranskningen (M4); (4) avgör spec-frågorna 56–65 och 68–70 (`docs/BUILD-NOTES.md`); (5) kör M5:s
grind (koordinatorn); (6) tagga `v0.3.0` på integrationens slutcommit, så att `vy-dev new` hittar
taggen den pinnar, och se att första releasekörningen, `repro` på macOS och Windows och `size` går
igenom. Marginalen mot 25 MB är 1,6 %: nästa större tillägg i skalet måste betala för sig.
