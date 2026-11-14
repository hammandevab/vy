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
| 0011:1 | `7563132` | `gl::pick` svarar `None` när displayen inte erbjuder någon konfiguration. glutins `DisplayBuilder::build` kräver en väljare som lämnar en konfiguration, så väljaren (`pick_or_unwind`) avbryter med `resume_unwind` (ingen panikrad: inget är fel i programmet) till `gl::guarded`, som gör det, och varje annan panik i GL-uppsättningen (displayen, kontexten, `GlRaster::new`), till ett skäl för CPU-vägen (`NoGl`) i stället för slutet på klienten. Fönstret lämnas tillbaka när det hann skapas. | `gl::tests::spec_11_3_2_no_gl_configuration_is_a_fallback` (vy-shell): `pick` med tom iterator är `None` (granskarens repro panikade), väljaren genom vakten ger "the GL display offers no configuration", en panik med och utan text blir ett skäl, `Ok` och `Err` går igenom orörda. GL-testet under Xvfb går genom samma `gl::open`. |
| 0011:2 | `f1352ed` | Inget beteende ändrat; fönstrets GL-komposition går nu att prova. Kroppen i `window::gpu::present` är flyttad till `gl::GlContext::compose(shell, changed, drawn)` (fönstret anropar den och byter sedan buffert), och `GlRaster::read_window` läser standardframebuffern före bufferbytet. | `tests/gl.rs` (under Xvfb med Mesa, `VY_GL_REQUIRED=1`), två nya delar. **`present`**: en sidduk 700 × 560, större än fönstret 640 × 480 och lagd så att den når utanför sidytan åt alla håll, under en ram där varje pixel är känd: ogenomskinlig chrome, en halvgenomskinlig chromeremsa utanför sidytan, genomskinlig sidyta och ett halvgenomskinligt överlägg. Chromen är ramens byte, remsan ligger över vitt och aldrig över sidan, sidytan är sidans egna pixlar exakt, överlägget inom 1 nivå; sedan samma med bara ändrade rader skickade och utan sida. **`compose`** med ett riktigt skal och riktiga flikprocesser, åtta kompositioner: `shop` 900 × 600, med devtools öppet, med en opkod markerad, zoom 110 % och 125 % (duken är då inte sidytans storlek), en sida som målar rött från −10 000 till +10 000, och `text` i 600 × 400 @ 1,5 med och utan devtools. Varje gång är pixlarna utanför sidytan byte för byte CPU-kompositionens (1 997 550 pixlar i allt) och sidytan inom toleransen (0,310 % över 2 nivåer, max 22); den röda sidan är röd till kanten av sin yta och ingenstans utanför. Mutation: utan saxen i `present` faller första delen på chromeremsan (10 240 pixlar). |
| 0011:3 | `c176a1c` | `GlContext::compose` håller reda på om sidans framebuffer får visas (`gl::PageState`: versionen som senast lämnades till pipelinen och om bilden blev ritad). En lista som referensen vägrar (`GlError::Render`) visas inte alls, varken i den kompositionen eller i nästa med samma sidversion (granskningens förslag `at = None` i felgrenen hade visat den gamla bilden igen vid nästa omritning av chromen), och skälet loggas i fliken som på CPU-vägen (`Shell::page_not_drawn`: `[host] display list not rendered: …`). | `tests/gl.rs`, delen *refused*: en röd och en blå sida i var sin flik; medan den röda visas skickar den blå flikens process en lista utan giltig opkod, och användaren byter till den blå fliken. Sidytan är tom (vit) i två kompositioner i rad, fönstret är CPU-kompositionens, loggen säger varför, och efter omladdning visas den blå sidan. Föll före rättningen: 257 280 av 257 280 pixlar i sidytan var den röda flikens. |
| 0011:4 (felet) | `62ceaa5` | **`rymd`s 133 nivåer var ett riktigt fel i GL-vägen**, inte en kant: pixeln (341, 184) i 800 × 600 ligger en pixel bortom ett spetsigt hörn (340,75; 184,66) som tre trianglar delar. `rymd` fyller varje triangel och stryker den sedan med 1 px linje i samma färg. Referensen stryker med `Pixmap::stroke_path`, som ritar en linje på högst en pixels bredd (bredd × skala ≤ 1) som en hårlinje utan fogar; GL-vägen fyllde i stället strykningens kontur, vars geringsfog sticker ut upp till två linjebredder bortom ett spetsigt hörn. `PathAtlas::add` stryker nu med samma anrop som referensen (`stroke_path` i ogenomskinligt vitt på en genomskinlig skissduk av fotavtryckets storlek; alfan är täckningen), så hårlinjer blir hårlinjer. Fyllda banor är oförändrade. | `tests/gl.rs`, ny klass *strokes*: 45 listor med spetsiga trianglar (fyllda och strukna) och öppna polylinjer, linjebredd 0,25–2,2, skala 1, 1,5 och 2, med gränsen `MAX_DELTA` = 64 nivåer per pixel. Före rättningen: upp till 8,859 % av pixlarna över 2 nivåer (alltså också över SPEC-11.3.2:s 3 %) och största avvikelse 209; efter: 0,178 %, värsta frame 0,892 %, max 41. Sidklassen: max 133 → 40, värsta frame 0,387 % (`rymd`) → 0,319 % (`block` 480 × 320 @ 2). Testet skriver nu var den största avvikelsen sitter (pixel och frame) för varje klass. |
| 0011:4 (gränsen och talen) | `456ec32` (`spec:`) | **SPEC-11.3.2** har fått gränsen: GPU-vägens bild har högst 3 % pixlar över 2 nivåer **och ingen pixel där någon kanal avviker med mer än 64 nivåer**. Meningen behövde den: andelen ensam tillät att 3 % av pixlarna var hur fel som helst, och det var så `rymd`s 133 nivåer gick igenom. `docs/notes/m5-gpu.md` har talen från HEAD och förklaringen av de 133. | `tests/gl.rs` hävdar `MAX_DELTA` = 64 för varje frame i varje klass (`frame_2000`, slumplistor, tunna strykningar, sidor) och för sidytan i fönstrets komposition. Uppmätt största avvikelse: 41, 37, 41, 40 och 22. Med strykningsfelet kvar faller gränsen på `rymd` (133) och på strykningsklassen (209). |
| 0011:5 | `f8d9be3` | **Tappad kontext.** Kontexten begärs först med återställningsavisering (`Robustness::RobustLoseContextOnReset`: EGL_EXT_create_context_robustness, WGL_ARB_create_context_robustness) och annars utan, i ordningen OpenGL 3.3 core med, utan, OpenGL ES 3.0 med, utan. Har kontexten `glGetGraphicsResetStatus` (OpenGL ≥ 4.5, OpenGL ES ≥ 3.2 eller KHR/ARB/EXT_robustness; namnet väljs efter version och tillägg och funktionen hämtas ur kontextens laddare) frågas drivrutinen före varje komposition (`GlContext::alive`, ett anrop utan argument); en tappad kontext är ett `GlError::Draw` och fönstret går över till CPU-vägen. Loggraden säger om en tappad kontext märks. **`leave_gpu`**: går ingen CPU-yta att skapa avslutar klienten med besked (`GL failed (…) and the window has no surface for the CPU path either; exiting`), efter `Shell::shutdown` så att sidornas lagring sparas, som startvägen gör. | `gl::tests::spec_11_3_2_a_lost_context_is_asked_for`: vad varje återställningsstatus betyder och under vilket namn frågan finns per API, version och tillägg (ingen fråga utan stöd). `window::tests::spec_11_3_2_leaving_gl_draws_on_the_cpu_or_says_why_not`: ett skal i lagerläge ritar sidan i sin egen ram igen och loggar skälet; utan CPU-yta blir svaret avslutsmeddelandet. `tests/gl.rs`: kontexten under Xvfb är skapad med avisering (llvmpipe: "noticed (reset notification)") och `alive()` är `Ok` före och efter alla kompositioner. Fönsterklienten provkörd under Xvfb med och utan `VY_NO_GL`. En verklig kontextförlust går inte att framkalla här. |
| 0011:6 | `bb1c280` | Bara test; koden var rätt. | `spec_11_3_2_damage_draws_what_the_reference_draws` (vy-render) fortsätter efter sina sju redigeringssteg på **samma** `Renderer` med det ett fönster gör utan att listan ändras: ny storlek, tillbaka, ny skala, tillbaka, samma stil-id med andra stilar (annat snitt, annan storlek, vikt och färg), tillbaka, och alla tre på en gång; sedan en redigering i det läget. Varje frame är referensens byte för byte, ny storlek eller skala ritar allt, och en ren stiländring ritas om fast ingen opkod ändrats (> 100 sådana omritningar i de 90 fallen). Mutationer som det gamla testet inte såg: skalan bort ur jämförelsen av duken (faller i skalsteget) och stilen bort ur jämförelsen av opkoder (faller i stilsteget). |
| tillägg 13 (0013:1–2) | `561375e` | **"Rensa sidans lagring"** i devtools, fliken *Studio*, under *Lagring* bredvid "Rensa cache". Raden namnger originen; en lokal fil har ingen origin och inget sparat, och får ingen knapp. Klienten **frågar först** med samma överlägg över sidytans topp som capability-frågan (`chrome::Ask::ClearStorage`): "Rensa allt som vy://… har sparat? Det går inte att ångra." med **Rensa** och **Avbryt**; Rensa räknas först när frågan stått i `DIALOG_ARM_MS` (700 ms). Rensningen görs i huvudprocessen på originens fil (`storage::save` med tom lagring, SPEC-9.5.2); ingen flikprocess tillfrågas. **Varje flik med den originen laddas om** (en sida som fortfarande kör skulle annars spara sin kopia igen när den släpps), och det de släppta processerna svarar med på vägen ut sparas inte (`Retired::discard`). Loggrad i varje berörd flik och besked i panelen. | `spec_5_2_9_the_user_can_clear_a_pages_storage` (vy-shell): två signerade sidor på var sin loopback-server (betrodda, får `storage`), tre flikar. Origin A sparar 4, origin B 5, en andra A-flik håller 6 i minnet. Avbryt ändrar inget; Rensa direkt räknas inte; efter 700 ms är A:s fil borta, B:s orörd, båda A-flikarna (och bara de) har ny generation, och när de gamla processerna svarat med 4 och 6 är lagringen fortfarande tom; sidan sparar sedan som förr (7). Mutation: utan `discard` kommer det gamla tillbaka och testet faller. `spec_9_6_1_chrome_layout_is_deterministic` (frågans text och knappar, samma platser), `spec_11_4_1_tree_log_recording_studio_panes`. |
| eget fynd (hittat under tillägg 14) | `2345591` | **Dataförlust som fanns före grenen:** en flikprocess vars sida aldrig startade (laddningen avvisad, eller `vy_init` avbruten) har ingen runtime och svarar på `Shutdown` med en tom lagring; huvudprocessen sparade den, och en tom lagring tar bort originens fil. En sida som låst sig på sin sparade data fick alltså sin data **tyst raderad vid andra omladdningen** (eller när fliken stängdes) och startade tom. Nu sparas ett lagringssvar bara från en process vars sida laddats och vars `vy_init` återvänt (`Tab::started`; för en släppt process `Retired::discard`). Flikprocessens protokoll är orört. | `spec_5_2_9_a_page_that_never_started_does_not_erase_its_storage` (vy-shell): en signerad sida på loopback sparar, laddas om och fastnar i `vy_init`; efter tre omladdningar, stängd flik och stängt fönster finns det sparade kvar och sidan fastnar likadant varje gång. Föll före rättningen ("round 1: … running": sidan startade tom). |
| tillägg 14 (0013, 0015) | `28e490a` | **Vyn för en stannad sida.** Före: rubriken "Sidan stannade", runtimens engelska rad ("call aborted after 64 ms of logical time (SPEC-5.3.3)") och "Andra flikar påverkas inte. F5 laddar om."; ett `vy_init` som avbröts räknades som en *avvisad* modul och visade manifestets reservtext (eller "Modulen avvisades"). Nu: när skälet är en av klientens egna tidsgränser (ett anrop avbrutet vid den hårda gränsen, eller avstängd efter upprepade budgetöverskridanden) står först vad det betyder på vanlig svenska, sedan runtimens rad i dämpad text, och under dem **Ladda om** och, när sidan har en origin, **Rensa sidans lagring** (som ställer frågan från tillägg 13). Ett `vy_init` som avbröts vid den hårda gränsen är en stannad sida, inte en avvisad modul. Alla andra stopp (fälla, `exit`, försvunnen process) visar sitt skäl som förr, med Ladda om. Igenkänningen görs på runtimens egen text (`vy_runtime::Error::is_over_budget`; flikprocessen är samma bygge), så IPC-ramarna är orörda. | `spec_5_3_3_a_page_stopped_at_the_hard_limit_says_so_and_offers_ways_out` (vy-shell): en signerad sida som fastnar i `vy_init` på sin sparade data visar förklaringen, runtimens rad, båda knapparna och inte reservtexten; Ladda om laddar om (och den fastnar igen, med datan kvar); Rensa sidans lagring frågar, och efter svaret kör sidan. En frame som aldrig återvänder i en lokal fil: samma vy, bara Ladda om; en fälla: skälet, ingen förklaring om tid. `shell::tests::spec_5_3_3_stops_by_the_budget_are_explained`, `spec_5_3_3_an_aborted_call_is_recognised_by_its_text` (vy-runtime). Skärmbild före och efter tagen med `--screenshot`. |
| tillägg 15 (0012:7) | `3714cdc` | **Kallstarten med och utan `a11y-linux` är mätt** (talen står under *Kallstart* nedan). `a11y-linux` kostar inget mätbart i någon av de tre starterna: −0,8 %, +0,2 % och +0,5 % vid medianen över 200 omväxlande körningar, mot SPEC-12.2.3:s 5 %. Adaptern skapas därför som förr, när fönstret skapas; ingen kod ändrad. | `tests/cold_start.rs`, `spec_13_1_1_the_clients_processes_start` (vy-shell): startar klientens flikprocess (som skalet startar den, till första display list) och det fönsterlösa skalet (`--screenshot`, start till avslut) på riktiga processer och skriver medianerna; med `VY_COLD_START_A` och `VY_COLD_START_B` jämför den två byggen omväxlande, också fönsterklienten där det finns en display. Ingen budget hävdas (ett debugbygge på en delad maskin säger inget om 150 ms). |

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
- **Tappad GL-kontext – vad som märks och inte (0011:5).** Märks: en grafikåterställning i en
  kontext skapad med avisering där drivrutinen har `glGetGraphicsResetStatus`; varje GL-fel efter
  en sidritning eller komposition (`glGetError`, som i en sådan kontext också ger
  `GL_CONTEXT_LOST`); ett fel från bufferbytet. Märks inte: en drivrutin utan aviseringstillägget
  eller utan frågan (äldre än OpenGL 4.5 utan KHR/ARB_robustness) som tappar kontexten utan att
  något anrop ger fel – fönstret blir då svart eller står still utan reserv; och en panik i vår
  egen ritkod under en frame (bara uppsättningen fångas, 0011:1), som avslutar klienten.
  `VY_NO_GL=1` är vägen runt. Prövat här bara med Mesa llvmpipe (EGL), som ger en kontext med
  avisering; WGL-vägen är bara typkontrollerad.
- **Två klientversioner som delar katalog** (en gammal klient igång medan en ny startar) tömmer
  varandras poster vid varje start. Ingen av dem läser den andras kod; det kostar kompileringar.

## Kallstart med och utan `a11y-linux` (tillägg 15, 0012:7)

Två releasebyggen av `vy-shell` från `28e490a`, byggda som `tools/build_release.sh` bygger
(`tools/repro_env.sh`, `--release --locked --target x86_64-unknown-linux-gnu`):

- **utan**: `--no-default-features --features window,gl`, 22 263 488 byte;
- **med**: standardfunktionerna (`window`, `gl`, `a11y-linux`), 24 633 000 byte.

Mätt med `tests/cold_start.rs` (`VY_COLD_START_A` = utan, `VY_COLD_START_B` = med): byggena startas
omväxlande, i omvänd ordning varannan omgång, efter en omgång som inte räknas. Maskinen: 2 vCPU
(Xeon 2,1 GHz, delad VM), Linux 6.18, Xvfb med Mesa llvmpipe (klienten väljer OpenGL 4.5) och en
sessionsbuss från `dbus-run-session`. Tre starter:

- **flik**: `vy-shell --tab` startad som skalet startar den (med sandlådan), laddad med `hello`
  och tickad, tills första display list är tillbaka; det `vy-cli bench` kallar `tab.cold-start`
  (där med `vy-cli --tab`, som inte har någon fönsterkod alls);
- **skal**: `vy-shell --screenshot` av `hello`, från start till avslut: huvudprocessen, dess
  flikprocess, första bilden, komposition och PNG;
- **fönster**: fönsterklienten från start tills den skriver `vy-shell: presentation: …`, vilket
  den gör i `resumed` när fönstret, GL-kontexten och AccessKit-adaptern finns och fönstret gjorts
  synligt (inte första presenterade bilden).

| Start | utan, P50 / P95 (ms) | med, P50 / P95 (ms) | med mot utan, median |
| --- | --- | --- | --- |
| flik | 19,0 / 26,5 | 18,9 / 25,9 | −0,8 % |
| skal | 49,7 / 57,4 | 49,7 / 57,6 | +0,2 % |
| fönster | 58,6 / 68,9 | 58,9 / 70,9 | +0,5 % |

200 körningar per bygge och start. Fyra kortare körningar dessförinnan gav samma bild och visar
brusets storlek: utan buss under Xvfb (60 körningar) +1,5 %, +0,7 %, −2,8 %; med buss (100
körningar, två gånger) −0,8 %, −0,8 %, +0,7 % och −0,4 %, −0,1 %, +0,8 %; utan display och buss
(60 körningar; bara flik och skal) −2,0 % och +1,0 %. Skillnaden mellan byggena är alltså mindre
än skillnaden mellan två körningar av samma bygge.

**Varför det inte kostar.** `accesskit_winit::Adapter::with_event_loop_proxy` gör på Linux bara
`accesskit_unix::Adapter::new`, som första gången startar en tråd med egen exekutor
(`accesskit_unix` 0.24.1, `context.rs`, `get_or_init_messages`); det är den tråden som ansluter
till sessionsbussen och frågar `org.a11y.Bus` om tillgänglighet är på. Fönstertråden väntar inte
på D-Bus. Trädet byggs först när en hjälpteknik ber om det (`InitialTreeRequested`), så det som
granskningen föreslog som åtgärd, att börja först vid första tillgänglighetsbegäran, är redan hur
det dyra görs. De 2,4 MB större binären märks inte i starten (sidorna läses in vid behov).
Flikprocessen och `--screenshot` skapar ingen adapter alls.

**Inte mätt:** ett skrivbord där AT-SPI-bussen finns och en skärmläsare är på. Där registrerar
adapterns tråd fönstret hos registret och ber om första trädet, och skalet bygger en ögonblicksbild
per ändrad frame. Det ligger efter att fönstret visats och är arbetet tillgängligheten kräver,
men det går inte att mäta här: `at-spi2-core` (registret) finns inte på maskinen, bara
`dbus-daemon`. `spec_13_1_1_cold_start` (150 ms till tomt fönster) är fortfarande ignorerat; talen
ovan (59–64 ms till visat fönster under Xvfb) är inte den mätningen, men säger var den hamnar.

## Grindar och storlek på grenens spets

Körda på trädet för grenens sista commit. Koden är `3714cdc`; commiten efter den ändrar bara den
här filen. Grenen ändrar `crates/vy-shell`, `crates/vy-runtime`, `crates/vy-render`, `SPEC.md`
(en mening, SPEC-11.3.2, i `456ec32`) och `docs/notes/` (den här filen och talen i `m5-gpu.md`);
ingen `Cargo.toml`, ingen `Cargo.lock`, inget i `crates/vy-abi`, SPEC §5, bilaga A eller C eller
`docs/ABI-1.0.md`. `crates/vy-proto` behövde inte röras.

| Grind | Resultat |
| --- | --- |
| `cargo fmt --all --check` | ingen diff |
| `cargo clippy --workspace --all-targets -- -D warnings` | inga varningar |
| `cargo test --workspace` | **429 gröna, 0 röda, 2 ignorerade** (`spec_13_2_1_raster_budget`, `spec_13_1_1_cold_start`, samma två som förr); `e431a60` hade 412 / 2 |
| `python3 tools/spec_trace.py` | 141 id:n, 141 spårade |
| `tools/build_modules.sh`, `check-abi target/wasm/hello.wasm`, `tools/test_modules.sh` | 19 moduler, ABI 1.0, modulernas egna grindar gröna (`modules/` och `vy-sdk` är orörda här) |
| `cargo run -p vy-cli -- conformance` | 91 av 91 på 0,000 % |
| GL-testet under Xvfb med Mesa (`VY_GL_REQUIRED=1`) | grönt; llvmpipe, OpenGL 4.5 med återställningsavisering. Över 2 nivåer / största avvikelse: `frame_2000` 0,132 % / 41, slumplistor 0,691 % (värsta frame 2,031 %) / 37, tunna strykningar 0,178 % / 41, sidor 0,069 % (värsta 0,319 %) / 40, fönstrets komposition 0,310 % / 22 |
| `check-cross --workspace --all-targets` | rent för `x86_64-pc-windows-msvc` och `aarch64-apple-darwin` |
| Klientens storlek (SPEC-12.3.1) | **24 633 000 byte**, marginal 367 000 mot 25 000 000 (`e431a60`: 24 591 912, alltså +41 088 på grenen). `tools/build_release.sh x86_64-unknown-linux-gnu` med `.cargo/config.toml` undanflyttad, sedan `spec_12_3_1_size_budget` i release. Binären är byte för byte den som byggdes för kallstartsmätningen (SHA-256 `fefb654e…`). Största arbetsenhet: `vy-shell` med 248 320 token av 400 000. |

## Förslag till `spec:`-commit (inte gjort här)

- **SPEC-9.4.2** räknar upp vad nyckeln görs av: modulens SHA-256, wasmtimes version och motorns
  inställningar. Klientens version är nu också med. Meningen motsägs inte, men den säger inte att
  sparad kod aldrig överlever den klientversion som sparade den. Förslag på tillägg sist i meningen:
  "Sparad kod används aldrig av en annan klientversion än den som sparade den." Testerna ovan
  refererar redan SPEC-9.4.2.

## Öppet

Inget av det här är rättat på grenen; ägaren eller koordinatorn avgör.

- **SPEC-9.4.2 nämner inte klientversionen** (förslaget ovan). Koden och testerna gör det.
- **`docs/notes/m5-integration.md`** (rad 182) kallar fortfarande `rymd`s 133 nivåer "en kantpixel".
  Det var ett fel i GL-vägen (0011:4) och är rättat; filen är inte min att ändra.
- **Frågan och den stannade sidans knappar nås bara med pekaren och finns inte i
  tillgänglighetsträdet.** `Shell::a11y_snapshot` bär adressfältet och den körande sidans träd;
  capability-frågan har aldrig funnits där, och "Rensa sidans lagring"-frågan (tillägg 13) och
  knapparna "Ladda om" och "Rensa sidans lagring" på en stannad sida (tillägg 14) ärver det. F5
  laddar om från tangentbordet; frågan går inte att besvara utan pekare. Hör till SPEC-10.2.1.
- **Ett `vy_init` som fäller eller returnerar fel** räknas fortfarande som en avvisad modul och
  visar manifestets reservtext. Bara avbrottet vid den hårda gränsen är en stannad sida (tillägg 14).
- **En panik i klientens egen GL-ritkod under en frame fångas inte** (bara uppsättningen, 0011:1);
  den avslutar klienten. En drivrutin som tappar kontexten utan avisering och utan GL-fel märks
  inte heller (se *Beslut och gränser*). En verklig kontextförlust har inte kunnat framkallas här.
- **GL-vägen är bara körd på Mesa llvmpipe (EGL under Xvfb).** Talen i `docs/notes/m5-gpu.md` och
  gränsen 64 nivåer är därifrån; en riktig GPU kan ge andra kanter.
- **Kallstarten med en skärmläsare igång är inte mätt** (se *Kallstart*).
- **Kompileringscachen på Windows:** de två sakerna under beslutet ovan (en utbrytning som lyckas
  ibland blir säker tills cachen rensas; ingen ACL läses).

### Bara typkontrollerat – att prova på Windows och macOS

`check-cross` typkontrollerar för `x86_64-pc-windows-msvc` och `aarch64-apple-darwin`; inget av
det här har körts där.

- **Windows, GL (0011:1, 0011:5):** WGL-kontexten med återställningsavisering
  (`WGL_ARB_create_context_robustness`), att `glGetGraphicsResetStatus` hittas genom displayens
  laddare, och ordningen core med, core utan, ES med, ES utan. Loggraden vid start säger vilken
  kontext det blev och om en tappad kontext märks.
- **Windows, cachen (0014:1–4):** stämpelfilen `VERSION` och tömningen vid versionsbyte (byte av
  namn över en befintlig fil); `dir_is_private` prövar bara att katalogen finns och inte är en
  länk; en post öppnas efter en titt på namnet (`symlink_metadata`), inte med något som motsvarar
  `O_NOFOLLOW`, så mellan titten och öppningen finns en lucka som inte finns på Unix. Testet
  `spec_9_4_2_code_store_touches_only_its_own_files` är Unix-bara.
- **Windows och macOS, start utan egen binär (0014:2):** `window::run` vägrar starta när
  `current_exe` inte går att läsa. Ska aldrig hända för en installerad klient; meddelandet är
  `cannot find the client's own executable to start tab processes from: …`.
- **macOS:** byggs utan `gl`, så 0011 rör den inte. Cachekatalogens kontroller är Unix-vägen
  (ägare och rättigheter, `O_NOFOLLOW`) och bör bete sig som på Linux, men är inte körda.
- **Alla tre:** "Rensa sidans lagring", vyn för en stannad sida och rättningen av den raderade
  lagringen är plattformsoberoende kod i skalets modell och provade headless på Linux.
- **`tests/cold_start.rs`** mäter fönsterstarten bara där `DISPLAY` eller `WAYLAND_DISPLAY` finns,
  alltså inte på Windows och macOS; flik och skal mäts överallt.
