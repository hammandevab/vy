# Att släppa Vy

Hur en release byggs, vad den innehåller, hur vem som helst bygger om den och jämför (SPEC-9.7.1),
och vad ägaren behöver för att den ska vara signerad.

## Vad en release är

`.github/workflows/release.yml` körs för en tagg `v*` (eller manuellt med en version) och fäster vid
en GitHub Release:

| Fil | Innehåll |
| --- | --- |
| `vy-<version>-<mål>.zip` | `vy-cli`, `vy-dev`, `vy-publish`, `vy-serve`, `vy-shell`, `studio.vyp` och `LICENSE`, för `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` och `x86_64-pc-windows-msvc` |
| `vy-site-<version>.zip` | referenssidans elva paket, `dev.key.pub` och `previews/` (`tools/publish_site.sh`) |
| `SHA256SUMS` | SHA-256 för varje zip och varje fil i dem, i den form `sha256sum -c` läser |
| `BUILD.md` | byggreceptet: commit, `SOURCE_DATE_EPOCH`, maskinerna och verktygskedjan per mål, kommandona, och `studio.vyp`:s paketidentitet per mål (ur `tools/BUILD.md.in`) |

## Reproducerbart bygge (SPEC-9.7.1)

**Vad som jämförs.** Samma commit byggd med `tools/build_release.sh <mål> <katalog>` på samma klass av
maskin (samma operativsystem och runner-avbild, samma verktygskedja ur `rust-toolchain.toml`) ger
samma byte för varje binär och för `LICENSE`, oavsett arbetskopians sökväg, användaren (och därmed
`CARGO_HOME`), tidszonen och tiden. Modulerna (`tools/build_modules.sh`) ger samma `.wasm` på samma
sätt. Mellan olika operativsystem jämförs ingenting: varje mål har sina egna binärer.

**Paket.** `studio.vyp` signeras med en engångsnyckel vid varje bygge, eftersom `vy-dev serve`
skriver in sin origin och sin nyckel och signerar om paketet med utvecklarens nyckel vid varje
start. Filen skiljer sig därför, men inte paketet: ett paket är reproducerbart när dess
*paketidentitet* (SPEC-4.1.2: SHA-256 över det kanoniska manifestet utan `sig`) är densamma, och
manifestet namnger modulens och varje resurs hash. `vy-publish inspect <paket>` skriver den som
`identity:`. Referenssidans paket signeras med `VY_KEY`; Ed25519-signaturer är deterministiska,
så med samma nyckel blir de, och `vy-site-<version>.zip`, samma byte.

**Signerade binärer.** En signatur (Windows Authenticode, macOS Developer ID) innehåller tid och
certifikat och är inte reproducerbar. `SHA256SUMS` har därför för ett signerat mål också den
osignerade binärens hash, under `vy-<version>-<mål>.unsigned/`; det är den ett ombygge jämförs med.

**Zip-filerna.** `tools/release_files.py zip` skriver posterna sorterade, med tiden
`SOURCE_DATE_EPOCH` (commitens tid), läge 0755 för binärerna och 0644 för resten, utan extra fält.
Komprimeringen är Pythons zlib, så zip-filens byte beror också på den; filerna i den gör det inte.

**Hur.** `tools/repro_env.sh`, som `build_release.sh` och `build_modules.sh` läser in, sätter:

- `CARGO_ENCODED_RUSTFLAGS`: de flaggor som redan fanns, sedan `--remap-path-prefix` för `CARGO_HOME`
  (`/cargo`; registrets källor står i panikplatser), `CARGO_TARGET_DIR` om den är satt (`/target`;
  byggskriptens utdata) och arbetskopian (`/vy`). Cargo hashar inte RUSTFLAGS in i `-C metadata`,
  så flaggorna gör inte bygget sökvägsberoende. `-Ztrim-paths` är inte stabilt i cargo 1.97.
- `CFLAGS`: `-ffile-prefix-map` för samma sökvägar, för C-koden i ring och zstd-sys (`__FILE__`),
  utom under MSVC, där `build_release.sh` i stället länkar med `/Brepro` (MSVC:s länkare skriver
  annars tiden i PE-huvudet).
- `SOURCE_DATE_EPOCH`: commitens tid, om den inte redan är satt.

Modulerna behöver en sak till: varje modul är en egen cargo-arbetsyta, och för sökvägsberoenden
utanför arbetsytan (`crates/vy-sdk`, `crates/vy-abi`) hashar cargo den absoluta sökvägen in i
`-C metadata`, som bestämmer ordningen på SDK:ns funktioner i modulen. `tools/repro_rustc.rs` är
därför `RUSTC_WRAPPER` i `build_modules.sh` och ger det värdet av cratens namn, version, typer,
mål och funktioner.

**Prov.** `spec_9_7_1_reproducible_build` (i `cargo test`) bygger modulerna i två kopior av
arbetsträdet på olika djup och jämför alla `.wasm` byte för byte och alla paketidentiteter. CI:s
jobb `repro` bygger hela releasen två gånger med `build_release.sh`, i två git-worktrees på olika
sökvägar, med två `CARGO_HOME` och två tidszoner, och jämför SHA-256; Linux är grind, macOS och
Windows rapporterar tills de har gått igenom en gång.

**Bygga om en release.** `BUILD.md` i releasen har kommandona; i korthet:

```sh
git checkout v<version>
export SOURCE_DATE_EPOCH=<ur BUILD.md>
tools/build_release.sh <mål> stage
python3 tools/release_files.py sums vy-<version>-<mål> stage/vy-* stage/LICENSE   # jämför med SHA256SUMS
stage/vy-publish inspect stage/studio.vyp                                          # identity: som i BUILD.md
```

## Släppa en version

1. Sätt `version` under `[workspace.package]` i `Cargo.toml` till den nya versionen och kör grindarna
   (låsfilerna följer med: `Cargo.lock` och `modules/*/Cargo.lock`, som `build_modules.sh` läser
   med `--locked`).
2. Commit, tagga `v<version>` och pusha taggen. Jobbet `version` stoppar releasen om taggen och
   arbetsytans version inte är samma.
