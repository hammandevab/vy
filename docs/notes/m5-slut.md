# M5 – slutet: första CI-körningen, releasen och grindprovet

Koordinatorns noteringar efter sammanslagningen (`0cd453c`). Här står det som hände mellan att M5
låg på `main` och att v0.3.0 släpptes: vad CI fann på tre plattformar, en krasch som fuzzningen
fann, vad mätningarna säger om två grindar som inte går att lita på i dag, och vad som är öppet.

## Commits

| Commit | Vad |
| --- | --- |
| `41399fd` | `vy-render`: en PNG med trasig palett vägras (se rättelsen nedan om vad felet fällde) |
| `ff43e63` | CI: tre fel från första körningen efter M5 |
| `e89b198` | `bench:` ny baslinje för byggmaskinen |
| `c93731b` | `vy-cli`: SPEC-13.1.4:s kalla halva som ignorerat test; `CLAUDE.md` räknar den till M3:s öppna grind |
| `a148b67` | release: macOS-bygget länkas utan felsökningskarta |
| `1501fc3`, `da1544b`, `b16edbf`, `5d44c97` | SPEC-9.5.4: en sidas lagring sparas medan sidan körs |
| `b05e3d0` | guiderna efter förpasset av grindprovet |
| `b7fb243` | ingen byggcache där sajten signeras; alla actions pinnade |
| `0e3085d`, `88fda59` | en panik i rasteriseringen är sidans fel, inte klientens |
| (nästa) | `kalkyl` håller budgeten när värden växer; smårättningar ur 0016 och 0018 |

## Kraschen fuzzningen fann (`41399fd`)

Nattens fuzzkörning 2026-11-12 fällde `png` 0.18.1: en PLTE-chunk vars längd inte är hela
RGB-poster får avkodaren att indexera utanför paletten och paniken går rakt upp.

**Rättelse (granskning 0016:1).** Commit-meddelandet för `41399fd`, kommentaren i `image.rs` och
den första versionen av det här avsnittet sade att bilder avkodas i huvudprocessen och att felet
därför fällde hela klienten. Det var fel. I fönsterklienten avkodas en sidas bilder där sidan
körs, i flikprocessen (`vy-runtime/src/host.rs`, `host.images.ensure`), och huvudprocessen får
färdiga pixlar som `ToShell::Image`. I den släppta v0.2.0 dog alltså fliken med en sådan bild,
inte klienten: sandlådan höll som den skulle (SPEC-9.1.1, SPEC-11.2.2). `vy-cli render` och
konformanssviten, som avkodar i sin egen process, dog helt.

Rättningen är rätt ändå, men den lagar något mindre än det sades: en sida med en trasig bild
förlorar bilden i stället för att stoppas. Den har två lager. `check_png_chunks` går igenom
chunkarna fram till IDAT och vägrar en palett som är tom, inte delbar med tre eller längre än 256
poster, och en tRNS som är längre än paletten i en indexerad bild. Därefter körs avkodaren inom
`catch_unwind`, så en panik vi inte känner till blir ett avkodningsfel i stället för en stoppad
sida. Tester: `spec_6_5_1_a_palette_that_is_not_whole_entries_is_refused`,
`spec_11_2_2_a_panic_in_the_png_decoder_is_a_refusal` (bygger filen med giltiga kontrollsummor och
visar att den oskyddade avkodaren verkligen får panik) och
`spec_6_5_1_chunk_check_refuses_only_malformed_palettes`. Fuzzerns fil ligger i
`fuzz/artifacts/png/` och spelas upp av `fuzz_corpora_replay`.

Det som däremot körs i huvudprocessen på en sidas data är rasteriseringen av display listan: text
(`cosmic-text`, `swash`) och banor (`tiny-skia`). Den snabba vägen föll tillbaka på referensen om
den fick panik, men referensvägen, kedjans sista led, hade inget skydd, och inte heller GL-vägens
ritning av en frame: en panik där hade fällt klienten och alla flikar (granskning 0016:2). Båda
har nu skydd. En panik i referensvägen blir `RenderError::Failed` för den framen: skalet loggar
att listan inte gick att rita, sidans yta lämnas tom och nästa lista ritas som vanligt
(`spec_9_1_2_a_panic_in_the_rasterizer_is_the_pages_error`). En panik medan GL ritar lämnar
GL-vägen för CPU-vägen, som ett GL-fel redan gjorde (`gl::guarded("drawing", …)`). Det som inte
fångas är det `catch_unwind` inte fångar: minnesbrist, stacköverflöd och en avkodare som aldrig
återvänder.

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
   nativa byggena. Därefter gick jobbet igenom: de fem Windows-binärerna får samma SHA-256 i två
   arbetskopior med två `CARGO_HOME` och två tidszoner, så Windows är grind från och med nu.
4. **`repro (macos)`.** Binärerna från två arbetskopior skilde sig i 48 byte: LC_UUID (16) och
   signaturens hash över den sidan (32). Allt annat var lika. `a148b67` länkar utan
   felsökningskarta (`-Wl,-S`) och sätter `ZERO_AR_DATE`, och nästa körning gick igenom: det var
   kartan. Jobbet skriver numera ut var `vy-serve` skiljer sig när byggena inte är lika.

`repro` är därmed grind på alla tre plattformarna. Generalrepetitionen av releasen (körning
43550703892, `dry_run`) byggde dessutom samma commit på andra maskiner än `repro`-jobbet, och
binärerna för Linux, macOS och Windows fick samma SHA-256 i båda: tre oberoende byggen per
plattform som stämmer byte för byte.

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

## Förpasset av grindprovet: en utomstående agent bygger en sida

Innan releasen fick en agent utan projektkunskap releasekandidatens Linux-binärer och adresserna
till de publika guiderna, med uppdraget att bygga, signera, servera och visa en egen sida
("Dricksräknaren": textfält, reglage, två knappar, sparade värden) på under en timme. Enda
avvikelsen från en riktig utomstående: taggen `v0.3.0` fanns inte än, så SDK:t togs med `--sdk`.

Den blev klar på 12,4 minuter med alla sex kriterierna uppfyllda (byggd med `vy-dev`, signerad med
egen nyckel, serverad av `vy-serve`, hämtad över `vy://` och ritad av klienten, visad efter
inmatning, sparade värden tillbaka i en ny process). Agentens minuter är inte en människas; det
som går att föra över är hindren den mötte, och de är rättade här:

| Hinder | Rättning |
| --- | --- |
| Det sidan sparat skrevs till disk först när fliken laddades om eller stängdes; efter 20 s och en avbruten klient var ändringen borta fast `state_put` svarat OK | SPEC-9.5.4 (nästa avsnitt) |
| `vy-shell --screenshot` börjar med tom lagring och tar ingen inmatning, utan att säga det | står i `--help` och i `docs/DEVELOPING.md` |
| Hur man skickar händelser utan fönster stod bara i huvudet på `conformance/cases.toml`, som releasen inte bär | `docs/SDK.md` avsnitt 10, "Händelser utan fönster", med rotens form och ett helt fall; `vy-cli --help` säger var filerna ska ligga |
| Ett drag i ett fall kräver `buttons = 1`, vilket formatbeskrivningen inte sade | huvudet på `cases.toml` och exemplet i guiden |
| Mocken flyttar inte fokus mellan kontroller | `docs/SDK.md` avsnitt 10 |
| Guiderna slutade vid `vy-dev build`: nyckel, `vy-serve` och klienten stod bara som `cargo run`-rader i README | nytt avsnitt "Publicera" i `docs/DEVELOPING.md` |
| `vy-dev new --name` är sidans namn, inte titeln, och felet slutade med "(pass --name)" | användningsraden säger `<page-name>` och felet säger var titeln sätts |
| Tangentkoder är `i32`; utvecklarnyckelns plats stod fel; `vy-serve` lyssnar på alla nätverkskort medan guiden sade loopback; reglaget ritar sitt värde men inte sitt namn; inget exempel på lagring | rättat i projektmallens `CLAUDE.md`, `docs/DEVELOPING.md` och `docs/SDK.md` |

Det som inte är gjort: `vy-dev new` skriver inget konformansfall för startsidan och `vy-dev build`
kör inga fall. Det hade tagit bort det andra hindret helt och är nästa steg för verktygen.

Agenten fick projektets `CLAUDE.md` i sitt sammanhang av körmiljön. Den beskriver arbetssättet,
inte SDK:t, men provet var alltså inte helt utan förkunskap.

## Lagringen sparas medan sidan körs (SPEC-9.5.4)

**Före.** Flikprocessen skickade sidans lagring till huvudprocessen när fliken gömdes, laddades om
eller stängdes, och bara då. En sida som sade "sparat" hade sitt värde i flikprocessens minne.
En krasch, ett strömavbrott eller en avslutad process tog allt sedan sidan öppnades. Testet
`spec_5_2_9_storage_saved_on_reload_and_close` hävdade till och med att inget sparas medan sidan
körs.

**Efter.** Huvudprocessen frågar. Varje gång den skickar något till en flik kan sidans kod köras
och lägga något i lagringen; en sekund senare (`STORAGE_SAVE_MS`) skickar den `ToTab::SaveStorage`.
Fliken jämför lagringens ändringsräknare (`Runtime::storage_revision`) med den den senast skickade
vid och svarar med `ToShell::StorageSnapshot` bara om något ändrats; svaret sparas som ett vanligt
lagringssvar, med samma prövning av capability och kvot. `vy_suspend` anropas inte (fliken göms
inte, SPEC-5.1.5). En flik som inget skickas till frågas inte, så en vilande sida väcks inte
(SPEC-13.3.2), och headless frågas ingen.

**Det som kunde gå fel.** Ett svar på frågan kan komma efter att processen släppts (en omladdning
mitt emellan). Hade det varit ett vanligt `Storage`-svar hade det tagits för processens sista ord:
sidan som väntar på sin origins lagring hade startat på det, och det riktiga svaret efter
`vy_suspend` hade kastats. Därför är ögonblicksbilden ett eget meddelande: från en släppt process
sparas den men avslutar ingenting. Testet låter sidans `vy_suspend` lägga en egen nyckel och ser
att den finns kvar efter omladdningen.

**Tester.** `spec_9_5_4_storage_revision_counts_changes` (vy-runtime),
`spec_9_5_4_a_save_is_answered_only_with_changes` (flikens slinga),
`spec_9_5_4_storage_is_saved_while_the_page_runs` (skalet: värdet på disk utan omladdning, ingen
fråga till en vilande sida, loppet mot en omladdning, inget headless).

**Kostnad.** Hela lagringen skickas och skrivs om, och bara när den ändrats. Granskaren mätte
skrivningen (debugbygge, ext4 på virtuell disk): 1,1 ms för 1 MB, 15,6 ms för 16 MB, på tråden
som bär alla flikars inmatning. Väntan före nästa fråga växer därför med det som senast sparades:
en sekund, och en sekund till per hel megabyte (`storage_save_delay_ms`). Upp till 1 MB, det en
sida får utan att användaren tillfrågas, är det sekunden; en sida med 16 MB som ändrar något i
varje frame skrivs var sjuttonde sekund, alltså ungefär en megabyte i sekunden oavsett kvot.

**Efter granskning 0017.** Sex fynd, fyra rättade i `5d44c97`:

| Fynd | Åtgärd |
| --- | --- |
| 1. Laddningen satte inget märke: det `vy_init` lagrar i en flik som ligger i bakgrunden frågades aldrig efter (reproducerat) | laddningen räknas som något skickat; testet öppnar en sådan flik och ser värdet på disk utan att fliken fått en tick |
| 2. Sidan styr skrivtakten på huvudtråden | väntan växer med storleken (ovan), och en ögonblicksbild tas emot en gång per fråga, också från en släppt process, så en flikprocess kan inte sätta takten |
| 3. Ingen `fsync`: "beständigt minne" gällde en avbruten process, inte ett strömavbrott | meningen i specen säger nu just det; `fsync` hör hemma på en skrivtråd och är inte gjord |
| 4. En inspelning fick frågan i sig | frågan når fliken men spelas inte in, och en uppspelning får den inte |
| 5. Skickat är inte sparat: misslyckas skrivningen tiger fliken tills något ändras igen | inte gjort; hör också till skrivtråden |
| 6. Tempfiler blev kvar efter en avbruten skrivning | `storage::sweep` tar bort dem som är äldre än en minut när klienten startar |

Två flikar med samma origin har var sin kopia av lagringen och sista skrivning vinner, som förut;
skillnaden är att det nu växlar på disk medan båda körs.

## Granskningarna 0016–0018 och det de ledde till

Tre oberoende granskare gick igenom allt efter granskning 0015: klienten och releasen (0016),
lagringen (0017, ovan) och SDK:t med exempelsidorna (0018). Den första granskaren tystnade två
gånger och stoppades; dess utkast med fem fynd togs över av 0016, som prövade vart och ett.

**0018:1, det allvarligaste: `kalkyl` kunde stoppas för gott av sitt eget blad.** Ett blad med
ett tal och 2 599 formler av det öppnades och fungerade. En vanlig inmatning som gav alla formler
långa värden flyttade varje formels etikett ur de 16 byte den hade, på en hög vars allokator
letar igenom sina hål vid varje frigörning: två frames över budgeten, sidan stoppad, och eftersom
bladet sparades i händelsen stoppad igen vid varje besök. Rättat i `kalkyl`: en etikett får sin
plats innan den skrivs och en formels rymmer vilket värde som helst, tal skrivs på stacken
(`NumText`) i stället för i ett block på högen, tal som är för breda mäts ur samma pott per frame
som texterna, och omräkningens steg är mindre i fönster med fler än 580 celler i bild. Testet
`spec_5_3_3_kalkyl_keeps_the_budget_when_values_grow` föll på den gamla modulen (9,1 M i framen
efter Enter) och går på den nya: dyraste frame 5,4 M i 1600 × 1000 och 6,4 M i 2560 × 1440.
Roten, SDK:ts allokator (`crates/vy-sdk/src/heap.rs`: första passande block i en adressordnad
lista, så varje frigörning kostar som antalet hål nedanför), är inte rörd. Den drabbar varje sida
som låter många små block växa, och är det första att göra i SDK:t.

**Övrigt som rättades:** skyddet runt rasteriseringen (0016:2, ovan), README om macOS och om det
som är öppet (0016:2, 0016:8), grindtabellen i `CLAUDE.md` säger att prestandagrinden bara håller
på en maskin (0016:3), testet för arbetsflödena prövar också `fuzz.yml` (0016:6), `vy-cli
conformance --update` räknar bara de bilder som skrevs (0018:3), medelvärdet i `kalkyl`s statusrad
kan inte slå runt (0018:4), och två noteringar som sade mer än som var mätt är rättade (0013:5,
0015:5).

**Kvar, och ägarens att avgöra:**

- Kompileringscachen är på också på Windows; granskning 0014:4 bad om ägarens beslut och fick
  koordinatorns (`cbe7d6f` säger varför).
- En generalrepetition (`dry_run`) signerar på riktigt, med `VY_KEY` och kodsigneringen när de
  finns, och lämnar resultatet som artefakter (0016:7).
- Röktestet körs efter att releasen publicerats; ett rött röktest lämnar en publicerad release
  (0016:5).
- Kodcachens nyckel bär versionssträngen, inte bygget: två utvecklingsbyggen med samma version
  delar poster. Wasmtime vägrar kod från en annan konfiguration, så följden är en omkompilering
  (0016:4).
- Releasens Linux-bygge är inte pinnat på en containerdigest (0012:6).
- `IME_COMPOSE` är oprövat med en riktig IME (0010:5), och en riktig GPU är okörd (0011:4, 0015:2).

## Releasen v0.3.0 och grindprovet

Fylls i när de är gjorda (se slutet av filen).
