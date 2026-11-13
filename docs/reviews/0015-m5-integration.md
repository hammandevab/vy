# PR #15 – M5: sammanslagningen av fem spår (`0fc0453..3fbcf36`, `m5/integration`)

**Vad ändrades.** Fem merge-commits (sdk, gpu, release, examples, polish) ovanpå v0.2.0, med två
konflikter (`vy-shell/Cargo.toml`, `release.yml`) och uppföljningarna version 0.3.0, SPEC-12.3.1 per
crate och ett minnesurklipp i konformansens värd. 76 commits, 200 filer, 30 500 rader.

**Spec-id som berörs.** Allt i 0010–0014. `SPEC.md`: 141 id; ändrat är bara SPEC-9.4.2 (ny), 9.7.1,
11.3.2 och 12.3.1, var och en i ett `spec:`-commit.

**Risk.** En sammanslagning kan tyst tappa ett spårs regel, och fem spårs noteringar kan säga mer än
det sammanslagna trädet håller.

**Bevis.** Grindarna, körda av mig på `3fbcf36` (Linux x86_64, två vCPU): `cargo fmt --all --check`
rent; clippy `-D warnings` rent; `cargo test --workspace` **412 gröna, 0 röda, 2 ignorerade**
(`spec_13_2_1_raster_budget`, `spec_13_1_1_cold_start`); `spec_trace.py` **141/141**;
`build_modules.sh` 19 moduler, `check-abi` OK, `test_modules.sh` 19/19; konformans **91/91 à
0,000 %**; GL-testet under Xvfb med llvmpipe grönt (talen i 0011). **ABI 1.0 orört:**
`git diff 0fc0453..HEAD -- crates/vy-abi docs/ABI-1.0.md` är tomt och `SPEC.md` är orörd i §5 och
bilaga A och C. Inga konfliktmarkörer, inga krockande IPC-taggar (nya: 15 och 79), alla 19
`modules/*/Cargo.lock` på 0.3.0 (bygget går med `--locked`). Inte kört av mig: release-bygget
(klienten 24 591 912 byte enligt noteringen), `vy-cli bench`, `publish_site.sh`, `check-cross`.
Fynd, efter allvar:

1. **Hög – två fel ur spåren stoppar releasen:** flerradsfältets radbrytning (0010 fynd 1) och att
   `skriv` och `kalkyl` låser sig på sin egen sparade data (0013 fynd 1–2).
2. **Medel – inget av M5 är kört på Windows, macOS, en riktig GPU eller GitHub.** GL-fönstret och
   WGL, cachekatalogen under `%APPDATA%`, reproducerbarheten och storleken där, CI-jobben `repro` och
   `size`, hela `release.yml` och signeringen är lästa eller typkontrollerade. De 27 nya guldbilderna
   är bara körda på Linux.
3. **Medel – prestandagrinden är inte i kraft.** `conformance/bench/ubuntu.json` finns inte, så
   CI-steget bara rapporterar; `build-vm.json` är från före M5 (`compile.form` över gränsen enligt
   noteringen, inga rader för `widgets` och exempelsidorna). `spec_13_1_4_compile_time` låter `hello`
   (5 KB) stå in för 300 KB; `widgets` (55 KB) tar 90–125 ms kallt enligt noteringen. Flera
   budgettester (storlek, 13.1.4, 13.3.1) hävdar bara i release och återvänder i `cargo test`.
   Rättning: ett `bench:`-commit med ny baslinje, och en modul nära 300 KB i sviten.
4. **Medel – CLAUDE.md bockar av M3 och M4 fast grindarna inte är uppfyllda** (`e309115`):
   SPEC-13.2.1 är varken hållen eller mätt, SPEC-13.1.1 är ignorerat och den externa
   sandlådegranskningen är inte gjord. Bockarna är ägarens att sätta.
5. **Låg – dokument som glidit.** `notes/m5-gpu.md` (toleranstalen), `notes/m5-examples.md`
   (storlekar, "sämsta frame"), CLAUDE.md:s strukturlista ("wgpu", inga exempelsidor) och README:s
   status (M2.5).

**Nya beroenden.** Bara spårens: `glow`, `glutin`, `glutin-winit` (0011).

**Hotbild.** Sammanslagningen öppnade inget eget. Ny yta i M5: sparad nativ kod (0014) och
GL-drivrutinen i huvudprocessen (0011).

**Rekommendation.** **Merge efter** 0010 fynd 1, 0013 fynd 1–2, 0014 fynd 1–2, 0011 fynd 1–2 och en
grön CI-körning på GitHub (0012 fynd 1–2). Tagga `v0.3.0` först därefter.

**Windows – att verifiera för hand** (release-zippen; `vy-shell.exe` har ingen konsol, läs devtools
→ *Logg*):
1. Loggen säger `raster: OpenGL …` med grafikkortets namn. Öppna `shop`, `text`, `kanban`, `rymd`:
   hovra, rulla, ändra storlek, dra mellan skärmar med olika skalning. Inget blinkar, inga rester,
   inget ritas över adressfältet. `set VY_NO_GL=1` ska ge samma bild med `softbuffer`.
2. Vila och väck datorn, och tryck Win+Ctrl+Skift+B, med fönstret öppet: det får inte bli svart.
3. 2 ms (i Git Bash, ur källkoden): `tools/build_modules.sh`, sedan
   `VY_GL_REQUIRED=1 VY_GL_BUDGET=1 cargo test --release -p vy-shell --test gl`. Skicka raden `GL:`, P50/P95 och de tre raderna med
   värsta frame och största avvikelse (ska vara under 3 %).
4. Cachen: första besöket loggar "compiled … and kept", efter omstart "from the compilation cache";
   `%APPDATA%\vy\cache\code` har en `.code` per modul. Hugg av en fil: sidan laddas ändå. Flikarnas
   integritetsnivå är fortfarande **Låg**.
5. Exempelsidorna: dra kort, skriv formler och Ctrl+C, rita och ångra, skriv med AltGr och
   dödtangenter, spela `block`, vrid `rymd`; starta om och se att det sparade finns kvar. Klistra inte
   in lång text utan mellanslag i `skriv` förrän 0010 fynd 1 är rättat.
6. Summor: `certutil -hashfile vy-shell.exe SHA256` står i `SHA256SUMS` (under `.unsigned/` om
   filen är signerad; då ska Egenskaper → Digitala signaturer visa utgivaren). Filen är under
   25 000 000 byte.
