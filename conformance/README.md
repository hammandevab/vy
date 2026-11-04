# Konformanssvit

- `cases.toml` – fallen: modul, logisk storlek, skala, antal frames (körs av `vy-cli conformance`).
- `golden/<modul>-<WxH>@<scale>.png` – guldbilder, en per referensmodul, storlek och DPI (från M1).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifest i JSON-form (kanonisk CBOR på tråden).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2); en pixel räknas som avvikande om någon kanal skiljer sig.
Vid avvikelse skrivs den faktiska bilden till `golden/<namn>.png.new` (ignoreras av git).
Ett guldbyte (`cargo run -p vy-cli -- conformance --update`) är ett eget commit med `golden:` som
prefix och en motivering i PR:en.
