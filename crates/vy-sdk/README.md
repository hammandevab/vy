# vy-sdk

Modulsidans SDK: en Vy-sida är en `struct` som implementerar `App`, och `vy_sdk::export!` genererar
de sex exporterna i SPEC-5.1 runt den. Alla 33 importer i namnrymden `vy` (SPEC-5.2) finns som
typade funktioner – en per import, inga alias (SPEC-2.1.1) – med statuskoder som `Result<_, VyError>`.

- `#![no_std]` + `alloc`. På `wasm32-unknown-unknown` är bindningarna riktiga importer, och SDK:t
  bidrar med allokator (`heap`), `vy_alloc`-arena (`arena`) och `panic_handler`. Nativt går samma
  anrop till `mock::MockHost`, så sid- och SDK-logik enhetstestas utan wasm-motor.
- `Frame` är display listen för ett `vy_frame`: den finns bara mellan `dl_begin` och `dl_end`
  (SPEC-5.3.2) och säger nej lokalt till det värden skulle neka – 65 537:e opkoden (SPEC-6.1.6),
  33:e klippet (SPEC-6.1.4), text över 65 535 byte (SPEC-9.3.2) – så sidan ser felet före loggen.
- `Frame::sem` bygger det semantiska trädet medan du ritar (SPEC-10.1.1): `heading`, `text`,
  `button`, `textbox`, `checkbox`, `link`, `image`, eller `f.group(Role::Main, …, |f| …)` för nästning.
- `Column`/`Row` är en liten immediate-mode-layout: padding, gap, nästa rektangel, text som mäter
  sig själv via värden. Ingen flexbox; layouten bor i modulen (SPEC-2.1.3).
- `Event` avkodar `vy_event` enligt bilaga A; `event::key` har de vanligaste USB HID-koderna.

## Exempel

```rust
#![no_std]
use vy_sdk::prelude::*;

struct Page { window: Window, title: StyleId, body: StyleId, clicks: u32 }

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Self {
        let title = text_style(Font::Sans, 28.0, 600, Color::rgb(0x1A, 0x1A, 0x2E)).unwrap_or(StyleId(0));
        let body = text_style(Font::Sans, 16.0, 400, Color::rgb(0x5F, 0x5E, 0x5A)).unwrap_or(StyleId(0));
        Self { window, title, body, clicks: 0 }
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let area = Rect::new(0.0, 0.0, self.window.width, self.window.height);
        f.rect(area, 0.0, Color::rgb(0xF5, 0xF5, 0xF0))?;
        let mut col = Column::new(area).padding(24.0).gap(12.0);
        f.group(Role::Main, area, "", |f| {
            let r = col.text(f, self.title, "Hej från Vy")?;
            f.sem.heading(r, "Hej från Vy")?;
            let r = col.text(f, self.body, "Klicka var som helst.")?;
            f.sem.text(r, "Klicka var som helst.")?;
            let button = col.next(36.0);
            f.rect(button, 6.0, Color::rgb(0x1A, 0x1A, 0x2E))?;
            f.sem.button(button, "Räkna")
        })?;
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        match event {
            Event::PointerDown { .. } => { self.clicks += 1; true }
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
