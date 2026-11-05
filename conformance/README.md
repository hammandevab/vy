# Konformanssvit

- `cases.toml` – fallen: modul, logisk storlek, skala, antal frames, beviljade `capabilities`
  och `events` som spelas upp före sista framen (körs av `vy-cli conformance`).
- `golden/<modul>-<WxH>@<scale>[-<tillstånd>].png` – guldbilder, en per referensmodul, storlek,
  DPI och scriptat tillstånd (från M1).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifest i JSON-form (kanonisk CBOR på tråden), ett per modul;
  sviten läser det: `memory_max` mot modulens initiala minne (SPEC-8.1.3), fallet får bara
  bevilja vad manifestet begär (SPEC-8.2.2), en sida med `textbox`-noder måste begära `ime`
  (SPEC-10.4.2), och bildresursers `w`/`h` stämmer mot filerna (`vy-manifest`, SPEC-8.1.2).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2); en pixel räknas som avvikande om någon kanal skiljer sig.
Ett fall vars semantiska träd har `textbox`-noder måste höra till ett manifest som begär `ime`
(SPEC-10.4.2). Tab (`key_down` 43) når aldrig modulen: värden flyttar fokus genom trädet och
skickar `FOCUS` (SPEC-10.3.1). Varje modul
under `modules/` ska ha minst två fall och ett semantiskt träd, och inga guldbilder får sakna
fall (SPEC-12.1.2, `spec_12_1_2_conformance_defines_client`).
Vid avvikelse skrivs den faktiska bilden till `golden/<namn>.png.new` (ignoreras av git).
Ett guldbyte (`cargo run -p vy-cli -- conformance --update`) är ett eget commit med `golden:` som
prefix och en motivering i PR:en.
