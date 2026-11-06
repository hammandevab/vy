# PR #5 – M4: `vy-dev` och Studio (`main..m4/dev`, topp `47cf8b7`, bedömd på `m4/integration` `ccef8b4`)

**Vad ändrades.** Nytt verktyg `vy-dev` (`new`, `build`, `serve`): en lokal server med två origins på
127.0.0.1, sidan under utveckling (A) och Studio med ett dev-API (B) som bygger sidan och startar
Claude Code i projektkatalogen. Studio är en vanlig ABI 1.0-sida (`modules/studio`, 60 KB) som får
origin och en nyckel per körning inskrivna innan `vy-dev` signerar den. 7 600 rader, 43 filer.

**Spec-id som berörs.** Inga nya; ett icke-normativt stycke i §17. Bygger på 7.1.3, 8.2.3, 9.5.1,
5.2.7–5.2.8. Spec-frågor 53 och 54.

**Risk.** Dev-API:t startar en agent med användarens inloggning och `Bash(cargo *)`. Den som kan
anropa det kör alltså kod som användaren. Skydden är fyra: bindning bara till 127.0.0.1,
`Host`-kontroll mot DNS-rebinding, nej till allt med `Origin`, `Referer` eller `Sec-Fetch-*`, och en
nyckel per körning. Nyckeln står i Studio-modulen och skrivs ut i terminalen.

**Bevis.** Grindarna på HEAD (se 0009) är gröna, och `vy-dev`:s 23 enhetstester och 6 integrationstester
är gröna. Prövat mot en körande `vy-dev serve` (portar 18080/18081, nyckeln utelämnad här):
båda lyssnar på 127.0.0.1 enbart (`/proc/net/tcp`); `/_dev/state` med fel `Host` → 403, med
`Origin` → 403, utan nyckel → 403, med rätt `Host` och nyckel → 200. Fynd, efter allvar:

1. **Medel – nyckeln går att läsa under ett främmande `Host` (bekräftat).** `vy-dev/src/serve.rs`,
   `serve_studio`: bara vägar under `/_dev/` prövas med `host_matches`, allt annat går till
   `Site::reply` oavsett `Host`. Repro: `curl -H 'Host: rebind.example:18081' -H 'Accept:
   application/vy-package+zstd' http://127.0.0.1:18081/index` → 200, hela paketet; manifestet ger
   modulens hash; `GET /_vy/<hash>` med samma `Host` → 200, 60 692 byte, nyckeln står i modulen en
   gång. En webbsida som fått sitt namn att peka på 127.0.0.1 kan alltså läsa nyckeln. Den kan inte
   använda den från webbläsaren, eftersom API:t kräver `Host: localhost:<port>`. Men noteringens
   "ingenstans där en sida kan läsa den" stämmer inte, och skyddet vilar nu på en enda kontroll.
   Rättning: `host_matches` (och nej till webbläsarhuvuden) för *varje* väg på Studios origin; gärna
   `Host`-kontroll på origin A också. Testet: utöka `serve_builds_and_serves_two_origins` med
   paket, manifest och blob under fel `Host`.
2. **Låg – nyckeln skrivs i begärandeloggen som standard.** `serve.rs`, `log_line` skriver
   `info.url` med frågan, och loggen är på om inte `--quiet` ges (`main.rs`, `log: !a.has("quiet")`).
   Studio frågar varje sekund, så terminalen (och en omdirigerad stderr) får
   `GET /_dev/state?token=<32 hex>` om och om igen; sett i provkörningen. Rättning: skriv vägen
   utan frågan, eller ersätt `token=` med `token=…`.
3. **Låg – `vy-dev build` hoppar över 8 ms-kontrollen** (SPEC-13.4.1). `build.rs`,
   `build_project`, anropar `vy_publish::build` men aldrig `vy_publish::check_frames` (0 träffar i
   `crates/vy-dev`). Agenten i Studio får ingen varning om en långsam frame. Rättning: kör
   `check_frames` efter bygget och skriv en varning i byggloggen (inget vägrat paket: det är loopback).

**Nya beroenden.** Inga nya crates (`getrandom` 0.2 fanns i trädet).

**Hotbild.** Inga nya ABI-importer. Nya ytor: dev-API:t (ovan) och agentens processträd. Kvar som
noteringen säger: en annan lokal process, eller en annan användare på samma maskin, kan läsa nyckeln
ur Studio-paketet (spec-fråga 54: ett `Origin`-liknande huvud satt av huvudprocessen vore den
riktiga lösningen). En fjärrsida kan navigera en flik till `vy://localhost:8081/`, men Studio gör
inget utan ett klick, och det sparade uppdraget är användarens eget. Sidan på A når inte B: efter
`1afeae6` släpper skalet plain http bara till sidans egen loopback-origin (se 0007).

**Rekommendation.** **Merge efter** fynd 1 och 2 (två små commits med test). Fynd 3 kan följa.
