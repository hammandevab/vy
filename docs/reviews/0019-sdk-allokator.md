# Granskning 0019 – sdk: allokatorn som inte letar bland hålen

Gren `m5/integration`, commit `8bd1517`. Granskat: `crates/vy-sdk/src/heap.rs` (hela),
hög-testerna i `crates/vy-sdk/src/tests.rs`, de två höjda taken, `docs/notes/sdk-heap.md` och
commit-meddelandet. Granskaren skrev ingen av koden.

**Rekommendation.** **Klar efter att texten i fynd 1 är rättad.** Jag hittade inget
korrekthetsfel i allokatorn, trots 860 000 egna slumpade steg mot den. Roten till 0018:1 är
borta för block upp till 1 024 byte, vilket enligt noteringen är fallet som stoppade `kalkyl`.
För större block är den kvar, och det säger README inte längre.

**Det som blockerar.** Bara textdelen av fynd 1: README och `docs/BUILD-NOTES.md` lovar mer än
koden håller. Fynd 2–4 är låga och kan tas efteråt.

**Vad ändrades.** SDK:ts globala allokator, som varje sida bär, är utbytt: varje block känner
sina grannar, fria block ligger i listor efter storlek med en bitkarta, ett "aktuellt" block
utanför listorna gör en hög utan hål lika billig som förut, och `realloc` växer på plats. Två
storlekstak är höjda eftersom varje modul blev drygt 1 000 byte större.

**Spec-id som berörs.** SPEC-4.2.4 och SPEC-5.3.3 (testerna bär dem). Ingen ABI, ingen
normativ mening och ingen guldbild är ändrad.

**Risk.** Allokatorn ligger i alla 19 sidor och i varje sida en utomstående bygger. Ett fel i
den ger förstörda data, fel bild eller en trap i sidan – i alla sidor på en gång, och svårt att
härleda. Värden drabbas inte: felet stannar i sidans sandlåda.

**Bevis.** Eget, i en skrotrot under `/tmp` mot en kopia av `heap.rs` som är byte för byte
lika med repots: två slumpade prov (område 1), sju avsiktliga fel mot repots fyra tester
(fynd 3), fem mätningar mot den gamla allokatorn (fynd 1, 2, 4). I repot: de fyra hög-testerna
går (4 av 4), `scaffold_builds` går (2 av 2), författarens tidsprogram omkört. Inget i repot är
ändrat utom den här sidan. Proven ligger i `/tmp/heaprev` (`src/adv.rs`, `mutants.py`).

**Nya beroenden.** Inga.

**Hotbild.** Oförändrad. Inget i ABI:t är rört, och en felande allokator kan bara skada sin
egen sida.

## Fynd

1. **Medel – "kostnaden beror inte på antalet hål" gäller bara block upp till 1 024 byte.**
   En begäran över 1 008 byte letar i listan med stora fria block efter det första som räcker
   och går förbi alla som är för små. Fall: en sida håller N strängar på 1 100 byte som i
   slumpad ordning växer till 2 300 – samma mönster som stoppade `kalkyl`, med längre texter.

   | strängar | förbigångna block per sträng | sämsta enskilda |
   | ---: | ---: | ---: |
   | 500 | 14 | 158 |
   | 2 000 | 53 | 565 |
   | 5 000 | 148 | 1 399 |
   | 10 000 | 285 | 2 773 |

   Med 10 000 strängar gås 2,85 miljoner block förbi i ett pass. Nativt är den nya ändå fyra
   gånger snabbare än den gamla i det provet (5,2 mot 22 µs per sträng), men kostnaden växer
   med antalet. Samma prov med små strängar (100 → 300 byte) ger 0, så för dem håller
   påståendet. Renodlat: med N hål på 1 056–1 552
   byte går varje begäran på 2 000 byte förbi alla N (20 000 hål: 20 600 block per allokering).
   Där är den nya inte bättre än den gamla: nativt 1,1–3,3 ms per allokering mot 0,34–0,47 ms
   (två körningar, brusigt); vid 1 000 och 5 000 hål är de lika. Minnet räcker till det:
   `memory_max` får vara 256 MiB. Bränsle är inte mätt, bara antal block och nativ tid.
   Noteringen säger detta ärligt under "Inte gjort", och kodens huvudkommentar likaså. Men
   commit-rubriken och `docs/BUILD-NOTES.md` ("Kostnaden beror inte på antalet hål") säger det
   utan förbehåll, och README:s öppna punkt är struken helt. Testet
   `spec_5_3_3_heap_cost_does_not_grow_with_the_holes` har bara hål på 32–96 byte.
   **Åtgärd (krävs):** sätt tillbaka en öppen punkt i README ("en begäran över 1 KB letar bland
   de stora hålen") och lägg förbehållet i BUILD-NOTES. **Åtgärd (ägarens val):** listor per
   storleksklass också över 1 024 byte, där en begäran tas ur första klassen som säkert räcker,
   plus ett test med tusentals stora hål. Det kostar byte i varje modul.

2. **Låg – vid minnestaket nekar den nya en begäran som den gamla klarade.** `extend` begär
   alltid `storlek + 16` byte i sidor, också när de nya sidorna fortsätter högen och
   slutmarkeringen återanvänds. Fall: tak 2 sidor, ett litet block, sedan ett block på 65 520
   byte. Gamla: lyckas med 2 sidor. Nya: null, sidan trappar. Utan tak tar den nya 3 sidor mot
   2. Det gäller begäran som med sitt huvud är exakt hela sidor (65 505–65 520 byte, och samma
   plus hela sidor). Ärvt och oförändrat: `extend` räknar
   inte med ett fritt block sist i högen, så tak 2 sidor, ett litet block och 100 000 byte ger
   null i båda, fast en sida till hade räckt. **Åtgärd:** begär utan de 16 när högen redan
   finns och väx en gång till om sidorna inte fortsatte den; dra av det fria blocket sist.

3. **Låg – testerna bevisar att blocken hålls hela, men tre andra saker vaktar de inte.**
   Sju avsiktliga fel prövade mot de fyra testerna; ett fälldes, sex gick igenom:

   | Avsiktligt fel | Utfall |
   | --- | --- |
   | block på exakt 1 024 byte växer kvar på fel lista | fälldes |
   | panik när nya sidor inte fortsätter högen | överlever: vägen körs aldrig |
   | `extend` utan plats för slutmarkeringen | överlever |
   | krympning ger inget tillbaka | överlever |
   | hål av exakt storlek används inte medan det aktuella blocket räcker | överlever |
   | `realloc` växer aldrig in i ett fritt block på en lista | överlever |
   | stor begäran tar det aktuella blocket först | överlever (policyval, inget fel) |

   (a) Vägen där minnet vuxit av någon annan körs inte: testarenan är alltid sammanhängande och
   `Heap::check` kan inte gå igenom en hög i två delar. Ingen sida eller SDK-kod anropar
   `memory.grow` i dag, så vägen är vilande. Mitt eget prov över 108 delar fann inget fel.
   (b) Ett null godtas var som helst som "taket nått". Med felet på rad tre ger en första
   allokering på 65 520 byte null med 7 sidor kvar, och testerna går.
   (c) Ingenting vaktar minnesåtgången: tre fel som slösar minne går igenom.
   (d) Räknaren `passed` finns bara i sökningen bland stora block, så `passed == 0` kan inte
   falla för små begäran vad koden än gör. Det som håller är att allokering och frigörning har
   en enda slinga, och den är räknad; en ny slinga utan räknare syns inte.
   **Åtgärd:** en arena med lucka och en `check` som går del för del; arenan minns om den
   nekade, och varje null kräver det; ett test att sidantalet står still över upprepade
   allokeringar och frigörningar av samma storlekar.

4. **Låg – "flyttas inte och kopieras inte" gäller bara medan det fria blocket efter räcker.**
   När högen måste växa flyttas blocket ändå: `realloc` tar då ett nytt block och kopierar, i
   stället för att växa in i de nya sidorna. En buffert som dubblas från 8 byte till 4 MiB
   flyttas 6 gånger av 19 (gamla: 19) och högen slutar på 132 sidor för 64 sidor data (gamla:
   136). Ingen försämring mot förut, men vinsten uteblir där bufferten är störst. Testet
   `..._without_holes_touches_no_list` växer högen till full storlek först och ser aldrig fallet. **Åtgärd:** pröva `resize` igen efter
   `extend`; skriv förbehållet i noteringen.

5. **Notering – småsaker i texten.** Testets kommentar säger att bokföringen håller "after
   every step"; `check` körs var 64:e (commit och notering säger rätt, och mitt prov med varje
   steg gick). "Samma marginal som förut" stämmer för `form` (912 → 891) men startsidans
   krympte (1 152 → 906), åt det försiktiga hållet. `carve` anropas på sex ställen, inte fem.
   "Minnet oförändrat" är mätt på sviten; i provet i fynd 1 tar den nya 2 % fler sidor (406
   mot 397).

## 1. Korrekthet

**Inget fel hittat.** Läst rad för rad: huvudets två fält och att vartenda ställe som ändrar en
storlek skriver nästa blocks `before`; slutmarkeringen; det aktuella blocket i `take`, `dealloc`,
`carve` och `resize`; grenen där ett stort block växer kvar på sin lista; `extend` åt båda håll.

Prövat: 16 frön × 20 000 slumpade steg (storlekar tyngda mot 1 008/1 024/1 040 byte och
sidgränsen ± 16, tak 4–12 sidor) med `Heap::check` efter **varje** steg och alla levande blocks
innehåll och överlapp var 16:e: 35 545 nekade vid taket, 49 864 ändringar på plats, 31 307
flyttade. Sedan 18 frön × 30 000 steg där en främmande sida ligger mellan tillväxterna (108
delar): innehåll och överlapp var 8:e steg, till sist ett fritt block per del med rätt antal
byte.

**wasm32 är läst, inte kört.** Testerna går nativt på 64 bitar. `memory_max` är högst 4 096
sidor (SPEC-8.1.3), så ingen adress eller summa av storlekar kommer nära 2^32; `Layout` håller
en begäran under 2^31. Ett fritt blocks länkar är 8 byte på wasm32 och 16 nativt, så testerna
kör det trängre fallet. `GlobalAlloc`-delen (ett tiotal rader) skickar rätt värden vidare, och
en justering över 16 ger null redan i `alloc`; den delen körs bara av modulerna och sviten.

## 2. Testerna

Se fynd 3. De bevisar väl det `spec_4_2_4_*` säger. `spec_5_3_3_*`-paret bevisar sitt för små
block; för stora se fynd 1.

## 3. Påståendena

| Påstående | Bedömning |
| --- | --- |
| 10 000 hål: 62 491 ns per steg blir 13 | **Stämmer.** Omkört: 62 091 och 13; 20 000 hål 219 389 och 13. |
| modulerna efter: `hello` 6 300, `form` 32 409, `kalkyl` 66 060, `widgets` 57 636 | **Stämmer** mot `target/wasm/`. Storlekarna före är inte kontrollerade. |
| huvudet 16 byte som förut | **Stämmer.** |
| testnamnen och vad de kräver | **Stämmer**, med fynd 3 och 5. |
| kostnaden beror inte på antalet hål | **Bara upp till 1 024 byte** (fynd 1). |
| en `Vec` i slutet av högen flyttas inte | **Med förbehåll** (fynd 4). |
| tio avsiktliga fel fälldes alla | Inte kontrollerat; mina sju gällde annat än helheten (fynd 3). |

## 4. Taken

**Ärligt höjda.** Båda vaktar att flerradshalvan av `ui::TextBox` (omkring 6 500 byte) viks
bort. `form.wasm` är 32 409 mot taket 33 300; startsidan är 30 194 (mätt: `prov_sida.wasm`)
mot 31 100. Marginalerna är 891 och 906 byte, så en tappad vikning fälls med 5 600 byte till
godo, och taken släpper inte igenom mer än förut.

## Inte kontrollerat

- `cargo test --workspace`, konformanssviten och budgettesterna i `vy-cli`: talen −0,4 %,
  +1,4 %, −8,0 % och "minnet oförändrat i alla 96 fall" är författarens.
- Format, clippy (också wasm32), spec-spårning och `tools/test_modules.sh`.
- Allokatorn körd på wasm32, och bränslekostnaden för sökningen i fynd 1.
- Talen för den första versionen utan aktuellt block, och de 364 byten för `carve`.
- Om konformanssviten fäller ett fall på minnesåtgång.
