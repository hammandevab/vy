# Vy

En klient för ett nytt innehållsformat: sidor är WebAssembly-moduler som ritar sig själva via ett
minimalt värd-API, hämtas över HTTPS med `vy://`-adresser och körs i en sandbox. Ingen HTML, ingen
CSS, ingen JavaScript-motor.

- `SPEC.md` – specifikationen, numrerad. Det är källkoden.
- `CLAUDE.md` – arbetsbeskrivning för agenterna som bygger klienten.
- `crates/` – klienten i Rust. `modules/` – referensmoduler. `conformance/` – guldbilder.

## Status: v0.3.0

Klienten (`vy-shell`) hämtar signerade sidor över `vy://`, kör varje flik i en egen sandlådad
process och ritar med OpenGL i fönstret på Windows och Linux (mjukvara på macOS, och som
referens och reserv överallt). ABI 1.0 är fryst.
Under `modules/` finns nitton sidor på `vy-sdk`: tolv referenssidor, Studio och sex större
exempel (`docs/EXEMPEL.md`); `conformance` jämför över hundra fall mot guldbilderna och håller
varje frame till budgeten. `vy-dev` skapar, bygger och serverar ett sidprojekt, med Studio för
att låta Claude Code bygga sidan (`docs/DEVELOPING.md`); `docs/SDK.md` är guiden till SDK:t.
En release byggs reproducerbart med kontrollsummor och byggrecept (`docs/RELEASING.md`).

Öppet (se `CLAUDE.md`, milstolparna, och `docs/notes/m5-slut.md`):

- SPEC-13.2.1 och SPEC-13.1.1 mäts på en riktig dator med GPU.
- Första kompileringen av en sida tar längre än SPEC-13.1.4 ger; cachen gör återbesöken gratis.
- Prestandagrinden (5 %, SPEC-12.2.3) håller bara på en och samma maskin; CI rapporterar utan
  att fälla.
- En sidas lagring skrivs utan `fsync` och på huvudtråden (SPEC-9.5.4): en avbruten process
  förlorar högst väntetiden, ett strömavbrott kan ta mer.
- Releasens Linux-binärer byggs på GitHubs avbild, inte i en container pinnad på digest.
- En panik när en sidas display list ritas stoppar sidan (CPU-vägen) eller lämnar GL-vägen, inte
  klienten. Det gäller inte kromet: sidans titel, reservtext, länkmål och, med devtools öppet,
  logg och träd ritas av klienten utan sådant skydd. En tappad GL-kontext märks bara där
  drivrutinen säger till.
- SDK:ts allokator letar bland de stora fria blocken när en sida begär mer än 1 024 byte: en
  sida med tusentals stora hål av olika storlek betalar för det (granskning 0019:1). Block upp
  till 1 024 byte kostar lika mycket hur många hål högen än har.
- Den externa sandlådegranskningen återstår, och Windows och macOS är byggda och testade i CI
  men inte körda för hand i varje del.

```sh
tools/build_modules.sh                 # modules/* -> target/wasm/*.wasm (före cargo test)
cargo test --workspace                 # gröna tester
cargo test --workspace -- --ignored    # backloggen
python3 tools/spec_trace.py            # spårning spec -> test
cargo run -p vy-cli -- check-abi target/wasm/form.wasm
cargo run -p vy-cli -- render target/wasm/form.wasm --out form.png --size 800x600 --grant ime \
    --dump-semantic form.json
cargo run -p vy-cli -- conformance     # pixeldiff <= 0,1 % mot guldbilderna
cargo run -p vy-publish -- keygen --out dev.key
cargo run -p vy-publish -- build --manifest conformance/manifests/hello.json \
    --module target/wasm/hello.wasm --key dev.key --out hello.vyp   # signerat paket (M3)
cargo run -p vy-serve -- --dir . --public-key dev.key.pub           # vy://localhost:8080/hello
```

Byggnoteringar (beroenden, approximationer, spec-frågor): `docs/BUILD-NOTES.md`; spårens
noteringar under `docs/notes/`; granskningarna under `docs/reviews/`.
SDK-dokumentation med exempel: `docs/SDK.md` och `crates/vy-sdk/README.md`.

Licens: MIT.
