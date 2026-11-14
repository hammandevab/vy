# Exempelsidor

Sex större sidor under `modules/`, skrivna mot `vy-sdk` som vilken sida som helst. De visar vad
formatet klarar utöver referenssidorna och prövar att `vy_sdk::ui` räcker för riktiga appar. Varje
sida är en egen crate med bara `vy-sdk` som beroende, har sitt manifestutkast i
`conformance/manifests/<namn>.json` och sina fall i `conformance/cases.toml`.

| Sida | Modul | Capabilities | Storlek |
| --- | --- | --- | --- |
| Tavlan | `kanban` | `ime`, `storage` 16 KB | 54 KB |
| Kalkylark | `kalkyl` | `ime`, `storage` 256 KB | 64 KB |
| Ritbord | `rita` | `storage` 64 KB | 37 KB |
| Anteckningar | `skriv` | `ime`, `storage` 16 KB | 61 KB |
| Block | `block` | `storage` 1 KB | 22 KB |
| Rymd | `rymd` | inga | 39 KB |

Gemensamt: Tab och Skift+Tab går mellan kontrollerna (fokus är värdens, SPEC-10.3.1) och sidan
ritar ringen. Fönster från 400 × 600 till 1600 × 1000 och skalorna 1, 1,5 och 2. Utan `storage`
(nekad, eller begränsat läge enligt SPEC-7.4.3) fungerar sidan som vanligt med tillståndet i minnet
och skriver "sparas inte" i sin statusrad. Utan `ime` nekas `ime_begin` (SPEC-5.2.6): fälten tar
fortfarande den text tangentbordet ger (`TEXT_INPUT`), men ingen komposition öppnas vid fältet.

Sparad data läses som indata. Ett värde som sidan inte själv kan ha sparat (okänt format, avklippt,
fler kort, punkter eller mer bläck än sidan tar emot, en cell två gånger) ersätts av exemplet eller
en tom sida, och det sidan visar håller budgeten vad värdet än är. `budget_tests` i `vy-cli` startar
varje sida på 300 slumpade värden, på avklippta värden och på hela sin kvot (granskning 0013).

## Köra dem lokalt

```sh
tools/publish_site.sh /tmp/vy-sajt     # bygger modulerna och signerar varje utkast till <namn>.vyp
cargo run -p vy-serve -- --dir /tmp/vy-sajt --port 8090 --public-key /tmp/vy-sajt/dev.key.pub
cargo run -p vy-shell -- vy://localhost:8090/kanban
```

Byt `kanban` mot `kalkyl`, `rita`, `skriv`, `block` eller `rymd`. Utan `--public-key` kör servern
varje sida i begränsat läge, vilket är ett enkelt sätt att se sidorna utan capabilities. En enskild
bild utan klient: `cargo run -p vy-cli -- render target/wasm/rymd.wasm --out rymd.png --size 1280x800`.

## Tavlan (`kanban`)

**Visar:** dra och släpp på råa pekarhändelser, med en lucka som visar var kortet hamnar och en kort
glidning på plats (`request_frame` och `dt_ms`); fokus som följer ett kort när det flyttas; ett
textfält som tar över kortets nod för redigering på plats; tre oberoende rullytor.

**Används:** tre kolumner (Att göra, Pågår, Klart). Dra ett kort till en annan plats eller kolumn;
Escape avbryter draget. Skriv i fältet längst ned i en kolumn och tryck Enter eller "+" för ett
nytt kort. Med fokus på ett kort flyttar piltangenterna det, Enter, F2 eller dubbelklick redigerar
titeln (Enter sparar, Escape ångrar) och Delete tar bort det; "×" på kortet gör detsamma. Under
640 px visas en kolumn i taget bakom flikar, och ett kort som släpps på en flik flyttas dit.

**Capabilities:** `ime` för textfälten, `storage` för tavlan (sparas efter varje ändring).

## Kalkylark (`kalkyl`)

**Visar:** ett virtualiserat rutnät på 26 × 100 celler med fasta rubriker och rullning i två led;
formler med `+ - * /`, parenteser, cellreferenser, områden och SUMMA, MEDEL, MIN, MAX, ANTAL,
omräknade i beroendeordning med `#CIRKEL` för cykler; fastkommatal som modulen formaterar själv, så
resultatet aldrig beror på plattformens flyttal (SPEC-2.1.5).

**Används:** klicka på en cell och skriv; texten hamnar i formelfältet. Enter bekräftar och går ned,
Tab bekräftar och går höger, Escape ångrar. Piltangenterna, Page Up, Page Down och Home flyttar
markeringen, med Skift utökas den; dra med pekaren för ett område. Dubbelklick eller F2 redigerar,
Delete tömmer. Ctrl+C kopierar markeringens visade värden som tabbavgränsad text (kopieringsgesten,
SPEC-8.2.4, kräver ingen capability). Hjulet rullar raderna, Skift+hjul kolumnerna.

**Capabilities:** `ime` för formelfältet, `storage` för cellernas källtext.

**Gränser:** en omräkning görs i steg om lika mycket arbete, ett per frame, så ett stort blad räknas
över flera frames medan statusraden visar "räknar…" och cellerna behåller sina förra värden (en ny
formel visar `…`). En formel läser högst två hela blad genom sina områden; fler ger `#FEL`.
En frame kostar vad cellerna visar, inte vad de rymmer: av en text som kapas vid sin cell skickas
början, högst 28 tecken (det smalaste tecknet är 3,6 px och cellen 96), och i ett fönster med fler
än 580 sådana celler i bild delar de på 16 KB. Trädet har varje cell; långa texter står hela så
länge de tillsammans ryms i 12 KB, därefter cellens namn, textens början och `…` (hela texten står
i formelfältet när cellen är markerad). Texter som kan rymmas i tomma celler till höger mäts en
gång, högst 8 KB per frame. Ur samma 8 KB mäts tal som är för breda för sin cell, upp till sju
gånger var medan de tappar decimaler; tal som inte hinner med ritas av nästa frame. En cells
etikett får sin plats innan den skrivs och en formels rymmer vilket värde som helst, så en
omräkning flyttar inget på högen (granskning 0018: 2 599 formler som fick långa värden stoppade
sidan för gott). I ett fönster med fler än 580 celler i bild är omräkningens steg så mycket
mindre som det finns mer att rita; i 2560 × 1440 kostar den dyraste framen 6,4 M av 8.

## Ritbord (`rita`)

**Visar:** frihandsstreck som förenklas (Ramer–Douglas–Peucker) och ritas som en `PATH` per streck,
med kurvor i böjarna och raka linjer till hörnen; en ritning som hålls inom budgetarna (högst 2 500
punkter och 2 000 000 i bläck, längd gånger bredd i papperets enheter, sedan säger sidan ifrån,
och en sparad ritning med mer läses inte); ångra och gör om; en dialog som håller Tab inom sig.

**Används:** dra på papperet för att rita. Verktygsraden har åtta färger, tre pennbredder, sudd (tar
bort de streck det rör vid), Ångra, Gör om och Rensa, som frågar en gång. Ctrl+Z ångrar, Ctrl+Y gör
om; Escape stänger dialogen. Papperet har fast storlek och skalas till fönstret.

**Capabilities:** `storage` för strecken, några byte per punkt.

## Anteckningar (`skriv`)

**Visar:** en Markdown-redigerare med förhandsvisning där modulen själv bryter och ritar rik text:
rubriker, stycken, fet stil, kod, citat, listor i två nivåer, linjer och länkar. Länkarna registreras
med `link_register` och finns i trädet, så Tab når dem och Enter följer dem.

**Används:** skriv i fältet till vänster; förhandsvisningen till höger uppdateras för varje tecken
och följer markören. Statusraden räknar ord och tecken. Under 720 px ersätts delningen av flikarna
"Skriv" och "Läs". Betoning (`*text*`) sätts i seriftypsnitt, eftersom `text_style` saknar lutning.

**Capabilities:** `ime` för textfältet, `storage` för texten.

## Block (`block`)

**Visar:** en spelslinga i fasta steg om 16 ms ur `dt_ms`, så samma händelser ger samma spel vid
varje bildfrekvens; slump ur värdens `random`; paus när fliken göms; spelet i det semantiska trädet.

**Används:** Enter eller "Starta" börjar. Pil vänster och höger flyttar biten, pil upp eller X
roterar medurs, Z moturs, pil ned släpper mjukt, mellanslag släpper hårt, C eller H lägger biten i
håll, P eller Escape pausar. Knapparna på sidan gör detsamma med pekaren och upprepar medan de
hålls. Fulla rader blinkar och försvinner; var tionde rad höjer nivån och farten.

**Capabilities:** `storage` för bästa poäng.

## Rymd (`rymd`)

**Visar:** 3D utan 3D-API. Sidan vrider en kropp, projicerar den, väljer bort ytor som är vända från
ögat, belyser varje yta från ett håll och ritar dem bakifrån som fyllda `PATH`-ytor, över ett
stjärnfält med parallax. Sinus och cosinus är sidans egna med heltalsvinklar, så samma indata ger
samma display list byte för byte (SPEC-2.1.5).

**Används:** dra i scenen för att vrida, rulla för att zooma. Piltangenterna vrider, Page Up och
Page Down zoomar, Home återställer vyn. Knapparna väljer ikosaeder, torus eller kub; "Trådmodell"
ritar kanterna i stället för ytorna, "Rotera" slår av och på den egna rotationen (av vilar sidan
och begär inga frames), och reglaget väljer 0–800 partiklar. Avläsningen visar bildens
ritoperationer och tiden sedan förra bilden.

**Capabilities:** inga.
