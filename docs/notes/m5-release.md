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
  värdet, som `spec_13_2_1_raster_budget`, och ägaren avgör spec-fråga 66.

**Spec-fråga 66 (m5/release).** SPEC-12.3.1:s "hela kodbasen inklusive spec och tester ryms under
400 000 tokens" hålls inte: 858 837 tokens räknat som byte/4 över git-spårade textfiler (och fler med
en riktig tokeniserare). Ska taket höjas (t.ex. 1 000 000), ska "kodbasen" betyda något smalare
(utan `docs/notes`, `docs/reviews`, låsfiler och genererade `conformance/semantic/*.json`: ungefär
720 000), eller ska meningen bli ett mål snarare än en grind? Räkningen i testet är ett förslag; en
annan definition är ett `spec:`-commit med testet.

## Steg 3 – `a11y-linux` under taket (pågår)

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
