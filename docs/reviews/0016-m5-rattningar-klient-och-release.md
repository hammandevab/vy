# PR #16 – rättningarna i klienten och releasen efter 0011–0015 (`1eda75b..b7fb243`, `m5/gate-fixes`)

Granskaren skrev ingen av de granskade raderna. Lagringen (SPEC-9.5.4) och SDK:t med
exempelsidorna granskas på 0017 och 0018 och ingår inte här.

**Rekommendation.** **Släpp v0.3.0 efter fynd 1**: commiten som taggas ska ha en hel grön
CI-körning, och det hade ingen när jag läste. Inget i koden stoppar releasen. Fynd 2 är en
mening i README som bör med i samma commit.

**Det som stoppar.** Bara fynd 1. Körningen 43557039351 (`b16edbf`) var inte klar kl. 05:01 UTC:
`repro (windows)` pågick efter 30 minuter, de sju andra jobben var gröna. HEAD (`b7fb243`) har två
commits till som inte är körda i CI, och den ena ändrar `ci.yml` och `fuzz.yml`.

**Vad ändrades.** Två rättningsrundor svarar på fynden i 0011, 0012, 0014 och 0015 (klienten:
GL-vägen, kompileringscachen, laddaren; releasen: generalrepetition, röktest, rättigheter,
summor). Därefter koordinatorns commits: en vakt runt PNG-avkodaren, CI-rättningar på tre
plattformar, ett ignorerat test för SPEC-13.1.4, och pinnade actions utan byggcache där något
signeras.

**Spec-id som berörs.** SPEC-9.4.2, 9.1.1, 11.2.2, 11.3.2 (ändrad i `456ec32`), 9.7.1, 12.3.1,
13.1.4, 6.5.1, 7.4.5. **ABI 1.0 är orört:** `git diff v0.2.0..HEAD -- crates/vy-abi
docs/ABI-1.0.md` är tomt, och i `SPEC.md` är bara §9, §11 och §12 ändrade sedan v0.2.0; §5 och
bilaga A och C är byte för byte desamma.

**Risk.** En sida som fäller hela klienten i stället för sin flik (fynd 2), sparad maskinkod
som når fel process, och en release som signeras med något annat än det som står i arbetskopian.

**Bevis.** *Kört av mig:* PNG-vaktens tre tester (`png_guard_tests`) och
`spec_11_3_2_damage_draws_what_the_reference_draws` gröna; `spec_trace.py` 143/143; varje pinnad
action prövad mot upphovets tagg med `git ls-remote` (checkout v4.4.0, upload-artifact v4.6.2,
download-artifact v4.3.0, rust-cache v2.9.3, rust-toolchain grenen 1.97.0, artifact-signing
v2.0.0, action-gh-release v2.6.2: alla stämmer); CI-körningarnas jobb lästa med `gh`.
*Läst:* varje rad i tabellen nedan där det står "läst", de tre arbetsflödena och
`tools/build_release.sh`, `repro_env.sh`, `publish_site.sh`. *Inte kört av mig:* testerna i
`vy-shell` och `vy-cli` (se sist).

**Nya beroenden.** Inga.

**Hotbild.** Ingen ny ABI-import. Kompileringscachen, prövad mot uppdragets tre frågor ur koden:
(1) sparad kod deserialiseras bara i `vy-runtime/src/compile_cache.rs:577`, som bara flikens
`Runtime::load` når med `CompileCache::Host`; skalet öppnar ingen cache utan egen binär, erbjuder
kod bara till en flik som är en process och tar bara emot från en sådan. (2) En flik kan inte
välja nyckel: `ToShell::Code` bär bara `cached`, `micros` och `code`, och nyckeln ligger i
`Pending`, gjord i `start_page` ur de byte som skickas. Rapporten räknas en gång per laddning och
sänds före `vy_init`; en modul med startfunktion vägras av ABI-kontrollen
(`abi_check.rs:193`), så ingen sidkod har körts dessförinnan. (3) Nyckeln: se fynd 4.
Bilder: avkodas i flikprocessen; huvudprocessen prövar pixlarna den får
(`ImageCache::set_decoded`: längd = b × h × 4, sidgräns, tak). Paket och manifest tolkas på
laddartrådar med panikvakt. Kvar: fynd 2.

## Tidigare fynd

| Fynd | Commit | Bedömning |
| --- | --- | --- |
| 0011:1 ingen EGL-konfiguration | `7563132` | Rättat (läst): `pick` ger `None`, `guarded` gör paniken till CPU-vägen. |
| 0011:2 GL-kompositionen otestad | `f1352ed` | Rättat enligt commit; testet kräver Xvfb och är inte kört av mig. |
| 0011:3 vägrad lista visar förra fliken | `c176a1c` | Rättat (läst ytligt: `compose` loggar och visar ingen sida). |
| 0011:4 tolerans utan gräns | `62ceaa5`, `456ec32` | Rättat i del: felet och gränsen 64 nivåer är inne; en riktig GPU är fortfarande okörd (sagt öppet). |
| 0011:5 tappad kontext | `f8d9be3` | Rättat i del: märks bara där drivrutinen aviserar; gränsen står i `m5-fix-b.md`. |
| 0011:6 storlek, skala, stil | `bb1c280` | Rättat; testet kört av mig. |
| 0012:1 `/Brepro` under Git Bash | `4f6aca8`, `ff43e63` | Rättat (läst); repetitionens Windows-bygge grönt. Se fynd 1 om sista körningen. |
| 0012:2 storleken okänd före taggen | `9fec9b7` | Rättat: `dry_run` finns och är körd grön på tre plattformar. |
| 0012:3 taggen finns inte än | `e22a9a1` | Rättat i del: röktestet är kört med `--sdk`; vägen med taggen körs första gången i den riktiga releasen (fynd 5). |
| 0012:4 mer rätt än behövligt | `e22a9a1` | Rättat (läst), med gränsen i fynd 6. |
| 0012:5 `SHA256SUMS` ofullständig | `e22a9a1` | Rättat: `BUILD.md` och varje zip har en rad; sajtzippens innehåll täcks av zippens summa, och filhuvudet säger nu det. |
| 0012:6 container på digest | – | Inte rättat. Står som öppet i `m5-fix-a.md`, inte i README. |
| 0012:7 kallstart med AT-SPI | `3714cdc` | Besvarat med mätning (under 1 %); `spec_13_1_1_cold_start` är fortfarande ignorerat. |
| 0014:1 version i nyckeln | `041dcda` | Rättat (läst), med gränsen i fynd 4. |
| 0014:2 sparad kod i huvudprocessen | `a7071b8` | Rättat (läst), i tre lager. |
| 0014:3 symlänk, andras filer | `afe7245` | Rättat (läst): `symlink_metadata`, `O_NOFOLLOW`, bara egna namn. |
| 0014:4 Windows | `cbe7d6f` | Besvarat i ord: cachen är på. Beslutet togs av koordinatorn, inte av ägaren som 0014 bad. |
| 0014:5 laddare som panikar | `1aab938` | Rättat (läst): vakt och `catch_unwind` per jobb. |
| 0014:6 inaktuella jobb | `c4efd8b` | Rättat enligt commit; inte läst i detalj. |
| 0014:7 storlekstak | `5faa507` | Rättat: `form.wasm` högst 32 000 byte i `test_modules.sh`. |
| 0015:1 | – | Hör till 0018. |
| 0015:2 inget kört på andra plattformar | flera | Rättat i del: CI och repetition på tre plattformar; riktig GPU och handkörning sagt öppet. |
| 0015:3 prestandagrinden | `e89b198`, `c93731b` | Rättat i del: ny baslinje för byggmaskinen och ett ignorerat test; CI rapporterar fortfarande bara (fynd 3), och ingen modul nära 300 KB finns. |
| 0015:4 M3 och M4 avbockade | `803d766` | Rättat. |
| 0015:5 dokument som glidit | `803d766` | Rättat i del: `m5-integration.md` kallar ännu `rymd`s 133 nivåer en kantpixel. |

**Första granskarens utkast.** 1 (bilder avkodas i fliken): stämmer, och `18f65c8` rättar
kommentaren och noteringen rätt och helt. 2 (PNG-vakten): stämmer; `catch_unwind` verkar i
release (ingen `panic = "abort"`). 3 (cache där sajten signeras): `b7fb243` svarar rätt och helt.
4 och 5: tagna som fynd 4 och 7.

## Egna fynd, efter allvar

1. **Medel – commiten som ska taggas har ingen hel grön CI-körning.** Körningen 43557039351 på
   `b16edbf` stod som `in_progress` 04:57–05:01 UTC med `repro (windows)` oavslutat; HEAD har
   därtill `18f65c8` och `b7fb243`, okörda i CI. Framkallat: läst med `gh run view`. De pinnade
   commitarna i `ci.yml` är samma som repetitionen redan kört, så risken är liten. Rättning: vänta
   ut körningen, skjut HEAD, tagga när den körningen är grön.
2. **Medel – en sidas display list kan fälla hela klienten, och README säger det inte.**
   `crates/vy-render/src/renderer.rs:231` (referensvägen) har ingen panikvakt; den snabba vägens
   vakt (rad 244) faller tillbaka på just den, så en panik i delad kod (text, banor) fångas en gång
   och fäller sedan huvudprocessen. `m5-slut.md` säger detta riktigt, men listan är inte hel:
   fönstrets GL-ritning är också ovaktad efter uppsättningen (står i `m5-fix-b.md`), och i
   `vy-shell` finns `catch_unwind` bara runt GL-uppsättningen och laddarjobben, så allt annat en
   flik skickar hanteras ovaktat på huvudtråden. Ingen panik framkallad: slutsats ur läsning.
   Stoppar inte: gäller sedan v0.2.0, kostar tillgänglighet och inte minnessäkerhet, och lagringen
   sparas nu löpande. Rättning: en mening under "Öppet" i README nu; sedan en vakt per flik runt
   `Renderer::render` och `GlContext::compose` som stoppar sidan.
3. **Låg – prestandagrinden står som grind i `CLAUDE.md` men är en rapport i CI.**
   `conformance/bench/ubuntu.json` finns inte (`m5-slut.md` förklarar varför). Varken README eller
   grindtabellen säger det. Rättning: en rad i grindtabellen.
4. **Låg – kodcachens nyckel bär versionssträngen, inte bygget**
   (`compile_cache.rs:70`, `150–159`). Två byggen med samma version (utvecklingsbyggen, en
   omtaggad release, ett paketerarbygge med en rättning) delar poster. Läst. Släppta versioner
   berörs inte. SPEC-9.4.2 nämner inte heller klientversionen. Rättning: commitens hash i nyckeln
   och ett `spec:`-commit.
5. **Låg – jobbet `release` och röktestets riktiga väg körs första gången i releasen.**
   `release.yml:421–483`: v0.2.0 släpptes med `action-gh-release@v2` och skrivrätt på hela flödet;
   nu är det en pinnad commit och skrivrätt bara på jobbet. Röktestet körs efter publiceringen, så
   ett rött röktest lämnar en publicerad release. Läst. Rättning: ingen före releasen; var beredd
   att köra om. Senare: publicera som utkast tills röktestet är grönt.
6. **Låg – stegdelningen i `site` skyddar mot läckor, inte mot ett fientligt byggskript.**
   `release.yml:348–367`: bygget och signeringen delar maskin, så ett byggskript kan byta
   `target/release/vy-publish` eller `tools/publish_site.sh` före steget med `VY_KEY`. Läst.
   Testet i `tests/release.rs` är textkontroller: det letar efter `rust-cache` och ser varken
   `actions/cache` eller hemligheter i `fuzz.yml`.
7. **Låg – en generalrepetition signerar på riktigt.** Kodsigneringen, notariseringen och
   `VY_KEY` används också med `dry_run`, på vilken gren som helst, och resultatet ligger som
   artefakter. Avsett enligt filhuvudet; ägaren bör veta det. Om `VY_KEY` är satt kunde jag inte
   se (hemligheterna går inte att lista härifrån).
8. **Låg – README: "ritar med OpenGL i fönstret" gäller inte macOS**
   (`crates/vy-shell/Cargo.toml:53`: GL-beroendena finns bara utanför macOS).

## Det jag inte kontrollerade

- Inga tester i `vy-shell` eller `vy-cli` är körda av mig, inte heller `tests/release.rs`: andra
  granskare hade oincheckade ändringar i `crates/vy-shell/src/shell.rs` och `lib.rs` i den delade
  arbetskopian medan jag arbetade. Jag litar där på koordinatorns grindkörning.
- GL-testet (kräver Xvfb), `cargo fmt`, clippy, konformansen, `bench`, modulbygget.
- Koden bakom 0011:2, 0011:4 och 0014:6, och `a148b67` (macOS-länkningen) utöver att
  `repro (macos)` är grönt.
- `vy-proto`:s tolkning av paket och manifest rad för rad; bara att den körs bakom laddarens vakt.
- Om sidans strängar (titel, logg, semantiskt träd) kan fälla chromens textkod.
- Windows och macOS i någon del; signeringsstegen, som bara körs med hemligheter.
