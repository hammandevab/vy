# Konformanssvit

- `golden/<modul>-<WxH>@<scale>.png` – guldbilder, en per referensmodul, storlek och DPI (från M1).
- `semantic/<modul>.json` – förväntat semantiskt träd (från M1).
- `manifests/<modul>.json` – manifest i JSON-form (kanonisk CBOR på tråden).

Pixeldiff-gränsen är 0,1 % (SPEC-12.2.2). Ett guldbyte är ett eget commit med `golden:` som prefix
och en motivering i PR:en.
