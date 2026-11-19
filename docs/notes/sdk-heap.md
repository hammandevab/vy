# SDK:ts allokator: små block kostar lika mycket hur många hål högen än har

2026-11-19, efter v0.3.0. Ändringen ligger i `crates/vy-sdk/src/heap.rs` och dess tester i
`crates/vy-sdk/src/tests.rs`, i två commits: `8bd1517` och, efter granskning 0019, den som bär
den här versionen av noteringen. Ingen ABI, ingen spec och ingen guldbild är ändrad. Talen
nedan gäller den andra.

## Vad som var fel

Varje sida bär SDK:ts allokator. Den höll sina fria block i en enda lista i adressordning och
tog det första som passade. En frigörning gick genom listan till blockets plats, och en
allokering gick förbi alla hål som var för små. Båda kostade alltså som antalet hål. En sida som
låter tusentals små strängar växa får tusentals hål, och `kalkyl` gick på det över framebudgeten
och stoppades på sitt eget sparade blad (granskning 0018:1). Sidan rättades då så att den inte
gör hål; roten låg kvar och stod som öppen i README.

## Vad som är gjort

- Varje block har i sitt huvud (16 byte, som förut) sin storlek och storleken på blocket före.
  Ett frigjort block hittar båda sina grannar utan att söka och slås alltid ihop med fria grannar.
- Fria block ligger i listor efter storlek: en lista per storlek upp till 1 024 byte (storlekarna
  är multiplar av 16) och en för allt större. En bit per lista som inte är tom ger det minsta
  block som räcker i ett steg. Bara en begäran över 1 024 byte letar, och bara bland de stora.
- Ett fritt block, det aktuella, ligger inte i någon lista. En liten begäran som inget fritt
  block har exakt storleken för skärs ur dess början, och ett block som frigörs intill går
  tillbaka in i det. I en hög utan hål kostar en allokering och en frigörning därför som att
  flytta en pekare. Utan det blev den första versionen dyrare än den gamla just där sidorna är
  tyngst (se nedan).
- Ett block växer på plats när blocket efter är fritt (`realloc`), och högens sista block växer
  in i nya sidor. En `Vec` eller `String` som fylls på i slutet av högen flyttas därför inte och
  kopieras inte, så länge minnestaket inte är nått. Ett block mitt i högen flyttas som förut när
  det som är fritt efter det inte räcker.
- Högen växer med de sidor som fattas: det som redan är fritt sist i den räknas, och
  slutmarkeringen används igen (granskning 0019:2). Den gamla begärde alltid sidor för hela
  blocket.

## Mätt

Allt är mätt på samma maskin i samma session, med modulerna byggda före och efter.

**Med hål (det som var felet).** Båda allokatorerna, nativt, samma arbete: N hål som inte kan
slås ihop, sedan 200 000 allokeringar, frigörningar och storleksändringar av block upp till
1 000 byte. Nanosekunder per steg:

| hål | före | efter |
| ---: | ---: | ---: |
| 0 | 33 | 11 |
| 100 | 184 | 11 |
| 1 000 | 5 252 | 13 |
| 5 000 | 30 805 | 12 |
| 10 000 | 62 491 | 13 |
| 20 000 | 235 271 | 14 |

Mätningen är gjord med ett program utanför repot (den gamla `heap.rs` ur `e41d19e` bredvid den
nya) och är en bild av skillnaden, inte en grind. Det som står kvar som grind är testet nedan.

**Utan hål (det som inte fick bli sämre).** Bränsle i dyraste frame, före och efter:

| | före | efter | |
| --- | ---: | ---: | ---: |
| konformanssviten, summan av 96 mätta fall | 20 522 932 | 20 477 654 | -0,2 % |
| sämsta enskilda fall (`kanban-800x600@1-stored`) | 102 559 | 104 318 | +1,7 % |
| `skriv` på en kvot radbrytningar, 1600 × 1000 | 5 563 239 | 5 176 232 | -7,0 % |
| `skriv` på fientliga anteckningar | 5 555 551 | 4 972 862 | -10,5 % |
| `kalkyl` när värden växer, 2560 × 1440 | 6 442 946 | 6 457 441 | +0,2 % |
| `kalkyl` på långa texter | 6 810 626 | 6 825 809 | +0,2 % |

De fyra sista raderna är budgettesterna (`spec_5_3_3_*` i `vy-cli`); av deras 19 mätta fall
är det sämsta +1,1 % (`kanban` på 415 sparade värden). `kalkyl` tjänar inget: sidan gör sedan
`d4a27e1` inga hål. Minnet är oförändrat i svitens alla 96 fall (19 sidor); sviten har inget
fall där minnet växer, så det säger inget om sidor som gör det.

**Minne.** En buffert som dubblas från 64 byte till 4 MiB sist i högen flyttas aldrig, och
högen är då 65 sidor: buffertens 64 och en för huvud och slutmarkering. Granskaren mätte samma
sak på `8bd1517` till 132 sidor och 6 flyttar, och den gamla allokatorn till 136 sidor och 19.

Första versionen, utan det aktuella blocket, var **sämre** utan hål: svitens summa +1,1 %,
`dashboard` +5,7 %, och `skriv` på en kvot radbrytningar +9,2 % (6 072 974 bränsle). Den gick
aldrig ut.

**Storlek.** Allokatorn är med i varje modul, och den nya är 1 200–1 800 byte större:
`hello` 5 274 → 6 534, `form` 31 088 → 32 636, `kalkyl` 64 877 → 66 287, `widgets`
56 096 → 57 863 (mest, +1 767). Av det är 230 byte det som kom efter granskningen.
Utskärningen (`carve`) hålls utanför anropsställena; inlinad kostade den 364 byte till per
modul. De två storlekstaken som vaktar att flerradshalvan av `ui::TextBox` (omkring 6 500
byte) viks bort är höjda: `form.wasm` 32 000 → 33 300 (`tools/test_modules.sh`; marginalen
var 912 byte och är 664) och startsidan 30 000 → 31 100
(`crates/vy-dev/tests/scaffold_builds.rs`; 30 421 byte nu, marginalen var 1 152 och är 679).
En kall start kompilerar alltså drygt 1 KB mer kod per sida; med dagens 2–3 ms per KB
(SPEC-13.1.4, öppen) är det 3–4 ms.

## Tester

- `spec_4_2_4_heap_within_cap`: som förut, med högens slutmarkering (16 byte) inräknad och
  med att en nekad begäran inte växer något.
- `spec_4_2_4_heap_keeps_every_block_whole`: 60 000 slumpade steg (samma varje körning) av
  allokering, frigörning och storleksändring från en byte till flera sidor, med minnestaket nått
  på vägen. Varje block behåller sina byte, och var 64:e steg går `Heap::check` igenom hela
  högen: storlekar, grannar, att inga två fria block ligger intill varandra, listorna och deras
  bitar, det aktuella blocket. Ett null godtas bara när varken ett fritt block eller de sidor
  som är kvar räcker (0019:3). Till sist är allt ett enda fritt block.
- `spec_4_2_4_heap_in_parts_keeps_every_block_whole`: samma steg när något annat växer minnet
  mellan högens egna tillväxter, så att högen ligger i delar (0019:3). Ingen sida gör så i dag.
- `spec_4_2_4_heap_asks_again_when_pages_do_not_continue`: första gången sidorna inte
  fortsätter högen och därför inte räcker begärs blocket om, helt.
- `spec_4_2_4_heap_grows_by_what_is_missing`: fallen ur 0019:2 (ett litet block och sedan ett
  på en hel sida, eller på 100 000 byte, inom två sidor) och bufferten som dubblas till 4 MiB
  utan att flyttas, på 65 sidor (0019:4).
- `spec_4_2_4_heap_gives_back_and_reuses`: ett hål av exakt rätt storlek tas före det aktuella
  blocket, ett block växer in i ett fritt block efter sig som ligger i en lista, och ett block
  som krymps lämnar tillbaka resten (0019:3).
- `spec_5_3_3_heap_cost_does_not_grow_with_the_holes`: med 10 000 hål som inte kan slås ihop går
  50 000 steg med block upp till 1 024 byte utan att ett enda block gås förbi i en sökning.
- `spec_5_3_3_heap_without_holes_touches_no_list`: i en hög utan hål rör 2 000 allokeringar,
  deras frigörning i omvänd ordning och ett block som växer steg för steg till 40 000 byte ingen
  lista alls.

Tio avsiktliga fel i allokatorn (en glömd storlek, en utebliven sammanslagning, en bit som inte
släcks, de två snabba vägarna borttagna eller fel, ett tappat aktuellt block) fälldes alla av
de fyra första testerna. Granskaren prövade sju andra, och sex gick igenom (0019:3). Efter de
fyra nya testerna fälls fem av de sju, och fem egna fel kring tillväxten. Två överlever: att en
stor begäran tar det aktuella blocket före ett stort hål, som är ett val och inget fel, och
`extend` utan plats för slutmarkeringen, som döljs av att en begäran som sidorna inte räckte
till görs om (i en hög i delar kan det felet kosta sidor, och inget test ser det).

## Granskning 0019

Granskaren fann inget korrekthetsfel. Det som ändrades efter den:

- **0019:1** (texten lovade mer än koden håller): rubriken här, README och `BUILD-NOTES` säger
  nu "block upp till 1 024 byte", och README har tillbaka en öppen punkt om större block.
  Koden för stora block är inte ändrad; det är ägarens val (se nedan).
- **0019:2** (den nya nekade vid taket en begäran som den gamla klarade): rättat, och det ärvda
  fallet med: högen växer med det som fattas.
- **0019:3** (luckor i testerna): fyra nya tester och ett skärpt, ovan.
- **0019:4** (bufferten flyttades när högen måste växa): högens sista block växer in i nya sidor.
- **0019:5** (småsaker i texten): rättade här och i testets kommentar.

## Inte gjort

- `kalkyl` har kvar sitt eget skydd (rum reserverat i förväg, tal formaterade på stacken). Det
  behövs inte längre för allokatorns skull men gör sidan billigare, och är inte rört.
- En stor begäran (över 1 024 byte) letar fortfarande efter första passande bland de stora fria
  blocken. De är få i varje sida vi har; en sida med tusentals stora hål av olika storlek
  betalar för det. Granskaren mätte det (0019:1): 10 000 strängar på 1 100 byte som växer i
  slumpad ordning går förbi 285 block per sträng, och i det renodlade fallet är den nya inte
  bättre än den gamla. Botemedlet är listor per storleksklass också över 1 024 byte. Det kostar
  byte i varje modul och är ägarens val; det står som öppet i README.
- Mätningen med hål är nativ. Ingen sida i sviten gör i dag tusentals hål, så det finns inget
  bränsletal för felet före och efter i en riktig sida.
