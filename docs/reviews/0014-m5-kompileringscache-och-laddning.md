# PR #14 – M5: kompileringscachen i skalet, parallella sidladdningar, textfältets delade kod (`m5/polish`, bedömd på `3fbcf36`)

**Vad ändrades.** Huvudprocessen sparar modulens kompilerade maskinkod i `<data>/cache/code` och ger
den till nästa flikprocess som laddas med samma modulbyte (två nya IPC-ramar, tagg 15 och 79).
Sidladdningar går på fyra trådar, en i taget per origin, och `TextBox`-logiken ligger i en
icke-generisk `Field`.

**Spec-id som berörs.** Ny: SPEC-9.4.2 (`spec:`-commit `f46daac` med tester). SPEC-13.1.4, 7.4.5,
9.1.3, 11.4.1.

**Risk.** En sparad post är nativ kod för den flik som läser den. Fel nyckel eller fel tillfälle ger
en sajt kod i en annan sajts flik, också efter omstart. Fel i laddaren ger två pinnar för en origin.

**Bevis.** Läst ur koden och provat, inte taget ur noteringarna:
*Nyckeln* räknas i `Shell::start_page` (`CodeKey::of(&page.wasm)`: SHA-256 över formatets magi, åtta
byte av wasmtimes `precompile_compatibility_hash` och modulens SHA-256) och bärs i `Tab::code`;
`ToShell::Code` har inget fält för nyckel, hash eller sökväg. *En rapport per laddning:*
`code_reported` kräver `State::Loading` och att ingen rapport räknats; varje `start_page` startar en
ny process och ramar från en äldre generation slängs i `Shell::handle`, så en gammal process kan inte
svara på en ny laddning. Fliken svarar före `vy_init` (`TabLoop::load`). *Tak:* 64 MB per post, 256 ×
modulen, 256 MB i katalogen med LRU på filtiden; skrivning till temporär fil och `rename` (ingen
`fsync`; hashen fångar en avhuggen fil). *Katalogen* prövas (ägare, inte skrivbar för andra) i `get`,
`put` och `remove`. *Av* i `headless.rs`, `vy-cli render` och konformansen; ingen offert under
inspelning, och uppspelningen hoppar över tagg 15. *Huvudprocessens `Engine`* används bara i
`CodeKey::of`; `Module::new` och `deserialize` finns bara i `vy-runtime/src/compile_cache.rs`, som
bara flikens `Runtime::load` når. `Engine::new` installerar wasmtimes signalhanterare i hela
processen (på macOS en Mach-port-tråd). *Eget prov:* fönsterklienten under Xvfb med egen
`VY_DATA_DIR`: katalog 0700, post 0600, och andra starten läser posten (filtiden flyttas). De tio
cachetesterna, IPC- och fuzztesterna och de två laddartesterna är gröna i min körning (0015).
*Laddaren:* `Fetcher::turn` håller hela hämtningen med förtroendebeslutet per origin (adressen är
normaliserad), nyckelförrådet är nycklat per origin, inget lås hålls över nätet och inget tas inuti
ett annat. Jag fann ingen väg där A läser B:s pinnar eller två första besök pinnar två gånger.
*Två origins med samma modulbyte* delar post; jag håller med noteringen: före rapporten har processen
bara avkodat huvudprocessens ramar, prövat hash och ABI och kompilerat just de byten. Noteringens
kvarstående risk (a)–(e) stämmer, med tilläggen i fynd 1–4. Fynd, efter allvar:

1. **Medel – sparad kod överlever en klientuppdatering som inte byter wasmtime.** `CodeKey::of`
   har wasmtimes version och inställningar men inte klientens. Rättas en utbrytning i vår egen
   värdkod (`vy-runtime/src/host.rs`) gäller redan förgiftade poster vidare, och ingen knapp rensar.
   Byts wasmtime blir gamla poster i stället oläsbara filer som ligger kvar upp till 256 MB. Repro:
   spara en post, höj `version` i `Cargo.toml`, bygg, ladda: träff. Rättning: klientens version i
   nyckeln, töm katalogen när en versionsfil i den skiljer, och "Rensa cache" i devtools.
2. **Medel – utan `self_exe()` kör huvudprocessen sparad kod.** `window.rs`, `resumed`:
   `exe: self_exe().ok()` med `code_cache: Some(..)`. Utan exe startar `spawn_tab` sidan som en tråd
   i huvudprocessen, utan OS-sandlåda, och `Runtime::load` deserialiserar posten där, mot SPEC-9.4.2.
   Sällsynt men tyst; testerna kör själva så (`shell(None, …, Some(cache))`). Rättning: fönstret
   vägrar starta utan exe (som `--screenshot`), och `start_page` erbjuder kod bara till en process.
   Test: skal utan exe med cache ger `tab_code == None` och ingen post.
3. **Låg – katalogen följs genom en symlänk, och städningen tar andras filer.** `dir_is_private`
   använder `fs::metadata`; `entries` raderar varje `*.tmp` äldre än tio minuter och `evict` varje
   `*.code`. Prov: `cache/code` som symlänk till en annan egen 0700-katalog: posten skrevs dit och en
   gammal `notes.tmp` där raderades. Kräver skrivrätt i `<data>/cache` (`VY_DATA_DIR` på delad
   plats). Rättning: `symlink_metadata`, vägra länk, och rör bara namn av cachens egen form.
4. **Låg – Windows.** Ingen ACL läses, och standardsandlådan hindrar inte läsning eller nät
   (spec-fråga 62), så beständig kod når mer där. Ägarens val: cachen av där tills strict är standard.
5. **Låg – en laddartråd som panikar lämnar sin origin upptagen.** `page.rs`, `start_loader`:
   `queue.done` körs inte efter en panik i `run_job`; originens sidor står sedan på "laddar".
   Rättning: en vakt vars `Drop` anropar `done` och rapporterar felet.
6. **Låg – inaktuella jobb körs klart.** Fem omladdningar mot en hängande origin ger fem hämtningar
   i följd före den sista. Släpp köade jobb för samma flik med äldre generation i `Loader::send`.
7. **Låg – inget vaktar att enradssidor slipper radbrytningskoden** (LLVM:s konstantvikning).
   `form.wasm` är 30 627 byte; lägg ett tak (32 000) i `tools/test_modules.sh`.

**Nya beroenden.** Inga.

**Hotbild.** Ny lagringsyta för nativ kod och två IPC-ramar; ingen ny ABI-import. Hotanalysen i
`notes/m5-polish.md` har de fyra rubrikerna och håller, med fynd 1–4 som tillägg.

**Rekommendation.** **Merge efter** fynd 1 och 2. Fynd 3 och 5–7 som uppföljningar; 4 avgör ägaren.
