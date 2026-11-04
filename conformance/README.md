# Konformanssvit

- `cases.toml` – fallen: modul, logisk storlek, skala, antal frames, beviljade `capabilities`
  och `events` som spelas upp före sista framen (körs av `vy-cli conformance`).
- `golden/<modul>-<WxH>@<scale>[-<tillstånd>].png` – guldbilder, en per referensmodul, storlek,
  DPI och scriptat tillstånd (från M1).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifest i JSON-form (kanonisk CBOR på tråden).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2); en pixel räknas som avvikande om någon kanal skiljer sig.
Ett fall vars semantiska träd har `textbox`-noder måste bevilja `ime` (SPEC-10.4.2). Varje modul
under `modules/` ska ha minst två fall och ett semantiskt träd, och inga guldbilder får sakna
fall (SPEC-12.1.2, `spec_12_1_2_conformance_defines_client`).
Vid avvikelse skrivs den faktiska bilden till `golden/<namn>.png.new` (ignoreras av git).
Ett guldbyte (`cargo run -p vy-cli -- conformance --update`) är ett eget commit med `golden:` som
prefix och en motivering i PR:en.
