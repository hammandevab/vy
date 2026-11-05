# PR #4 – M3 del 2: vy-shell, bench, Windows i CI, indexsida och release (`de40003..acc607e`)

**Vad ändrades.** `vy-shell` (8 800 rader): fönster med winit + softbuffer, en flikprocess per flik
(`vy-shell --tab`) över rör med egen ramning (`ipc.rs`), skalets modell (flikar, adressfält, historik,
grants-förråd, lagring per origin, chrome som display list, kompositering), sidladdning genom vy-proto,
`--record`/`--replay`, `--screenshot`; additiva API:er i runtime/render/manifest/proto (extern klocka,
ChaCha20-slump, extern transport, navigering, capability under körning); `vy-cli bench` och fyra
`shell = true`-fall; Windows i CI. Dessutom vy-serves indexsida, förhandsvisningar och Host-följande
länkar, Dockerfile, `publish_site.sh`, `release.yml`. 12 900 rader, 57 filer.

**Spec-id som berörs.** 2.1.7, 5.2.7–5.2.12, 6.2.1, 7.1.3, 7.2.2, 7.4.3, 8.2.3, 8.3.1, 9.1.1–9.1.2,
9.2.1, 9.3.1–9.3.2, 9.5.1–9.5.2, 9.6.1, 11.1.1, 11.2.1–11.2.2, 11.3.2, 11.4.1, 13.1.1–13.3.2.
Spec-frågor 45–52 i notes/m3-shell.

**Grindar (körda av granskaren, före och efter fixarna).** Modulbygge 11/11, `test_modules` 11/11, fmt,
clippy, `cargo test --workspace` 248 gröna / 19 ignorerade (250 efter fixarna), spec_trace 132/132,
check-abi 11/11, konformans 54/54 à 0,000 % (fyra skalfall), `abi-dump` = `docs/ABI-1.0.md`, release-
bygge av `vy-shell` (26,2 MB, strippad 21,8 MB) med skärmdump av hello 800×600 och shop 1000×700 i 53
respektive 122 ms, bench i release och debug, båda workflow-filerna parsar. **Men CI på `main` är rött
sedan `de40003`:** macOS-jobbet faller i vy-proto med "Peer disconnected" i olika tester per körning (även
efter 5ca7863), och Windows-jobbet – som kört en enda gång – faller i tre vy-cli-tester på CRLF. Därmed
har varken vy-shells tester, skalfallen, klientbygget eller skärmdumpen någonsin körts på macOS eller
Windows i CI; noteringens "CI bygger vy-shell.exe på windows-latest" stämmer inte (fynd 5, 6 – fixade).
**Fuzz** (tre tillfälliga harnesser, ~8 min, raderade): *(a)* ramningen: 6,2 M muterade ramar (bitflipp,
trunkering, insättning, extrema längdfält, taggförväxling mellan `ToTab`/`ToShell`, ren slump): 0
panics, varje accepterat meddelande kodar om byte för byte (kanonisk kodning), en ram som påstår 257 MB
allokerar lat (RSS +0). *(b)* obetrodd flik: meddelanden matade in i `Shell::handle` som från
flikprocessen – gav fynd 2–4. *(c)* adressfältet 69,5 M redigeringar med surrogater, ligaturer, bidi,
kontrolltecken: 0 panics; `Source::parse`/`is_external` på slump: 0 panics; grants-filen 127 400
fientliga filer: 0 panics, **0 pinnar för annan origin** (origin är alltid en parsad `vy://`-adress utan
tabb/radbrytning, så 0003:s fynd 1 går inte att upprepa här); lagringsfil och Markdown muterade: 0
panics. *(d)* uppspelningsfiler: 19 500 slump-/muterade `--replay`-filer genom `TabLoop`: 0 panics,
varje fel ett rent `Stopped`.

**Risk.** Klienten är nu en produkt som öppnar främmande innehåll, och två saker gjorde att en sida
ägde mer än sin yta: en process på en Windows-maskin (fynd 1) och huvudprocessens nätverk (fynd 2).
Båda nåddes utan capability och utan gest. Fixade på `main`. Det som återstår är tillgänglighet och
klickkapning (fynd 3, 4).

**Bevis.** Un-ignorerade: 2.1.7, 8.3.1, 9.1.1, 9.2.1, 9.6.1, 11.1.1, 11.2.1, 11.4.1, 13.3.1; backlogg
28 → 19. Testerna är verkliga: två pid skilda från skalets, dödad process ger "gone" medan grannfliken
ritar vidare, `--replay` ger samma display lists byte för byte för spelet, en modul som målar ±10 000
färgar bara sidytan (blit med klipp till sidytan, `compose.rs`), dold flik får 0 frames och 0 CPU-ticks.
Två svagheter: `spec_11_2_1_ring_buffer_and_crash_containment` bevisar rör, inte delat minne (spec-fråga
46), och `--record`/`--replay` nås bara från tester – skalet skickar aldrig flaggorna (SPEC-11.4.1 är
mekanism, inte devtools). Guldbilder: fyra nya skalfall, inga byten; verifierade bara på Linux (CI
rött). **Fixat i egna commits, varje med test som faller utan fixen:** fynd 2 `ac93554` + `91e8fe6`,
fynd 1 `2d1292f` (typkontrollerad för Windows, inte körd), fynd 5 `1e3a969`, fynd 6 `a2f86da`.

**Fynd.**
1. *Kommandoinjektion på Windows* (**fixat**). `cmd /C start "" <url>`: std citerar bara argument med
   blanksteg, så adressen nådde cmd.exe ociterad, där `&` är kommandoseparator och `%NAMN%` miljövariabel –
   båda släpps igenom av `is_external`. `navigate("https://x/?a&calc")` från vilken sida som helst, även i
   begränsat läge, utan gest, körde ett kommando. Nu `ShellExecuteW` via windows-sys (redan i trädet).
2. *Huvudprocessen tog flikens ord* (**fixat**). `FetchRequest` gick till laddaren utan kontroll av
   `net`-grant eller manifestets origins (sond: flik utan `net` → laddaren svarade) – SPEC-5.2.8 och
   2.1.4 hölls bara i den obetrodda processen; `Clipboard` spreds utan grant, `Storage` skrevs utan grant
   och kvot, `Navigate` tog filsökvägar, och avkodaren saknade SPEC-9.3.2:s tak: 2 M noder behölls
   (+219 MB), 1 M länkar gjorde varje pekarrörelse 85 ms. Nu avgör huvudprocessen, och taken gäller i
   `ToShell::decode`.
3. *Extern öppning utan gest och utan tak* (rapporteras). 1 000 `navigate(https://…)` gav 1 000
   webbläsarfönster; från en dold flik också. Kräv en användargest inom 1 s (som SPEC-5.2.12) och högst
   en öppning per gest; samma för `vy://`-navigering från dolda flikar om ägaren vill.
4. *Klickkapning av capability-frågan* (rapporteras). Dialogen ritas ovanför sidytan och skjuter ner den
   40 px; "Tillåt" tar klicket direkt. En sida som navigerar om sig på pekarrörelse får frågan att dyka
   upp under användarens nästa klick (cacheträff 36–56 ms). Ignorera klick i dialogen de första ~500 ms
   eller lägg den över sidan utan att flytta den.
5. *Windows-jobbet föll på CRLF* (**fixat**, `.gitattributes`). 6. *macOS-jobbets slumpfel* (**fixat**):
   accepterad socket ärver `O_NONBLOCK` på BSD, inte på Linux; nu uttryckligen blockerande.
7. *Laddartråden är seriell och obegränsad:* en sidas begäran med 30 s timeout blockerar all navigering
   i alla flikar; antalet köade begäranden från en flik begränsas inte (SPEC-9.3.1: 64) – 64 per flik i
   huvudprocessen och en egen tråd (eller pool) för sidornas egna hämtningar. Inkorgen är en obegränsad
   kanal; med taken i fynd 2 är tillväxten bunden av rörets hastighet, inte av skalet – `sync_channel`
   per flik ger mottryck som bara drabbar den fliken.
8. *Processisolering är krasch-, inte privilegieisolering:* flikprocessen är samma binär utan jobbobjekt,
   AppContainer eller seccomp; wasmtime är enda barriären (M4:s externa sandbox-granskning). Rören är
   enda kanalen; barnet dör av sig självt när stdin stängs.
9. *§13:* missade rader i release här: 13.1.4 (P95 61–72 ms för 22–27 KB-moduler, ingen kompileringscache
   – wasmtimes `Module::serialize` i blobcachen per hash ger "0 ms vid träff"), 13.2.1 (16,8 / 15,8 /
   29,8 ms mot 2 ms – snabbväg för opaka rektanglar utan mask kräver guldbyte, spec-fråga 41, och en
   formad-text-cache per (stil, bredd, text)), 13.2.3 (omätt; texten formas om varje frame, så den missas).
   Hållna: 13.1.2 (36–56 ms), 13.2.2 (P95 11,5 ms, på gränsen), 13.3.1 (225 MB), 13.3.2. Omätta: 13.1.1,
   13.1.3. 10. *Storlek:* 21,8 MB strippad, men release-profilen strippar inte – `release.yml` zippar
   26,2 MB mot SPEC-12.3.1:s 25 MB; `.text` 14,5 MB varav cranelift + wasmtime ≈ 7 MB, typsnitt 2,2 MB,
   `pulley_interpreter` 0,2 MB med (princip 6; tolken används inte). `strip = true` räcker för grinden.
11. *Smått:* grants.txt skrivs icke-atomärt (torn fil = standardpolicy, ofarligt); `Gone` väntar 200 ms på
    UI-tråden; `X-Forwarded-Host` litas på utan proxy (bara den egna begärans länkar, `max-age=60`);
    actions pinnade på tagg, inte SHA (SPEC-9.7.1); `ARG VY_KEY` ligger i byggstadiets lager (dokumenterat).

**Nya beroenden.** `winit 0.30` (`rwh_06`, `x11`, `wayland`, `wayland-dlopen`; ingen CSD), `softbuffer
0.4` (ingen `kms`), `getrandom` (fanns) – motiverade i notes/m3-shell; `windows-sys 0.61` som direkt
beroende för fynd 1 med samma funktioner softbuffer redan drar. vy-shell: 204 crates Linux, 172 Windows,
187 macOS; vy-cli 161; 18 dubbletter (nya: rustix, linux-raw-sys, smol_str, miniz_oxide ×2 via winit).

**Hotbild.** Inga nya ABI-importer. Nya ytor: flikprocessen som motpart (fynd 2), systemets öppnare
(fynd 1), filer i datakatalogen (grants, lagring – fuzzade), uppspelningsfiler (fuzzade). SPEC-9.6.1
håller: chromen rasteriseras för sig, sidan blittas med klipp till sidytan, markören "begränsat läge"
kan inte påverkas av sidan. Grants är per exakt origin (schema + värd + port) och origin är adressens,
så en omdirigering byter varken nyckel eller grants.

**Rekommendation.** **Merge efter** fynd 3 och 4 (gest och fördröjning – två små commits) och en grön
CI-körning på macOS och Windows där klienten byggs. Fynd 1, 2, 5 och 6 är redan på `main`.

**Testomgång 1: klar att ge till en människa på Windows: nej** – (i) CI har aldrig byggt eller testat
`vy-shell.exe`; en grön Windows-körning med "Build the client" och "Shell screenshot" krävs först;
(ii) fynd 1 är fixat men bara typkontrollerat – en utvecklare klickar en https-länk på Windows först
och ser att webbläsaren öppnas utan konsolfönster; (iii) fynd 3 och 4 stängs (annars kan en sida spamma
webbläsaren och lura fram ett "Tillåt"); (iv) `strip = true` så att zippen håller 25 MB; (v) säg till
testaren att bara besöka den egna vy-serve-sajten: flikprocessen har ingen OS-sandbox (fynd 8).

## Uppföljning

Byggagent, efter granskningen. Rekommendationens villkor fynd 3 och 4 samt testomgångens (iii) och
(iv) är åtgärdade på `main`; varje fix har ett test som faller utan den. Guldbilder: de fyra
skalfallen oförändrade (0,000 %), ett nytt skalfall med frågan. Grindar efter fixarna: modulbygge
11/11, `test_modules` 11/11, fmt, clippy, `cargo test --workspace` 252 gröna / 19 ignorerade,
spec_trace 132/132, konformans 55/55 à 0,000 %, `abi-dump` = `docs/ABI-1.0.md`, release-bygge av
`vy-shell` på Linux 21,8 MB strippad (var 26,2 MB).

| Fynd | Status | Commit |
| --- | --- | --- |
| 3. Extern öppning utan gest och utan tak | **Stängt.** En sidas `navigate("https://…")` och en registrerad https-länk öppnar webbläsaren bara på en användargest i den fliken (pekarsläpp eller tangentnedtryckning levererad till sidan) inom 1 000 ms av skalets klocka, och gesten förbrukas: en öppning per gest; annars släpps begäran och loggas på info-nivå med adressen. Dold flik får inga händelser, alltså ingen gest. Adressfältet kräver ingen gest. Test `spec_2_1_7_page_opens_externally_once_per_gesture`: 1 000 anrop efter ett klick → en öppning; utan gest, efter en sekund, från dold flik → ingen. | `bfdbf7b` |
| 4. Klickkapning av capability-frågan | **Stängt.** Frågan är en egen display list som komponeras över sidytans överkant efter sidan: sidytan och chromens höjd är desamma med och utan fråga, pekaren över frågan är skalets. "Tillåt" räknas först när frågan varit uppe i 700 ms, per flik och per fråga (nästa fråga i kön och flikbyte armar om); "Neka" räknas alltid. Test `spec_8_2_3_capability_question_overlays_and_arms_late`; chromens layouttest. Sviten: skalfall kan ha `address` (signerad, betrodd sida) och `shell-chat-800x600@1-ask` visar frågan över chat-sidan. | `efa7975`, `5c3adca`, `4d929b4` |
| Testomgång (iv): `strip = true` | **Stängt.** Release-profilen strippar; `vy-shell` 21,8 MB på Linux, under SPEC-12.3.1:s 25 MB. | `471cf62` |
| Testomgång (v) och checklista | **Stängt.** `docs/notes/m3-shell.md`, "Testomgång 1 – manuella steg på Windows": sju steg för en icke-utvecklare, med varningen att bara besöka den egna sajten och vad som ska rapporteras hur. | (detta commit) |
| 1, 2, 5, 6 | Stängda före granskningen (se ovan). | – |
| 7–11 | Öppna. Vid checklistan observerat: en fliks `Storage`-svar vid navigering/F5 (gammal generation) och vid Ctrl+W (fliken borta) släpps i `Shell::handle`; lagring sparas säkert bara vid dold flik och fönsterstängning (noterat i notes/m3-shell, "Stubbat och ogjort"). | – |

Kvar för testomgång 1: (i) en grön CI-körning på Windows som bygger klienten och (ii) en utvecklares
klick på en https-länk på Windows.
