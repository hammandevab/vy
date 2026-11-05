# M3, första delen: vy-manifest, vy-publish, vy-proto, vy-serve

Byggagentens noteringar för grenen `m3/proto-serve`. Granskningssidan skrivs av en annan agent.
Rörda kataloger: `crates/vy-manifest` (omskriven), `crates/vy-proto` (omskriven), `crates/vy-publish`
(ny), `crates/vy-serve` (ny), `Cargo.toml` (medlemmar och beroenden, bara tillägg), `Dockerfile`,
`.dockerignore`, `railway.toml`, den här sidan. Inget annat.

## Vad som byggdes

| Del | Innehåll |
| --- | --- |
| `vy-manifest` | `Manifest` enligt §8 med kanonisk CBOR (RFC 8949 §4.2) ovanpå minicbors tokenizer; JSON-form via serde; identitet = SHA-256 av kodningen utan `sig` (SPEC-4.1.2); Ed25519 `sign`/`verify` (SPEC-7.4.1); `validate(module, resources) -> Vec<Problem>`; `default_grant` (SPEC-8.2.3); paketformatet (SPEC-4.4.1) som zstd + egen ustar-läsare/-skrivare; `Draft` som utgivarens indata; vy-key-textformen. |
| `vy-publish` | `keygen`, `pubkey`, `build`, `inspect`. Bibliotek + binär (1,7 MB release). |
| `vy-proto` | `Address` (SPEC-7.1.*), `Fetcher` med ureq (rustls/ring), båda svarsformerna, `Error::NotVy`, innehållsadresserad cache med LRU-tak (SPEC-7.3.1), HTTP-cache för manifest (SPEC-7.3.2), `KeyStore` med trust-on-first-use och rotation (SPEC-7.4.2, 7.4.4), `trusted = false` som begränsat läge (SPEC-7.4.3). |
| `vy-serve` | Statisk server på tiny_http: Accept-förhandling, `/_vy/<sha256>` immutable, ETag/max-age, `/.well-known/vy-key`, `/.well-known/vy-index/<sökväg>`, HTML-reserv ur `fallback`. Bibliotek + binär (1,3 MB release, 1,4 MB statisk musl). `Site::reply` är en ren funktion av anropet. |
| End-to-end | `crates/vy-serve/tests/end_to_end.rs`: publish → serve → fetch `vy://localhost` → runtime → frame; modulen lämnas ut exakt en gång och display listen är lika byte för byte. |
| Deploy | `Dockerfile` (rust:1.97-slim → statisk musl + hello-paket → scratch), `railway.toml`. |

## Tester: ignorerade → gröna

| Crate | Gröna nu | Kvar ignorerade |
| --- | --- | --- |
| `vy-manifest` | 4.1.1, 4.1.2, 4.1.3, 4.3.1, 4.3.2, 4.4.1, 4.5.1, 8.1.1, 8.1.2, 8.2.2, 8.2.3 (+ nya 7_4_1_sign_and_verify, 8_1_3_memory_max_bounds) | 8.3.1 (vy-shell avgör när reserven visas) |
| `vy-proto` | 7.1.3, 7.2.2, 7.3.1, 7.3.2, 7.4.1, 7.4.2 (well-known-delen), 7.4.3, 7.4.4, 7.5.1 | ny: `spec_7_4_2_dns_txt_must_agree` (M4: DNS TXT kräver en resolver) |
| `vy-serve` | `spec_7_2_1_end_to_end_localhost` + 8 enhetstester | – |

Backloggen (`cargo test --workspace -- --ignored --list`) gick från 48 till 29.

## Nya beroenden

Antal crates i `cargo tree --workspace -e normal`: 299 → 373 rader (169 → 212 unika). Per binär:
`vy-manifest` 49, `vy-publish` 52, `vy-serve` 55, `vy-proto` 70, `vy-cli` oförändrat 164.
Alla versioner låsta i `Cargo.lock`.

| Crate | Används i | Varför | Varför inte 200 rader själv | Drar in |
| --- | --- | --- | --- | --- |
| `minicbor` 2 (`std`, inget derive) | manifest, proto | CBOR-tokenizer. De kanoniska kontrollerna (kortaste form, bestämda längder, nyckelordning) görs i `vy_manifest::cbor` ovanpå den. | Avkodaren tar fientliga byte från nätet; en genomfuzzad tokenizer som hanterar alla majortyper, f16, taggar och bignum så att vårt lager bara behöver säga nej är värd en crate. Valdes före `ciborium`: 1 crate mot 4 + serde, och ciboriums serde-väg kan inte se kodningen alls. | 0 |
| `serde` (`derive`), `serde_json` | manifest | JSON-formen för människor och utkast (`Draft`). serde fanns redan via wasmtime. | En JSON-läsare med fält, strängescape och felpositioner är mer än 200 rader; serde_derive ger `deny_unknown_fields` gratis. | serde_json, itoa, zmij (serde, serde_derive, syn fanns) |
| `ed25519-dalek` 3 (`fast`, `alloc`, inget rand_core/pkcs8) | manifest | Signering och strikt verifiering (SPEC-7.4.1). | Kryptoprimitiver skrivs inte själv. | curve25519-dalek, curve25519-dalek-derive, ed25519, signature, subtle, zeroize, syn 2 (derive-macrot) |
| `zstd` 0.14 (inga standardfunktioner) | manifest | Paketets komprimering (SPEC-4.4.1), både packning och uppackning i samma implementation (referensbiblioteket, C via zstd-sys). | zstd-avkodning är tusentals rader. Alternativet `ruzstd` (ren Rust, bara avkodning) hade gett klienten ett rent Rust-träd men två implementationer; byt om reproducerbara byggen (M5) kräver det. | zstd-safe, zstd-sys (cc fanns) |
| `wasmparser` | manifest | Modulens initiala minnessidor för beslut 29. | Fanns i arbetsytan. | 0 |
| `ureq` 3 (`rustls`, ingen gzip/cookies/json) | proto | Blockerande HTTPS-klient med rustls + ring och inbyggda Mozilla-rötter. Valdes före `reqwest` (113 crates med tokio/hyper/tower mot 25). Omdirigeringar och gränser sköts i vy-proto; ureq står för TLS och HTTP/1.1. | TLS. | ureq-proto, http, httparse, rustls, rustls-webpki, rustls-pki-types, webpki-roots, ring, untrusted, base64, bytes, percent-encoding, utf8-zero, getrandom 0.2 |
| `tiny_http` 0.12 | serve | HTTP/1.1-server med keep-alive, chunking och trådsäker `recv`. Valdes före `axum` (45 crates med tokio). | En robust HTTP-tolk för produktion är mer än 200 rader; testservern i vy-proto är just 200 rader och bara för tester. | ascii, chunked_transfer, httpdate, log |
| `getrandom` 0.2 | publish | 32 byte OS-entropi för `keygen`. Samma version som ring redan drar in. | OS-anrop per plattform. | 0 extra |

Skrivet själv i stället för beroende: ustar-läsare/-skrivare (~120 rader, fasta metadata så samma indata
ger samma byte; `tar -tf` läser resultatet, kontrolleras i test), URL-upplösning för de former en server
kan skicka (~60 rader, i stället för `url` + `idna`), Cache-Control/Accept-tolkning, PNG-huvud (IHDR),
HTML-escape och Markdown-delmängden (rubriker och stycken), LRU-index för cachen.

## Beslut och tolkningar

- **Kanonisk CBOR.** Nycklar sorteras bytewise efter sin kodning (RFC 8949 §4.2.1): kortare nycklar
  först, sedan bytewise. Hash är `bstr(32)`, signatur `bstr(64)`, heltal utan tecken, inga taggar, inga
  flyttal. Tomma valfria listor/mappar måste utelämnas (ett uttryck per sak). Avkodaren kontrollerar
  kortaste form via antal förbrukade byte, bestämda längder, ordning, dubbletter, okända fält, efterbyte
  och 64 KB-gränsen före tolkning.
- **JSON-formen** är härledd: `"hash": "sha256:<hex>"`, `"bytes": "ed25519:<hex>"`, fältordning som i
  structen. `Draft` är utkastet utgivaren skriver (`resources[].path`, inga hash, ingen `sig`);
  `conformance/manifests/*.json` är utkast i gammal form (`capabilities: []`, `type`, platshållare) och
  behöver skrivas om efter merge till formen i `crates/vy-manifest/tests/fixtures/hello.json`.
- **Nyckel-id** = de 16 första hextecknen av SHA-256(publik nyckel). Härlett, så en nyckel har exakt ett
  id; `.well-known/vy-key` behöver bara lista nycklar.
- **Signatur och identitet.** `sig` krävs i kodningen (SPEC-8.1.1); `Manifest::encode` på en osignerad
  manifest är `Error::Unsigned`. Identiteten räknas utan `sig`, så omsignering (rotation) behåller
  identiteten. ETag i vy-serve är därför SHA-256 av svarskroppen, inte identiteten – annars skulle en
  omsignerad sida svara 304 med gammal signatur.
- **Begränsat läge** (SPEC-7.4.3) är `Fetched.trusted == false`: okänd nyckel, nyckel som inte går att
  upptäcka, eller signatur som inte verifierar. `Fetched::grants()` ger då `Denied` för allt, även `ime`.
- **Trust on first use.** Första besöket pinnar alla nycklar i `/.well-known/vy-key` för origin. Senare
  accepteras bara pinnade nycklar eller en som ett betrott manifest annonserat i `next_key_id`; den
  hämtas då från well-known och befordras. En nyckel som bara dyker upp i filen förkastas.
- **HTTP-cache.** Primärsvaret (manifest eller paket) cachas per URL med `ETag`/`max-age`; färskt svar
  ger noll anrop, gammalt svar revalideras med `If-None-Match`; `no-store`/`no-cache` lagras inte. Blobbar
  ur ett helt paket läggs i innehållscachen så ett senare manifestsvar inte hämtar dem igen.
- **Innehållscachen** verifierar hash vid varje läsning och raderar en fil som inte stämmer. LRU-ordningen
  är ett logiskt löpnummer i `blobs/index`, inte filtider, så utkastningen är deterministisk.
- **Ingen proxy ur miljön.** ureq läser `HTTPS_PROXY` som standard; vy-proto stänger av det (princip 4:
  en miljövariabel får inte styra om en sidhämtning). Uttrycklig proxy blir ett alternativ i vy-shell.
- **Accept-förhandling i vy-serve.** Klienten säger båda typerna utan q-värden (SPEC-7.1.1); servern
  väljer manifestformen så modul och resurser kan cachas per hash (SPEC-7.3.1). Bara-paket-Accept ger
  paketet; allt annat ger HTML.
- **Kroppar läses aldrig i onödan.** `NotVy` avgörs på `Content-Type`; kroppen för en okänd typ läses
  inte. Manifest ≤ 64 KB, paket ≤ 41 MB komprimerat och okomprimerat, blob ≤ sin deklarerade storlek,
  nyckelfil ≤ 16 KB.

## Spec-frågor

Numrerade 17–28 efter BUILD-NOTES 16 på den här grenens bas; main har sedan dess fått egna 17–21 (M2.6), så numren får skrivas om vid merge.

17. **SPEC-8.1.1 mot SPEC-7.4.3 – saknad signatur.** `sig` är obligatoriskt, så ett manifest utan
    `sig` avvisas vid avkodning; "utan giltig signatur" tolkas som *närvarande men ogiltig eller med
    okänd nyckel* → begränsat läge. Om specen vill att osignerade sidor ska köras begränsat bör `sig`
    flyttas till de valfria fälten.
18. **SPEC-7.4.4 saknar fält.** Rotation kräver ett signerat fält som namnger nästa nyckel-id, men
    SPEC-8.1.2 listar inget sådant (och `sig` ligger utanför signaturen). Infört som valfritt
    toppnivåfält `next_key_id`; bör in i 8.1.2.
19. **SPEC-7.2.1 – "adresser till modul och resurser".** `url` är valfritt på `module`, `resources[]`
    och `chunks[]`; saknas det hämtas `/_vy/<hex sha256>` relativt manifestets URL. vy-serve serverar
    den vägen. Konventionen bör stå i specen.
20. **SPEC-7.1.3 – localhost.** Även `127.0.0.1` och `[::1]` är undantagna; de är samma maskin.
21. **SPEC-8.2.3 – luckor i policyn.** `net` mot främmande origin och `storage` över 1 MB nämns inte;
    båda ger *fråga*. `origin_is_own` betyder att alla listade origins är sidans egen.
22. **Fälttyper som inte är specificerade.** `published` är text `YYYY-MM-DD`; `icon` och `skeleton`
    är resursindex (≥ 1); `chunks` är en lista `{hash, size, url?}`; `links`/`embeds` är textlistor
    (`vy://`/`https://` respektive `vy://`); `min_size` är `{w, h}`; `lang` kontrolleras som BCP 47-syntax.
    Hash är `bstr(32)` i CBOR och `sha256:<hex>` i JSON.
23. **SPEC-4.1.3 – tomma listor.** Kanonisk form förbjuder `resources: []`, `capabilities: {}` osv.;
    fältet utelämnas. Bör stå i specen så andra klienter gör lika.
24. **SPEC-7.4.2 – DNS TXT.** Inte implementerat (ingen resolver i trädet); bara well-known
    konsulteras, så "båda måste stämma" kan inte verkställas i M3. Ignorerat test med motivering.
25. **Nyckel-id.** Specen definierar inte "nyckel-id" (7.4.4). Härlett ur nyckeln enligt ovan.
26. **SPEC-4.1.5 – WebP/AVIF.** `w`/`h` krävs för alla bilder men verifieras bara mot PNG-huvudet;
    WebP/AVIF kräver avkodare som inte finns (BUILD-NOTES M2). Utkastet får ange `w`/`h` för dem.
27. **SPEC-7.3.2 – ETag över signaturen.** Se beslutet ovan: identiteten (SPEC-4.1.2) utesluter `sig`
    och duger inte som ETag.
28. **SPEC-9.5.1 – egen origin för localhost.** `vy://localhost:8080` räknas som samma origin som
    `http://localhost:8080` i `net`-listan, eftersom hämtningen går över http där.

## Stubbat och ogjort

- DNS TXT-uppslag (M4), HTTP/3 (ureq talar HTTP/1.1; HTTP/2 och /3 är öppna), proxyer (av).
- WebP/AVIF-huvuden läses inte.
- `vy-cli` läser fortfarande inte manifest (gräns i den här PR:en); SPEC-10.4.2/beslut 22 väntar.
- `conformance/manifests/*.json` måste skrivas om till utkastformen (gräns).
- `docker build` är inte körd: sandlådan saknar Docker-daemon. Musl-bygget
  (`CC_x86_64_unknown_linux_musl=musl-gcc cargo build --release --locked --target
  x86_64-unknown-linux-musl -p vy-serve -p vy-publish`) och paketsteget är körda lokalt med samma
  kommandon som i Dockerfile. Taggen `rust:1.97-slim-bookworm` förutsätts finnas.
- Railway: `PORT` och `RAILWAY_PUBLIC_DOMAIN` läses automatiskt; sätt `VY_HOST_NAME` för egen domän
  och byggsecret `vy_key` (32-byte frö) för en stabil utgivarnyckel. Utan secret får varje bygge en ny
  nyckel och klienter som redan pinnat den gamla ser sidan som obetrodd tills de glömmer pinnen.
- Kompressionsnivå 19 vid packning (långsam men deterministisk för en given zstd-version); ett
  8 MB-paket tar sekunder.

## Vad en merge mot main:s vy-manifest (5b26760) ska behålla

Main har under tiden fått en egen typad `Manifest`. De andra crates:en på den här grenen använder
följande av den här grenens `vy-manifest`; behåll dem (namn och signaturer) så att vy-proto,
vy-publish, vy-serve och end-to-end-testet kompilerar:

| Item | Används av |
| --- | --- |
| `Manifest` med fälten `abi: AbiVersion`, `name`, `summary`, `lang`, `module: Blob {hash, size, url?}`, `memory_max`, `published`, `resources: Vec<Resource {hash, size, mime, url?, w?, h?}>`, `capabilities: CapabilityRequests`, `links`, `fallback`, `min_size`, `icon`, `source_url`, `skeleton`, `chunks`, `embeds`, `next_key_id`, `sig: Option<Signature {key_id, bytes: SigBytes}>` | alla |
| `Manifest::{encode, decode, signed_bytes, identity, sign, verify, validate, to_json, from_json, entry}` | proto (`decode`, `verify`, `identity`), serve (`encode`), publish (`sign`, `validate`, `to_json`) |
| `Hash` (`Hash::of`, `.hex()`, `.0`, `Display` = `sha256:<hex>`, `Ord`), `from_hex::<N>`, `hex` | proto (cache-nycklar, blob-URL:er), serve (`/_vy/<hex>`) |
| `CapabilityRequests::{iter, requests, grants, is_empty}`, `Requested`, `Grant`, `default_grant` | proto (`Fetched::grants`) |
| `Package {manifest, module, resources}` med `pack`, `unpack`, `validate`, `identity`; `PackageError`; `MAX_PACKAGE_BYTES`; `MEDIA_PACKAGE`, `MEDIA_MANIFEST`, `MIME_WASM` | proto, serve, publish |
| `SigningKey::{from_seed, seed, public, sign_bytes}`, `PublicKey::{from_bytes, to_bytes, key_id, verify_bytes, FromStr/Display som hex}`, `key_id`, `key_file`, `parse_key_file` | proto (KeyStore), publish, serve |
| `Draft::{from_json, finish}`, `DraftResource`, `ResourceInput {mime, bytes, size_hint}`, `mime_for_path` | publish, e2e-test |
| `Problem` (Display), `Error` (Display; varianterna `NonCanonical`, `UnknownField`, `MissingField`, `Unsigned`, `Json` används i tester) | publish, proto |
| `png_dimensions`, `module_initial_pages` | validering; fria funktioner som main:s version kan ersätta om semantiken är samma |

Testerna i `crates/vy-manifest/src/lib.rs` pinnar dessutom den kanoniska kodningen av fixturen
(`spec_4_1_2_identity_is_hash_of_canonical_cbor`): nyckelordningen och typerna ovan är det som gör två
klienter kompatibla, så den som vinner merge-konflikten bör behålla just de byten eller ändra testet
medvetet.
