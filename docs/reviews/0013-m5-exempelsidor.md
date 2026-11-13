# PR #13 – M5: sex exempelsidor – kanban, kalkyl, rita, skriv, block, rymd (`m5/examples`, bedömd på `3fbcf36`)

**Vad ändrades.** Sex större sidor under `modules/` (en tavla med dragbara kort, ett kalkylark med
formler, ett ritbord, en Markdown-redigerare, ett blockspel och en 3D-scen i mjukvara), med
manifestutkast, 27 nya fall, 19 nya träd och `docs/EXEMPEL.md`. Ingen fil utanför `modules/`,
`conformance/` och `docs/` är rörd; sidorna publiceras på sajten av `publish_site.sh`.

**Spec-id som berörs.** Inga nya eller ändrade. Sidorna prövar SPEC-5.3.3, 8.2.x, 10.1.1,
10.3.x, 10.4.2 och 13.4.1 i praktiken.

**Risk.** Sidorna är det ägaren och utomstående kör först och det andra agenter kopierar. En sida som
stannar på sin egen sparade data ser ut som ett fel i formatet, och klienten har ingen knapp som
rensar en sidas lagring.

**Bevis.** Konformans 91/91 à 0,000 % och modulernas nativa tester (19/19 moduler) gröna i min
körning. Storlekar: `skriv` 61 502, `kalkyl` 61 340, `kanban` 54 647, `rymd` 38 963, `rita` 37 376,
`block` 21 746 byte. *Capabilities* är de minsta: `kanban`, `kalkyl`, `skriv` begär `ime` och
`storage`, `rita` och `block` `storage`, `rymd` inget; kopiering går genom gesten. *Eget prov* mot
`vy-runtime` (utanför repot, raderat), bränsle per frame mot budgeten 8 M: utan någon capability går
alla sex i sju fönsterstorlekar från 1 × 1 till 2560 × 1440 @2 utan fel (0,08–0,45 M), och `kanban`
visar "sparas inte" i ett riktigt fönster. Värsta fall som håller: `kanban` 60 kort 1,17 M; `rita`
2 500 punkter 2,16 M och högst 27 % av rasterbudgeten, även efter storleksändring; `kalkyl` 2 600 tal
i 1600 × 1000 2,9 M; `skriv` där varje ord byter stil 4,1–4,4 M, 500 länkar 7,3 M. *Sparad data:*
avkodningarna i `kanban`, `rita` och `block` är lästa (kontrollerade delningar, tak på antal,
`% len` vid indexering) och tålde 300 slumpade och några riktade värden var utan fel; `skriv` och
`kalkyl` tålde de slumpade men inte de riktade (fynd 1–2). *`rymd`:* heltalsvinklar, egen sinus,
cosinus och rot, sortering på heltalsnyckel med index som särskiljare, stjärnor ur en fast följd;
jag fann inget plattformsberoende. *Träden* har rimliga roller och namn med tillståndet i texten
("Färg: svart (vald)", "B4: 32500"). Fynd, efter allvar:

1. **Hög – `skriv` låser sig på sin egen sparade text.** (a) 6 000 byte utan mellanslag: första
   framen 49,3 M vid 1024 × 720, och vid 400 × 700 avbryts den vid den hårda gränsen på 64 M (SDK:ts
   radbrytning, 0010 fynd 1). (b) 6 000 × `[`: framen avbryts också vid 1024 × 720; `md.rs`,
   `link_at` söker `]` till textens slut för varje `[`. Texten sparas i händelsen, före framen
   (`Page::save`), så varje omladdning dör i första framen. Repro: lagringsnyckeln `skriv` =
   6 000 × `[`, sedan `Runtime::frame`: "call aborted after 64 ms". Rättning: 0010 fynd 1, och i
   `inline` ett minne av att ingen `]` följer, så att sökningen görs en gång.
2. **Hög – `kalkyl` låser sig på sitt eget sparade blad.** `sheet.rs`, `recalc`: `WORK_MAX` räknar
   bara områdesbesök, och "en miljon besök ≈ 30 ms" är fel med en faktor fyra. (a) Utan cirklar: 100
   tal i kolumn A och 2 500 × `=SUMMA(A1:A100)`: `init` avbryts vid 64 ms; 2 400 går. (b) 200 celler
   med `=SUMMA(A1:Z100)` (3,6 KB sparat): `init` avbryts; 150 går. Cirkelmarkeringen söker i stacken
   för varje beroende och räknas inte. `commit` räknar om före `save`, men `init` (avkodning och
   omräkning) kostar mer än händelsen, så ett blad nära gränsen sparas och går sedan inte att öppna. Repro: nyckeln `kalkyl` =
   `[1]` + 200 × (index u16, längd 15, `=SUMMA(A1:Z100)`), sedan `Runtime::init`. Rättning: räkna
   varje beroende och markeringssteg i `work`, markera cirklar med en flagga, och sätt taket efter
   mätt bränsle i `vy-runtime` (högst en fjärdedel av den hårda gränsen), inte efter nativ tid.
3. **Medel – `kalkyl` med långa texter i bild går över rasterbudgeten.** 1 260 celler à 200 byte
   (inom kvoten 256 KB): första framen 13,9 M och rasteriseringen avbruten vid 1024 × 720; 25 rader
   × 8 kolumner à 200 byte ger 5,3 M och 43 %. Varje cell ritar hela sin text under ett klipp.
   Rättning: kapa texten till cellens bredd före `dl_text` och spara längden med den mätta bredden.
4. **Låg – inget fall kör en sida utan `storage`**, och sviten kan inte ge ett fall ett sparat
   värde. Läget "sparas inte" och avkodningen av gammal data är alltså oprövade i sviten. Rättning:
   ett fall per sida utan `storage`, och `storage = { nyckel = "hex" }` i `cases.toml`.
5. **Låg – noteringens "sämsta frame 4,1 M, 49 % marginal" gäller de fall som mättes**, inte fynd
   1–3. Mätverktyget raderades; se 0010 fynd 2.

**Nya beroenden.** Inga; varje sida beror bara på `vy-sdk`.

**Hotbild.** Oförändrad: inget nytt i klienten eller ABI:t. Sparad data kommer från samma origin,
så fynd 1–2 är robusthet, inte intrång: en sida kan bara låsa sig själv.

**Rekommendation.** **Merge efter** fynd 1 och 2 (publicera inte `skriv` och `kalkyl` på sajten
före det). Fynd 3–5 som uppföljningar. De 27 guldbilderna är bara körda på Linux (0015).
