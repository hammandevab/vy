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

**Rekommendation.** **Merge efter** fynd 1 (en rad och ett test). Fynd 2 och 3 avgör ägaren, före
eller efter releasen; 4–6 som uppföljningar.
