# M5 – slutet: första CI-körningen, releasen och grindprovet

Koordinatorns noteringar efter sammanslagningen (`0cd453c`). Här står det som hände mellan att M5
låg på `main` och att v0.3.0 släpptes: vad CI fann på tre plattformar, en krasch som fuzzningen
fann, vad mätningarna säger om två grindar som inte går att lita på i dag, och vad som är öppet.

## Commits

| Commit | Vad |
| --- | --- |
| `41399fd` | `vy-render`: en PNG med trasig palett vägras i stället för att fälla klienten |
| `ff43e63` | CI: tre fel från första körningen efter M5 |
| `e89b198` | `bench:` ny baslinje för byggmaskinen |
| `c93731b` | `vy-cli`: SPEC-13.1.4:s kalla halva som ignorerat test; `CLAUDE.md` räknar den till M3:s öppna grind |
| `a148b67` | release: macOS-bygget länkas utan felsökningskarta |

## Kraschen fuzzningen fann (`41399fd`)

Nattens fuzzkörning 2026-11-12 fällde `png` 0.18.1: en PLTE-chunk vars längd inte är hela
RGB-poster får avkodaren att indexera utanför paletten och paniken går rakt upp. Bilder avkodas i
huvudprocessen (SPEC-9.1.1: den äger rasteriseringen), så en sida med en sådan resurs fällde hela
klienten, inte bara sin flik. Felet finns i den släppta v0.2.0.

Rättningen har två lager. `check_png_chunks` går igenom chunkarna fram till IDAT och vägrar en
palett som är tom, inte delbar med tre eller längre än 256 poster, och en tRNS som är längre än
paletten i en indexerad bild; det är fel som formatet förbjuder, så inget giltigt går förlorat.
Därefter körs avkodaren inom `catch_unwind`, så en panik vi inte känner till blir ett avkodningsfel
(SPEC-11.2.2) i stället för en död klient. Tester: `spec_6_5_1_a_palette_that_is_not_whole_entries_is_refused`,
`spec_11_2_2_a_panic_in_the_png_decoder_is_a_refusal` (bygger filen med giltiga kontrollsummor och
visar att den oskyddade avkodaren verkligen får panik) och `spec_6_5_1_chunk_check_refuses_only_malformed_palettes`.
Fuzzerns fil ligger i `fuzz/artifacts/png/` och spelas upp av `fuzz_corpora_replay`.

Det som återstår av samma slag: text och banor rasteriseras också i huvudprocessen, med
`cosmic-text`, `swash` och `tiny-skia`. De har fuzzmål för display listan men inte för varje väg in
i typsnittskoden. Ett skydd per flik runt rasteriseringen (en panik stänger fliken, inte klienten)
är nästa naturliga steg och är inte gjort.

## Vad CI fann (`ff43e63`, `a148b67`)

Första körningen efter M5 var också första gången jobben `repro`, `size` och GL-steget kördes
någon annanstans än på byggmaskinen.

1. **`gates`, GL-provet under Xvfb.** `winit` laddar `libxkbcommon-x11` för tangentbordet i ett
   X11-fönster; runner-bilden har det inte. Paketet installeras nu tillsammans med Mesa och Xvfb.
2. **`platform (windows)`, `spec_9_7_1_reproducible_build`.** `Command::new("bash")` letar i
   systemkatalogen före PATH och hittar WSL-startaren, som inte har någon distribution och skriver
   sitt fel på stdout. Testet letar nu upp Gits bash (PATH utan systemkatalogen, annars
   `%ProgramFiles%\Git\bin`), och ett misslyckat bygge skriver ut status och stdout. De andra
   testerna som kör bash är `cfg(unix)` och hade därför aldrig prövat detta. Efter rättningen är
   testet grönt på Windows: modulerna blir samma byte i två kataloger också där.
3. **`repro (windows)`.** `/Brepro` exporterades i `tools/build_release.sh` och följde med till
   wasm-länkningen av Studio, där `rust-lld` tog det för en fil. Flaggan ges nu bara till de två
   nativa byggena.
4. **`repro (macos)`.** Binärerna från två arbetskopior skilde sig i 48 byte: LC_UUID (16) och
   signaturens hash över den sidan (32). Allt annat var lika. `a148b67` länkar utan
   felsökningskarta (`-Wl,-S`) och sätter `ZERO_AR_DATE`; om det räcker visar nästa körning.
   Jobbet skriver numera ut var `vy-serve` skiljer sig när byggena inte är lika.

Linux är grind för `repro` och håller. macOS och Windows rapporterar tills de gått igenom en gång
(`docs/RELEASING.md`).

## Baslinjen för prestanda går inte att lita på mellan maskiner (`e89b198`)

SPEC-12.2.3 säger högst 5 % regression. `vy-cli bench --compare` jämför P50 mot en incheckad
baslinje med 5 % plus ett brusgolv. Två mätningar visar att det bara fungerar på en och samma
maskin i en och samma session:

- CI:s `bench`-jobb kördes två gånger samma natt på samma kod så när som på CI-filerna. Raderna
  skilde sig 1,44–1,55 gånger (två olika runner-maskiner). Med den snabba körningen som baslinje
  hade den långsamma fällt 33 av 70 rader.
- Byggmaskinen går i dag 1,3–2 gånger fortare än när M4:s baslinje mättes på "samma" maskin.

Därför är `conformance/bench/ubuntu.json` inte incheckad och CI:s jämförelse förblir en rapport.
`build-vm.json` är uppdaterad med alla 70 rader, så att en jämförelse före och efter en ändring i
samma session har något att stå på. En grind som håller på delade maskiner behöver mäta båda
sidor i samma jobb: bygg `vy-cli` ur förra releasen och ur ändringen, kör `bench` med båda på
samma runner och jämför de två rapporterna. Det är inte byggt.

## SPEC-13.1.4: första besöket håller inte kompileringsbudgeten (`c93731b`)

Meningen ger en modul på 300 KB 60 ms. `spec_13_1_4_compile_time` lät `hello` (5 KB) stå in för
den. Det nya ignorerade testet `spec_13_1_4_every_page_compiles_cold_within_budget` mäter varje
modul under `modules/`, P95 över 100 körningar i release:

| Var | Minsta (`hello`, 5 KB) | Största (`kalkyl`, `skriv`, `kanban`, 53–62 KB) | Per KB kod |
| --- | --- | --- | --- |
| byggmaskinen | 19 ms | 130–147 ms | 2–3 ms |
| GitHubs runner (P50 ur `bench`) | 8–12 ms | 98–111 ms | ca 1,7 ms |

Tretton av nitton moduler går över 60 ms på byggmaskinen. `text` (30 KB, mest data) tar 16 ms:
det är koden som kostar, inte filens storlek. 300 KB kod skulle ta omkring en halv sekund.
Kompileringscachen (SPEC-9.4.2) gör varje senare laddning gratis, så det gäller första besöket
efter varje ny version av en sida eller av klienten.

Wasmtime är konfigurerat med Cranelift på en tråd (inga trådar i flikprocessen). Vägarna framåt,
som alla är ägarens beslut:

- **Två steg**: en snabb baskompilator (Winch) för första bilden och Cranelift i bakgrunden till
  cachen. Ger budgeten, men lägger en kompilator till i klienten, som har 367 KB kvar till 25 MB
  (SPEC-12.3.1), och koden går långsammare tills den optimerade är klar.
- **Parallell kompilering**: kräver trådar i flikprocessen och räcker inte ensam (fyra kärnor ger
  kanske en fjärdedel av tiden).
- **Ändra talet** till det en optimerande kompilator klarar och låta cachen bära resten. Ett
  `spec:`-commit.

Tills dess är testet backlogg och M3:s grind öppen också på den här punkten.

## Releasen v0.3.0 och grindprovet

Fylls i när de är gjorda (se slutet av filen).
