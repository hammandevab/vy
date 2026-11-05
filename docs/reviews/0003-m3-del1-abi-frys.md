# PR #3 – ABI-frysning och M3 del 1: manifest, publish, proto, serve (`6cae528..a6d7ea4`)

**Vad ändrades.** ABI-kandidaten v0.3: 17 importer returnerar `i32`, `dl_text -> i64`, `dl_clip_push`
med radie, `pointer_get` struken (32 importer), `POINTER_LEAVE`, värden äger fokus, bytetak, lagring i
minnet och inspelad hämtning headless, SDK-städning (beslut 23–28) och scriptade värdhändelser i
runnern. M3 del 1: `vy-manifest` (kanonisk CBOR, validering, Ed25519, tar+zstd-paket), `vy-publish`,
`vy-proto` (adresser, hämtning, hashcache, TOFU-nycklar) och `vy-serve`. 15 500 rader, 44 nya crates.

**Spec-id som berörs.** 4.1.*, 4.4.1, 5.1.*, 5.2.1–5.2.13, 5.3.5, 6.1.4, 7.1.1–7.5.1, 8.1.1–8.2.3,
9.3.2, 9.5.2, 10.3.1–10.3.3, A.1.1–A.5.1, C.1.1. Spec-frågor 17–38 i BUILD-NOTES och notes/m3.

**Grindar (körda av granskaren, före och efter fixarna).** Modulbygge 11/11, `test_modules` 11/11,
fmt, clippy, `cargo test --workspace` 184 gröna / 28 ignorerade (189 efter fixarna), spec_trace
131/131, check-abi 11/11, konformans 50/50 à 0,000 % (26 träd lika), `abi-dump` = `docs/ABI-1.0.md`
utan diff. **Fuzz** (fyra tillfälliga harnesser, ~8 min, raderade): *(a)* `vy_manifest` 1 152 322
inmatningar – slump, bitflipp/trunkering/insättning i elva signerade paket, muterad tar omkomprimerad:
0 panics, varje accepterad avkodning återkodar byte för byte; zstd-bomb 1 GiB → 32 KB avvisad på 73 ms
med +7 MB RSS; tar med `../`, absoluta namn, 2 och 8 GiB-storlekar avvisade. *(b)* `vy_proto` 14,1 M
adress-/URL-strängar, 32 handgjorda fientliga svar (trasiga statusrader, `Content-Length` −5 och 10¹⁴,
chunked-skräp, 5 000 headers, 200 KB header, loop-, `data:`-, `ftp:`- och `http:`-omdirigeringar),
60 MB-strömmar och 1 128 525 slumphämtningar: 0 panics, 0 hängningar – men **två fynd** (1, 2).
*(c)* `vy_serve` 45,4 M förfrågningar i process: 0 panics, ingen fil utanför katalogen (`..`, `%2e%2e`,
`%00`, `\`, `..%2f`), ingen oescapad reflektion i HTML, 1 MB-sökväg 7 ms. *(d)* runtime 614 körningar
à upp till 80 slumpade värdhändelser över alla elva moduler (24 905 händelser, 1 775 frames, 1 290
reinit; 0×0, 16384×16384 och 2³²−1×1, skala 0,01–8, 70 KB `TEXT_INPUT`, 70 MB inspelat svar): 0
värdpanics, 0 modultrappar, 159 rena `Hidden`-avslag. Sonder gav fynd 3 och 4.

**Risk.** Signaturen är hela förtroendemodellen (SPEC-7.4.1): den visade sig korrekt avgränsad, men
nyckelförrådet runt den gick att skriva i från ett betrott manifest (fynd 1). Klienten tog dessutom emot
obegränsat många byte på ett manifests ord (fynd 2). Båda nås av varje utgivare man besökt. I M3 ligger
rasteriseraren i huvudprocessen: en laglig display list kan låsa hela klienten i minuter (fynd 4).

**Bevis.** Un-ignorerade: 4.1.1–4.1.3, 4.3.1–4.3.2, 4.4.1, 4.5.1, 5.2.8, 7.1.3, 7.2.1–7.2.2,
7.3.1–7.3.2, 7.4.1–7.4.4 (well-known-delen), 7.5.1, 8.1.1–8.1.3, 8.2.2–8.2.3; backlogg 48 → 28.
Testerna är verkliga: proto mot en loopback-server, serve ren funktion av anropet, end-to-end
publish → serve → fetch → runtime. Guldbyten: 10 (pixelsnappning), 8 (rundade klipp), 5 (fokus),
2 + 2 (lagring), 1 (`POINTER_LEAVE`), 5 nya fall – alla motiverade i BUILD-NOTES. **Fixat i egna
commits, varje med test som faller utan fixen:** fynd 1 `7dacbc0`, fynd 2 `d30518c`, fynd 3
`ef7a481`, fynd 5 `ac730f1`, fynd 8 `b336230`. Signaturomfånget (3a) är rätt: `signed_bytes` är
hela manifestet utom `sig`, avkodaren kräver kanonisk ordning, kortaste form, inga dubbletter och
inga okända fält, `verify` kräver att `key_id` är nyckelns härledda id och använder `verify_strict`;
omordning, smugglade fält och fel nyckel faller slutet. Cachen (3b) verifierar hash vid varje
läsning, raderar fel fil, har LRU-tak och atomär skrivning; förtroende knyts till adressens origin,
aldrig till URL eller cache. Begränsat läge (3c): `Fetched::grants()` ger `Denied` för allt när
signaturen inte stämmer, även `ime`.

**Fynd.**
1. *Förfalskade pinnar via `next_key_id`* (**fixat**). Nyckelförrådet är radorienterad text och
   `next_key_id` skrevs dit oförändrat: `"x\npin vy://bank.example <nyckel>"` från ett betrott manifest
   gav bank.example en angriparnyckel vid nästa start – bank.examples riktiga sidor blir obetrodda
   (SPEC-9.5.2). Nu kräver `validate` 16 hex-tecken och `KeyStore` vägrar allt annat.
2. *Obegränsad hämtning på manifestets ord* (**fixat**). I manifestformen var `module.size`/`size`
   kroppstak innan något validerats: `size = 1 TB` gav 300 MB i minnet innan hashfelet; `u64::MAX` gav
   `limit + 1`-overflow. Nu prövas SPEC-4.1.4/4.1.5 före första bloben och varje blob tas under 32 MB.
3. *`sem_node` var O(n²)* (**fixat**). Dubblettkontrollen sökte hela trädet per anrop: 65 536 lagliga
   noder = 3,1 s värd-CPU (10 s debug) för 1,2 ms bränsle (SPEC-5.3.3). Nu en mängd per frame.
4. *Rasteriseringen har ingen budget* (rapporteras). 65 000 helskärmsrektanglar: 16 s; 32 nästlade
   rundade klipp + 65 000 rektanglar: **312 s** för en frame à 1280×800 (`raster.rs:158` bygger masken i
   full storlek per push/pop, 0001 fynd 6 kvar; fyllnadsyta debiteras inte). Behöver ett arbetstak
   (pixelyta × djup) som avbryter listen, eller debitering i bränsle, före `vy-shell`.
5. *Localhost-undantaget gällde varje hopp och blob-URL* (**fixat**). En fjärrsida kunde omdirigera
   eller peka `module.url` mot `http://localhost:<port>/…` (SPEC-7.1.3). Nu bara när sidans egen
   adress är loopback.
6. *`HttpCache` saknar tak*: varje URL:s svar (paket upp till 41 MB) sparas för alltid, dubbelt mot
   blobcachen (`cache.rs:254`). *`Fetched.package.manifest.capabilities` är publik*, så ett skal kan
   bevilja förbi `trusted`; `grants()` är konvention, inte typ.
7. *Manifestet avviker från SPEC-8.1.2*: `resources` är `{hash, size, mime, url?, w?, h?}`, inte
   `{path, type, hash, size}`; `next_key_id` finns inte i specen (spec-fråga 28). Kodningen är pinnad av
   test, så specen måste följa efter före M3:s slut. `accepts` finns i två kopior (proto, serve).
8. *Smått*: `Accept …;q=0` valde typen (**fixat**); `write_atomic` delar `.tmp`-namn mellan processer
   (en per flik i M3, självläkande); `wasmparser 0.220` nu i två crates (0001 fynd 4); zstd-fönstret kan
   begränsas med `window_log_max` (128 MB möjligt idag); `conformance` + `--size` tillåter 16384² = 1 GB.

**Nya beroenden.** `minicbor 2`, `serde_json`, `ed25519-dalek 3`, `zstd 0.14` (C), `ureq 3`
(rustls + ring, C/asm), `tiny_http 0.12`, `getrandom 0.2` – alla motiverade och låsta i notes/m3; +44
unika crates (169 → 213); `vy-cli` 164 → 182 eftersom `vy-manifest` drar zstd-sys, dalek och serde_json
in i klienten. Inga nya dubbletter; kvar: wasmparser 0.220/0.261, sha2/digest 0.10/0.11, syn 2/3,
hashbrown ×3. Inga kända RUSTSEC på versionerna (ring 0.17.14 ≥ 0.17.12, tiny_http ≥ 0.8, dalek 3/5);
ingen `cargo audit` i miljön. Standardfunktioner avstängda där det går.

**Hotbild.** Inga nya ABI-importer (33 → 32). Nya ytor: nätverksindata, cache och nyckelförråd på disk,
server. Fynd 1 och 5 var verkliga läckor över origin-gränsen i förtroende respektive nätverksåtkomst;
båda stängda. Fynd 4 är tillgänglighet i huvudprocessen och öppet.

**Rekommendation.** **Merge** (fynd 1–3, 5 och 8 är redan på `main` i den här granskningen) **efter**
att fynd 4 fått en post före `vy-shell` och fynd 7 en `spec:`-commit; 6 kan gå som M3-poster.

**ABI-frysning: inte klar – signaturerna är klara, sju värden saknas.** `docs/ABI-1.0.md` stämmer med
SPEC §5 och bilaga A/C signatur för signatur (38/38, alla koder, kinds, opkoder och bytestorlekar lika;
enda skillnaderna är tankstreck mot bindestreck i tabellerna och att 0x07/0x08 är en mening i stället för
rader). Men följande *tal* som moduler redan kompilerar in står varken i SPEC eller i förteckningen, och
kan inte ändras efter SPEC-5.3.5: (i) `fetch_begin.method` – SDK:t säger 0 GET, 1 POST, 2 PUT,
3 DELETE, värden ignorerar argumentet; (ii) `log.level` 0–3 (bara SDK); (iii) knappnummer i
`POINTER_DOWN/UP.c` och bitmasken i `POINTER_MOVE.c`; (iv) `text_style.weight` 1–1000 (värden ger
`E_ARG`, bara i kod); (v) typsnittsrollerna −1…−5 ur SPEC-6.4.2 saknas i `vy-abi` och förteckningen;
(vi) 4 096 länkar per frame (SPEC-9.3.2) saknas i `vy_abi::limits` och Gränser-tabellen; (vii) Tab
0x2B/Enter 0x28/Escape 0x29 står i förteckningen men inte i SPEC-10.3.1. Returvärdesemantik utan text:
`clipboard_read` (hela längden som `state_get`?), `fetch_poll` (spec-fråga 22), `vy_event` ≠ 1. Allt
är en `spec:`-commit plus konstanter i `vy-abi` och ett nytt `abi-dump`; sedan kan versionsraden sättas.
