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
vy-dev new min-sida                  # --sdk <vy-checkout> för vy-sdk från disk i stället för git
cd min-sida
cargo build --release --target wasm32-unknown-unknown
vy-cli check-abi target/wasm32-unknown-unknown/release/min_sida.wasm
vy-cli render target/wasm32-unknown-unknown/release/min_sida.wasm --out preview.png --size 800x600
vy-dev build                         # utkastet mot modulen och ett signerat paket
```

Titta på `preview.png`, ändra `src/lib.rs`, börja om. Projektets `CLAUDE.md` har hela loopen,
reglerna och ett sammandrag av SDK:t; den gäller för människor också.

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

Allt binder `127.0.0.1`. API:t kräver dessutom en nyckel som byts vid varje start och bara finns i
Studio och på `api:`-raden i terminalen. Ber en sida under utveckling om nät mot `localhost:8081`:
säg nej. Andra portar: `vy-dev serve --port 8090 --studio-port 8091`; Studio följer med.
Utvecklarnyckeln skapas första gången i klientens katalog (`dev.key`), eller `--key <fil>`.

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
