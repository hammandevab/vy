# M5 – putsning före v0.3.0 (`m5/polish`)

Byggagentens noteringar för grenen `m5/polish` (från `m5/integration` `bc22baa`). Fyra punkter i
ordning: kompileringscachen i skalet (SPEC-13.1.4, granskning 0008:1), sidladdningar parallellt mellan
flikar (granskning 0009:2), `widgets`-sidans hjälptext, och textfältets kodstorlek i SDK:t.
Granskningssidan skrivs av en annan agent. Sessionen kan startas om: läget står här och i `git log`.

## Läge

| Punkt | Läge | Commits |
| --- | --- | --- |
| 1 Kompileringscachen i skalet | klar; release-mätningen görs om sist | `f46daac` (`spec:` SPEC-9.4.2), `1d8b7a2`, `2339f12` (noteringar) |
| 2 Parallella sidladdningar | klar | `b362580` |
| 3 `widgets`-hjälptexten | ej påbörjad | – |
| 4 Textfältets kod delad mellan radlägena | utkast utanför arbetskopian (`/tmp/m5p/textbox.rs`) | – |

## Punkt 1 – kompileringscachen i skalet (SPEC-13.1.4, SPEC-9.4.2)

**Före.** `tab.rs` satte `CompileCache::Off`: varje laddning kompilerade, och SPEC-13.1.4:s "0 ms vid
kompileringscache-träff" gällde bara benchen (granskning 0008:1). `vy-runtime` hade en katalogbaserad
cache (`ReadOnly`/`ReadWrite`, `precompile`) som ingen produktväg använde, och som hade krävt att
flikprocessen läste filer (omöjligt efter låsningen, SPEC-9.1.3) eller att huvudprocessen
deserialiserade.

**Nu.** Flikprocessen rör aldrig filsystemet och huvudprocessen deserialiserar aldrig.

- **Fliken (`vy-runtime`, `tab.rs`).** `CompileCache::Host { code }` ersätter de katalogbaserade
  lägena: med kod från huvudprocessen deserialiseras modulen i stället för att kompileras; utan kod,
  eller om koden inte går att deserialisera, kompileras den som förr och den serialiserade modulen
  sparas i `Runtime` tills värden hämtar den (`Runtime::take_code`). `LoadStats::refused` säger att
  erbjuden kod avvisades. Fliken svarar en gång per laddning, före `vy_init` (alltså innan någon av
  sidans kod körts; en start-funktion vägras av ABI-kontrollen, SPEC-5.3.4), med
  `ToShell::Code { cached, micros, code }` (tagg 79).
- **Huvudprocessen (`vy_runtime::CodeStore`, `vy-shell/src/code_cache.rs`, `shell.rs`).** Före
  `Load` slår skalet upp modulen (`CodeCache::offer`): nyckeln `CodeKey::of(wasm)` är SHA-256 över
  formatets magi, motorns kompatibilitetshash (wasmtimes version och varje kompileringsinställning,
  `Engine::precompile_compatibility_hash`) och SHA-256 av **de modulbyte huvudprocessen själv ska
  skicka**. Träffen skickas i en egen ram precis före laddningen, `ToTab::CodeCache { code }` (tagg
  15), som gäller exakt nästa meddelande. Nyckeln ligger kvar i huvudprocessen (`Tab::code`) tills
  fliken svarat; svaret räknas en gång per laddning, bara medan laddningen är obesvarad och bara när
  huvudprocessen slog på cachen för den laddningen. Koden sparas under den nyckeln, vad fliken än
  säger: inget i svaret namnger nyckel, hash eller sökväg.
- **Posten.** `<data>/cache/code/<hex>.code` (`paths::code_cache_dir`), innehåll `VYCODE01`, SHA-256
  av koden, koden (`Module::serialize`). Katalogen skapas 0700 och används bara om den ägs av
  användaren och varken grupp eller andra får skriva (`dir_is_private`, prövas vid varje läsning och
  skrivning; annars är cachen av). Skrivs till en temporär fil och döps om; läses hel till minnet och
  kontrolleras mot hashen; en trasig, avhuggen eller för stor fil tas bort vid uppslaget.
- **Tak.** Högst 64 MB kod per modul och högst 256 byte kod per byte modul (`MAX_CODE_BYTES`,
  `MAX_CODE_PER_MODULE_BYTE`); samma 64 MB är IPC-ramens tak åt båda håll. Katalogen rymmer 256 MB
  (`COMPILE_CACHE_CAP_BYTES`); över det tas de poster bort som användes för längst sedan (en läsning
  sätter filens tid, så ordningen är senast använd). Gamla `.tmp`-filer efter avbrutna skrivningar
  städas efter tio minuter.
- **När koden inte duger.** Avvisar fliken koden (`cached: false` fast kod erbjöds) tas posten bort
  och ersätts med flikens nya kod. Dör flikprocessen medan den håller erbjuden kod, eller innan den
  hunnit svara, tas posten bort och erbjuds inte igen.
- **Utan cache.** Huvudlöst (`headless_shell`: `--screenshot`, konformansens skalfall, testerna),
  `vy-cli render` och konformanssviten har `code_cache: None` respektive `CompileCache::Off`: allt
  kompileras, inget sparas, inget läses. En inspelad flik får aldrig kod (skalet slår inte på cachen,
  och skrivartråden släpper inte igenom en kodram till en inspelad flik), så en inspelning innehåller
  ingen maskinkod; en uppspelning matas aldrig med en kodram ur sin fil (skrivartråden hoppar över
  tagg 15), så en fil någon fått av en annan kan inte bära maskinkod in i fliken.
- **Trådar.** Uppslaget (en hash och en filläsning) görs där det begärs, i `start_page`. Skrivning,
  borttagning och den utrensning som följer en skrivning görs på cachens egen tråd, i ordning, så
  UI-tråden aldrig väntar på disken för en skrivning. Samma tråd bygger wasmtime-motorn första gången
  (nyckeln behöver motorns inställningar): huvudprocessen har alltså nu en wasmtime-`Engine`, men
  kompilerar, deserialiserar och kör ingenting med den.

### Hotbild – kompileringscachen i skalet

- **Vad.** En ny lagringsyta i huvudprocessen (`<data>/cache/code`), två nya IPC-ramar (kod till
  fliken före laddningen, flikens svar med kod), och deserialisering av sparad kod i flikprocessen.
  `Module::deserialize` validerar inte maskinkoden: en post **är** nativ kod för den process som
  deserialiserar den.
- **Vem kan missbruka.** (1) En sida som tagit sig ut ur wasm-sandlådan i sin flikprocess och lämnar
  förgiftad kod i sitt svar. (2) En sida som försöker få sin kod sparad för en annan modul eller en
  annan origin. (3) En fil utifrån: en inspelning (`.vyrec`) med en kodram i. (4) Ett annat konto
  eller program som skriver i cachekatalogen. (5) En flik som skickar för mycket, för ofta eller vid
  fel tillfälle.
- **Hur begränsas det.**
  - *Nyckeln är huvudprocessens.* Den räknas ur de modulbyte huvudprocessen själv skickar fliken och
    bärs i huvudprocessen från erbjudandet till svaret. En post för modul X kan därför bara komma
    från en flik som laddades med X och bara sparas under X:s nyckel; den lämnas bara till en flik
    som laddas med exakt samma byte. En komprometterad flik kan förgifta **sin egen moduls** post,
    aldrig en annan moduls eller en annan origins sida med andra byte.
  - *Koden körs bara i en sandlådad flik.* Huvudprocessen håller byte och deserialiserar aldrig.
    Fliken som kör posten har samma OS-sandlåda som alla flikar (SPEC-9.1.3).
  - *En gång, före sidans kod.* Svaret räknas en gång per laddning, bara före `Loaded`, bara när
    cachen är på för laddningen; allt annat loggas och ignoreras. Ett påstått "träff" utan erbjuden
    kod ignoreras.
  - *Tak.* 64 MB per ram och post, 256 × modulens storlek, 256 MB i katalogen.
  - *Filer utifrån.* Kodramar släpps aldrig igenom till en uppspelande flik och skrivs aldrig i en
    inspelning.
  - *Katalogen.* Ägarkontroll och läge vid varje läsning och skrivning; atomär skrivning; hash över
    koden fångar trasiga filer (inte den som kan skriva katalogen).
- **Kvarstående risk, rakt sagt.** (a) *Beständighet för den modulen.* En sida som en gång brutit
  sig ut ur wasmtime i sin flik kan lämna kod som körs nativt, utan wasm-sandlådan, varje gång
  samma modulbyte laddas igen, också efter omstart, tills posten tas bort eller modulen ändras.
  Koden körs fortfarande bara i en sandlådad flikprocess och bara för den modulen, så den når inte
  mer än utbrytningen redan nådde, men den överlever fliken. (b) *Windows standardsandlåda* hindrar
  inte läsning av filer eller nätverk (spec-fråga 62), så där når sådan kod mer än på Linux och
  macOS. (c) *Samma modul på flera origins.* Nyckeln är modulens byte, inte originen: serverar två
  sajter exakt samma modul delar de post. Det som hindrar att en sajt förgiftar posten för en annan
  är när svaret räknas: varje laddning får en ny process, och det enda svaret tas emot innan
  `vy_init`, alltså när processen bara hunnit läsa huvudprocessens ramar, kontrollera hash och ABI
  och kompilera just de modulbyten. Resurser (PNG) avkodas först vid första `dl_image`, och en
  process som komprometteras senare, av sidans kod eller av en resurs, får inget svar till räknat.
  Kvar är en utbrytning genom själva kompileringen av modulen, och den som har en sådan modul kan
  servera den var som helst. (d) *Lokal manipulation* av katalogen av samma användare
  ligger utanför modellen. (e) Huvudprocessen har nu en wasmtime-`Engine` (signalhanterare
  installeras processvitt), utan moduler.
- **Så rensar användaren cachen.** Ta bort katalogen `cache/code` under klientens katalog
  (`~/.config/vy/cache/code`, `~/Library/Application Support/vy/cache/code`,
  `%APPDATA%\vy\cache\code`, eller under `VY_DATA_DIR`); den skapas igen vid nästa start. Det finns
  ingen knapp för det än (se *Öppet*).
- **Test som bevisar begränsningen.** Se tabellen nedan.

### Tester (punkt 1)

| Test | Var | Visar |
| --- | --- | --- |
| `spec_13_1_4_a_second_load_does_not_compile` | vy-shell `code_cache.rs` | riktiga flikprocesser: första laddningen kompilerar och posten sparas under modulens nyckel; ny process, omladdning och omstartad klient tar modulen ur cachen (flikens rapport före sidan körs), samma display list; processerna rapporterar hel sandlåda och sonden kan fortfarande inte skriva en fil; utan cache sägs och sparas inget |
| `spec_9_4_2_code_is_kept_under_the_main_process_key` | vy-shell `code_cache.rs` | en flik laddad med `hello` lämnar `text`s kod: den sparas under `hello`s nyckel, `text` får ingen post; andra svaret, svar efter `Loaded`, "träff" utan erbjudande, kod över taket och svar utan påslagen cache ändrar inget |
| `spec_13_1_4_unusable_code_is_dropped_and_compiled` | vy-shell `code_cache.rs` | vänd byte och halv fil: tas bort vid uppslaget, kompileras, sparas på nytt; hel fil som inte är en modul: erbjuds, avvisas, ersätts; avvisad utan ersättning: borta; flikprocess som dör med erbjuden kod: posten borta |
| `spec_9_4_2_no_cache_in_a_directory_others_may_write` | vy-shell `code_cache.rs` | 0777: ingen cache; öppnad privat och sedan 0777: varken läst eller skriven |
| `spec_9_4_2_recordings_hold_no_code` | vy-shell `code_cache.rs` | inspelad flik kompilerar själv fast cachen har modulen, filen saknar kodramar; en inspelning med en kodram före laddningen spelas upp som om ramen inte fanns |
| `spec_13_1_4_compile_cache_hit_skips_compilation` | vy-runtime | `Host` utan kod kompilerar och lämnar koden; med koden: träff, samma frames; `Off` lämnar inget; lägen 0700/0600 |
| `spec_9_4_2_code_store_keeps_bytes_under_module_keys` | vy-runtime | nyckeln ur modulens byte; skräp och halv kod avvisas och kompileras; trasiga filer tas bort; taken; katalog som andra får skriva |
| `spec_13_1_4_code_store_evicts_least_recently_used` | vy-runtime | taket hålls, minst nyligen använd först, läsning räknas som användning, gamla `.tmp` städas |
| `spec_11_2_1_messages_round_trip`, `spec_11_2_2_bad_frames_are_errors`, `spec_9_3_2_main_process_holds_tab_messages_to_the_caps` | vy-shell `ipc.rs` | 15 `ToTab`- och 16 `ToShell`-varianter; en träff bär ingen kod; 64 MB-taket åt båda håll |
| `fuzz_ipc_corpus_has_the_code_cache` | vy-shell `tests/fuzz_regressions.rs` | fyra nya frön i `fuzz/corpus/ipc/`, mutationer panikar inte |
| `spec_13_1_4_compile_time` | vy-cli | träffhalvan mäter skalets väg i release (`bench::cache_hit_times`) |

### Storlek efter punkt 1

`vy-shell` i release (Linux x86_64, strippad, receptet ur `tools/build_release.sh` med
`repro_env.sh`, byggd ur en ögonblicksbild av `2339f12`): **24 572 200 byte** (SHA-256
`9b9e6c89…abfa0e83`), 47 888 mer än `bc22baa` (24 524 312). Marginal mot 25 000 000: 427 800 byte
(1,7 %). Träffhalvan av `spec_13_1_4_compile_time` i release hann inte köras före en omstart av
servern; den mäts om sist tillsammans med slutstorleken (se *Slutmätningar*).

### Mutationskontroller (punkt 1)

Tre skydd stängdes av ett i taget i en provbyggnad och testerna föll som de ska: uppspelningens
filter för kodramar (`spec_9_4_2_recordings_hold_no_code`), borttagningen av posten när
flikprocessen dör (`spec_13_1_4_unusable_code_is_dropped_and_compiled`) och villkoret att svaret
bara räknas medan laddningen är obesvarad (`spec_9_4_2_code_is_kept_under_the_main_process_key`).

## Punkt 2 – sidladdningar parallellt mellan origins (SPEC-7.4.5, granskning 0009:2)

**Före.** En laddartråd ägde den enda `Fetcher`. Sidladdningar och live-omladdningens kontroller
kördes i tur och ordning, så en origins hängande DNS-uppslag (upp till 6 s) eller långsamma server
höll varje annan fliks sida.

**Nu.**

- **`vy_proto::Fetcher` delas mellan trådar.** `fetch` och `revalidate` tar `&self`. Blobcachen,
  HTTP-cachen och nyckelförrådet ligger bakom var sitt lås som hålls runt en läsning eller skrivning
  och aldrig över en nätverksförfrågan. Hämtningar av **samma origin turas om** (`Fetcher::turn`:
  en mängd upptagna origins och en villkorsvariabel): hela hämtningen med sitt förtroendebeslut –
  pinnar, efterträdare, logghuvud – görs av en hämtning i taget per origin, så två samtidiga första
  besök ger ett DNS-uppslag, en hämtning av nyckelfilen och en pin; den andra väntar och finner
  origin pinnad. Hämtningar av olika origins väntar inte på varandra. `Fetcher::waiting()` säger hur
  många som väntar på sin tur (test och felsökning). `blobs()` och `keys()` ger ett lås i stället
  för en referens: titta och släpp, aldrig över en hämtning.
- **Skalets laddare är en pool** (`page.rs`): `LOADER_WORKERS` = 4 trådar över en gemensam kö. En
  arbetare tar det äldsta jobbet vars origin ingen annan arbetare håller på med; jobb för samma
  origin körs alltså ett i taget i den ordning de begärdes och **tar aldrig en andra arbetare**
  (annars kunde fyra flikar mot en hängande origin ta hela poolen), jobb för olika origins körs sida
  vid sida. En lokal fil har ingen origin och väntar aldrig. När skalet stängs släpps det som
  väntar i kön; arbetarna slutar när deras pågående jobb är klart. Sidornas egna anrop går som förr
  på en tråd per flik.
- **Gräns.** Fyra olika origins som alla hänger samtidigt tar ändå hela poolen tills deras
  tidsgränser löpt ut (DNS 6 s, HTTP 30 s). Poolen är avsiktligt liten och fast.

**Tester (hållna svar, inga tider).**

| Test | Var | Visar |
| --- | --- | --- |
| `spec_7_4_5_first_visits_at_once_pin_once` | vy-proto | första besöket hålls i sitt DNS-uppslag; ett andra besök av samma origin väntar på sin tur (`waiting() == 1`) utan eget uppslag; en annan origin hämtas, betros och pinnas under tiden; efter släppet: båda betrodda, ett uppslag, en nyckelfil, en pin |
| `spec_7_4_5_page_loads_do_not_wait_for_other_origins` | vy-shell `page.rs` | origin A:s server håller sin sida, fem sidor till för A köar (fler än arbetarna); origin B:s sida och live-kontroll besvaras ändå; efter släppet kommer A:s sex sidor i begärd ordning |

Mutationskontroll: utan turordningen gör det andra besöket ett eget DNS-uppslag och vy-proto-testet
faller; utan köns origin-regel tar A:s jobb alla arbetare och skaltestet faller (B hålls).
Befintliga `spec_5_2_7_requests_do_not_wait_for_other_tabs` är oförändrat grönt.

## Beslut

- **De katalogbaserade lägena och `precompile` är borttagna ur `vy-runtime`**, inte bara oanvända:
  `precompile` deserialiserade en befintlig post i huvudprocessen, vilket är precis det den nya
  modellen förbjuder. Ett sätt (princip 1).
- **Koden går i en egen ram före `Load`, inte i `Load`.** `Load`-ramens kodning är oförändrad, så
  äldre inspelningar spelas upp som förr, och allt som har med cachen att göra går att känna igen på
  en tagg (uppspelningens filter).
- **Flikens svar är ett meddelande med tre fält** (`cached`, `micros`, `code`) i stället för bara
  `Compiled { bytes }`: skalet får veta att erbjuden kod avvisades (posten tas bort), testerna får
  ett svar från fliken på att den inte kompilerade, och §13-testet får flikens tid.

## Öppet

- Ingen knapp eller flagga för att rensa cachen; katalogen tas bort för hand.

## Berör andra spår

- (fylls i)
