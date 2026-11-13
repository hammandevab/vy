# PR #10 – M5: SDK-komponenter, guiden, startsidan och den publicerade sviten (`m5/sdk` + `7d73fe7`, bedömd på `3fbcf36`)

**Vad ändrades.** `vy_sdk::ui` ger sidor färdiga komponenter (`TextBox` i en rad och flera,
`Button`, `Checkbox`, `RadioGroup`, `Slider`, `ScrollView`, `List`, tema och layout) som vanlig
modulkod. Galleriet `modules/widgets` visar dem, `form` är portad till dem, `docs/SDK.md` är en guide
med en hel sida, `vy-dev new` ger en startsida med fält och knapp, och
`tools/package_conformance.sh` paketerar sviten för andra klienter.

**Spec-id som berörs.** Inga nya. Testerna bär SPEC-5.1.6, 5.2.5, 5.2.6, 6.4.3, 8.2.4, 10.1.1,
10.3.1–10.3.3, 10.4.2, 12.1.2, A.2.2. Spec-frågor 68–70 (lista utan fokuserbar roll, hela trädet för
långa listor, tillstånd i nodtexten) är öppna och ärligt beskrivna.

**Risk.** Komponenterna hamnar i varje sida som en utomstående agent skriver (M5-grinden). Ett fel
här blir ett fel i alla de sidorna, och sidans egna tester ser det inte: mocken räknar inte bränsle.

**Bevis.** vy-sdk:s 33 enhetstester och 5 doctester, `sdk_guide_example_builds` (guidens sida byggs
till wasm och körs), `scaffold_builds_to_wasm_and_passes_check_abi` och wasm32-clippy för SDK:t är
gröna i min körning. `form` är byte för byte som förr i alla sex fall (konformans 91/91 à 0,000 %),
men modulen är nu 30 627 byte (16 694 på main enligt noteringen); `widgets` är 55 493. Ett väntevärde
i skalets urklippstest ändrades med avsikt: en inklistrad radbrytning blir ett mellanslag i ett
enradsfält. Läst i `textbox.rs`: markör och markering flyttas per `char` och klampas med
`floor_char_boundary`; inmatning kapas vid teckengräns mot `max_len`; Ctrl+X tar bort markeringen
bara om skrivningen lyckades; `ime_begin`/`ime_end` följer fokus. Paketet byggt här: 91 fall, 19
moduler, 5 755 756 byte, 200 filer som stämmer mot sin `SHA256SUMS`, och `vy-cli conformance --root
<uppackat>` ger 91/91. Bränsle mätt med ett eget prov mot `vy-runtime` (utanför repot, raderat): ett
enradsfält med 1 KB kostar 0,08 M per frame. Fynd, efter allvar:

1. **Hög – ett långt ord utan mellanslag i ett flerradsfält spränger budgeten.** `textbox.rs`,
   `fit_prefix` (ur `wrap_paragraph`): för varje rad bisekteras hela resten av ordet, och varje steg
   mäter ett prefix av den längden, alltså kvadratiskt. Uppmätt i `widgets` (800 × 600): 4 096 byte
   "W" inklistrat ger en frame på **63,9 M bränsle** (budget 8 M, hård gräns 64 M) och 39,7 M efter en
   storleksändring; 1 024 fyrbytestecken 13,0 M; 4 KB vanliga ord 1,25 M. Två sådana frames i rad (en
   fönsterdragning) stoppar sidan, och i ett smalare fält avbryts framen: så dör `skriv` (0013).
   Varken mocken eller `vy-publish build` (inga händelser) ser det. Repro: `Runtime` med
   `widgets.wasm`, tryck i största `textbox`, `text_input(&"W".repeat(4096))`, `frame()`,
   `last_frame().fuel`. Rättning: sök brytpunkten i ett fönster kring förra radens teckenantal, så
   att en rad mäter i storleksordningen sin egen längd; ett test i `vy-cli` som hävdar ≤ 8 M.
2. **Medel – inget verktyg mäter en frame efter händelser.** `vy-publish build` och `vy-dev build`
   kör sidan utan indata, så SPEC-13.4.1 fångar inte fynd 1 eller de i 0013. Rättning:
   `vy-cli conformance --stats` (m5/examples eget förslag) och en gräns per fall i sviten.
3. **Låg – konformanspaketet beror på byggarens umask.** `package_conformance.sh` zippar med
   systemets `zip -X`, som sparar filernas lägen: umask 022 och 002 gav olika SHA-256 för samma träd
   (`5d684288…` mot `74438d70…`), mot SPEC-9.7.1. Rättning: skriv zippen med
   `tools/release_files.py zip`, som releasen redan gör; då finns också bara en zip-skrivare.
4. **Låg – ett tomt textfält har inget namn i trädet.** `Field::draw` ger noden värdet;
   platshållaren når aldrig trädet. `kanban`s tre "Nytt kort"-fält och `kalkyl`s hjälpfält är
   namnlösa `textbox`-noder för en skärmläsare. Rättning: en `draw` som tar fältets namn och
   registrerar det som en `text`-nod före fältet.
5. **Låg – komposition och ovanliga tecken.** `Field::event` tömmer `compose` vid ett tryck utan att
   värden får veta, så IME:n kan lämna sin text vid den nya markören, och en markering visas inte
   ersatt under kompositionen (oprövat med en riktig IME; 0015). `Field::insert` tar bara bort
   `is_control`, så U+2028 och U+2029 kan ge två rader i ett enradsfält.

Kända gränser som noteringen redovisar och som jag bekräftar: Tab når inte `List` (spec-fråga 68),
markören går per `char` och inte per grafemkluster, ingen ångra, ingen inaktiverad knapp.

**Nya beroenden.** Inga; komponenterna är `no_std` + `alloc` i SDK:t.

**Hotbild.** Oförändrad: inget nytt i ABI:t eller klienten. Kopiering går genom kopieringsgesten
(SPEC-8.2.4) och textfältets värde står i trädet, alltså även i sökning och markering; det finns inget
lösenordsfält.

**Rekommendation.** **Merge efter** fynd 1. Fynd 2 bör följa direkt, annars kommer samma slags fel
tillbaka osett. Fynd 3–5 som uppföljningar.
