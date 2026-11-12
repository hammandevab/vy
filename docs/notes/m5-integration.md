# M5 – sammanslagningen av spåren (`m5/integration`)

Integrationsagentens noteringar. Grenen `m5/integration` utgår från main `0fc0453` (release v0.2.0).
Spåren slås ihop som riktiga merge-commits (`--no-ff`) i ordningen m5/sdk, m5/gpu, m5/release, med
grindarna efter varje sammanslagning och uppföljningarna som egna små commits. m5/examples (från
m5/sdk) slås ihop sist, när koordinatorn säger till. Granskningssidan skrivs av en annan agent.
Sessionen kan startas om: läget står här och i `git log`.

"Grindar" är alltid alla, från arbetskopians rot: `cargo fmt --all --check`, `cargo clippy --workspace
--all-targets -- -D warnings`, `tools/build_modules.sh` + `check-abi target/wasm/hello.wasm` +
`tools/test_modules.sh`, `cargo test --workspace`, `python3 tools/spec_trace.py`, `vy-cli conformance`,
`check-cross --workspace --all-targets` (Windows och macOS) och arbetsflödenas YAML parsad med
`python3 yaml`. Modulbygget måste gå före `cargo test` (vy-cli:s tester läser `target/wasm/`).

## Sammanslagningar

| Commit | Spår | Konflikter | Grindar efteråt |
| --- | --- | --- | --- |
| (denna) Slå ihop m5/sdk | `1de409e`, 15 commits | inga | 390 gröna / 4 ignorerade, spec 140/140, 13 moduler, 64/64 à 0,000 % |
