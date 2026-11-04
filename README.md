# Vy

En klient för ett nytt innehållsformat: sidor är WebAssembly-moduler som ritar sig själva via ett
minimalt värd-API, hämtas över HTTPS med `vy://`-adresser och körs i en sandbox. Ingen HTML, ingen
CSS, ingen JavaScript-motor.

- `SPEC.md` – specifikationen, numrerad. Det är källkoden.
- `CLAUDE.md` – arbetsbeskrivning för agenterna som bygger klienten.
- `crates/` – klienten i Rust. `modules/` – referensmoduler. `conformance/` – guldbilder.

## Status: M2, första halvan

`vy-cli render` kör en modul headless i wasmtime, rasteriserar display listen i mjukvara
(text, rektanglar, banor, PNG-bilder) och skriver en deterministisk PNG; `conformance` jämför
14 fall mot guldbilderna i `conformance/golden/`, med scriptade händelser och capabilities per
fall. `crates/vy-sdk` är modulsidans SDK (`App` + `export!`, `Frame`, layout, `Event`, mock för
nativa tester); referenssidorna `hello`, `text` (lång artikel med rullning) och `form`
(textfält, IME, kryssruta, knapp) bygger på det. Andra halvan av M2: fler referenssidor och
ABI-frys.

```sh
tools/build_modules.sh                 # modules/* -> target/wasm/*.wasm (före cargo test)
cargo test --workspace                 # gröna tester
cargo test --workspace -- --ignored    # backloggen
python3 tools/spec_trace.py            # spårning spec -> test
cargo run -p vy-cli -- check-abi target/wasm/form.wasm
cargo run -p vy-cli -- render target/wasm/form.wasm --out form.png --size 800x600 --grant ime \
    --dump-semantic form.json
cargo run -p vy-cli -- conformance     # pixeldiff <= 0,1 % mot guldbilderna
```

Byggnoteringar (beroenden, approximationer, spec-frågor): `docs/BUILD-NOTES.md`.
SDK-dokumentation med exempel: `crates/vy-sdk/README.md`.

Licens: MIT.
