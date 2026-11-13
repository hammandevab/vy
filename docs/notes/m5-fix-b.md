# M5 – rättningar efter granskning 0011 och 0014 (`m5/fix-b`)

Byggagentens noteringar för grenen `m5/fix-b` (från `m5/integration` `e431a60`). Ändrat: bara
`crates/vy-shell`, `crates/vy-runtime`, `crates/vy-render`, `crates/vy-proto`, de här noteringarna
och `SPEC.md` där det står. Fix A rättar samtidigt i `vy-sdk`, `vy-cli`, `modules/`, `conformance/`
och `tools/`. Sessionen kan startas om: läget står här och i `git log`.

## Fynd, commit och test

| Fynd | Commit | Vad | Test |
| --- | --- | --- | --- |
| 0014:1 | se `git log` (`granskning 0014:1`) | Klientens version ligger i nyckeln (`CodeKey::of` = `CodeKey::of_client(wasm, CLIENT_VERSION)`), så en post som en version sparat aldrig är den en annan slår upp. Katalogen bär en stämpel, filen `VERSION` med innehållet `VYCODE01 <version>`; skiljer den, saknas den eller är den ingen stämpel töms katalogen och stämplas om när `CodeStore::open` körs, alltså när klienten startar. Det tar också bort poster som ett wasmtime-byte gjort oläsbara. Devtools, fliken *Studio*: **Rensa cache (kompilerad kod)** tar bort alla poster och säger hur många; en laddning som var obesvarad när cachen rensades får inte sin kod sparad. | `spec_9_4_2_code_does_not_outlive_its_client_version` (vy-runtime): versionen i nyckeln, två versioners poster i samma katalog, stämpeln vid annan version, ingen, tom, annat format, för lång; `clear`. `spec_9_4_2_the_cache_is_cleared_on_update_and_on_request` (vy-shell, riktiga flikprocesser): en katalog från en annan version töms vid start och modulen kompileras; en post under en annan versions nyckel erbjuds aldrig; knappen med pekaren; obesvarad laddning; därefter kompilering och träff som förr; utan cache ingen knapp. Mutationer: versionen bort ur nyckeln och tömningen bort ur stämplingen fäller vy-runtime-testet. |

## Beslut och gränser

- **"Rensa cache" rensar bara kompilerad kod** (`<data>/cache/code`). Innehållscachen i `vy-proto`
  (paket och HTTP-svar under `<data>/cache`) och de pinnade nycklarna rörs inte: de är inte nativ
  kod, de kontrolleras mot hash och signatur vid varje användning, och en knapp som också glömmer
  pinnar vore en annan sak än den granskningen bad om. Det var också den mindre ändringen.
- **Stämpeln är klientens version, inte wasmtimes.** Ett wasmtime-byte utan versionsbyte finns bara
  i utvecklingsbyggen; där hittas gamla poster ändå aldrig (motorns hash ligger i nyckeln) och de
  trängs ut av storlekstaket som förr. Stämpeln läses innan motorn byggts, så starten väntar inte
  på wasmtime.
- **Två klientversioner som delar katalog** (en gammal klient igång medan en ny startar) tömmer
  varandras poster vid varje start. Ingen av dem läser den andras kod; det kostar kompileringar.

## Förslag till `spec:`-commit (inte gjort här)

- **SPEC-9.4.2** räknar upp vad nyckeln görs av: modulens SHA-256, wasmtimes version och motorns
  inställningar. Klientens version är nu också med. Meningen motsägs inte, men den säger inte att
  sparad kod aldrig överlever den klientversion som sparade den. Förslag på tillägg sist i meningen:
  "Sparad kod används aldrig av en annan klientversion än den som sparade den." Testerna ovan
  refererar redan SPEC-9.4.2.

## Öppet

(fylls på)
