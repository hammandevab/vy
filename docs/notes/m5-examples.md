# M5 examples – sex större exempelsidor

Byggagentens noteringar för grenen `m5/examples` (från `m5/sdk` 1de409e). Granskningssidan skrivs av en
annan agent. Ägda delar: `modules/{kanban,kalkyl,rita,skriv,block,rymd}`, deras utkast i
`conformance/manifests/`, deras fall, guldbilder och träd i `conformance/`, `docs/EXEMPEL.md` och den
här filen. Det som rör andra spår står under *Berör andra spår*.

Varför: ägaren vill ha mer avancerade sidor att köra i klienten än referenssidorna. De visar vad
formatet klarar, prövar att komponenterna i `vy_sdk::ui` räcker för riktiga appar, och publiceras på
den publika sajten (`tools/publish_site.sh` tar varje utkast i `conformance/manifests/`).

## Läget (uppdateras efter varje sida)

| # | Sida | Modul | Läge | Commits |
| --- | --- | --- | --- | --- |
| 1 | Tavlan | `kanban` | pågår | – |
| 2 | Kalkylark | `kalkyl` | ej påbörjad | – |
| 3 | Ritbord | `rita` | ej påbörjad | – |
| 4 | Anteckningar | `skriv` | ej påbörjad | – |
| 5 | Block | `block` | ej påbörjad | – |
| 6 | Rymd | `rymd` | ej påbörjad | – |
| 7 | `docs/EXEMPEL.md` | – | ej påbörjad | – |

Ordningen är uppdragets. Varje sida är två commits: `modules/<namn>: …` (modul, utkast, tester; röd
på `spec_12_1_2_conformance_defines_client` tills fallen finns, som `widgets` och `studio` landade)
och `golden: …` (fall, guldbilder, träd).

## Så arbetar spåret

- Grindarna körs från roten före varje commit (se CLAUDE.md); `CARGO_INCREMENTAL=0` i varje skal.
- Bränsle per frame mäts med ett litet verktyg utanför repot (ett ocommittat exempel i `vy-cli` som
  kör ett fall som `vy-cli conformance` gör och skriver ut `Runtime::last_frame().fuel` per frame);
  `vy-publish build` körs för varje sida (SPEC-13.4.1).
- Varje sida ses som PNG vid 1280 × 800, 800 × 600 och 400 × 700 @2 före commit.

## Beslut (gemensamma)

- **Varje sida är en egen crate** under `modules/` med bara `vy-sdk` som beroende, som
  referenssidorna. Ingen delad hjälpcrate: det lilla som upprepas (lagringsomslag, en växlingsknapp)
  är några tiotal rader per sida och står i *SDK-luckor* som underlag för nästa SDK-runda.
- **Utan `storage`** (begränsat läge, eller nekad): sidan kör med tillståndet i minnet och visar
  "sparas inte" diskret i sin statusrad. `state_get` vid start avgör läget; ett `state_put` som nekas
  senare (kvot) visar samma sak.
- **Fokus är värdens** (SPEC-10.3.1): allt som kan få fokus har fasta negativa nod-id, och sidorna
  ritar ringen själva (SPEC-10.3.3).

## SDK-luckor (fylls på per sida)

## Berör andra spår

Inget ännu.

## Kvar och öppet
