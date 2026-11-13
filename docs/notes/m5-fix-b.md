# M5 – rättningar efter granskning 0011 och 0014 (`m5/fix-b`)

Byggagentens noteringar för grenen `m5/fix-b` (från `m5/integration` `e431a60`). Ändrat: bara
`crates/vy-shell`, `crates/vy-runtime`, `crates/vy-render`, `crates/vy-proto`, de här noteringarna
och `SPEC.md` där det står. Fix A rättar samtidigt i `vy-sdk`, `vy-cli`, `modules/`, `conformance/`
och `tools/`. Sessionen kan startas om: läget står här och i `git log`.

## Fynd, commit och test

| Fynd | Commit | Vad | Test |
| --- | --- | --- | --- |
| 0014:1 | `041dcda` | Klientens version ligger i nyckeln (`CodeKey::of` = `CodeKey::of_client(wasm, CLIENT_VERSION)`), så en post som en version sparat aldrig är den en annan slår upp. Katalogen bär en stämpel, filen `VERSION` med innehållet `VYCODE01 <version>`; skiljer den, saknas den eller är den ingen stämpel töms katalogen och stämplas om när `CodeStore::open` körs, alltså när klienten startar. Det tar också bort poster som ett wasmtime-byte gjort oläsbara. Devtools, fliken *Studio*: **Rensa cache (kompilerad kod)** tar bort alla poster och säger hur många; en laddning som var obesvarad när cachen rensades får inte sin kod sparad. | `spec_9_4_2_code_does_not_outlive_its_client_version` (vy-runtime): versionen i nyckeln, två versioners poster i samma katalog, stämpeln vid annan version, ingen, tom, annat format, för lång; `clear`. `spec_9_4_2_the_cache_is_cleared_on_update_and_on_request` (vy-shell, riktiga flikprocesser): en katalog från en annan version töms vid start och modulen kompileras; en post under en annan versions nyckel erbjuds aldrig; knappen med pekaren; obesvarad laddning; därefter kompilering och träff som förr; utan cache ingen knapp. Mutationer: versionen bort ur nyckeln och tömningen bort ur stämplingen fäller vy-runtime-testet. |
| 0014:2 | `a7071b8` | Sparad kod körs bara i en flikprocess. `window::run` vägrar starta utan sökväg till den egna binären (`tab_exe`, som `--screenshot`), i stället för att tyst köra sidor som trådar i huvudprocessen. `Shell::new` öppnar ingen cache när `Config::exe` saknas, `start_page` erbjuder kod bara när flikens förbindelse är en egen process (`TabConnection::is_process`), och `code_reported` tar bara emot från en sådan. | `spec_9_4_2_kept_code_never_runs_in_the_main_process` (vy-shell): skal utan exe med en cachekatalog som håller god kod för modulen: ingen cache, `tab_code == None`, inget "module code" i loggen, inget sparat för en modul utan post, posten orörd, en påtvingad rapport ignoreras; samma katalog med flikprocesser ger träff. Föll före rättningen (cachen öppnades och koden deserialiserades i huvudprocessen). `spec_9_1_1_no_window_without_a_tab_executable` (window.rs). Cachetesterna som körde `shell(None, …, Some(cache))` går nu på riktiga flikprocesser: `spec_9_4_2_code_is_kept_under_the_main_process_key`, `spec_13_1_4_unusable_code_is_dropped_and_compiled`, `spec_9_4_2_no_cache_in_a_directory_others_may_write`. |
| 0014:3 | `afe7245` | `dir_is_private` använder `symlink_metadata`: en cachekatalog som är en länk är ingen cache, varken vid öppning eller senare (`get`, `put`, `remove`, `clear`, `size`). Cachen läser, räknar, tränger ut och raderar bara vanliga filer med sina egna namn: en post är 64 små hexsiffror och `.code` (`is_entry_name`), en temporär fil är `.<post eller VERSION>.<pid>.<nanosekunder>.tmp` (`is_tmp_name`, det `CodeStore::tmp_path` ger). En post öppnas en gång, med `O_NOFOLLOW` på Unix, och läses och tidsstämplas genom samma handtag; en länk under en posts namn är en miss, och länken (inte målet) tas bort. | `spec_9_4_2_code_store_touches_only_its_own_files` (vy-runtime, Unix): länkad katalog vid öppning och efteråt, ingenting skrivet, läst eller raderat genom den; `notes.tmp`, `backup.code`, versaler, 63 siffror och temporära namn av fel form överlever taket, `clear` och ett versionsbyte fast de är äldst och störst, medan cachens egna gamla temporära filer städas och dess äldsta post trängs ut; en länk under en posts namn läses inte och målet är orört (innehåll och tid). Föll före rättningen. `compile_cache::tests::own_names`. `spec_13_1_4_code_store_evicts_least_recently_used` använder nu temporära filer med cachens egen namnform. vy-shell: `spec_9_4_2_no_cache_in_a_directory_others_may_write` har ett steg med länkad katalog. |
| 0014:4 | `cbe7d6f` | Beslut, ingen beteendeändring: kompileringscachen är **på** också på Windows. Skälet står som kommentar där cachen slås på (`window.rs`, `resumed`) och under *Beslut och gränser* nedan (spec-fråga 62). | Inget nytt test: inget beteende ändras. Att en post inte överlever sin klientversion bevisas av testerna för 0014:1. |
| 0014:5 | `1aab938` | En laddararbetare håller en vakt (`page::Running`) medan den gör ett jobb. När vakten släpps, också under en panik, rapporteras ett jobb som inte blev klart som ett laddningsfel till sin flik (`LoadError::Fetch`/`File` för en sida, `Err` för en live-kontroll) och originens tur släpps. Arbetaren fångar paniken och tar nästa jobb, så poolen krymper inte. `run_job` lämnar tillbaka sitt svar i stället för att rapportera själv. | `spec_7_4_5_a_loader_panic_fails_the_page_and_frees_its_origin` (vy-shell `page.rs`): ett jobb som panikar besvaras med fel och rätt generation, nästa sida för samma origin laddas, en live-kontroll som panikar besvaras, och efter fler paniker än arbetare (också en lokal fil) laddar poolen fortfarande. Föll före rättningen (inget svar inom 20 s). |
| 0014:6 | `c4efd8b` | `Loader::send` släpper de sidladdningar och live-kontroller som fortfarande köar för samma flik med äldre generation när en ny sidladdning för fliken kommer (`Job::is_older`); `close_tab` släpper flikens köade sidladdningar och kontroller. Jobbet en arbetare redan håller på med körs klart (en hämtning kan inte avbrytas). Fem omladdningar mot en hängande origin blir alltså den pågående hämtningen och den sista. | `spec_7_4_5_stale_page_loads_are_dropped_from_the_queue` (vy-shell `page.rs`, hållna svar): servern håller flikens första laddning medan fliken laddas om fyra gånger, en annan flik ber om en sida och en tredje ber och stängs; efter släppet kommer den pågående laddningen, den andra flikens sida och flikens sista navigering, i den ordningen, och servern har bara fått de förfrågningarna. Föll före rättningen (`[(1,1),(1,2),(1,3)]`). |
| 0011:1 | se `git log` (`granskning 0011:1`) | `gl::pick` svarar `None` när displayen inte erbjuder någon konfiguration. glutins `DisplayBuilder::build` kräver en väljare som lämnar en konfiguration, så väljaren (`pick_or_unwind`) avbryter med `resume_unwind` (ingen panikrad: inget är fel i programmet) till `gl::guarded`, som gör det, och varje annan panik i GL-uppsättningen (displayen, kontexten, `GlRaster::new`), till ett skäl för CPU-vägen (`NoGl`) i stället för slutet på klienten. Fönstret lämnas tillbaka när det hann skapas. | `gl::tests::spec_11_3_2_no_gl_configuration_is_a_fallback` (vy-shell): `pick` med tom iterator är `None` (granskarens repro panikade), väljaren genom vakten ger "the GL display offers no configuration", en panik med och utan text blir ett skäl, `Ok` och `Err` går igenom orörda. GL-testet under Xvfb går genom samma `gl::open`. |

## Beslut och gränser

- **"Rensa cache" rensar bara kompilerad kod** (`<data>/cache/code`). Innehållscachen i `vy-proto`
  (paket och HTTP-svar under `<data>/cache`) och de pinnade nycklarna rörs inte: de är inte nativ
  kod, de kontrolleras mot hash och signatur vid varje användning, och en knapp som också glömmer
  pinnar vore en annan sak än den granskningen bad om. Det var också den mindre ändringen.
- **Stämpeln är klientens version, inte wasmtimes.** Ett wasmtime-byte utan versionsbyte finns bara
  i utvecklingsbyggen; där hittas gamla poster ändå aldrig (motorns hash ligger i nyckeln) och de
  trängs ut av storlekstaket som förr. Stämpeln läses innan motorn byggts, så starten väntar inte
  på wasmtime.
- **Kompileringscachen är på också på Windows (0014:4, spec-fråga 62).** Avgjort av koordinatorn
  på ägarens delegation. Skälet: (1) en post är nycklad på modulens egna byte och fylls bara av en
  flikprocess som laddats med just de byten, innan någon av sidans kod körts; en förgiftad post kan
  därför bara komma från en modul som bryter sig ut medan den kompileras, och den körs bara igen
  för samma modul, som utan cache kunde upprepa sin utbrytning vid varje besök. (2) Med 0014:1
  överlever en post aldrig den klientversion som sparade den, så en rättad kompilator eller
  värdkod gäller från första starten efter uppdateringen. (3) Att Windows standardsandlåda inte
  hindrar läsning och nät är en egenskap hos sandlådan (strict stänger det), inte hos cachen:
  koden körs i samma flikprocess med eller utan den. Jag har prövat resonemanget och håller med.
  Två saker som det inte säger och som ägaren bör känna till: (a) *tillförlitlighet* – en
  utbrytning som bara lyckas ibland (en kapplöpning i kompilatorn) blir med cachen säker vid
  varje senare laddning av samma byte, tills klienten uppdateras eller cachen rensas; det gäller
  alla plattformar men når mer på Windows i standardläget. (b) *Ingen ACL läses på Windows*
  (`dir_is_private` prövar där bara att katalogen finns och inte är en länk); cachen ligger under
  `%APPDATA%`, som bara användaren, SYSTEM och administratörer får skriva, och en flikprocess på
  låg integritetsnivå får inte skriva där. Oprövat på en riktig Windows-maskin.
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
