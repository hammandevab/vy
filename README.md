# Vy

En klient för ett nytt innehållsformat: sidor är WebAssembly-moduler som ritar sig själva via ett
minimalt värd-API, hämtas över HTTPS med `vy://`-adresser och körs i en sandbox. Ingen HTML, ingen
CSS, ingen JavaScript-motor.

- `SPEC.md` – specifikationen, numrerad. Det är källkoden.
- `CLAUDE.md` – arbetsbeskrivning för agenterna som bygger klienten.
- `crates/` – klienten i Rust. `modules/` – referensmoduler. `conformance/` – guldbilder.

## Status: M1

`vy-cli render` kör en modul headless i wasmtime, rasteriserar display listen i mjukvara och
skriver en deterministisk PNG; `conformance` jämför mot guldbilderna i `conformance/golden/`.
Nästa steg är M2: Rust-SDK och tio referenssidor.

```sh
tools/build_modules.sh                 # modules/* -> target/wasm/*.wasm (före cargo test)
cargo test --workspace                 # gröna tester
cargo test --workspace -- --ignored    # backloggen
python3 tools/spec_trace.py            # spårning spec -> test
cargo run -p vy-cli -- check-abi target/wasm/hello.wasm
cargo run -p vy-cli -- render target/wasm/hello.wasm --out hello.png --size 800x600 --scale 1 \
    --dump-semantic hello.json
cargo run -p vy-cli -- conformance     # pixeldiff <= 0,1 % mot guldbilderna
```

Byggnoteringar för M1 (beroenden, approximationer, spec-frågor): `docs/BUILD-NOTES.md`.

Licens: MIT.
