# PR #11 – M5: rasterisering – snabb CPU-väg, OpenGL-väg och toleransen i SPEC-11.3.2 (`m5/gpu`, bedömd på `3fbcf36`)

**Vad ändrades.** Fönstret ritar sidan med `vy_render::Renderer`: en CPU-väg som ger referensens
byte men bara ritar om det som ändrats och delar ytan i band, och ovanpå den en OpenGL-väg (Linux
och Windows, inte macOS) som ritar sidan i en framebuffer och lägger skalets bild över. Kedjan är GL →
snabb CPU-väg → referensen; headless och konformans går alltid på referensen.

**Spec-id som berörs.** SPEC-11.3.2 (två `spec:`-commits: byte för byte för CPU-vägen, högst 3 %
pixlar över 2 nivåer för GPU-vägen), SPEC-13.2.1, 9.6.1, 9.1.2, 13.2.2. Spec-fråga 66.

**Risk.** Kvarglömda pixlar, sidinnehåll utanför sidytan eller över adressfältet, ett tomt fönster
när GL fallerar, och en tolerans som godkänner mer än den mätt.

**Bevis.** *CPU-vägen:* de fyra testerna i `renderer.rs` och sidtestet i `vy-cli` (alla 19 moduler,
två storlekar) är gröna; konformans 91/91 à 0,000 %. Läst: skadan är unionen av fotavtrycken mellan
första och sista ändrade opkod i båda listorna, med omslutande klipp jämförda; regionen vitmålas
innan den ritas (`raster::draw_region`), så de delade skissdukarna läcker inget mellan flikar; ny
storlek, skala, bildgeneration eller avbruten budget ritar allt. *GL-testet*, kört här under Xvfb
med Mesa llvmpipe (`VY_GL_REQUIRED=1`): `frame_2000` 4 frames, 0,132 % pixlar över 2 nivåer (värsta
0,147 %, största avvikelse 41); slumplistor 120 frames, 0,691 % (värsta 2,031 %, max 37); sidor **195
frames, 0,095 %, värsta 0,387 % (`rymd` 800 × 600), max 133**. Inom 3 %: toleransen är ärlig mot
testet. *Eget fönsterprov* (testet läser bara sidans framebuffer, se fynd 2): klienten under Xvfb i
1024 × 720 med `text`, `kanban`, `kalkyl`, `skriv` och `shop`: GL-fönstret mot CPU-fönstret
0,001–0,104 % pixlar över 2 nivåer (max 22) och **0 avvikande pixlar ovanför sidytan**; CPU-fönstret
mot `--screenshot` 0 pixlar. Med `VY_NO_GL` tar starten CPU-vägen med en rad på stderr; på en
8-bitars skärm ger EGL fel och klienten avslutar med besked (softbuffer stöder inte visualen).
**SPEC-13.2.1 är varken hållen eller mätt:** CPU-vägen tar ≈ 11 ms för en hel ram enligt
noteringen (testet är ignorerat), och GL-vägen har bara körts på llvmpipe. Fynd, efter allvar:

1. **Medel – klienten panikar om EGL inte erbjuder någon konfiguration.** `vy-shell/src/gl.rs`,
   `pick`: `.expect("glutin offers at least one config")`. glutins EGL-del svarar `Ok` med en filtrerad
   iterator som kan vara tom (`glutin/src/api/egl/config.rs`, `find_configs`), och
   `DisplayBuilder::build` anropar väljaren ändå. Då dör klienten vid start i stället för att ta
   CPU-vägen, just där reserven behövs. Inte framkallat här (8, 16 och 30 bitars Xvfb gav EGL-fel
   eller en kontext); repro: `pick(Box::new(std::iter::empty()))`. Rättning: gör uppslaget utan
   `expect` eller fånga paniken runt `crate::gl::open` och svara `NoGl`.
2. **Medel – fönstrets GL-komposition har inget test.** `GlRaster::present` (sidans plats, saxen,
   skalets bild över) körs aldrig av `tests/gl.rs`, så SPEC-9.6.1 i GL-läget vilar på läsning och
   på provet ovan. Rättning: kör `present` med en skalram i testet, läs standardframebuffern och
   kräv chromens pixlar byte för byte och sidpixlar bara inom `clip`.
3. **Låg – en lista som referensen vägrar visar förra flikens sida.** `window.rs`, `gpu::present`:
   `Err(GlError::Render(_)) => {}` sätter ändå `*drawn` och `at`, så framebufferns gamla innehåll
   (senast ritade flik) läggs under den aktiva flikens adress; CPU-vägen visar inget i samma läge.
   Kräver en flikprocess som skickar en oavkodbar lista. Rättning: `at = None` vid det felet; test
   med en skräplista efter ett flikbyte.
4. **Låg – noteringens toleranstal gäller inte HEAD, och toleransen har ingen gräns för storleken.**
   Noteringen säger 88 sidframes, värst 0,25 %, max 40; med exempelsidorna är det talen ovan. Marginalen
   mot slumplistorna är 1,5 gånger, på en enda mjukvarudrivrutin, och meningen tillåter att 3 % av
   pixlarna är hur fel som helst. Ingen har sett efter var `rymd`s 133 nivåer sitter. Rättning: för in
   talen, lägg en gräns för största avvikelse per klass i testet, och kör det på en riktig GPU.
5. **Låg – en tappad kontext märks bara om GL säger det** (`gl_error`, `swap`). En
   drivrutinsåterställning utan fel ger ett svart fönster utan reserv. Och i `App::leave_gpu`: går
   `cpu_surface` inte att skapa står fönstret kvar med sista bilden; avsluta som startvägen gör.
6. **Låg – testerna byter aldrig storlek, skala eller stil på en och samma `Renderer`.** Vägen är
   kort (`same_canvas` falskt ger hel ram), men ett sådant steg i
   `spec_11_3_2_damage_draws_what_the_reference_draws` kostar tio rader.

**Nya beroenden.** `glow 0.17`, `glutin 0.32` och `glutin-winit 0.5` (nio crates i låsfilen; EGL och
WGL, inte på macOS), motiverade i `notes/m5-gpu.md`: +247 624 byte mot wgpu:s 3,9–5,0 MB.

**Hotbild.** Ingen ny ABI-import. GL-kontexten finns bara i huvudprocessen (SPEC-9.1.2), men
grafikdrivrutinen får nu geometri och texturer härledda ur sidans lista; skuggningarna är fasta och
sidan kan inte ge källkod. Sidan ritas med sax mot sidytan och skalets bild läggs över (fynd 2).

**Rekommendation.** **Merge efter** fynd 1 och 2. Fynd 3–6 som uppföljningar. 2 ms-mätningen och
fynd 5 står i checklistan för ägarens dator (0015).
