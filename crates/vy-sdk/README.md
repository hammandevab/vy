# vy-sdk

Modulsidans SDK: en Vy-sida är en `struct` som implementerar `App`, och `vy_sdk::export!` genererar
de sex exporterna i SPEC-5.1 runt den. Alla 32 importer i namnrymden `vy` (SPEC-5.2) finns som
typade funktioner – en per import, inga alias (SPEC-2.1.1) – med statuskoder som `Result<_, VyError>`.
Sedan v0.3 returnerar varje import som kan misslyckas sin kod (SPEC-5.3.1), så ett `Err` från
`Frame::rect`, `ime_begin`, `link_register`, `random` med flera är värdens svar, inte bara SDK:ts
egen kontroll.

- `#![no_std]` + `alloc`. På `wasm32-unknown-unknown` är bindningarna riktiga importer, och SDK:t
  bidrar med allokator (`heap`), `vy_alloc`-arena (`arena`) och `panic_handler`. Nativt går samma
  anrop till `mock::MockHost`, så sid- och SDK-logik enhetstestas utan wasm-motor.
- `Frame` är display listen för ett `vy_frame`: den finns bara mellan `dl_begin` och `dl_end`
  (SPEC-5.3.2) och säger nej lokalt till det värden skulle neka – 65 537:e opkoden (SPEC-6.1.6),
  33:e klippet (SPEC-6.1.4), text över 65 535 byte (SPEC-9.3.2) – så sidan ser felet före loggen.
  `Frame::text` returnerar `Size` (bredd och höjd, SPEC-5.2.4); `Frame::clip_push(r, radius)` och
  `Frame::clipped(r, radius, |f| …)` klipper till en rektangel med hörnradie (SPEC-5.2.3).
- `Frame::sem` bygger det semantiska trädet medan du ritar (SPEC-10.1.1): `node(Role::Heading, r, text)`
  för varje nod (ett sätt, SPEC-2.1.1), `node_with_id` för ett id sidan väljer, `begin`/`end` eller
  `f.group(Role::Main, …, |f| …)` för nästning.
  Automatiska id är positiva; ett id sidan väljer med `node_with_id` är negativt (SPEC-5.2.13).
  Registrera hela trädet varje frame: värden äger fokus och går Tab-ordningen ur trädets
  fokuserbara noder (SPEC-10.3.1); sidan får `Event::Focus` och ritar ringen själv.
- Ingen layoutmotor: layouten bor i modulen (SPEC-2.1.3). Mönstret referenssidorna använder är en
  `layout()`-funktion som räknar ut rektanglarna ur fönstret, så att `event` träfftestar och `frame`
  ritar samma rektanglar – se nedan. `vy_sdk::math` har `floor`, `round`, `ceil` och `snap(v, scale)`
  (närmaste fysiska pixelgräns) för 1 px-linjer; `core` saknar dem och SDK:t drar inte in `libm`.
- `Event` avkodar `vy_event` enligt bilaga A (med `PointerLeave`); `event::key` har de vanligaste
  USB HID-koderna. Tab når aldrig en sida – värden gör `FOCUS` av den; Enter och Escape kommer
  som `KeyDown`. `dt_ms` i `App::frame` är tiden sedan föregående frame och 0 efter start och
  efter att fliken visats igen (SPEC-5.1.2).

## Exempel

```rust
#![no_std]
use vy_sdk::prelude::*;

/// Rektanglarna räknas ut på ett ställe, ur fönstret: `event` träfftestar
/// mot samma rektanglar som `frame` ritar (layout()-mönstret).
struct Layout { card: Rect, button: Rect }

struct Page { window: Window, title: StyleId, body: StyleId, clicks: u32 }

impl Page {
    fn layout(&self) -> Layout {
        let card = Rect::new(24.0, 24.0, (self.window.width - 48.0).max(0.0), 160.0);
        let button = Rect::new(card.x + 24.0, card.bottom() - 60.0, 120.0, 36.0);
        Layout { card, button }
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        // Ett fel här blir vy_init:s returvärde (SPEC-5.1.1); värden avvisar sidan.
        let title = text_style(Font::Sans, 28.0, 600, Color::rgb(0x1A, 0x1A, 0x2E))?;
        let body = text_style(Font::Sans, 16.0, 400, Color::rgb(0x5F, 0x5E, 0x5A))?;
        Ok(Self { window, title, body, clicks: 0 })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        let area = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(area, 0.0, Color::rgb(0xF5, 0xF5, 0xF0))?;
        f.rect(l.card, 12.0, Color::rgb(0xFF, 0xFF, 0xFF))?;
        f.group(Role::Main, l.card, "", |f| {
            let x = l.card.x + 24.0;
            let w = l.card.w - 48.0;
            // dl_text returnerar storleken, så nästa element följer efter (SPEC-5.2.4).
            let h = f.text(x, l.card.y + 24.0, w, self.title, "Hej från Vy")?.h;
            f.sem.node(Role::Heading, Rect::new(x, l.card.y + 24.0, w, h), "Hej från Vy")?;
            let y = l.card.y + 24.0 + h + 8.0;
            let h = f.text(x, y, w, self.body, "Klicka på knappen.")?.h;
            f.sem.node(Role::Text, Rect::new(x, y, w, h), "Klicka på knappen.")?;
            f.rect(l.button, 6.0, Color::rgb(0x1A, 0x1A, 0x2E))?;
            // En 1 px-linje på en fysisk pixelgräns, även vid skala 1,5.
            let rule_y = vy_sdk::math::snap(l.button.y - 12.0, self.window.scale);
            f.rect(Rect::new(x, rule_y, w, vy_sdk::math::snap(1.0, self.window.scale)), 0.0, Color::rgb(0xC9, 0xC2, 0xB4))?;
            f.sem.node_with_id(-1, Role::Button, l.button, "Räkna")
        })?;
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { x, y, .. } if self.layout().button.contains(x, y) => { self.clicks += 1; true }
            Event::Focus { node, gained, .. } => node == NodeId(-1) && gained, // rita ringen
            _ => false,
        }
    }

    fn resize(&mut self, window: Window) { self.window = window; }
}

vy_sdk::export!(Page);
```

Bygg med `cargo build --release --target wasm32-unknown-unknown` (se `tools/build_modules.sh`),
kontrollera med `vy-cli check-abi` och rendera med `vy-cli render`. Referenssidor: `modules/`.

## Testa nativt

```rust
vy_sdk::mock::reset();
let mut page = Page::init(Window { width: 800.0, height: 600.0, scale: 1.0 }, 0);
let mut f = Frame::begin();
page.frame(&mut f, 16).unwrap();
assert_eq!(f.end(), Ok(4), "bakgrund, två texter, knapp");
vy_sdk::mock::with(|h| assert_eq!(h.sem.len(), 4, "main, heading, text, button"));
```

Mockens textmodell är fast (varje tecken `storlek × 0,6` brett, rader `storlek × 1,25` höga); pixlar
kommer bara från `vy-cli render` och konformanssviten.
