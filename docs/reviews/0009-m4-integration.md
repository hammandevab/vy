# PR #9 – M4: sammanslagningen av fyra spår (`3bc0bcb..ccef8b4`, `m4/integration`)

**Vad ändrades.** Fyra merge-commits (dev, ux, security, perf) med konflikter i `vy-runtime` (nätgrinden),
`vy-shell` (`Tab`-fälten, modullistan) och `vy-publish` (usage), plus tolv uppföljningar. De viktigaste
uppföljningarna: en enda plain-http-regel i skalet (`1afeae6`); en tråd per flik för sidornas egna
förfrågningar (`3b1cbac`); "begränsat läge: <orsak>" i chromen (`7ffd33e`); release-zippen får
`vy-dev` och `studio.vyp` (`87ee2ab`); Studio publiceras inte på den offentliga sajten (`c9af929`).
26 700 rader, 168 filer mot `main`.

**Spec-id som berörs.** Allt i 0005–0008. SPEC.md: 139 id, ändringar bara i §7, §8, §9 och §17.

**Risk.** En sammanslagning kan tyst tappa en regel som bara ett spår hade (nätgrinden fanns i tre
versioner) eller ge krockande IPC-taggar.

**Bevis.** Grindar körda av granskaren på HEAD, last < 1: `cargo fmt --check` rent; clippy
`-D warnings` rent; `cargo test --workspace` **352 gröna, 0 röda, 4 ignorerade** (13.2.1, 13.1.1,
12.3.1, 9.7.1; main: 257/19); `spec_trace.py` **139/139**; `build_modules.sh` 12 moduler, `check-abi`
OK för alla 12, `test_modules.sh` 12/12; konformans **59/59 à 0,000 %** (main 55); bench utan
regression (0008). **ABI 1.0 orört:** `git diff main..HEAD -- crates/vy-abi docs/ABI-1.0.md` är tomt,
och SPEC.md är orörd i §5 och bilaga A och C. Storlek i release, Linux x86_64, strippad
(`cargo build --release` gjorde inget, binärerna är aktuella): `vy-shell` **22 322 112 byte**, under
SPEC-12.3.1:s 25 000 000; `vy-cli` 20 358 944; `vy-publish` 16 429 696; `vy-dev` 3 225 056; `vy-serve`
1 083 448. `vy-shell --features a11y-linux` är 25 435 064 (integrationens tal, inte ombyggt) – se 0006.
Fynd, efter allvar:

1. **Medel – inget av det plattformsspecifika är kört.** AppContainer, `sandbox_init`, `DnsQuery_UTF8`,
   AccessKit-adaptrarna, `taskkill` och `fake_agent.ps1` är bara typkontrollerade. `release.yml`,
   `fuzz.yml` och Docker-bygget har inte körts. En grön CI-körning på Windows och macOS krävs före
   en testomgång.
2. **Medel – sidladdningar är fortfarande seriella mellan flikar** (öppen punkt 1). Ett DNS-uppslag
   som hänger håller alla flikars laddningar och live-omladdningen. Med 0007 fynd 2 är det
   obegränsat; utan det ≤ 6 s. Rätta 0007 fynd 2 nu; parallella laddningar (lås per origin i
   `vy-proto`) i M5.
3. **Låg – `net` med tom lista visas som beviljad** (`vy-manifest`, `default_grants`; spec-fråga 63).
   Inget släpps igenom, men märket ljuger. Neka `net` för en tom lista i policyn.
4. **Låg – modulbygget beror på sökvägen** (öppen punkt 7: `chat.wasm` och `form.wasm` skiljer per
   arbetskopia). Inte provat här. Blockerar SPEC-9.7.1 i M5; `--remap-path-prefix` i
   `build_modules.sh`.

**Nya beroenden.** Inga utöver spårens (0005–0008).

**Hotbild.** Sammanslagningen öppnade inget. Plain http följer nu en regel i tre lager: fliken
(`origin_of` + listan), huvudprocessen (`request_verdict`: `net` beviljat och sidans egen
loopback-origin) och laddaren (`transport_allowed`). `spec_5_2_8_plain_http_only_to_own_loopback_origin`
prövar det. Kvar: 0005 fynd 1, 0007 fynd 1–3.

**Rekommendation.** **Merge efter** 0005 fynd 1–2, 0007 fynd 1–2 och en grön CI-körning på Windows
och macOS. Övrigt som uppföljningar i M5.

**Windows – att verifiera för hand** (efter grön CI, binären under `C:\Program Files\Vy\`):
1. `vy-shell.exe` startar; Aktivitetshanteraren → Detaljer → Integritetsnivå: skalet Medel, varje
   `--tab` **Låg**. Avsluta skalet: flikprocesserna försvinner direkt.
2. `vy-shell.exe --tab --sandbox-probe`: `file-write: refused`; läsning, socket och process
   tillåtna är väntat i standardläget.
3. `set VY_SANDBOX=strict`: flikarna visar **AppContainer** och sidorna fungerar. "Sidan stannade"
   eller en tyst återgång rapporteras med Windows-version.
4. Första besök av en fjärrsajt med TXT-post: betrodd. Utan post: chromen visar "begränsat läge: …".
5. Urklipp och AltGr (`@ { } \ $ |`), Ctrl+V i `form`, markering och Ctrl+C på `document`, Ctrl+F, och
   Skärmläsaren läser `form` (notes/m4-ux, sex steg).
6. `vy-dev serve` med `claude.cmd` på `PATH`: Studio startar agenten, och Stoppa avslutar hela
   trädet (Aktivitetshanteraren).
7. Release-zippens `vy-shell.exe` är under 25 000 000 byte.
8. Spela inte in främmande sidor i devtools förrän 0007 fynd 1 är rättat.
