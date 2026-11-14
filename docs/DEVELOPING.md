# Bygga en Vy-sida

En Vy-sida är en Rust-crate på `vy-sdk` som byggs till en WebAssembly-modul och beskrivs av ett
manifestutkast. Det finns två sätt att bygga en: **för hand**, med loopen bygg → kontrollera →
rendera → titta, och **med Studio**, där `vy-dev serve` kör Claude Code i projektet och sidan visas i
Vy medan den byggs. Båda börjar med `vy-dev new`.

## Förutsättningar

- Rust via rustup, med målet för moduler: `rustup target add wasm32-unknown-unknown`.
- `vy-dev`, `vy-cli` och klienten `vy-shell` ur releasezippen, i samma katalog (där ligger också
  `studio.vyp`). Ur en checkout: `cargo build --release -p vy-dev -p vy-cli` och
  `tools/build_studio.sh` (skriver `target/studio.vyp`, där `vy-dev` letar).
- För Studio: Claude Code installerat och inloggat, så att `claude` finns på `PATH` (kör `claude`
  en gång i en terminal och logga in).

## För hand

```sh
vy-dev new min-sida                  # vy-sdk ur git på releasens tagg; --sdk <vy-checkout> tar den från disk
cd min-sida
cargo build --release --target wasm32-unknown-unknown
vy-cli check-abi target/wasm32-unknown-unknown/release/min_sida.wasm
vy-cli render target/wasm32-unknown-unknown/release/min_sida.wasm --out preview.png --size 800x600
vy-dev build                         # utkastet mot modulen och ett signerat paket
```

`vy-dev new` pinnar `vy-sdk` på taggen för sin egen version (`tag = "v0.3.0"` för vy-dev 0.3.0),
så att projektet bygger mot samma SDK som releasen det kom ur; med `--sdk` blir det en sökväg till
`crates/vy-sdk` i en checkout i stället. Titta på `preview.png`, ändra `src/lib.rs`, börja om.
Projektets `CLAUDE.md` har hela loopen, reglerna och ett sammandrag av SDK:t; den gäller för
människor också. Hela guiden till SDK:t och dess komponenter (`vy_sdk::ui`) står i
[`docs/SDK.md`](SDK.md).

## Publicera

`vy-dev build` signerar med utvecklarnyckeln och räcker på din egen dator. En sida som andra ska
öppna signeras med en nyckel du själv förvarar och serveras av `vy-serve`:

```sh
vy-publish keygen --out min.key          # skriver också min.key.pub; förvara min.key som ett lösenord
vy-dev build --key min.key --out sajt/min-sida.vyp
vy-publish inspect sajt/min-sida.vyp --public-key min.key.pub   # innehållet, och att signaturen gäller
vy-serve --dir sajt --port 8080 --public-key min.key.pub
vy-shell vy://localhost:8080/min-sida
```

`vy-serve` serverar varje `<namn>.vyp` i katalogen som `vy://<värd>/<namn>`, och nyckeln på
`/.well-known/vy-key`. Den lyssnar på alla nätverkskort, eftersom den är till för en värd på nätet;
`--bind 127.0.0.1` håller den till den egna datorn. På `localhost` litar klienten på nyckeln vid
första besöket. På en riktig värd ska nyckeln dessutom stå i DNS, som TXT-posten
`_vy.<värd>` med värdet `v=vy1; k=<innehållet i min.key.pub>` (SPEC-7.4.5); utan den körs sidan i
begränsat läge, utan capabilities.

## Pröva inmatning och sparat tillstånd

En bild utan fönster: `vy-shell --screenshot bild.png --size 800x600 vy://localhost:8080/min-sida`
hämtar sidan som klienten gör och ritar den med adressfält och allt. Det är sidans första bild:
kommandot tar ingen inmatning, och sidan startar med tom lagring.

Inmatning och sparat tillstånd prövas utan fönster med konformansfall: en fil som säger vilka
händelser sidan får (Tab, text, tangenter, pekare, en ny start med den gamla lagringen) och en
bild och ett semantiskt träd att jämföra med. Hur filen ser ut och var den ska ligga står i
[`docs/SDK.md`](SDK.md), avsnitt 10. Fallen kör modulen, inte det signerade paketet från servern;
det finns ännu inget sätt att ge den riktiga klienten inmatning utan fönster. I fönstret sparas
det sidan lagt i sin lagring inom ett par sekunder och när fliken göms, laddas om eller stängs.

## Med Studio

```sh
cd min-sida
vy-dev serve                         # bygger, bevakar filerna, serverar sidan och Studio
```

Öppna `vy://localhost:8081/` i Vy. Skriv ett uppdrag ("gör en sida som visar dagens väder"), tryck
**Kör**. Agentens text och verktyg syns i Studio, byggloggen bredvid, och sidan under utveckling
öppnas från länken (`vy://localhost:8080/min-sida`). `vy-dev` bygger om när en fil sparas och efter
varje körning; **Kör** blir **Stoppa** medan agenten arbetar, **Bygg om** bygger direkt.

## Portar och origins

| Adress | Vad | Varför |
| --- | --- | --- |
| `vy://localhost:8080/<namn>` | sidan under utveckling (origin A) | egen origin: `net` beviljas bara mot den utan fråga (SPEC-8.2.3, SPEC-9.5.1) |
| `vy://localhost:8081/` | Studio och dev-API:t (origin B) | API:t startar Claude Code med din inloggning och ska inte nås av sidan du bygger |

`vy-dev serve` binder bara `127.0.0.1`. API:t kräver dessutom en nyckel som byts vid varje start och
bara finns i Studio och på `api:`-raden i terminalen. Ber en sida under utveckling om nät mot
`localhost:8081`: säg nej. Andra portar: `vy-dev serve --port 8090 --studio-port 8091`; Studio
följer med. Utvecklarnyckeln skapas första gången `vy-dev build` eller `vy-dev serve` körs utan
`--key`, i klientens datakatalog: `%APPDATA%\vy\dev.key` på Windows,
`~/Library/Application Support/vy/dev.key` på macOS och `~/.config/vy/dev.key` på Linux
(`$XDG_CONFIG_HOME/vy/` om den är satt). `--key <fil>` eller `VY_DEV_KEY` väljer en annan.

## Felsökning

- **"starting claude … is Claude Code installed and on PATH?"** – `claude` hittas inte. Installera
  Claude Code, eller ange kommandot: `--agent-cmd "C:\sökväg\claude.cmd"` eller miljövariabeln
  `VY_AGENT_CMD`. På Windows hittas `claude.cmd` och `claude.exe` via `PATHEXT`.
- **Agentens logg säger "fel: API Error …" eller om inloggning** – logga in i Claude Code (`claude`
  i en terminal) och kör igen.
- **Status "fel"** – byggloggen visar varför; sidan från senaste lyckade bygge serveras vidare. Spara
  en rättelse eller tryck **Bygg om**. "can't find crate for `core`" betyder att wasm-målet saknas:
  `rustup target add wasm32-unknown-unknown`.
- **"cannot listen on 127.0.0.1:8080"** – porten är upptagen, oftast av en annan `vy-dev serve`.
  Stoppa den eller välj andra portar.
- **"no studio.vyp next to this binary"** – lägg `studio.vyp` bredvid `vy-dev`, kör
  `tools/build_studio.sh`, eller ange `--studio <fil>`. API:t svarar ändå.
- **Studio säger 403** – sidan är öppnad från en tidigare körning; nyckeln byts vid varje start.
  Ladda om `vy://localhost:8081/`.
- **Sidan körs i begränsat läge** – klienten har pinnat en annan nyckel för `localhost:8080`
  (SPEC-7.4.2), t.ex. efter en ny `dev.key`. Använd samma nyckel (`--key`) eller låt klienten glömma
  origin.
