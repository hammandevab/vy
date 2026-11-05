# Vy – arbetsbeskrivning för agenter

Du bygger **Vy**: en klient för ett nytt innehållsformat där sidor är WebAssembly-moduler
som ritar sig själva via ett minimalt värd-API, hämtas över HTTPS med `vy://`-adresser
och körs i en sandbox. Ingen HTML, ingen CSS, ingen JavaScript-motor.

`SPEC.md` är källkoden. Koden är en byggartefakt. Läs det här dokumentet helt innan du gör något.

## Sanningskällor, i prioritetsordning

1. **`SPEC.md`** – varje normativ mening har ett id (`SPEC-5.1.3`). Ändra aldrig en normativ mening
   utan att ändra testet som refererar den. En ändring i specen är ett eget commit med `spec:` som prefix.
2. **Konformanssviten** – `cargo test --workspace` + `tools/spec_trace.py` + `conformance/`.
   En klient som passerar sviten är en Vy-klient. Inget annat krävs eller räcker.
3. **Koden** – får skrivas om helt så länge 1 och 2 är gröna.

## Sex principer (SPEC §2) – när två kolliderar vinner den som står högre

1. Maskinen är den primära författaren. Ett kanoniskt sätt att göra varje sak. Inga alias.
2. Verifiering före läsbarhet. Allt som kan kontrolleras av en maskin ska det.
3. Minimal yta. Värd-API:t har under 40 funktioner i 1.0. Layout och tillstånd lever i modulen.
4. Ingen ambient auktoritet. Allt utanför ren beräkning och ritning är en capability.
5. Determinism. Samma modul, samma input, samma fönster ger samma display list, byte för byte.
6. Kompilera ner, aldrig tolka. Klienten innehåller ingen tolk för något språk.

## Så arbetar du

- **En uppgift = ett test som ska bli grönt.** Formulera alltid din uppgift så: "gör `spec_5_1_1_*` grön
  utan att bryta något annat". Aldrig "bygg renderaren".
- **Ignorerade tester är backloggen.** `cargo test --workspace -- --ignored` visar allt som återstår.
  När du implementerar något: ta bort `#[ignore]`, aldrig testet.
- **Headless först.** M1 är `vy-cli render` till PNG. Rör inte `vy-shell` förrän M1:s guldbilder är gröna.
- **Lägg inget i klienten "för enkelhetens skull".** Fråga dig vid varje tillägg: kan detta leva i
  modulen eller SDK:t i stället? Oftast ja (princip 3).
- **Nya beroenden kräver motivering** i PR-texten: vad det gör, varför det inte går att skriva själv
  på 200 rader, och vad det drar in transitivt.
- **Ingen ny ABI-import utan hotanalys** – en sida i `docs/reviews/` med rubrikerna
  *Vad*, *Vem kan missbruka*, *Hur begränsas det*, *Test som bevisar begränsningen*.

## Grindar som varje PR måste passera

| Grind | Krav | Kommando |
| --- | --- | --- |
| Format | `rustfmt` utan diff | `cargo fmt --all --check` |
| Lint | clippy utan varningar | `cargo clippy --workspace --all-targets -- -D warnings` |
| Test | alla icke-ignorerade tester gröna | `cargo test --workspace` |
| Spec-spårning | varje `SPEC-id` har minst en testreferens | `python3 tools/spec_trace.py` |
| Modul | `modules/*` bygger till wasm och passerar ABI-kontroll; varje moduls fmt, nativa tester och wasm32-clippy | `tools/build_modules.sh && cargo run -p vy-cli -- check-abi target/wasm/hello.wasm && tools/test_modules.sh` |
| Pixeldiff (från M1) | max 0,1 % avvikande pixlar mot `conformance/golden/` | `cargo run -p vy-cli -- conformance` |
| Prestanda (från M3) | ingen regression över 5 % mot SPEC §13 | `cargo run -p vy-cli -- bench` |

## Granskning

Varje PR får en granskningssida i `docs/reviews/<nummer>-<slug>.md`, skriven av en agent som
**inte** skrev koden. Mallen finns i `docs/reviews/README.md`. Människan läser granskningssidan,
aldrig koden. Skriv den för en läsare som har fem minuter.

## Milstolpar

| | Leverans | Grind |
| --- | --- | --- |
| M0 | numrerad spec, testskal, CI, `hello` bygger till wasm | spec granskad av ägaren |
| M1 | `vy-runtime`, `vy-dl`, `vy-render`, `vy-cli render` till PNG | 20 guldbilder gröna på Linux + macOS |
| M2 | Rust-SDK, tio referenssidor | ABI 1.0 fryst, inga öppna frågor i SPEC §15 |
| M3 | `vy-shell`, `vy-proto`, processisolering | prestandabudget SPEC §13 hållen |
| M4 | signering, semantiskt träd, devtools, fuzz, macOS + Windows | hotanalys klar, extern sandbox-granskning |
| M5 | reproducerbara byggen, publicerad svit, SDK-docs | utomstående agent publicerar en sida på < 1 h |

## Struktur

```
SPEC.md              specen, numrerad – källkoden
crates/vy-abi        konstanter delade av värd och moduler (no_std)
crates/vy-manifest   CBOR-schema, validering, capability-policy
crates/vy-proto      vy://-adresser, hämtning, cache, signatur
crates/vy-dl         display list-buffert och validering
crates/vy-runtime    wasmtime-instans, ABI-importer, budget
crates/vy-render     rasterisering (wgpu, mjukvarureserv), text
crates/vy-shell      fönster, flikar, adressfält (rörs ej före M3)
crates/vy-cli        headless: render, check-abi, conformance, bench
crates/vy-publish    keygen, build, inspect – utkast + modul + resurser till signerat paket
crates/vy-serve      statisk server: paket, nycklar, index, HTML-reserv
modules/hello        referensmodul 0 – text och en rektangel
conformance/         guldbilder, förväntade semantiska träd och manifestutkast per modul
tools/               spec_trace.py, build_modules.sh, test_modules.sh
docs/reviews/        granskningssidor, en per PR
docs/notes/          byggagentens noteringar per gren
```

## Språk

Spec, commit-meddelanden och granskningssidor på svenska. Kod, identifierare och felmeddelanden
på engelska. Inga emoji i kod eller commits.
