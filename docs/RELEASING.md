# Att släppa Vy

Hur en release byggs, vad den innehåller, hur vem som helst bygger om den och jämför (SPEC-9.7.1),
och vad ägaren behöver för att den ska vara signerad.

## Vad en release är

`.github/workflows/release.yml` körs för en tagg `v*` (eller manuellt med en version) och fäster vid
en GitHub Release:

| Fil | Innehåll |
| --- | --- |
| `vy-<version>-<mål>.zip` | `vy-cli`, `vy-dev`, `vy-publish`, `vy-serve`, `vy-shell`, `studio.vyp` och `LICENSE`, för `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` och `x86_64-pc-windows-msvc` |
| `vy-site-<version>.zip` | den publika sajtens arton paket (ett per utkast i `conformance/manifests/` utom Studio, som bara körs lokalt), `dev.key.pub` och `previews/` (`tools/publish_site.sh`) |
| `vy-conformance-<version>.zip` | konformanssviten för andra klienter (`tools/package_conformance.sh`, SPEC-12.1.2) |
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
Det gäller alla tre sorterna: målzipparna, sajtzippen och konformanszippen
(`tools/package_conformance.sh`) skrivs av samma skrivare, så ingen av dem beror på byggarens
umask, klocka eller tidszon.

**Hur.** `tools/repro_env.sh`, som `build_release.sh` och `build_modules.sh` läser in, sätter:

- `CARGO_ENCODED_RUSTFLAGS`: de flaggor som redan fanns, sedan `--remap-path-prefix` för `CARGO_HOME`
  (`/cargo`; registrets källor står i panikplatser), `CARGO_TARGET_DIR` om den är satt (`/target`;
  byggskriptens utdata) och arbetskopian (`/vy`). Cargo hashar inte RUSTFLAGS in i `-C metadata`,
  så flaggorna gör inte bygget sökvägsberoende. `-Ztrim-paths` är inte stabilt i cargo 1.97.
- `CFLAGS`: `-ffile-prefix-map` för samma sökvägar, för C-koden i ring och zstd-sys (`__FILE__`),
  utom under MSVC, där `build_release.sh` i stället länkar med `/Brepro` (MSVC:s länkare skriver
  annars tiden i PE-huvudet).
- `SOURCE_DATE_EPOCH`: commitens tid, om den inte redan är satt.
- `MSYS2_ENV_CONV_EXCL`: `CARGO_ENCODED_RUSTFLAGS` och `CFLAGS`, efter det som redan stod där. Git
  Bash på Windows skriver annars om det som ser ut som en POSIX-sökväg i miljön till ett nativt
  program: `/Brepro` och ommappningens mål `/vy`, `/cargo` och `/target` skulle bli sökvägar under
  Gits egen katalog.

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
2. Commit, tagga `v<version>` och pusha taggen (eller kör `release` manuellt med versionen). Jobbet
   `version` stoppar releasen om taggen och arbetsytans version inte är samma.

Varför versionen måste stämma: `vy-dev new` skriver `vy-sdk = { git =
"https://github.com/hammandevab/vy", tag = "v<vy-devs version>" }` i ett nytt projekt (utan
`--sdk`), så att projektet bygger mot SDK:n i den release vars `vy-dev` skapade det och inte mot
main. Versionen är arbetsytans, och taggen måste därför finnas med den versionen.

## Storlek (SPEC-12.3.1)

`release.yml` skriver varje fils storlek i loggen och klientens (`vy-shell`) i körningens
sammanfattning, per plattform, och stoppar releasen om klienten är över 25 000 000 byte. CI:s jobb
`size` prövar samma sak på Linux för varje ändring (`spec_12_3_1_size_budget` i release).

## Kodsignering

Utan signatur varnar Windows SmartScreen för `vy-shell.exe` och macOS Gatekeeper vägrar starta en
nedladdad binär utan att användaren går via Systeminställningar. `release.yml` signerar när
secrets finns och hoppar annars över med en notis; jobbet är grönt i båda fallen. Är bara några av
ett sätts secrets satta blir det en varning och ingen signering.

### Windows

**Rekommenderat: Azure Artifact Signing** (hette Trusted Signing). Microsoft anger "ungefär 9,99
USD/månad" för nivån Basic, som rymmer 5 000 signaturer i månaden (Premium: 100 000; priset visas
inte på prissidan utan JavaScript, kontrollera aktuellt pris). Tjänsten säljs till organisationer i
USA, Kanada, EU och Storbritannien; enskilda utvecklare bara i USA och Kanada, så från Sverige är
det en organisation (bolaget) som registrerar sig. Microsoft validerar identiteten först, räkna med
några arbetsdagar. Äldre riktlinjer krävde tre års verifierbar historik för en organisation;
kontrollera om det gäller när ansökan görs. Källa: Microsoft Learn, *Code signing options for
Windows app developers*, uppdaterad 2026-08-29.

Vad ägaren gör:

1. Ett Azure-abonnemang (betala per användning) och resursleverantören `Microsoft.CodeSigning`.
2. Ett Artifact Signing-konto i en region, t.ex. North Europe; kontots *endpoint* är en adress av
   formen `https://neu.codesigning.azure.net/`.
3. Identitetsvalidering för organisationen (*Public Trust*), sedan en certifikatprofil av typen
   Public Trust.
4. En appregistrering i Entra ID med en klienthemlighet, och rollen *Certificate Profile Signer*
   (Artifact/Trusted Signing) för appen på kontot eller profilen.
5. Repots secrets: `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`,
   `AZURE_SIGNING_ENDPOINT` (endpoint-adressen), `AZURE_SIGNING_ACCOUNT` (kontots namn),
   `AZURE_SIGNING_PROFILE` (profilens namn).

Steget är `azure/artifact-signing-action` v2.0.0 (pinnat på commit), som kör signtool med
Microsofts signeringsklient över varje `.exe`, SHA-256 och tidsstämpel från
`http://timestamp.acs.microsoft.com`; därefter `signtool verify /pa`.

**Alternativ: ett certifikat som PFX.** `WINDOWS_PFX_BASE64` (filen i base64) och
`WINDOWS_PFX_PASSWORD`; tidsstämpeln från repovariabeln `WINDOWS_TIMESTAMP_URL`, annars
`http://timestamp.digicert.com`. Ett OV-certifikat kostar enligt samma källa 150–300 USD/år, men
sedan juni 2023 måste nyckeln ligga i en HSM eller hårdvarutoken, så ett nytt publikt certifikat går
sällan att lägga som fil i CI; vägen är till för ett äldre certifikat eller en egen CA. EV-certifikat
(400+ USD/år) ger inte längre något försprång i SmartScreen. Öppen källkod kan få gratis signering
genom SignPath Foundation (egen integration, inte inkopplad här).

SmartScreen bygger rykte över tid även för signerade binärer; samma signeringsidentitet release
efter release hjälper.

### macOS

**Apple Developer Program**, 99 USD per medlemsår, i lokal valuta där den finns (kontrollera
aktuellt pris i kronor vid registreringen). Developer ID-certifikat och notarisering ingår. En
organisation behöver vara en juridisk person med ett D-U-N-S-nummer, en fungerande webbplats på
en egen domän och en e-postadress på den domänen, och den som registrerar måste få binda
organisationen; en enskild person registrerar sig med sitt juridiska namn, som då visas som
utgivare. Källa: developer.apple.com, *Membership Details* och *Enrollment*.

Vad ägaren gör:

1. Registrera organisationen i Apple Developer Program.
2. Skapa ett certifikat av typen *Developer ID Application* (kontoinnehavaren gör det), exportera
   det med nyckeln som `.p12` med ett lösenord.
3. Skapa en API-nyckel för App Store Connect (*Users and Access → Integrations*, en teamnyckel med
   rollen Developer), ladda ned `AuthKey_<id>.p8` och notera nyckel-id och issuer-id.
4. Repots secrets: `APPLE_CERTIFICATE_P12_BASE64` (`base64 -i cert.p12`),
   `APPLE_CERTIFICATE_PASSWORD`, `APPLE_SIGNING_IDENTITY` (t.ex. `Developer ID Application: Namn
   (TEAMID)`), `APPLE_API_KEY_P8_BASE64` (`base64 -i AuthKey_<id>.p8`), `APPLE_API_KEY_ID`,
   `APPLE_API_ISSUER_ID`.

Steget lägger certifikatet i en tillfällig nyckelring, signerar varje binär med `codesign
--options runtime --timestamp` (härdad körmiljö, som notariseringen kräver), skickar en zip med
binärerna till `xcrun notarytool submit --wait` och stoppar om svaret inte är `Accepted` (loggen
skrivs då ut). `vy-shell`, `vy-cli`, `vy-dev` och `vy-publish` kör wasmtime, som gör kompilerad kod
körbar med `mprotect` utan `MAP_JIT`; de får därför rättigheten
`com.apple.security.cs.allow-unsigned-executable-memory` ur `tools/macos-entitlements.plist`.
`vy-serve` signeras utan. En fristående binär kan inte bära en häftad notariseringsbiljett;
Gatekeeper slår upp den på nätet vid första start.

**Inget av signeringsstegen är kört**: det finns inga secrets här och ingen Windows- eller
macOS-maskin. Första releasen med secrets är provet; `codesign --verify` och `signtool verify` i
jobbet fäller den om något är fel.

### Reproducerbarhet och signatur

Signaturen ändrar binären. `SHA256SUMS` har därför för ett signerat mål både de levererade filerna
(`vy-<version>-<mål>/…`) och de osignerade (`vy-<version>-<mål>.unsigned/…`), och ett ombygge
jämförs med de senare. `BUILD.md` säger per mål om binärerna är signerade och hur.
