# M5 – reproducerbara byggen och releasekvalitet (`m5/release`)

Byggagentens noteringar. Grenen utgår från main `0fc0453` (release v0.2.0). Granskningssidan skrivs av
en annan agent. Sessionen kan startas om: läget står här och i `git log`.

## Läge

| Steg | Vad | Läge |
| --- | --- | --- |
| 1 | SPEC-9.7.1 reproducerbara byggen | moduler och releasejobb klara; spec-förtydligande kvar |
| 2 | SPEC-12.3.1 storleksbudget som test | – |
| 3 | `a11y-linux` under taket, på som standard på Linux | – |
| 4 | kodsignering i `release.yml`, `docs/RELEASING.md` | – |
| 5 | versionen: `vy-dev new` pinnar taggen, arbetsytans version, kontroll i `release.yml` | – |
| 6 | noteringarna klara | – |

## Steg 1 – reproducerbara byggen (SPEC-9.7.1)

**Orsaken till att `chat.wasm`, `form.wasm` och `studio.wasm` skiljer mellan arbetskopior.** Inte
inbäddade sökvägar (`strings` hittar inga), utan ordningen: typ- och funktionssektionerna skiljer,
storleken är densamma. Varje modul är en egen cargo-arbetsyta, så `crates/vy-sdk` och `crates/vy-abi`
är sökvägsberoenden *utanför* arbetsytan, och för ett sådant paket hashar cargo den absoluta sökvägen in
i `-C metadata` (`SourceId::stable_hash` tar bort arbetsytans rot bara när paketet ligger under den).
Det värdet är cratens StableCrateId, som står i symbolnamnen, och rustc ordnar SDK:ns instansierade
generiska funktioner i modulen efter symbolnamn. Provat: samma commit byggd i arbetskopian och i en
kopia under `/tmp/tp/other-place/deeper/vy` gav `libvy_sdk-6b739dc8…` mot `libvy_sdk-d7179c9d…` och
tre olika moduler (266, 720 och 1 174 byte olika). `--remap-path-prefix` rör inte cargos hash, och
`-Ztrim-paths` (`profile.trim-paths`) är inte stabilt i cargo 1.97 ("feature `trim-paths` is
required"). RUSTFLAGS ingår inte i `-C metadata` (provat: två olika `--remap-path-prefix` gav samma
metadata och extra-filename), så att lägga sökvägen i RUSTFLAGS gör inte bygget sökvägsberoende.

**Rättningen.** `tools/repro_rustc.rs` (std, ~80 rader) byggs av `build_modules.sh` med ren `rustc`
till `target/tools/repro-rustc` och sätts som `RUSTC_WRAPPER`: den byter cargos `-C metadata=<hash>`
mot ett värde av cratens namn, `CARGO_PKG_VERSION`, cratetyper, mål och `--cfg` (funktioner), inget
annat. `-C extra-filename` (bara filnamn i `target/`) lämnas. Dessutom
`--remap-path-prefix=<rot>=/vy` i RUSTFLAGS (efter de RUSTFLAGS som redan finns, CI:s `-D warnings`)
och `--locked` mot modulens `Cargo.lock`. `build_studio.sh` bygger Studio genom `build_modules.sh
studio`. Modulerna i den här arbetskopian blev byte för byte desamma som före ändringen.

`spec_9_7_1_reproducible_build` (vy-shell, un-ignorerat): arbetsträdet (`git ls-files --cached
--others --exclude-standard`) kopieras till `$TMP/vy-repro-<pid>/a/vy` och `…/b/further/down/vy`,
`build_modules.sh` körs i båda (egna modulkataloger, `TZ=UTC` respektive `Asia/Tokyo`), och alla
tolv `.wasm` jämförs byte för byte; för varje utkast i `conformance/manifests/` ger båda kopiornas
modul och resurser samma paketidentitet, och signerat med två nycklar blir paketen olika men packas
upp till den identiteten. 32 s i debug. **Föll** med `["form", "chat", "studio"]` när
`RUSTC_WRAPPER`-raden togs bort.

**Paket.** Definitionen: ett paket är reproducerbart när dess osignerade manifest är det, alltså
paketidentiteten (SPEC-4.1.2, SHA-256 över manifestet utan `sig`), som namnger modulens och varje
resurs hash. `vy-publish inspect` skriver den (`identity:`).

**Releasebinärerna.** Ett skript för releasen, `tools/build_release.sh <mål> <katalog>`, som
`release.yml`, CI:s `repro` och `BUILD.md` alla kör: `vy-cli vy-dev vy-publish vy-serve` i ett
cargo-anrop och `vy-shell` i ett annat (som förut), `--release --locked --target`, sedan
`studio.vyp` och `LICENSE` i katalogen. Miljön sätts av `tools/repro_env.sh` (läses in av både
`build_release.sh` och `build_modules.sh`):

- `CARGO_ENCODED_RUSTFLAGS` = befintliga flaggor + `--remap-path-prefix` för `CARGO_HOME` (`/cargo`),
  `CARGO_TARGET_DIR` om satt (`/target`) och arbetskopian (`/vy`), i den ordningen (rustc tar den
  sista som passar). Den kodade formen håller sökvägar med mellanslag hela.
- `CFLAGS` = `-ffile-prefix-map` för samma sökvägar (ring, zstd-sys), inte under MSVC; där länkar
  `build_release.sh` med `-Clink-arg=/Brepro` (PE-huvudets tidsstämpel blir en hash).
- `SOURCE_DATE_EPOCH` = commitens tid om den inte är satt (zip-posternas tid).

Uppmätt på `vy-shell` (release, `--target x86_64-unknown-linux-gnu`, miljön ovan): `strings` hittar
**inga** absoluta sökvägar till arbetskopian eller `/root/.cargo`; 749 rader börjar med `/cargo/…`
(registrets källor i panikplatser) eller `/vy/target/…/out/…` (cranelifts genererade kod). Utan
omskrivningen hade varje sådan rad burit användarens hem och arbetskopians sökväg.

**Zip, summor, recept.** `tools/release_files.py` (standardbiblioteket): `zip` skriver posterna
sorterade, rekursivt, med tiden `SOURCE_DATE_EPOCH`, läge 0755 för `vy-*` och 0644 annars, utan
katalogposter och extra fält (provat: samma byte med olika mtime, umask och tidszon); `sums` skriver
`sha256sum`-rader; `build-md` fyller `tools/BUILD.md.in` med en tabell per mål ur fragment som
jobben skriver (avbild `ImageOS`/`ImageVersion`, `rustc -V`, `cargo -V`, signerad, `studio.vyp`:s
identitet). `release.yml` publicerar nu `SHA256SUMS` (varje zip, varje fil i dem som
`vy-<v>-<mål>/<fil>`, och för ett signerat mål de osignerade under `vy-<v>-<mål>.unsigned/`) och
`BUILD.md`. Referenssidans zip skrivs samma väg; med `VY_KEY` är den reproducerbar (Ed25519 är
deterministiskt). Binärjobben har ingen byggcache längre (en release byggs ur en ren utcheckning,
som receptet säger), och alla actions i `release.yml` är pinnade på commit (`git ls-remote`, taggen
i en kommentar; granskning 0004 punkt 11).

**CI `repro`.** Matris över Linux, macOS och Windows: två `git worktree` på olika sökvägar, andra
bygget med eget `CARGO_HOME` (nytt registerträd) och `TZ=Asia/Tokyo`, båda med `build_release.sh`;
SHA-256 för `vy-*` och `LICENSE` plus `studio.vyp`:s identitet jämförs. Linux är grind; macOS och
Windows har `continue-on-error` tills en körning visat att de håller (inget av det är kört här).

## Steg 2 – storleksbudgeten som test (SPEC-12.3.1)

SPEC-12.3.1 har två halvor, och de blev två tester i `crates/vy-cli/src/bench_tests.rs`:

- **`spec_12_3_1_size_budget`** (un-ignorerat): `vy-shell` bredvid testbinärens profilkatalog
  (`target[/<mål>]/release/`) är högst `CLIENT_MAX_BYTES` = 25 000 000 byte (decimalt, det
  strängare; spec-fråga 56). I debug skriver testet hur det körs och återvänder. CI:s nya jobb
  `size` bygger `vy-shell` i release och kör testet i release; `release.yml` skriver varje fils
  storlek i loggen, klientens i en `::notice` och i körningens sammanfattning, och stoppar över
  taket. **Prövat i release** mot dagens `vy-shell --features a11y-linux` (25 476 976 byte): faller
  med "the client is 25476976 bytes, over SPEC-12.3.1's 25000000".
- **`spec_12_3_1_codebase_tokens`** (ignorerat, med talet): kodbasen under 400 000 tokens. Räkningen,
  i testets dokumentation och `codebase_tokens`: varje fil `git ls-files` listar och som är UTF-8
  utan NUL är text; tokens = textens byte / 4, uppåt avrundat. Fyra byte per token är den vanliga
  uppskattningen för engelska och kod; en modells tokeniserare ger fler för svenska och Rusts
  skiljetecken, så talet är snarare för lågt. **Hålls inte:** 858 837 tokens (3 435 346 byte) på
  `54ca612`, mer än dubbla taket; `crates/` ensamt är 2,3 MB (583 000 tokens), `docs/` 104 000,
  `modules/` 79 000, `conformance/` 45 000, `Cargo.lock` 27 000, `SPEC.md` 9 000. Inget urval av
  filer ger under 400 000 utan att utelämna koden. Testet är därför ignorerat med det uppmätta
  värdet, som `spec_13_2_1_raster_budget`, och ägaren avgör spec-fråga 67 (66 är m5/gpu:s, GL-vägen).

**Spec-fråga 67 (m5/release).** SPEC-12.3.1:s "hela kodbasen inklusive spec och tester ryms under
400 000 tokens" hålls inte: 858 837 tokens räknat som byte/4 över git-spårade textfiler (och fler med
en riktig tokeniserare). Ska taket höjas (t.ex. 1 000 000), ska "kodbasen" betyda något smalare
(utan `docs/notes`, `docs/reviews`, låsfiler och genererade `conformance/semantic/*.json`: ungefär
720 000), eller ska meningen bli ett mål snarare än en grind? Räkningen i testet är ett förslag; en
annan definition är ett `spec:`-commit med testet.

## Steg 3 – `a11y-linux` under taket, på som standard

Utgångsläget på `0fc0453` + steg 1 (release, `--target x86_64-unknown-linux-gnu`, miljön ur
`repro_env.sh`, strippad): `vy-shell` **22 364 840** byte, `--features a11y-linux` **25 476 976**
(+3 112 136; 476 976 över taket). Integrationens tal på `3b1cbac` utan `--target`: 22 322 112 och
25 435 064.

**wasmtime.** `vy-runtime` har redan `default-features = false` och bara `cranelift`, `runtime`,
`std`. `pulley-interpreter` går inte att välja bort i wasmtime 50: `runtime` slår på
`pulley-interpreter/interp` och `wasmtime-internal-cranelift` beroendet har `features = ["pulley"]`
ovillkorligt; `incremental-cache` kommer från `wasmtime-internal-cranelift`:s standardfunktioner,
som wasmtime inte stänger av. Inget att vinna utan att laga wasmtime.

**Typsnitten, första försöket: alla fyra zstd-komprimerade, uppackade vid start.** 1 009 331 byte i
stället för 2 192 440 (nivå 19); `vy-cli` blev 1 213 456 byte mindre. Men uppackningen kostar i
varje process: i release 1,6 ms för DejaVu Sans, 1,5 bold, 0,9 serif, 0,8 mono (last 2–3). Och
cosmic-text läser både sans och sans bold redan i `TextEngine::new` (provat: efter `new` var de två
uppackade, serif och mono inte), så att packa upp lat per snitt räddar bara serif och mono. Mätt
med `vy-cli bench --runs 100 --only tab.cold-start`, A/B med två release-binärer omväxlande:
**P50 14,9 → 20,3 ms** (P95 21,1 → 29,5), +36 %, långt över SPEC-12.2.3:s 5 %. Förkastat.

**Valt: serif och mono komprimerade, sans och sans bold som de är.** Snitten registreras med
`fontdb::Database::push_face_info` och metadata (familj, PostScript-namn, vikt, monospace), så att
inget tolkas i `new`; bytena läses först när texten formas, och de komprimerade packas upp då, en
gång per process (`OnceLock`, `zstd::bulk::decompress` till exakt storlek ur ramhuvudet). Sparar
354 728 byte. `bundled_faces_are_the_ttf_files` jämför bytena med `.ttf` och metadata med vad
`fontdb` läser ur dem. Ändringen i `text.rs` är bara typsnittsladdningen (statiska data och slingan
i `TextEngine::new`) och testet; `.ttf`-filerna ligger kvar i `fonts/` (testet läser dem).

**AT-SPI-stacken för storlek.** Med typsnitten var `vy-shell --features a11y-linux` 25 099 216 byte,
fortfarande 99 216 över. De 34 crates som bara `a11y-linux` drar in på Linux (zbus, zvariant,
zbus_names, zbus_xml, atspi, atspi-common, atspi-proxies, accesskit_atspi_common, accesskit_unix,
async-*, blocking, event-listener, futures-*, …; inga proc-makron) får `opt-level = "s"` i
release-profilen (`[profile.release.package]` i rotens `Cargo.toml`): **24 365 264** byte, 733 952
mindre. Kontrollerat med `cargo tree`: ingen av dem finns i `vy-cli`, `vy-dev`, `vy-publish`,
`vy-serve` (alla mål), i Linux-skalet utan `a11y-linux` eller i skalet för Windows och macOS, så
inget annat bygge ändras. Stacken svarar en skärmläsare över D-Bus i en egen tråd och ligger inte i
någon frame; den är inte mätt (det kräver en session-buss och en skärmläsare, och
`spec_13_1_1_cold_start` kräver fönster). `accesskit_consumer`, som alla plattformar använder, står
kvar på 3.

**`a11y-linux` på som standard** (`default = ["window", "a11y-linux"]` i vy-shell). Funktionen gör
inget på Windows och macOS: `accesskit_unix` är ett målberoende i `accesskit_winit`
(`cfg(target_os = "linux" | BSD)`), och korskontrollen för `x86_64-pc-windows-msvc` och
`aarch64-apple-darwin` är ren. `vy-cli` tar `vy-shell` utan standardfunktioner och påverkas inte.

| `vy-shell`, release, Linux x86_64, strippad | Byte | Mot 25 000 000 |
| --- | --- | --- |
| `0fc0453`, utan `a11y-linux` | 22 364 840 | −2 635 160 |
| `0fc0453`, `--features a11y-linux` | 25 476 976 | **+476 976** |
| + serif och mono komprimerade | 25 099 216 | **+99 216** |
| + AT-SPI-stacken `opt-level = "s"`, `a11y-linux` som standard | 24 365 264 | −634 736 (2,5 %) |
| + X11- och Wayland-protokollcraten `opt-level = "s"` | **24 212 688** | −787 312 (3,1 %) |

`spec_12_3_1_size_budget` i release mot den sista: "24365264 bytes, cap 25000000, margin 634736",
grönt. `vy-cli` blev 20 428 856 → 19 215 400 med alla fyra snitt komprimerade (förkastat, ovan);
med bara serif och mono räknas ungefär 355 000 mindre än förut. Konformans 59/59 à 0,000 % efter
varje steg (snitten är samma byte efter uppackning).

**Mer marginal för GL-vägen.** Samordnaren: m5/gpu lägger till en OpenGL-väg på som standard för
247 624 byte (`vy-shell` med `a11y-linux` 25 790 640 på main + m5/gpu), och spåret här bör frigöra
minst 0,8 MB, helst 1,2. Därför också de 17 X11/Wayland-crates under winit som bara Linux-skalet
bygger (x11rb, x11rb-protocol, wayland-client, wayland-backend, wayland-protocols(-plasma, -wlr),
wayland-sys, wayland-cursor, smithay-client-toolkit, calloop(-wayland-source), xkbcommon-dl,
xkeysym, as-raw-xcb-connection, x11-dl, tiny-xlib) på `opt-level = "s"`: 152 576 byte till. De
avkodar fönstersystemets händelser och kodar förfrågningar, mikrosekunder per händelse; softbuffer
(som kopierar bilden till fönstret) och winit själv står kvar på 3. Totalt **1 264 288 byte**
mindre än `0fc0453` med `a11y-linux`.

**Typsnitten i detalj.** cosmic-text:s `FontMatchKey::new` läser *varje* snitt vars vikt skiljer sig
från den begärda (`with_face_data`, för att leta efter en variabel `wght`-axel), för alla familjer.
Text i vikt 400 läser alltså bold, och text i vikt 700 läser sans, serif och mono. Serif och mono
packas därför upp första gången en sida (eller chromen) visar fet text eller serif/mono, inte bara
vid serif/mono; en sida i ren sans 400, som `hello`, packar aldrig upp något. Kostnaden är ~1,7 ms en
gång per process där det händer, mot `first-frame.text`:s 74 ms.

**Mätt på main + m5/gpu + det här spåret** (`git merge --no-commit --no-ff m5/gpu` i den här
arbetskopian, konflikten i `vy-shell/Cargo.toml` löst som `default = ["window", "gl",
"a11y-linux"]`, bygget, sedan `git merge --abort`; inget committat):

| `vy-shell`, release, Linux x86_64, strippad | Byte | Marginal mot 25 000 000 |
| --- | --- | --- |
| med GL och AT-SPI (standard efter sammanslagningen) | **24 524 312** | 475 688 (1,9 %) |
| utan GL (`--no-default-features --features window,a11y-linux`) | 24 274 936 | 725 064 (2,9 %) |

GL-vägen kostar här 249 376 byte (m5/gpu: 247 624). Samordnarens tal för main + m5/gpu med AT-SPI
var 25 790 640; spårets besparing på 1 264 288 ger 24 526 352, inom 2 040 byte av det uppmätta.
Sammanslagningen hade en konflikt (raden `default` i `vy-shell/Cargo.toml`), allt annat, också
`vy-render/src/text.rs` (m5/gpu: `Glyph::key`), slogs ihop automatiskt.

## Steg 4 – kodsignering i `release.yml`

Ett steg *Signing secrets* avgör per runner hur det signeras: på Windows `azure` om alla sex
Azure-secrets finns, annars `pfx` om båda PFX-secrets finns; på macOS `apple` om alla sex
Apple-secrets finns. Saknas alla blir det en `::notice` ("the binaries are not code-signed") och
jobbet fortsätter; finns bara några av ett sätts secrets blir det en `::warning` med de saknade
namnen och ingen signering (provkört lokalt med bash: inga, en av sex, båda PFX). Signeringen sker
efter att de osignerade binärernas SHA-256 skrivits och före zip-filen.

- **Windows, Azure Artifact Signing:** `azure/artifact-signing-action` v2.0.0, pinnat på
  `c7ab2a8…` (`git ls-remote`; repot heter också `azure/trusted-signing-action`), `files-folder`
  `stage`, filter `exe`, SHA-256, tidsstämpel `timestamp.acs.microsoft.com`. Secrets:
  `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`,
  `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE` (uppdragets ENDPOINT/ACCOUNT/PROFILE med
  prefix, så att namnen säger vad de är).
- **Windows, PFX:** signtool ur den nyaste Windows Kits, `/fd SHA256 /tr <WINDOWS_TIMESTAMP_URL
  eller digicert> /td SHA256`, i PowerShell (MSYS skriver annars om `/f`-flaggorna till sökvägar);
  filen tas bort i `finally`. Båda Windows-vägarna följs av `signtool verify /pa /v`.
- **macOS:** tillfällig nyckelring, `codesign --force --timestamp --options runtime`, med
  `tools/macos-entitlements.plist` (`com.apple.security.cs.allow-unsigned-executable-memory`) för
  de fyra binärer som kör wasmtime; wasmtime 50 gör koden körbar med `mprotect` utan `MAP_JIT`
  (sökt i källan: ingen `MAP_JIT`), vilket den härdade körmiljön bara tillåter med den rättigheten.
  `codesign --verify --strict`, sedan `notarytool submit --wait --output-format json`; inte
  `Accepted` → loggen hämtas och steget faller. Nyckelringen och nyckelfilerna tas bort i en
  `trap`.
- `BUILD.md` får kolumnen *Signerad* ur fragmentet (`ja (azure)`, `ja (pfx)`, `ja (apple)`, `nej`),
  och `SHA256SUMS` de osignerade raderna för ett signerat mål.

Kostnader och vad ägaren gör står i `docs/RELEASING.md` (*Kodsignering*), med källor: Azure Artifact
Signing ~9,99 USD/månad (Basic, Microsoft Learn 2026-08-29; organisationer i EU, enskilda bara i
USA/Kanada); OV-certifikat 150–300 USD/år (nyckel i HSM sedan 2023); Apple Developer Program 99 USD
per medlemsår (D-U-N-S för en organisation). **Inget av det är kört**: inga secrets, ingen Windows-
eller macOS-maskin, `pwsh` finns inte här. YAML:en parsas; bash-delarna är syntaxkontrollerade
(`bash -n`) och detekteringen provkörd.
