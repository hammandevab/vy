# M3, andra delen: vy-shell – fönster, flikar, adressfält, en process per flik

Byggagentens noteringar. Granskningssidan skrivs av en annan agent. Rörda delar: `crates/vy-shell`
(nytt innehåll), små additiva API:er i `vy-runtime`, `vy-render`, `vy-manifest` och `vy-proto`,
`crates/vy-cli` (`--tab`, skalfall i konformanssviten, `bench`), `conformance/cases.toml` och fyra
nya guldbilder, `.github/workflows/ci.yml` (Windows-jobb, klientbygge, skärmdump, bench),
`Cargo.toml` (winit, softbuffer). Inget i `crates/vy-serve`, `Dockerfile`, `release.yml` eller
`tools/publish_site.sh`.

## Arkitektur

```
vy-shell (huvudprocess)                          vy-shell --tab (en process per flik)
┌──────────────────────────────────────┐          ┌──────────────────────────────┐
│ window.rs   winit + softbuffer       │  stdin   │ tab.rs   TabLoop             │
│ shell.rs    flikar, adress, historik │ ───────► │   vy_runtime::Runtime        │
│             grants, tick-rytm,       │  frames  │   ingen rasterisering,      │
│             pekare/tangenter → sida  │ ◄─────── │   inget nätverk, inget      │
│ chrome.rs   display list för chromen │  stdout  │   fönster                    │
│ compose.rs  chrome + sida + status   │          └──────────────────────────────┘
│ page.rs     laddartråd: vy-proto,    │
│             sidornas egna hämtningar │   headless.rs: samma Shell utan fönster
│ vy-render   rasterisering (CPU)      │   (--screenshot, konformans, tester, bench)
└──────────────────────────────────────┘
```

- **SPEC-9.1.1, 9.1.2.** Varje flik är samma körbara fil startad med `--tab` (en binär att
  distribuera). Flikprocessen har bara `Runtime`: den avkodar bilder och formar text för mätning,
  men rasteriserar inte och har inget nätverk – `fetch_begin` går tillbaka till huvudprocessen
  som utför anropet med `vy_proto::Fetcher::request` (ingen Referer, inga omdirigeringar,
  TLS-regeln, 256 MB/min per flik). Huvudprocessen rasteriserar flikens display list med
  flikens stiltabell (`StyleSpec` speglas vid varje frame) och de bilder fliken avkodat
  (`ToShell::Image`, en gång per resurs).
- **IPC (`ipc.rs`, SPEC-11.2.1).** Ram: `u32 len | u8 tag | payload`, little-endian, en kanonisk
  kodning per meddelande, ingen självbeskrivning. `ToTab`: Load, Tick, Event, TextInput, Resize,
  Visibility, Grant, FetchResponse, Suspend, Shutdown, Clipboard, Raster. `ToShell`: Loaded, Frame,
  Idle, Image, Links, Semantic, Log, Navigate, FetchRequest, Storage, Clipboard, Ime, Stopped.
  Tak per ram 257 MB (en maximal avkodad bild). En ram som inte avkodas avslutar förbindelsen –
  aldrig en gissning.
- **Trådar.** UI-tråden äger `Shell`; per flik en skrivartråd (kanal → barnets stdin, så
  UI-tråden aldrig blockerar på en långsam flik) och en läsartråd (barnets stdout → en inkorg,
  `Incoming`); en laddartråd äger `Fetcher` och utför sidladdningar och sidornas egna
  hämtningar i tur och ordning. Varje avsändare väcker fönsterloopen med ett användarevent.
- **Tick-rytm (SPEC-6.2.1).** Skalet skickar `Tick` bara när något hänt (input, svar på en
  hämtning, Resize, Grant) eller när senaste `Frame` sade `wants_more`; högst ett obesvarat
  Tick åt gången. Svaret är alltid `Frame` eller `Idle`, sist i sin skur (loggar, bilder, länkar,
  träd kommer före), så ett svar kan tas som slutet på skuren. Dolda flikar får `Visibility 0`
  (efter `vy_suspend`) och inga Tick alls – 0 CPU i bakgrunden (SPEC-13.3.2); vid visning
  `Resize` om storleken ändrats, `Visibility 1`, Tick.
- **Klocka och slump (SPEC-5.2.11, 11.4.1).** Klockan går aldrig i flikprocessen: varje
  meddelande bär skalets monotona klocka (`Runtime::set_clock_ms`, aldrig bakåt). Slumpen är en
  ChaCha20-ström nycklad med 256 bitar OS-entropi (`getrandom`) som skalet drar per sidinstans och
  skickar i `Load`. Därmed är den inkommande strömmen hela sessionen: `--record` skriver varje ram,
  `--replay` läser dem i rörets ställe och ger samma display lists byte för byte
  (`spec_11_4_1_record_replay`, spelet med slump och klocka). Huvudlöst (skärmdumpar, sviten,
  tester) är klockan 16 ms per Tick och slumpen det fasta fröet, som i `vy-cli render`.
- **Chrome (SPEC-9.6.1).** Flikrad, adressrad (bakåt/framåt/ladda om, fältet med markering och
  caret), markörerna "begränsat läge" (SPEC-7.4.3) och "otillgänglig" (SPEC-10.4.1), zoomprocent,
  capability-pluppar (klick återkallar/beviljar, SPEC-9.2.1), capability-frågan med Tillåt/Neka
  (SPEC-8.2.3), laddningsstapel, och i sidytan vyerna tom flik, hämtar, fel (SPEC-7.2.2, med
  "Öppna … i webbläsaren" för icke-Vy-svar), reserv (SPEC-8.3.1, Markdown-delmängd) och
  stannade (trap, exit, budget, död process). Allt är en display list genom `vy-render` med
  DejaVu; inget toolkit. Sidans bild rasteriseras för sig och blittas i sidytan och ingenstans
  annars: en modul som målar −10 000…10 000 färgar bara sidytan (`spec_9_6_1`).
- **Zoom (Ctrl+Plus/Minus/0)** ändrar `scale` som modulen ser (`vy_resize` med ny logisk storlek
  och skala × zoom); chromen ritas i fönstrets skala.
- **Policy.** Förtroende avgörs i `vy-proto` (`Fetched::grants()`); lokala `.wasm`/`.vyp` körs i
  begränsat läge (ingen origin att lita på en nyckel för). `GrantStore` (textfil) minns
  användarens svar per origin och capability; `Resolved` säger beviljat/fråga. Lagring sparas per
  origin på disk vid suspend, dold flik och avslut (`storage.rs`, SHA-256-namngiven fil).
- **Felsidor.** `LoadError`: NotVy (med extern öppning), Status (404 har egen titel), Fetch
  (nät, manifest, paket, hash), File. En dödad eller kraschad flikprocess ger "Sidan stannade";
  F5 startar en ny process. Andra flikar påverkas inte (`spec_11_2_1_ring_buffer_and_crash_containment`).

## Nya beroenden

Linux-trädet för `vy-shell`: 204 unika crates (vy-cli utan fönster: 161, oförändrat frånsett
vy-shells modell). winit och softbuffer drar tillsammans in 44 unika crates på Linux (de flesta
Wayland/X11-protokoll), färre på Windows och macOS. Alla låsta i `Cargo.lock`.

| Crate | Varför | Varför inte 200 rader själv | Drar in |
| --- | --- | --- | --- |
| `winit` 0.30 (`rwh_06`, `x11`, `wayland`, `wayland-dlopen`; ingen `wayland-csd-adwaita`) | Fönster, input, DPI, IME på Windows, macOS, X11, Wayland. | Fyra plattformars fönster- och inputlager. X11/Wayland laddas vid körning (`dlopen`), så inga systemheaders krävs för att bygga. CSD på Wayland avstått: sparar sctk-adwaita och en fontstack; på GNOME/Wayland saknas då titelrad (Alt+F4 fungerar) – en Linux-följd, Windows är målet. | dpi, raw-window-handle, smol_str, cursor-icon, bitflags, xkbcommon-dl, x11-dl, x11rb, wayland-client/-backend/-protocols, calloop, polling, rustix, memmap2 m.fl. |
| `softbuffer` 0.4 (`x11`, `x11-dlopen`, `wayland`, `wayland-dlopen`; ingen `kms`) | Presenterar den CPU-rasteriserade bilden i fönstret utan GPU-kontext. | Plattformsspecifik blit (GDI, CoreGraphics, XShm/xcb, wl_shm). wgpu är M4/M5-steget bakom samma `vy-render`-väg (SPEC-11.3.2). | tiny-xlib, as-raw-xcb-connection, fastrand, bytemuck |
| `getrandom` 0.2 (fanns via vy-publish och ring) | 32 byte OS-entropi per sidinstans till slumpnyckeln. | OS-anrop per plattform. | 0 extra |

Skrivet själv i stället för beroende: datakatalogen ur tre miljövariabler (`paths.rs`, ingen
`dirs`), extern öppning med `cmd /C start "" <url>` / `open` / `xdg-open` för enbart
`https://` i utskrivbar ASCII (`external.rs`, ingen `open`), ChaCha20-strömmen (40 rader ARX i
`vy-runtime`, verifierad mot RFC 8439-vektorn via OpenSSL), Markdown-delmängden för reserven,
tangentkartan winit → USB HID.

## Rör nu, delat minne sedan (SPEC-11.2.1)

SPEC-11.2.1 säger delat minne med ringbuffert per flik; M3 skickar samma ramar över barnets
stdin/stdout. `IpcTransport` är den enda ytan loopen talar med (`PipeTransport`,
`ChannelTransport` i processen för tester; `ReplayTransport` för uppspelning), och meddelandena är
oberoende av transporten. En ringbuffert i delat minne byter `PipeTransport` mot en
`SharedRingTransport` och tar bort kopian av display listen (idag: en kopia in i röret, en ut).
Kostnaden idag är inte mätbar mot budgeten (en 2 000-opkoders lista är ~60 KB; en 1280×800-ram
rasteriseras på ~1 ms i release – se bench). Bilder går en gång per resurs. Spec-fråga 46.

## Tester: ignorerade → gröna

| Test | Var | Bevis |
| --- | --- | --- |
| spec_11_1_1_crate_boundaries | vy-shell | Cargo-beroendena följer CLAUDE.md; inget SDK i klienten, inget under skalet beror på det |
| spec_2_1_7_https_opens_externally | vy-shell | https:// från adressfält, `navigate` och länkklick → öppnaren; fliken orörd |
| spec_9_1_1_process_isolation | vy-shell | två flikar = två pid ≠ skalets; bilden rasteriserad i huvudprocessen |
| spec_9_2_1_capability_ui | vy-shell | pluppar för net/clipboard/ime på en signerad chat-sida, fråga besvaras, net återkallas → chat "offline", beviljas → "ansluten" |
| spec_9_6_1_chrome_owned_by_client | vy-shell | modul som målar −10 000…10 000: chromen opåverkad, adressen ritad av skalet, zoom rör inte chromen |
| spec_11_2_1_ring_buffer_and_crash_containment | vy-shell | dödad flikprocess: "gone", andra fliken ritar vidare, F5 ny process |
| spec_11_4_1_record_replay | vy-shell | `--record`/`--replay`: identiska display lists; annan nyckel ger annan serve |
| spec_13_3_1_memory_and_background | vy-shell | dold animerande flik får inga frames och ingen CPU-tid (/proc); RSS mäts, gränsen hävdas i release |
| spec_8_3_1_fallback_conditions | vy-manifest | `Manifest::needs_fallback` (avvisad modul eller under min_size) |
| nya | vy-runtime | extern transport, navigeringskö, capability under körning, extern klocka + ChaCha20, RFC-vektor |
| nya | vy-proto | `Fetcher::request` mot testservern (GET/POST/status/limit/TLS-regel/ingen Referer) |
| nya | vy-shell | ipc rundtur och felramar, flikloopens svar, adressfält, historik, grants, lagring, Markdown, chrome-layout, blit, tangentkarta; `tests/bin.rs` kör binären |
| nya | vy-cli | `tests/bin.rs` (`abi-dump`, `--tab`); fyra `shell = true`-fall i sviten |

Kvar ignorerade: spec_13_1_1 (kräver fönster; flikprocessens kallstart mäts i bench), M4-testerna
(AccessKit, textmarkering, fokusordnings-UI, otillgänglig-markören – markören ritas redan),
M5 (reproducerbara byggen), vy-cli:s bench-tester (13.1.2, 13.1.3, 13.2.2, 13.2.3 väntar på tal
från referensmaskinen; `bench` mäter dem), vy-runtime (6.2.2 bevisas nu av 13.3.1-testet på
skalnivå men meningen refereras därifrån; 9.3.1, 9.5.1), vy-render 13.2.1 (mäts i bench).
Backloggen: 29 → 21.

## Vad som kräver ett riktigt fönster – manuell kontroll på Windows

CI bygger `vy-shell.exe` på `windows-latest` och tar en huvudlös skärmdump, men ingen i kedjan
har öppnat fönstret. Steg, i ordning:

1. `cargo build --release -p vy-shell` (eller zip-filen från release.yml). Dubbelklicka
   `target\release\vy-shell.exe`: ett fönster "Vy" 1024×720 med en tom flik och adressfältet
   fokuserat. Ingen konsol ska öppnas (GUI-subsystem i release).
2. Skriv `vy://localhost:8080/hello` (med `vy-serve` igång lokalt, se notes/m3-proto-serve.md)
   och Enter: hello-sidan, fliken får manifestets namn, ingen "begränsat läge"-markör (nyckeln
   pinnas vid första besöket). Skriv en adress till den publicerade sajten (Railway) för
   samma sak över TLS.
3. `vy-shell.exe target\wasm\shop.wasm` från en kommandorad: sidan med "begränsat läge". Rulla
   med hjulet i rutnätet; klicka på ett kort: adressen `vy://shop.example/produkt/…` laddas
   (och misslyckas med felsidan "Sidan kunde inte hämtas" eftersom shop.example inte finns);
   Alt+Vänster tillbaka.
4. Ctrl+T, Ctrl+Tab, Ctrl+W, Ctrl+L, Escape, F5: flikar och fält som väntat. Ctrl+Plus/Minus/0:
   sidan ändrar skala, chromen inte; procenten syns i fältet.
5. Dra fönstret till en skärm med annan DPI (eller ändra skalning): chrome och sida ritas om
   skarpt i ny skala (`ScaleFactorChanged` → `Shell::resize`).
6. form-sidan: Tab till ett fält, skriv svenska tecken, Backspace; IME (t.ex. japansk
   inmatning via Windows IME): commit-texten når fältet, kandidatfönstret placeras vid fältet
   (`ime_begin`-rektangeln → `set_ime_cursor_area`). Förkomposition (`IME_COMPOSE`) levereras
   inte i M3 (se stubbat).
7. Döda en fliks process i Aktivitetshanteraren (`vy-shell.exe --tab`): fliken visar "Sidan
   stannade", andra flikar lever, F5 startar om.
8. Stäng fönstret: alla `--tab`-processer försvinner (Shutdown, annars kill efter 500 ms).

På Linux (X11/Wayland) och macOS: samma steg; på GNOME/Wayland saknas titelrad (ingen CSD).

## Bench (release, byggmaskinen, 10 iterationer)

`cargo run --release -p vy-cli -- bench --iterations 10` på byggmaskinen (en delad VM, inte
referensmaskinen i §13; P95 är brusigt). CI skriver tabellen i varje körning.

```
mätning                                                    spec            p50 ms    p95 ms    budget
kompilering chat (24 KB)                                   SPEC-13.1.4      61.18     85.55        60  ÖVER
kompilering dashboard (22 KB)                              SPEC-13.1.4      58.77     77.83        60  ÖVER
kompilering document (27 KB)                               SPEC-13.1.4      50.18     98.59        60  ÖVER
kompilering form (16 KB)                                   SPEC-13.1.4      38.67     42.20        60
kompilering gallery (16 KB)                                SPEC-13.1.4      38.45     47.24        60
kompilering game (14 KB)                                   SPEC-13.1.4      57.94     59.03        60
kompilering hello (5 KB)                                   SPEC-13.1.4      22.66     23.85        60
kompilering list (18 KB)                                   SPEC-13.1.4      52.24     72.95        60  ÖVER
kompilering settings (22 KB)                               SPEC-13.1.4      49.95     52.96        60
kompilering shop (24 KB)                                   SPEC-13.1.4      56.47     66.90        60  ÖVER
kompilering text (30 KB)                                   SPEC-13.1.4      20.44     21.10        60
rasterisering 2 000 rektanglar 1280×800                    SPEC-13.2.1      17.20     18.19         2  ÖVER
rasterisering 200 textrader à 40 tecken 1280×800           SPEC-13.2.1      25.19     47.04         2  ÖVER
rasterisering 1 800 rektanglar + 200 textrader 1280×800    SPEC-13.2.1      31.77     33.27         2  ÖVER
flikprocess: start, laddning, första display list (hello)  SPEC-13.1.1      20.86     29.91       150
Enter till första bild, huvudlöst (hello, lokal fil)       SPEC-13.1.2      44.39     50.83       100
Enter till första bild, huvudlöst (text, lokal fil)        SPEC-13.1.2      57.60     78.07       100
pekarhändelse till pixel (dashboard)                       SPEC-13.2.2      10.64     19.31        16  ÖVER
```

Läsning:

- **Processmodellen kostar lite.** En flikprocess startar, laddar hello och svarar med första
  display listen på 21 ms; Enter till komponerad bild 44–58 ms huvudlöst (budget 100 vid
  cacheträff). Omladdning/navigering i samma flik startar en ny process (~20–30 ms).
- **Kompileringen** ligger kring budgeten redan för 14–27 KB-moduler (budgeten är 60 ms för
  300 KB). `Runtime::load` räknar hash, kör ABI-kontrollen och kompilerar med Cranelift utan
  kompileringscache; wasmtime 50 bygger här utan parallell kompilering (`parallel-compilation`
  av, se M1-noteringen). Maskinen är långsam, men 300 KB på 60 ms kräver kompileringscachen
  (SPEC-13.1.4 "0 ms vid träff") – M4.
- **Rasteriseringen är den stora avvikelsen (SPEC-13.2.1):** 2 000 rundade, genomskinliga
  rektanglar tar 17 ms (budget 2), 200 textrader 25 ms. Rektanglarna går var och en genom
  tiny-skias pipeline med mask (granskning 0003 fynd 4 tog bort den omaskade snabbvägen för
  guldbildernas skull); texten formas om varje frame (ingen formningscache, SPEC-13.2.3 kräver
  > 95 % träff). Båda är `vy-render`-arbete som inte rörts i den här delen: en formad-text-
  cache nycklad på (stil, bredd, text) och en snabbväg för opaka/raka fyllningar utan
  klipp (med nya guldbilder, spec-fråga 41) är nästa steg mot budgeten. Sidornas verkliga
  frames är mycket mindre än 2 000 ops (shop: ~150), så klienten känns inte långsam, men
  budgeten hålls inte.
- **Input till pixel** 10,6 ms P50 i dashboard (1280×822 sida + chrome): två rundturer över
  rören, flikens frame, rasterisering och kompositering. Chromen rasteriseras bara om dess lista
  ändrats (cache i `compose`). P95 på budgetgränsen på den här maskinen.
- Binären `vy-shell` är 26,2 MB på Linux (release, thin LTO, ostrippad; strippad 21,8 MB).
  SPEC-12.3.1 (≤ 25 MB) är M5-grinden; `strip = true` i release-profilen räcker.

## Stubbat och ogjort

- **Förkomposition (`IME_COMPOSE`)** levereras inte: `Runtime` har ingen väg för den (bara
  `text_input` via `vy_alloc`); commit-text går. M4 (devtools/tillgänglighet) lägger till
  `Runtime::ime_compose`.
- **OS-urklipp.** Urklippet är skalets eget (delas mellan flikar i samma skal, SPEC-5.2.12);
  systemets urklipp kräver `arboard` eller plattformsanrop – M4, med motivering.
- **wgpu.** CPU-raster + softbuffer. Bench säger om det räcker; GPU-vägen är M4/M5.
- **AccessKit** (SPEC-10.2.1): trädet tas emot per frame (`Tab::sem`) men exponeras inte.
- **Wayland CSD** avstått (ovan). **DNS TXT** (SPEC-7.4.2) som förr.
- **`links` i manifestet** verkställs inte (spec-fråga 48).
- **Lagring sparas** vid suspend/dold/avslut, inte vid varje `state_put`; en krasch i
  huvudprocessen mellan två suspend förlorar ändringar sedan senaste.
- **`spec_13_3_1` i debug** rapporterar bara RSS (45 MB per flikprocess i debug på Linux);
  gränsen 400 MB hävdas i release.

## Spec-frågor

45. **SPEC-9.1.2 "GPU-kontext finns endast i huvudprocessen"** – M3 har ingen GPU-kontext alls;
    meningen är uppfylld trivialt. Bör den säga "rasterisering, och GPU-kontext när sådan finns"?
46. **SPEC-11.2.1 delat minne.** Uppfylld i mekanism (ramar per flik över `IpcTransport`) men
    inte i bokstav (rör, inte ringbuffert). Antingen ändras meningen till "över en transport per
    flik; delat minne när kopian mäts" eller så görs ringbufferten i M4. Testet refererar
    meningen och säger vad som gäller.
47. **SPEC-13.1.1 "kallstart till tomt fönster"** kan inte mätas utan fönster; bench mäter
    flikprocessens kallstart till första display list (processhalvan) och Enter-till-bild
    huvudlöst. Bör §13 skilja på fönstrets och flikens kallstart?
48. **`links` i manifestet (SPEC-8.1.2)** – "länkar sidan får navigera till": ska skalet vägra
    `navigate`/`link_register` till adresser utanför listan? M3 verkställer inte; en tom lista
    tolkas som "inga begränsningar".
49. **Capability-frågan ställs vid laddning**, inte vid första användning: sidan körs utan
    capabilityn tills svaret kommer (CAPABILITY-eventet). Alternativet (fråga vid första
    `clipboard_read`) kräver att värden rapporterar nekade försök; är nuvarande ordning avsedd?
50. **Zoom är skala (SPEC-6.3.1).** Ctrl+Plus ger modulen `vy_resize` med mindre logisk yta och
    större `scale`; en sida som layoutar för logiska pixlar får alltså ett smalare fönster.
    Det är webbens "zoom", inte "textstorlek". Bekräftas?
51. **Lokala filer i begränsat läge** (SPEC-7.4.3): en utvecklare får inga capabilities utan en
    server med nyckel. En `--trust-key <hex>`-flagga för lokala paket vore en bekvämlighet; avstått
    tills någon ber om det.
52. **Omladdning i samma flik startar ny process** (origin-gräns, SPEC-9.5.2) – ~30 ms i
    release per navigering (bench). Återanvändning inom samma origin vore möjlig; inte gjort.
