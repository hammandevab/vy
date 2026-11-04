# M2 (andra halvan, del B) – fyra referenssidor: chat, shop, document, settings

Skrivet av byggagenten på grenen `m2/pages-b`. Konsolideras av ägaren i `docs/BUILD-NOTES.md`.

## Vad som byggdes

| Modul | Storlek (release, strip) | Fall | Semantiska träd | Exercerar |
| --- | --- | --- | --- | --- |
| `modules/chat` | 23 976 B | 4 | 2 | `now_ms`, `ime_begin`/`ime_end` (även efter `vy_resize`), `TEXT_INPUT`, `fetch_begin` mot listad origin, `FETCH_DONE` + `fetch_read`, `KIND_CAPABILITY`, `SCROLL` i klippt lista, `KIND_FOCUS` |
| `modules/shop` | 22 672 B | 4 | 2 | `dl_image` ur resurser 1–8 (cover), `link_register` per kort och frame, `navigate` (nekas headless), rutnät som flödar om vid `vy_resize`, pekar-träff mot knappar, `list`/`listitem`/`image`/`link`/`button`/`region` |
| `modules/document` | 26 183 B | 6 | 3 | tvåspaltig brödtext balanserad efter `text_measure`, enspaltig under 640 px, `table`/`row`/`cell`, `nav`, noter som `list`, `KIND_FOCUS` + Tab/Skift+Tab med fokusring och rulla-i-bild, Enter → `navigate` eller rulla till avsnitt, `link_register` per frame |
| `modules/settings` | 22 261 B | 3 | 2 | `state_get` vid init, `state_put` vid ändring och i `vy_suspend`, `state_delete` vid återställning, slider dragen med `POINTER_DOWN/MOVE/UP`, `checkbox`/`radio`/`slider`-noder med värde i texten, palettbyte, kortkolumner från 720 px |

Alla fyra ligger långt under 64 KB. Sviten har nu 31 fall (17 nya guldbilder, 9 nya semantiska
träd), alla på 0,000 % avvikelse. Varje sida har `//! # <namn>` överst i `lib.rs` med vad den
exercerar. Bilderna i `modules/shop/assets/` (1–2 KB var) genereras av `generate.py` i samma
katalog så de kan återskapas.

Persistensen i `settings` bevisas av modulens nativa test mot SDK-mocken
(`cd modules/settings && cargo test`, två tester: värden överlever en ny instans; nekad lagring
syns men stoppar inte sidan). Testet körs inte av `cargo test --workspace` eftersom `modules/`
är exkluderat ur workspace – förslag: lägg `cargo test` per modul i CI-steget som redan kör
clippy per modul.

## Ändringar utanför `modules/` och `conformance/`

Ingen ändring i `vy-sdk`, `vy-runtime` eller `vy-render`. I `vy-cli` (konformansläsaren),
minimalt och additivt:

- `resources = ["sökväg", …]` i `cases.toml`: filer relativt repots rot, lästa av
  `run_case_inner` och skickade som `RenderRequest.resources` (`resources[i]` = resurs-id `i + 1`,
  SPEC-4.5.1). Utan det kan ingen sida med bilder ha guldbilder.
- `{kind = "focus", node, gained, keyboard}` som scriptad händelse: levererar `KIND_FOCUS` som
  värdens fokusordning skulle (SPEC-10.3.1). Headless har ingen fokusordning, så fallet namnger
  noden (id ur det semantiska trädet, som är stabilt eftersom sidan registrerar hela trädet).
  `gained` och `keyboard` är `true` om de utelämnas.
- Parsertestet `spec_5_1_3_cases_with_events_and_capabilities` täcker båda nycklarna och
  felfallen (`focus` utan `node`, `gained = 1`, `resources` som sträng eller tal).

## SDK-luckor och ABI-frågor (underlag för ABI-frysen)

Numrerade i den ordning de dök upp. **Fet** = bör avgöras före frysen.

1. **`state_get` för en nyckel som saknas är odefinierat (SPEC-5.2.9).** Mocken svarar
   `E_RESOURCE`, headless-värden `E_DENIED` för allt. `settings` tolkar `E_RESOURCE` som "inget
   sparat" och `E_DENIED` som "lagring otillgänglig". Specen bör säga vilket (förslag:
   `E_RESOURCE`, eftersom det redan betyder "okänd resurs") så att sidor inte behöver gissa.
2. **Headless-värden har ingen lagring ens med `storage` beviljad** (`state_put` svarar
   `E_DENIED (storage (headless host))`). Ett konformansfall kan därför varken skriva, läsa eller
   initiera om modulen med bevarat tillstånd; `vy_suspend` går inte heller att scripta.
   Förslag: en minneslagring per `render` i headless-värden plus scriptade händelser
   `{kind = "suspend"}` och `{kind = "reinit"}` (eller `state = {nyckel = "bytes"}` som
   förinställning i fallet). Tills dess bevisar modulens nativa test persistensen.
3. **SPEC-10.3.1 är tvetydig om Tab/Enter/Escape.** Meningen säger att de "levereras som
   `KIND_FOCUS`-event", men `FOCUS` bär bara (nod, in/ut, mus/tangentbord): varken aktivering
   (Enter) eller avbryt (Escape) kan uttryckas, och det står inte om modulen *också* får
   `KEY_DOWN` för Tab. Headless levererar `KEY_DOWN`; `form`, `document` och `settings` hanterar
   därför både `KEY_DOWN` (lokal fokusordning) och `FOCUS` (värdens). Före frysen: antingen ett
   `KIND_ACTIVATE` (nod-id, källa) eller `FOCUS` med `c` som orsak, och en mening om att Tab
   konsumeras av värden.
4. **Ska det semantiska trädet innehålla noder utanför fönstret?** `text` registrerar bara
   synliga block; `document` och `settings` registrerar allt (med negativa y) så att nod-id blir
   stabila och en fokusordning ur trädet når alla länkar. Specen (SPEC-10.1.1) säger inget.
   Förslag: hela trädet, värden klipper själv; annars kan en värd aldrig tabba till en länk
   nedanför fönstret.
5. **Fasta nod-id kolliderar med automatiska.** `Sem::node_with_id(id)` flyttar `next_id` förbi
   `id`, så ett fast id som ligger över de automatiska (t.ex. 100) gör att nästa automatiska nod
   får 101 – och nästa fasta id 101 blir en dubblett. `form` klarar sig med 10–13 under de
   automatiska; `chat` använder 1000/1001 sist i trädet; `document`/`settings` undviker fasta id
   och läser i stället av vilket id varje länk/kontroll fick i senaste framen. SDK:t bör ge ett
   eget id-rum (negativa id, eller `Sem::reserve(n)` i början av framen) eller låta
   `node_with_id` inte påverka `next_id`.
6. **`vy_abi::cap` nås inte via SDK:t.** `Event::Capability { id }` bär ett capability-id men
   `vy-sdk` re-exporterar inte `vy_abi::cap`, så `chat` beror på `vy-abi` direkt för att känna
   igen `net`. Förslag: `Event::Capability { cap: Capability }` med en enum i SDK:t (och
   konstanterna i bilaga A, se spec-fråga 12 i BUILD-NOTES).
7. **`fetch_begin` headless svarar alltid `E_DENIED`**, även med `net` beviljad, och `E_ORIGIN`
   uppstår aldrig eftersom manifestet inte läses. Offline-vägen i `chat` täcks, men inte
   `E_ORIGIN`-grenen, `FETCH_DONE` eller `fetch_read`. Förslag när manifestläsaren finns (M3):
   headless svarar `E_ORIGIN` för origin utanför listan och tar scriptade svar
   `{kind = "fetch_done", request, status, body}` för listade.
8. **`FETCH_DONE.status` "HTTP-status eller negativ felkod"** står bara i SDK:ts doc, inte i
   bilaga A. `chat` hanterar båda (`VyError::from_code` för negativa). Bör vara normativt, liksom
   när svarskroppen frigörs (efter `fetch_read`? efter nästa `fetch_begin`? `fetch_cancel`?).
9. **`now_ms` är monoton utan epok** (SPEC-5.2.11): en sida kan inte visa klockslag. `chat`
   låtsas att `now_ms = 0` är 09:12. Avgör om det är avsiktligt (fingeravtryck) eller om en
   `clock_ms`/`PREFS` med tidszon hör till 1.1 (§16 nämner `prefs_get`).
10. **Högerställd text kostar två formningar.** `dl_text` returnerar bara höjd, så ett högerställt
    pris, en tidsstämpel eller en tabellcell kräver `text_measure` + `dl_text` på samma sträng –
    dubbel värdkostnad mot budgeten (SPEC-5.3.3) och det vanligaste mönstret i tre av fyra
    sidor. Förslag: `dl_text` returnerar `i64` med bredd × 100 i de höga bitarna (samma packning
    som `text_measure`), eller en `align`-parameter. Ändringen är bakåtkompatibel i wasm bara
    om den görs före frysen.
11. **Länkars livslängd (SPEC-5.2.10).** `link_register` byts ut per id i värden och lever över
    frames, så en sida som rullar måste registrera om varje länk varje frame (shop, document gör
    det) – annars pekar länken fel. Specen säger inte om länkar hör till framen (som trädet) eller
    instansen. Förslag: per frame, nollställda vid `dl_begin`, så att samma regel gäller som för
    `sem_node`.
12. **`dl_clip_push` saknar radie.** Bilder i rundade kort får fyrkantiga hörn (`shop`); en
    `radius`-parameter på klippet skulle ge samma uttryck som `dl_rect`. Liten sak, men den är
    synlig på varje kort med bild.
13. **Vem ritar fokusringen?** Med en fokusordning i värden (SPEC-10.3.1) vet värden exakt vilken
    nod som har fokus och dess rektangel ur trädet; i dag ritar varje sida sin egen ring
    (`form`, `document`, `settings`) med egna färger. Avgör: värden ritar (konsekvent, mindre
    kod per sida) eller modulen (sidan styr utseendet). Sidorna här antar modulen.
14. **`Column`/`Row` används inte av någon av de fyra sidorna.** Alla fyra räknar rektanglar själva
    i en `layout()`-funktion, därför att träfftest (`POINTER_DOWN`) och `ime_begin` behöver samma
    rektanglar *utanför* framen. SDK:ts layout är rit-tid-bunden. Ett litet retained-läge
    (`Layout` som byggs i `init`/`resize`, frågas i `event` och ritas i `frame`) vore det
    kanoniska mönstret; det som finns nu uppmuntrar dubbel aritmetik.
15. **`floor`/`round` saknas i `core`.** Sidorna snäpper med `as i32`-cast; vid skala 1,5 blir
    1 px-linjer suddiga. SDK:t kunde ge `snap(x, scale)` så att linjer och rutnät hamnar på
    fysiska pixlar – relevant för `text-1024x768@1.5` och alla tabellinjer.
16. **Ingen scriptad `resize` i fallen.** `vy_resize` exerceras bara via `vy_init` i olika
    storlekar; omflödet (document, shop, settings) körs i koden men ingen guldbild visar
    "samma instans efter fönsterändring". Rastreringsstorleken är bunden till fallet, så
    `{kind = "resize", width, height, scale}` kräver att sista storleken vinner.
17. **Manifestets JSON-form saknar plats för `net.origins` och `storage.quota_kb`** när
    `capabilities` är en lista med namn. `chat.json` och `settings.json` lägger dem som syskon
    (`"net": {"origins": [...]}`, `"storage": {"quota_kb": 16}`) tills `vy-manifest` avgör
    formen (objekt per capability vore kanoniskt, SPEC-8.2.1).
18. **Loggbrus headless.** Varje `settings`-fall loggar `state_get: E_DENIED` och varje ändring
    `state_put: E_DENIED` – korrekt enligt SPEC-5.3.1 men det gör loggen oanvändbar för att
    hitta riktiga fel. Förslag: nivå `Debug` för "nekad i headless", `Warn` för "nekad för att
    capability saknas".
19. `text_caret` layoutar utan radbrytning (sedan M1); räcker för enradiga fält i `chat`, men
    en flerradig kompositör går inte att bygga. Noteras igen eftersom en chatt är den sida som
    först vill ha den.

## Approximationer i de fyra sidorna

- `chat` visar tidsstämplar som `BASE + now_ms`; i headless går klockan 16 ms/frame så alla
  skickade meddelanden stämplas 09:12.
- `shop` skalar 240 px-bilder med närmaste granne (M1-rasteriseraren); vid 2× syns pixlarna.
- `document` bryter spalterna bara mellan block (en rubrik avslutar aldrig vänsterspalten) och
  klipper första tabellkolumnen i stället för att bryta rad i celler.
- `settings` sparar inte under pågående dragning, bara vid `POINTER_UP` – en avsiktlig
  tolkning för att inte skriva lagring 60 gånger per sekund.
