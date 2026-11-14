# PR #17 – lagringen sparas medan sidan körs (`m5/gate-fixes`, bedömd på `b7fb243`)

**Vad ändrades.** Huvudprocessen frågar en flikprocess efter sidans lagring en sekund efter att den
skickat den något (`ToTab::SaveStorage`, tagg 16), och fliken svarar med `ToShell::StorageSnapshot`
(tagg 80) bara om `Runtime::storage_revision` rört sig sedan lagringen senast skickades; svaret
skrivs som ett vanligt lagringssvar. Commits `1501fc3` (`spec:`), `da1544b`, `b16edbf`.

**Spec-id som berörs.** Ny: SPEC-9.5.4. Vilar på SPEC-5.2.9, 5.1.5, 8.2.3, 9.3.1, 9.5.2, 11.4.1,
13.3.2.

**Risk.** Tas en ögonblicksbild för en släppt process sista ord startar nästa sida på gammal
lagring, och sparas den efter "Rensa sidans lagring" kommer det rensade tillbaka. Skrivningen sker
nu i sidans takt, inte användarens, på tråden som bär alla flikars inmatning.

**Bevis.** Läst ur koden; det jag provat står som prov. Proven kördes i skalet med flikar på tråd
(`exe: None`), inte i fönstret, och filerna är återställda.
*Gammalt över nytt (fråga 1):* jag fann ingen väg. Ordningen från en process är rörets. Ramar från
en äldre generation går till `retired_message` (`shell.rs:1274`), där bilden sparas under den
släppta processens egen origin, beviljande och kvot utan att posten tas bort (`:1928`); sidan som
väntar på originen startar först på `Storage` eller processens slut. "Rensa" laddar om originens
alla flikar och märker posterna `discard` (`:3553`), så en sen bild sparas inte. Stängd flik,
stängt fönster, död process och byte av origin och tillbaka går samma väg. *Prov:* med `:1929`
ändrad till `self.retired.remove(at)` faller `spec_9_5_4_storage_is_saved_while_the_page_runs` på
rätt rad (`[k]` i stället för `[k, s]`): testet bevisar det det säger. Två flikar med samma origin
har var sin kopia och sista skrivning vinner, som förut; nu växlar det på disk medan båda körs.
*Komprometterad flik (2):* inget nytt i sak. Origin, beviljande och kvot tas ur huvudprocessens
flik eller post, aldrig ur ramen (`save_storage`, `:1873`); en process utan startad sida sparar
inget (`:1700`, `:4197`); `Storage` togs redan emot obedd. Nytt: en släppt process kan skriva flera
gånger under sina 500 ms i stället för en. *Väckning (3):* headless, uppspelning, lokal fil, sida
utan `storage` och ostartad sida frågas inte. `save_at` sätts bara i `ask_for_storage`, och varje
poll nollar det eller lämnar det i framtiden, så `poll_in` fastnar inte på noll. En gömd flik får
en fråga en sekund efter gömningen och svarar inget: en väckning av flikslingan, ingen sidkod.
*IPC och ABI (5):* taggarna är unika (1–16, 64–80), bilden avkodas med `get_entries` som `Storage`,
`git diff 0dc4225..HEAD -- crates/vy-abi docs/ABI-1.0.md` är tom och enda ändringen i `SPEC.md` är
raden i §9. *Gröna hos mig:* de sju `spec_9_5_4`/`spec_5_2_9` i vy-shell, `ipc::` (4),
vy-runtimes räknartest, `spec_trace.py` (143 av 143). Fynd, efter allvar:

1. **Medel – laddningen sätter inget märke: det `vy_init` lagrar frågas inte efter i en flik som
   ligger i bakgrunden.** `shell.rs:1530` och `:1536` skickar `CodeCache` och `Load` med
   `conn.send`, inte `Tab::send` (`:397`), så `touched` sätts inte; `b16edbf` (`:3775`) behåller
   ett märke som laddningen aldrig satte. I den aktiva fliken räddar första ticken det. En flik som
   inte är aktiv när laddningen besvaras får ingen tick: fönstret startat med två adresser
   (`window.rs:374`, alla flikar utom den sista), och live reload (`shell.rs:3808`) eller "Rensa"
   (`:3551`) av en bakgrundsflik när inget gammalt märke råkar stå kvar. Scenario: sidan räknar ett
   besök i `vy_init`, klienten avbryts innan fliken visats, besöket är borta fast SPEC-9.5.4 lovar
   högst en sekund. **Reproducerat** med ett tillfälligt test: tre sekunder i bakgrunden, inget på
   disk och `poll_in() == None`; sparat en sekund efter att fliken visats. Ingen regression: förut
   sparades inget medan sidan körs. Rättning: `tab.touched.set(true)` där laddningen skickas i
   `start_page`. Provad: på disk efter 1,04 s, inget förfallet efteråt, de sju testerna gröna.
   Testet som saknas är det provet; `b16edbf` har inget eget (de sju är gröna utan den).
2. **Medel – kostnaden styrs av sidan och är bara delvis mätt.** `shell.rs:1706` anropar
   `storage::save` (`storage.rs:71`) på huvudtråden; `tab.rs:532` kopierar hela lagringen i
   flikslingan. En sida med beviljade 16 MB som ändrar en byte per frame, eller per nätsvar i en
   gömd flik, får 16 MB omskrivna varje sekund: 1,4 TB per dygn, och alla flikars inmatning står
   still under skrivningen. **Mätt:** `storage::save` med 16 MB tar 15,6 ms i median (högst 24),
   med 1 MB 1,1 ms (debugbygge, ext4 på virtuell disk): två gånger en frames 8 ms, en gång per
   sekund. Utan användarens ja är kvoten högst 1 MB (SPEC-8.2.3): 1 ms och 86 GB per dygn.
   **Inte mätt:** röret och flikens del, releasebygge, långsam disk, macOS, Windows. Rättning:
   skrivningen på egen tråd, senaste per origin vinner, och ett intervall som växer med storleken;
   det senare ändrar meningen i specen.
3. **Låg – "beständigt minne" gäller en avbruten process, inte ett strömavbrott.** `storage.rs:82`
   skriver en tempfil och byter namn utan `sync_all`. Dödas klienten ligger bytena i systemets
   cache och meningen håller. Vid strömavbrott kan mer än sekunden gå förlorad, och på filsystem
   som inte tvingar ut data vid namnbytet kan filen bli avhuggen; `storage::load` läser då en tom
   lagring. Det fönstret stod förut öppet strax efter gömning och stängning, nu så länge sidan
   lagrar. Härlett, inte reproducerat. Mätt: `fsync` lägger till 12 ms vid 16 MB, 1 ms vid 1 MB.
   Rättning: `sync_all` före `rename` på tråden i fynd 2, eller "klientens process" i SPEC-9.5.4;
   noteringen nämner strömavbrott.
4. **Låg – en inspelning får frågan i sig.** `ask_for_storage` (`shell.rs:3769`) undantar
   uppspelning men inte inspelning, och `write_from_shell` (`process.rs:390`) siktar bara bort tagg
   15. Uppspelad svarar fliken, huvudprocessen vägrar spara (uppspelningen har varken origin eller
   beviljande) och loggar en rad; bilderna blir desamma, för frågan kör ingen sidkod. En klient
   från före ändringen stannar på den okända taggen. Härlett; inget test spelar in i fönster.
   Rättning: sikta bort tagg 16 ur inspelningen.
5. **Låg – skickat är inte sparat.** `tab.rs:535` noterar lagringen som skickad. Kan huvudprocessen
   inte skriva (full disk; `shell.rs:1706` loggar) tiger fliken vid nästa fråga tills sidan ändrar
   något eller fliken göms. Härlett. Rättning: huvudprocessen behåller bilden och försöker igen.
6. **Låg – tempfiler blir kvar.** `storage.rs:81`: en klient som dödas mitt i en skrivning, nu upp
   till en gång per sekund, lämnar `<hash>.tmp<pid>` som inget tar bort. Härlett. Rättning: städa
   `storage/*.tmp*` vid start.

*SPEC-9.5.4 som skriven (4)* stämmer för händelser, text, tickar, nätsvar, storlek, synlighet,
beviljanden och urklipp (allt går genom `Tab::send`), inte för laddningen (fynd 1) och inte för
"beständigt" vid strömavbrott (fynd 3). "Högst en sekund" är väntan före frågan; förlusten är den
plus svaret och skrivningen. *Inte prövat:* fönsterklienten och flikar som egna processer, röret
med 16 MB, macOS och Windows, strömavbrott, inspelning i fönster, hela grindarna (koordinatorns).

**Nya beroenden.** Inga.

**Hotbild.** Två nya IPC-ramar, ingen ny ABI-import, fryst ABI orörd. Flikprocessen får ingen ny
befogenhet; nytt är att en sida kan få huvudprocessen att skriva varje sekund (fynd 2).

**Rekommendation.** **Merge efter** fynd 1: gjort i `5d44c97`. U1 och U2 är lösta i `58cf262`.
Före releasen: rätta V1 i andra uppföljningen nedan (testet är ostadigt, tre rader i testet).
V2, fynd 2 (egen skrivtråd), 3 och 5 står kvar som uppföljningar.

## Uppföljning efter rättningarna (`5d44c97`, bedömd på `6e09d5b`)

Läst `git show 5d44c97` och provat som förut: flikar på tråd, tillfälliga ändringar återställda.
De 17 riktade testerna (`spec_9_5_4`, `spec_5_2_9`, `spec_11_4_1`, `ipc::`) är gröna efter
`cargo build -p vy-shell`. "Faller utan" betyder att jag tog bort raden, körde testet och
återställde filen.

- **Fynd 1: löst.** `shell.rs:1569` sätter märket vid laddningen. Testet faller utan raden, vid
  sin tidsgräns: "what vy_init stored in a background tab is never saved".
- **Fynd 2: lindrat.** Väntan växer med storleken (`:74`, `:3841`). Prov: en sida med 1 MB är på
  disk efter 1,01 s första gången och 2,01 s de två följande. Det ger högst omkring 1 MB per
  sekund oavsett kvot (16 MB var sjuttonde sekund). Skrivningen på huvudtråden är kvar, 15 ms vid
  16 MB. Se U1–U3.
- **Fynd 3: meningen ändrad, koden densamma.** Meningen säger nu lagringsfil, avbruten process
  och att ett strömavbrott kan ta mer: sann på den punkten.
- **Fynd 4: löst.** `process.rs:397` skriver frågan till fliken men inte till inspelningen, och
  `:446` hoppar över den vid uppspelning. `spec_11_4_1_record_replay` faller utan `:397` ("the
  ask is not recorded"). `:446` prövade jag inte.
- **Fynd 5: öppet**, som commit-meddelandet säger.
- **Fynd 6: löst.** `storage::sweep` (`storage.rs:90`) tar bara namn i `storage/` vars ändelse
  börjar på `tmp` och som är minst en minut gamla; en länk följs inte, en katalog tas inte, en
  tid i framtiden sparar filen. Fel fil kan den ta bara från en annan klient som stått stilla
  mer än en minut mitt i en skrivning: den skrivningen går då förlorad och loggas.

Nytt, efter allvar:

- **U1. Medel – av två obesvarade frågor sparas bara det första svaret. Reproducerat.**
  `snapshot_asked` är en flagga (`shell.rs:3845` sätter, `:1906` tar). En flik som ligger mer än
  väntetiden efter skalet, med en kö av händelser att arbeta av, får fråga två innan den svarat på
  fråga ett. Den svarar ärligt på båda; det andra svaret kastas med loggraden "none was asked
  for", och fliken har noterat det som skickat (`tab.rs:535`), så inget mer kommer förrän sidan
  ändrar något igen eller fliken göms. Prov: tangent 4, fråga, tangent 5, fråga, och svaren lästa
  först därefter (det som står i stället för en flik som släpar): på disk `k=4`, tre sekunder
  senare fortfarande `k=4` och `poll_in() == None`, fast sidan har `k=5`. Då är varken "svaret
  skrivs när det kommer" eller "högst den tidens ändringar" sant. Rättning: räkna öppna frågor
  med ett tak (till exempel fyra) i stället för flaggan, och bär talet i `Retired`; eller låt
  fliken svara på varje fråga, också med "oförändrat", så att skalet aldrig har två öppna. Provet
  är testet som saknas. Före `5d44c97` sparades varje bild, så detta är nytt med rättningen.
- **U2. Låg – "ett svar som ingen begärt sparas inte" gäller bara ögonblicksbilden.
  Reproducerat.** `shell.rs:1728` ger `Storage` till `storage_answer(.., false)`, som inte prövar
  något: tre obedda `Storage` från en levande flik gav tre skrivningar. En komprometterad
  flikprocess skickar alltså `Storage` i stället och sätter takten som förut. Det nya är att en
  normativ mening och commit-meddelandet säger motsatsen. Rättning: ta emot `Storage` från en
  levande flik bara efter en skickad `Visibility { visible: false }`, eller skriv "en
  ögonblicksbild som ingen begärt" i meningen.
- **U3. Låg – tre saker som inget test märker.** Testerna är gröna med `:3841` tillbaka på
  `STORAGE_SAVE_MS` (bara funktionen prövas, inte att väntan används; mitt prov ovan är ett
  sådant test), med `snapshot_asked: false` i `retire_process` (`:4258`) och utan anropet av
  `sweep` vid start (`:610`).

*Frågorna i uppdraget.* Flaggan nollas i `start_page` och flyttas i `retire_process` rätt: fråga,
omladdning och ny fråga till den nya processen går igenom, också när processen dött eller sidan
saknar origin. Förlusten sitter i U1, inte där. `saved_bytes` kan inte ge mer än 17 sekunders
väntan (bilden är högst 16 MB och räknas högst till kvoten) och nollas vid varje laddning; ingen
flik blir ofrågad. Meningen räknar "senast sparade" som senaste ögonblicksbild, inte det som
sparades när fliken gömdes. *SPEC-9.5.4 som skriven* är nu sann utom i U1, U2 och fynd 5.
*Inte prövat:* hela grindarna, flikar som egna processer i mina prov, en sida som verkligen
släpar (U1 är provat genom att inte läsa inkorgen emellan), macOS och Windows.

## Andra uppföljningen (`58cf262`, bedömd på `9940b7c`)

Läst `git show 58cf262` och provat som förut; tillfälliga ändringar återställda. De 17 riktade
testerna var gröna i en körning efter `cargo build -p vy-shell`, men se V1.

- **U1: löst.** Fliken besvarar varje fråga (`tab.rs:233`, `StorageUnchanged`, tagg 81) och skalet
  håller en öppen i taget (`shell.rs:3861`). En ärlig flik kan då inte få ett svar kastat som
  obegärt. Med villkoret i `:3861` borttaget faller testet: "the later change was never saved".
- **U2: löst i meningen.** Den säger "en ögonblicksbild som ingen begärt", vilket är sant; ett
  obett `Storage` från en levande flik tas emot som förut, och commit-meddelandet säger det.
- **U3: två av tre bevisade.** Testet faller utan `sweep` vid start ("swept at start") och med
  `snapshot_asked: false` i `retire_process`. Att väntan växer prövas av en rad som jag läst men
  inte sett falla: körningen med `STORAGE_SAVE_MS` i stället föll först på raden i V1.
- **Kan flaggan fastna?** Inte för en flik som lever och svarar (läst). Dör processen har fliken
  ingen process att fråga, och `start_page` nollar flaggan vid nästa laddning. En stoppad sida har
  kvar sin flikslinga, som svarar. En laddning som misslyckas har aldrig frågats. Vid omladdning
  flyttas flaggan till posten för den släppta processen; dess `StorageUnchanged` kastas i
  `retired_message` och når aldrig den nya processens flagga, eftersom generationen skiljer. En
  uppspelning frågas inte; en inspelad flik frågas, svarar, och frågan spelas inte in. Kvar: en
  process som lever men aldrig svarar frågas inte igen; dess lagring kommer då vid gömning eller
  stängning, eller så dödas den vid tidsgränsen.
- **`poll_in`.** Snurrar inte: en flik med öppen fråga räknas inte (`:3816`). Sover inte förbi:
  svaret väcker slingan, `drain` nollar flaggan, `poll` skickar det som förfallit, och `poll_in`
  räknar fliken igen.
- **Komprometterad flik.** Ett andra svar på samma fråga kastas som obegärt. Ett obett
  `StorageUnchanged` nollar bara flaggan, och nästa fråga går ändå i skalets takt. Från en
  släppt process gör det ingenting.

Nytt:

- **V1. Medel – testet är ostadigt. Reproducerat.** `spec_9_5_4_storage_is_saved_while_the_page_runs`
  föll i 2 av 8 körningar på oförändrad kod, på `lib.rs:1357` ("the snapshot asked for before
  the process was let go of was saved"). Felet sitter i testet, inte i klienten: slingan
  `while shell.retiring() > 0` (`:1349`) kan ta slut innan bilden hanterats, när flikens tråd
  hunnit sluta, och `shell.drain()` hanterar sedan bilden och slutsvaret i ett svep utan att
  titta emellan. Prov: med en titt efter varje meddelande också i det svepet var 10 av 10
  körningar gröna, och i 4 av dem hade slingan tagit slut före bilden. Rättning: just den titten,
  tre rader. Grinden "alla tester gröna" faller annars ungefär var fjärde gång på den här
  maskinen.
- **V2. Låg – inget test märker om `StorageUnchanged` inte öppnar för nästa fråga.** Med armen i
  `shell.rs:1731` tömd är skalets test grönt: varje fråga i testet följer på en ändring, så varje
  svar är en bild. Det är just felet "flaggan fastnar". Test: frågan efter laddningen besvaras
  med "oförändrat", och en ändring därefter ska nå disken.

*SPEC-9.5.4 som skriven* är sann, med två förbehåll: en skrivning som misslyckas görs inte om
(fynd 5), och "den tidens ändringar" är väntan plus den tid en släpande flik tar på sig att svara.
*Inte prövat:* hela grindarna, flikar som egna processer, macOS och Windows.
