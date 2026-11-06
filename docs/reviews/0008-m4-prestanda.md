# PR #8 – M4: prestanda – bench, textcache, snabbvägar, kompileringscache, 8 ms-kontroll (`main..m4/perf`, topp `0623dab`, bedömd på `ccef8b4`)

**Vad ändrades.** `vy-cli bench` (P95 över `--runs`, `--json`, `--compare` mot en baslinje) och en
cache för formad text i `TextEngine`. Rasteriseringen fick snabbvägar som ger samma byte och
debiterar samma budget som M3, bevisat mot M3:s kod i tester. Kompileringscachen finns i
`vy-runtime` (av som standard). Nätgrinden i fliken är skärpt (tom lista når inget). §13-testerna
mäter i release och hoppar i debug. `vy-publish build` vägrar en modul vars frame går över 8 ms
bränsle utan `--allow-slow`. 5 700 rader, 23 filer.

**Spec-id som berörs.** 5.2.8, 6.2.2, 12.2.3, 13.1.2–13.1.4, 13.2.1–13.2.3, 13.4.1 (un-ignorerat).
Spec-frågor 63–65.

**Risk.** En snabbväg som ritar en annan byte bryter determinismen (princip 5) och guldbilderna.
En cache som deserialiserar fel bytes kör främmande maskinkod. En regressionsgrind som ger falsklarm
blir avstängd.

**Bevis.** Konformans 59/59 à 0,000 % på HEAD. Differentialtesterna mot `raster_reference.rs`
(M3 ordagrant) och `over_highp_is_what_draw_pixmap_does` är gröna. Benchen i release på HEAD,
30 körningar per rad, last 0,2–0,5: **ingen regression** mot `conformance/bench/build-vm.json`
(SPEC-12.2.3, slutkod 0, 15 s). P95, ms:

| Rad | P95 | Budget |
| --- | --- | --- |
| ram med 2 000 opkoder i sidornas proportioner (13.2.1) | 17,5 | 2 – **över** |
| referenssidornas rasterisering, 12 sidor | 0,25–4,6; galleri 3,6 och shop 4,6 över | 2 |
| kompilering, kall, 5–30 KB | 8,8–36 | 60 |
| kompilering, kall, **studio 59 KB** | **62,4** | 60 – **över** |
| kompileringscache, träff | 0,09–0,31 | 1 |
| rullning `text` / textcache-missar | 1,3 / 2,5 % | 16,7 / 5 % |
| Enter till bild (hello, text) / pekare till pixel | 20 / 29 / 2,8 | 100 / 16 |

Talen är 2–3 gånger bättre än noteringarna, som mättes vid last 4–6. Fynd, efter allvar:

1. **Medel – SPEC-13.1.4 hålls inte i produkten.** Kall kompilering av den största modulen (Studio,
   60 KB) ligger redan på 62 ms mot 60. Budgeten avser 300 KB. Cachen som ger "0 ms vid träff" är
   av i skalet (`vy-shell/src/tab.rs`, `CompileCache::Off`; integrationens öppna punkt 2). Det
   kräver förkompilering i huvudprocessen och läsning genom IPC eller ett fd öppnat före låsningen
   (se 0007, fynd 6: uid-kontroll först). Ägaren bör veta att 13.1.4 i dag är uppfyllt i benchen,
   inte i klienten.
2. **Låg – skälet för det ignorerade `spec_13_2_1_raster_budget` är inaktuellt.**
   `vy-render/src/lib.rs:805` säger "P95 44–56 ms"; vid låg last är P95 17,5 ms och P50 14,9 ms.
   Budgeten hålls ändå inte (8 gånger över). Skriv in båda talen med lasten, så att ägaren avgör
   spec-fråga 65 på rätt underlag.
3. **Låg – `vy-dev build` hoppar över 8 ms-kontrollen** (0005, fynd 3).
4. **Låg – ingen CI-baslinje.** `conformance/bench/ubuntu.json` saknas, så CI-jobbet `bench`
   jämför inget förrän en första artefakt checkats in i ett `bench:`-commit. Grinden i CLAUDE.md
   gäller därför bara den här maskinen.

**Nya beroenden.** Inga nya externa crates. `vy-publish` drar nu in `vy-runtime` (wasmtime) och växer
till 16,4 MB. Det är verktyget, inte klienten, och det behövs för att köra modulen (princip 6).

**Hotbild.** Kompileringscachen är den nya ytan: `Module::deserialize` kör bytes som maskinkod.
Begränsningen är att bara huvudprocessen skriver, nyckeln binder wasmtimes version och inställningar
och modulens hash, katalogen ska vara privat, och posten läses hel och kontrolleras. Eftersom cachen
är av nås ytan inte från en sida i dag. Den skärpta nätgrinden (`755a595`) minskar ytan.

**Rekommendation.** **Merge.** Fynd 1 är en känd lucka mot §13 och bör vara M5:s första punkt eller
ett uttryckligt undantag från ägaren. Fynd 2–4 är små uppföljningar.
