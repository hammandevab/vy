# M5 – reproducerbara byggen och releasekvalitet (`m5/release`)

Byggagentens noteringar. Grenen utgår från main `0fc0453` (release v0.2.0). Granskningssidan skrivs av
en annan agent. Sessionen kan startas om: läget står här och i `git log`.

## Läge

| Steg | Vad | Läge |
| --- | --- | --- |
| 1 | SPEC-9.7.1 reproducerbara byggen | pågår |
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
