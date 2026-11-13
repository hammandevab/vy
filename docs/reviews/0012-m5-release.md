# PR #12 – M5: reproducerbara byggen och releasekvalitet (`m5/release` + `b261ff1`, `ac2bd80`, bedömd på `3fbcf36`)

**Vad ändrades.** Modulerna och releasebinärerna byggs oberoende av sökväg, användare och tid
(`tools/repro_env.sh`, `repro_rustc.rs`, `build_release.sh`), och releasen publicerar `SHA256SUMS` och
`BUILD.md`. Linux-klienten har AT-SPI på som standard och ryms under 25 MB genom komprimerade typsnitt
(serif, mono) och `opt-level = "s"` för 51 crates; `release.yml` kan kodsignera; versionen är 0.3.0,
`vy-dev new` pinnar SDK:t på releasens tagg, och tokentaket gäller per crate.

**Spec-id som berörs.** SPEC-9.7.1 och SPEC-12.3.1 omskrivna i egna `spec:`-commits med tester
(`spec_9_7_1_reproducible_build`, `spec_12_3_1_size_budget`, `spec_12_3_1_codebase_tokens`, alla
av-ignorerade). SPEC-10.2.1. Spec-frågor 56 (öppen: MB eller MiB) och 67 (avgjord).

**Risk.** En release som inte går att bygga om går inte att lita på; ett signeringssteg som läcker
eller fäller jobbet; en klient över taket som stoppar releasen samma dag den ska ut.

**Bevis.** *Kört här:* `spec_9_7_1_reproducible_build` är grönt: trädet kopieras till två djup,
`build_modules.sh` körs i båda med olika tidszon, alla 19 `.wasm` är lika byte för byte och varje
utkast ger samma paketidentitet. Det jämför **inte** binärerna. `bundled_faces_are_the_ttf_files` och
konformansen (91/91 à 0,000 %) visar att de komprimerade snitten ger samma byte. Modulbygget med
`--locked` mot 0.3.0 går, och `sdk_tag_is_the_release_version` är grönt. Tokens, räknade om av mig:
största enhet `vy-shell` 216 644 (tak 400 000), `vy-render` 93 085, `vy-runtime` 91 874; hela
kodbasen 1 162 344, rapporterad men utan tak. *Läst:* `release.yml` körs bara på tagg och manuell
start, `ci.yml` på `pull_request` utan hemligheter, och inget på `pull_request_target`: **en PR från
en förgrening når inga signeringshemligheter**. Kodsigneringens hemligheter ges bara till
signeringsstegen; saknas de blir det en notis och jobbet fortsätter. Varje action i `release.yml` är pinnad på commit.
`SHA256SUMS` får varje zip, varje fil i målzipparna och de osignerade summorna för ett signerat mål.
*Inte kört av någon:* CI-jobben `repro` och `size`, hela `release.yml`, signeringen, och allt på
Windows och macOS. Klientens storlek har jag inte byggt om: 24 591 912 byte enligt noteringen. Fynd:

1. **Medel – binärernas reproducerbarhet är okörd där den är grind.** De fem binärerna jämförs bara
   av jobbet `repro` (aldrig kört; macOS och Windows `continue-on-error`) och en gång lokalt av
   byggagenten. På Windows går `-Clink-arg=/Brepro` genom en miljövariabel under Git Bash, där MSYS
   kan skriva om `/…` till en sökväg; då faller länkningen i `release.yml`, som inte har
   `continue-on-error`. Rättning: en grön körning av `ci.yml` före taggen; sätt
   `MSYS2_ENV_CONV_EXCL=CARGO_ENCODED_RUSTFLAGS` i `repro_env.sh`.
2. **Medel – marginalen mot 25 MB är 408 088 byte (1,6 %), och taket stoppar releasen.**
   `release.yml` fäller varje plattform över 25 000 000, och Windows- och macOS-klientens storlek är
   okänd. `spec_12_3_1_size_budget` hävdar bara i release; i `cargo test` återvänder den, så grinden
   är jobbet `size`. Rättning: en `dry_run`-ingång i `release.yml` som hoppar över jobbet `release`,
   körd före taggen. Spec-fråga 56 avgör ägaren.
3. **Medel – `vy-dev new` pinnar en tagg som inte finns än.** `scaffold.rs`, `SDK_TAG` = `v0.3.0`:
   tills taggen är publicerad och repot går att hämta bygger ett nytt projekt bara med `--sdk`.
   Testet prövar strängen, inte hämtningen. M5-grinden (utomstående agent) måste köras efter taggen.
   Rättning: ett rökprov sist i `release.yml`: `vy-dev new` och bygg utan `--sdk`.
4. **Låg – stegen har mer rätt än de behöver.** `permissions: contents: write` gäller hela
   `release.yml`, också byggjobben som kör tredjeparts byggskript, och i jobbet `site` ligger
   paketnyckeln `VY_KEY` i miljön och på disk medan `publish_site.sh` bygger `vy-cli` och
   `vy-publish` (från före M5). Rättning: `read` på arbetsflödet och `write` bara på jobbet
   `release`; bygg i ett steg utan nyckeln och signera i nästa.
5. **Låg – `SHA256SUMS` täcker inte `BUILD.md` och inte filerna i `vy-site`-zippen**, fast
   filhuvudet säger "varje fil i dem". Sajtzippen är reproducerbar bara med `VY_KEY`, och
   konformanszippen beror på umask (0010 fynd 3).
6. **Låg – en utomstående kan bara återskapa bygget på samma runner-avbild.** C-koden (ring,
   zstd-sys) och länkningen görs med runnerns verktyg; `BUILD.md` namnger avbilden, men den går inte
   att hämta. Rättning: bygg Linux-releasen i en container pinnad på digest.
7. **Låg – kallstarten med AT-SPI som standard är omätt.** zbus startar i huvudprocessen vid varje
   start på Linux; `spec_13_1_1_cold_start` är ignorerat. Fet text packar upp serif och mono (≈ 1,7 ms
   en gång per process enligt noteringen).

**Nya beroenden.** Inga crates; `vy-render` får en kant till `zstd`, som redan fanns. En ny action,
`azure/artifact-signing-action` v2.0.0, pinnad på commit, körs bara med hemligheter.

**Hotbild.** Signeringsnycklar finns bara som hemligheter och når bara en taggad eller manuellt
startad körning av den som får skriva till repot. macOS-binärerna som kör wasmtime signeras med
`allow-unsigned-executable-memory`; det är oprövat mot flikens sandlåda.

**Rekommendation.** **Merge efter** en grön CI-körning på GitHub med `repro` och `size` (fynd 1–2).
Tagga först därefter, och kör M5-grinden efter taggen (fynd 3). Fynd 4–7 som uppföljningar.
