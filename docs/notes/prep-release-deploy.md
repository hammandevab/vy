# Förberedelse: deploy på Railway och releaser med binärer

Byggagentens noteringar för grenen `prep/release-deploy` (från main `406b056`). Rörda filer:
`crates/vy-serve`, `Dockerfile`, `.dockerignore`, `.gitignore`, `tools/publish_site.sh` (ny),
`.github/workflows/release.yml` (ny) och den här sidan. Inget annat.

## Vad som byggdes

| Del | Innehåll |
| --- | --- |
| `tools/publish_site.sh <ut-katalog> [nyckel]` | Bygger alla moduler, signerar de elva utkasten i `conformance/manifests/` till `<ut>/<namn>.vyp`, renderar en förhandsvisning per sida (`vy-cli render`, 1280×800 @ 1, utkastets resurser och capabilities) till `<ut>/previews/<namn>.png` och skriver `<ut>/dev.key.pub`. Nyckeln är filen från `vy-publish keygen` (32 byte) eller samma frö som 64 hextecken; utan nyckel en tillfällig. Antalet resurser som lästes ur utkastet kontrolleras mot vad `vy-publish build` byggde. |
| `Dockerfile` | Kör samma skript i byggstadiet: musl-bygge av `vy-serve`, gnu-bygge av `vy-cli` och `vy-publish` (används bara i bygget), `tools/publish_site.sh /srv`. Slutbilden är fortfarande `FROM scratch` med `/vy-serve` och `/srv`. Nyckeln kommer som byggargument `VY_KEY` (hex), inte som secret-mount (se beslut). |
| `vy-serve` | `GET /` för webbläsare: indexsida med varje pakets namn, sammanfattning, "Öppna i Vy: vy://värd/namn" och förhandsvisning när `<dir>/previews/<namn>.png` finns. `GET /_previews/<namn>.png`: `image/png`, `ETag`, `max-age=3600`; namnet går genom samma kontroll som paketnamn (`..`, dolda filer, `%00`, `\` → 400). HTML-reserven per paket bäddar in bilden och länkar till indexet. Vy-klienter (Accept med paket- eller manifesttypen) får på `/` som förr paketet `index`. All manifesttext escapas; inga anropsdata speglas i HTML. |
| `.github/workflows/release.yml` | Tagg `v*` eller manuell körning med `version`: `vy-<version>-<target>.zip` för `x86_64-unknown-linux-gnu`, `aarch64-apple-darwin` och `x86_64-pc-windows-msvc` (vy-cli, vy-publish, vy-serve, LICENSE; vy-shell så snart `cargo metadata` visar en binär), `vy-site-<version>.zip` (paketen, `dev.key.pub`, `previews/`), allt fäst vid en GitHub Release via `softprops/action-gh-release@v2` med genererade noteringar. |

## Railway – vad Jonathan klickar

1. [railway.com](https://railway.com) → **New Project** → **Deploy from GitHub repo** → välj
   `hammandevab/vy` (första gången: **Configure GitHub App** och ge Railway tillgång till repot).
2. Railway hittar `railway.toml` i repots rot och bygger med `Dockerfile`
   (`builder = "DOCKERFILE"`, hälsokontroll `GET /hello`). Inget att välja.
3. *Valfritt, för en stabil utgivarnyckel:* tjänsten → **Variables** → **New Variable**:
   namn `VY_KEY`, värde = fröet som 64 hextecken. Fröet görs lokalt en gång och sparas utanför repot
   (`*.key` är git-ignorerat):

   ```sh
   cargo run -p vy-publish -- keygen --out dev.key
   od -An -v -tx1 dev.key | tr -d ' \n'      # klistra in som VY_KEY
   ```

   Railway skickar tjänstens variabler som byggargument till `ARG VY_KEY` i Dockerfile. Utan
   variabeln signeras varje bygge med en ny tillfällig nyckel, och en klient som pinnat den gamla ser
   sidorna som obetrodda (SPEC-7.4.2, 7.4.3) tills den glömmer pinnen. Sätts variabeln senare:
   **Redeploy** så att paketen signeras om.
4. Tjänsten → **Settings** → **Networking** → **Generate Domain** (porten är 8080). vy-serve läser
   `RAILWAY_PUBLIC_DOMAIN` och sätter den som värd i alla `vy://`-länkar; för en egen domän sätt
   variabeln `VY_HOST_NAME` (värd, med `:port` om inte 443).
5. **Deploy** (sker automatiskt vid push till main när repot är kopplat). Första bygget tar
   10–20 minuter: wasmtime och cranelift byggs i release-läge för både musl och gnu.

### Adresser efteråt

| Adress | Svar |
| --- | --- |
| `https://<domän>/` | indexsidan: elva sidor med förhandsvisning och `vy://`-länk |
| `https://<domän>/hello` (och `/text`, `/form`, `/list`, `/gallery`, `/game`, `/dashboard`, `/chat`, `/shop`, `/settings`, `/document`) | HTML-reserven med förhandsvisning för en webbläsare; manifest eller paket för en Vy-klient (`Accept`) |
| `https://<domän>/.well-known/vy-key` | utgivarnyckeln (SPEC-7.4.2) |
| `https://<domän>/_previews/hello.png` | förhandsvisningen, 1280×800, `max-age=3600` |
| `https://<domän>/_vy/<sha256>` | modul eller resurs per hash, immutable (SPEC-7.3.1) |
| `vy://<domän>/hello` | sidan i en Vy-klient |

## Release – vad Jonathan skriver

```sh
git tag v0.1.0
git push --tags
```

Eller GitHub → **Actions** → **release** → **Run workflow** → `version` = `v0.1.0` (taggen skapas på
den valda commiten om den saknas). Resultatet under **Releases**:
`vy-0.1.0-x86_64-unknown-linux-gnu.zip`, `vy-0.1.0-aarch64-apple-darwin.zip`,
`vy-0.1.0-x86_64-pc-windows-msvc.zip`, `vy-site-0.1.0.zip`, med noteringar genererade ur PR-listan.

*Valfritt:* repot → **Settings** → **Secrets and variables** → **Actions** → **New repository
secret** `VY_KEY` (samma hexsträng som på Railway) så att paketen i `vy-site-*.zip` signeras med samma
nyckel som sajten; annars en tillfällig.

## Lokalt

```sh
tools/publish_site.sh deploy            # eller: tools/publish_site.sh deploy dev.key
cargo run --release -p vy-serve -- --dir deploy --public-key deploy/dev.key.pub --port 8080
```

`deploy/` är git-ignorerat. Öppna `http://localhost:8080/` i en webbläsare eller
`vy://localhost:8080/hello` i Vy.

## Beslut

- **Byggargument i stället för secret-mount.** Den tidigare Dockerfilen tog nyckeln som
  `RUN --mount=type=secret,id=vy_key`. Railways dokumentation säger att tjänstens variabler når ett
  Dockerfile-bygge som byggargument (`ARG`), och Railway-personal har svarat i deras forum att
  `--mount=type=secret` bara stöds för Railpack. Därför `ARG VY_KEY`, läst i ett enda `RUN` som
  skriver fröet till `/tmp/key`, publicerar och raderar det. Byggstadiets lager kan bära värdet i sin
  metadata, men slutbilden är `FROM scratch` + `COPY` och innehåller bara `dev.key.pub`; vill man inte
  lita på byggcachen lämnar man `VY_KEY` tom.
- **Fröet som hex.** En miljövariabel kan inte bära 32 råa byte. `vy-publish` läser bara råformen
  (utanför grenens gräns), så `tools/publish_site.sh` avkodar hex till en temporär fil själv.
- **Förhandsvisningar är filer i `<dir>/previews/`**, inte en del av paketet: de är klientens
  rendering, inte utgivarens innehåll, och vy-serve pekar på dem bara när filen finns.
- **`/` för Vy-klienter är oförändrat** (paketet `index` eller 404): indexsidan är en webbläsarvy,
  som HTML-reserven, och styrs av `Accept` på samma sätt.
- **Indexet läser om katalogen** vid varje anrop (`read_dir` + mtime per paket) så att ett paket som
  publicerats efter start listas, precis som det redan serverades vid anrop på sitt namn.
- **Utkastet tolkas med sed/grep** i skriptet (resursobjekten är platta, ett `{…}` per resurs) i
  stället för en JSON-tolk, som varken finns i `rust:slim`-bilden eller på macOS utan installation.
  Antalet resurser kontrolleras mot `vy-publish build`; ett `--grant`-namn som inte finns avvisas av
  `vy-cli`. Bättre vore `vy-cli render --manifest <utkast>` som läser resurser och capabilities ur
  utkastet självt; det rör `vy-cli` och föreslås som uppföljning.
- **Ingen `#[ignore]`-ändring, inga spec-id** berörs: indexsida och förhandsvisningar är
  webbläsarhjälp utanför specen (SPEC-7.2.2 gäller klienten).

## Verifierat lokalt

Varje kommando Dockerfilen kör är kört i arbetskopian med samma argument (ingen Docker-daemon i
sandlådan, så `docker build` är inte kört): `cargo build --release --locked --target
x86_64-unknown-linux-musl -p vy-serve` (statisk PIE, 1,46 MB), `cargo build --release --locked -p
vy-cli -p vy-publish`, och publiceringssteget ordagrant under `dash` (`set -eu`, `ARG`-grenen) både
med `VY_KEY` satt och tom: elva paket, elva förhandsvisningar (1280×800), `dev.key.pub`, `/tmp/key`
raderad. Samma frö som råfil och som hex ger samma `key_id` och byte för byte samma paket;
förhandsvisningarna är byte för byte lika mellan körningar (SPEC-2.1.5); `vy-publish inspect
--public-key` säger *signature: valid*. Felvägarna (saknad fil, fel längd, inte hex, inga argument)
avslutar före bygget. Den statiska `vy-serve` är körd mot resultatet med `RAILWAY_PUBLIC_DOMAIN`
satt: `GET /` listar elva sidor med `vy://<domän>/…`-länkar, `/hello` bäddar in bilden,
`/_previews/hello.png` är `image/png` med `ETag` och `max-age=3600`, `/_previews/../dev.key.pub.png`
och `..%2F` ger 400, `/.well-known/vy-key` ger nyckeln, `/` med Vy-Accept ger 404 (inget `index.vyp`)
och `/hello` manifestet. `release.yml` och `ci.yml` läses av PyYAML; `jq`-uttrycket för
vy-shell-binären är kört mot `cargo metadata` (ingen binär → hoppas över; `vy-serve` som kontroll →
hittas). Grindarna: fmt, clippy `--all-targets -D warnings`, `cargo test --workspace` (vy-serve 11 +
end-to-end), `spec_trace.py` 132/132, konformans 50/50 à 0,000 %, `check-abi` 11/11.

## Inte verifierat

- `docker build` och Railways bygge (BuildKit med `ARG`, `rust:1.97-slim-bookworm` på deras byggare).
- Release-flödet på riktiga runners: `zip` på macOS, `7z` på Windows, `softprops/action-gh-release`.
  Windows-bygget av vy-cli (wasmtime, zstd-sys via cc) är inte provat på denna gren.
