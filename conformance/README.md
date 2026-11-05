# Konformanssvit

- `cases.toml` – fallen: modul, logisk storlek, skala, antal frames, beviljade `capabilities`,
  `resources`, inspelade svar (`fetch`) och `events` som spelas upp före sista framen (körs av
  `vy-cli conformance`). Strängar skrivs `"…"` utan escapes eller som TOML-literalsträngar `'…'`,
  som får innehålla `"` (ett inspelat JSON-svar i `body`) men inte `'`. Händelserna är både indata (pekare, tangenter, text, rullning, fokus)
  och värdhändelser: `visibility`, `budget`, `resize`, `suspend`, `reinit` och `fetch_done`,
  var och en med valfritt `after_frames = N` som begränsar hur många frames som körs före den.
- `golden/<modul>-<WxH>@<scale>[-<tillstånd>].png` – guldbilder, en per referensmodul, storlek,
  DPI och scriptat tillstånd (från M1).
- `golden/shell-<modul>-<WxH>@<scale>[-<tillstånd>].png` – skalfall (`shell = true`, från M3):
  modulen körs med sitt utkast i en flikprocess under det huvudlösa skalet (`vy-cli --tab`), och
  bilden är hela fönstret `width × height` med chromen – flikrad, adressfält, markörer, pluppar –
  över sidytan (SPEC-9.6.1, SPEC-11.3.2). Lokala filer körs i begränsat läge (SPEC-7.4.3). Inga
  events, inget träd, inga `capabilities`. Samma bild tas med `vy-shell --screenshot <png> --size
  WxH --manifest <utkast> <modul>`. Med `address = "vy://…"` tas sidan som hämtad därifrån:
  sviten signerar paketet med sin egen nyckel och verifierar det (SPEC-7.4.2), så standardpolicyn
  avgör grants (SPEC-8.2.3) med adressens auktoritet som sidans egen origin (SPEC-9.5.1); en
  capability policyn frågar om (`net` till annan origin, `clipboard`, `persist_frame`) ger frågan
  med Tillåt/Neka, som ligger över sidytans överkant utan att flytta sidan
  (`shell-chat-800x600@1-ask`).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifestutkast (`vy_manifest::Draft`, samma fil som
  `vy-publish build --manifest` läser; kanonisk CBOR på tråden), ett per modul: inga hashar,
  storlekar eller signatur, resurser som sökvägar relativt utkastet med `mime` och bildens
  `w`/`h`. Sviten fyller i modulen och fallets resurser och kör `Manifest::validate`:
  `memory_max` mot modulens initiala minne (SPEC-8.1.3) och som minnestak för körningen
  (SPEC-4.2.4), bildresursers `w`/`h` mot filerna (SPEC-8.1.2), gränserna i §4 och §8. Fallets
  `resources` måste namnge utkastets filer i samma ordning (SPEC-4.5.1) och en storlek fallet
  anger måste vara manifestets; värden får storleken ur manifestet. Fallet får bara bevilja
  vad manifestet begär (SPEC-8.2.2), en sida med `textbox`-noder måste begära `ime`
  (SPEC-10.4.2), `net.origins` avgör vilka adresser `fetch_begin` får nå (SPEC-5.2.8) och
  `storage.quota_kb` är lagringens kvot (SPEC-9.3.1).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2); en pixel räknas som avvikande om någon kanal skiljer sig.
Ett fall vars semantiska träd har `textbox`-noder måste höra till ett manifest som begär `ime`
(SPEC-10.4.2). Tab (`key_down` 43) når aldrig modulen: värden flyttar fokus genom trädet och
skickar `FOCUS` (SPEC-10.3.1). Varje modul
under `modules/` ska ha minst två fall och ett semantiskt träd, och inga guldbilder får sakna
fall (SPEC-12.1.2, `spec_12_1_2_conformance_defines_client`).
Vid avvikelse skrivs den faktiska bilden till `golden/<namn>.png.new` (ignoreras av git).
Ett guldbyte (`cargo run -p vy-cli -- conformance --update`) är ett eget commit med `golden:` som
prefix och en motivering i PR:en.
