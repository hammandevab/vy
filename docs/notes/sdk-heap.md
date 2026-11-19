# SDK:ts allokator: kostnaden beror inte längre på antalet hål

2026-11-19, efter v0.3.0. Ändringen ligger i `crates/vy-sdk/src/heap.rs` och dess tester i
`crates/vy-sdk/src/tests.rs`. Ingen ABI, ingen spec och ingen guldbild är ändrad.

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
- Ett block växer på plats när blocket efter är fritt (`realloc`), så en `Vec` eller `String` som
  fylls på i slutet av högen flyttas inte och kopieras inte.

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
| konformanssviten, summan av 96 mätta fall | 20 522 932 | 20 438 044 | -0,4 % |
| sämsta enskilda fall (`kanban-800x600@1-stored`) | 102 559 | 104 039 | +1,4 % |
| `skriv` på en kvot radbrytningar, 1600 × 1000 | 5 563 239 | 5 115 814 | -8,0 % |
| `skriv` på fientliga anteckningar | 5 555 551 | 5 092 092 | -8,3 % |
| `kalkyl` när värden växer, 2560 × 1440 | 6 442 946 | 6 455 582 | +0,2 % |
| `kalkyl` på långa texter | 6 810 626 | 6 823 905 | +0,2 % |

De fyra sista raderna är budgettesterna (`spec_5_3_3_*` i `vy-cli`); av deras 19 mätta fall
är det sämsta +0,9 % (`kanban` på 415 sparade värden). `kalkyl` tjänar inget: sidan gör sedan
`d4a27e1` inga hål. Minnet är oförändrat i alla 96 fall (19 sidor).

Första versionen, utan det aktuella blocket, var **sämre** utan hål: svitens summa +1,1 %,
`dashboard` +5,7 %, och `skriv` på en kvot radbrytningar +9,2 % (6 072 974 bränsle). Den gick
aldrig ut.

**Storlek.** Allokatorn är med i varje modul, och den nya är drygt 1 000 byte större:
`hello` 5 274 → 6 300, `form` 31 088 → 32 409, `kalkyl` 64 877 → 66 060, `widgets`
56 096 → 57 636 (mest, +1 540). Utskärningen (`carve`) hålls utanför anropsställena; inlinad på
fem ställen kostade den 364 byte till per modul. De två storlekstaken som vaktar att
flerradshalvan av `ui::TextBox` viks bort är höjda med samma marginal som förut:
`form.wasm` 32 000 → 33 300 (`tools/test_modules.sh`) och startsidan 30 000 → 31 100
(`crates/vy-dev/tests/scaffold_builds.rs`). En kall start kompilerar alltså 1 KB mer kod per
sida; med dagens 2–3 ms per KB (SPEC-13.1.4, öppen) är det 2–3 ms.

## Tester

- `spec_4_2_4_heap_within_cap`: som förut, med högens slutmarkering (16 byte) inräknad.
- `spec_4_2_4_heap_keeps_every_block_whole`: 60 000 slumpade steg (samma varje körning) av
  allokering, frigörning och storleksändring från en byte till flera sidor, med minnestaket nått
  på vägen. Varje block behåller sina byte, och var 64:e steg går `Heap::check` igenom hela
  högen: storlekar, grannar, att inga två fria block ligger intill varandra, listorna och deras
  bitar, det aktuella blocket. Till sist är allt ett enda fritt block.
- `spec_5_3_3_heap_cost_does_not_grow_with_the_holes`: med 10 000 hål som inte kan slås ihop går
  50 000 steg utan att ett enda block gås förbi i en sökning.
- `spec_5_3_3_heap_without_holes_touches_no_list`: i en hög utan hål rör 2 000 allokeringar,
  deras frigörning i omvänd ordning och ett block som växer steg för steg till 40 000 byte ingen
  lista alls.

Tio avsiktliga fel i allokatorn (en glömd storlek, en utebliven sammanslagning, en bit som inte
släcks, de två snabba vägarna borttagna eller fel, ett tappat aktuellt block) fälldes alla av
dessa tester.

## Inte gjort

- `kalkyl` har kvar sitt eget skydd (rum reserverat i förväg, tal formaterade på stacken). Det
  behövs inte längre för allokatorns skull men gör sidan billigare, och är inte rört.
- En stor begäran (över 1 024 byte) letar fortfarande efter första passande bland de stora fria
  blocken. De är få i varje sida vi har; en sida med tusentals stora hål av olika storlek
  betalar för det.
- Mätningen med hål är nativ. Ingen sida i sviten gör i dag tusentals hål, så det finns inget
  bränsletal för felet före och efter i en riktig sida.
