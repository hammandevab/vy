# Vy

En klient för ett nytt innehållsformat: sidor är WebAssembly-moduler som ritar sig själva via ett
minimalt värd-API, hämtas över HTTPS med `vy://`-adresser och körs i en sandbox. Ingen HTML, ingen
CSS, ingen JavaScript-motor.

- `SPEC.md` – specifikationen, numrerad. Det är källkoden.
- `CLAUDE.md` – arbetsbeskrivning för agenterna som bygger klienten.
- `crates/` – klienten i Rust. `modules/` – referensmoduler. `conformance/` – guldbilder.

## Status: M0

Specen är numrerad, testskalen finns, CI kör grindarna, och referensmodulen `hello` bygger till
wasm och passerar ABI-kontrollen. Nästa steg är M1: `vy-cli render` till PNG.

```sh
cargo test --workspace                 # gröna tester
cargo test --workspace -- --ignored    # backloggen
python3 tools/spec_trace.py            # spårning spec -> test
tools/build_modules.sh                 # modules/* -> target/wasm/*.wasm
cargo run -p vy-cli -- check-abi target/wasm/hello.wasm
```

Licens: MIT.
