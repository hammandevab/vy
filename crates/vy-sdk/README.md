# vy-sdk

Modulsidans SDK: en Vy-sida är en `struct` som implementerar `App`, och `vy_sdk::export!` genererar
de sex exporterna i SPEC-5.1 runt den. `vy_sdk::ui` har färdiga komponenter – textfält, knappar,
kryssruta, radiogrupp, slider, lista, rullning och layoutstackar – se *Komponenter* nedan; guiden
`docs/SDK.md` går igenom en hel sida från början. Alla 32 importer i namnrymden `vy` (SPEC-5.2) finns som
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

## Komponenter (`vy_sdk::ui`)

Färdiga kontroller, så att en sida inte skriver textfält, knappar, listor och rullning själv. De är
vanlig modulkod (inget nytt i ABI:t) och följer `layout()`-mönstret: sidan räknar ut rektangeln,
`draw` ritar i den och registrerar noden med rätt roll (SPEC-10.1.1), `event` träfftestar mot samma
rektangel. Regler för alla:

- **Ett tema**, byggt en gång i `init`: `Theme::new()?` registrerar fyra textstilar och mäter
  radhöjden. `Theme::custom(Palette { accent: …, ..Palette::LIGHT }, Font::Sans, 16.0)?` för andra
  färger, `Palette::DARK` för mörkt, `theme.words = Words::EN` för engelska tillståndsord.
- **Samma form överallt:** `x.draw(f, rektangel, &theme, …)` i `frame` och
  `x.event(&mut ui, rektangel, &theme, …) -> Response` i `event`, med samma rektangel.
- **Händelser genom `Input`:** `let mut ui = Input::new(event);`, ge `&mut ui` till varje komponent –
  inre före yttre – och returnera `ui.redraw()`. En komponent som använder en pekar-, rullnings-,
  tangent- eller texthändelse tar den, så att komponenter efter den inte ser den; en fokushändelse
  tas aldrig. Det ingen tog finns i `ui.unhandled()` för sidans egna tangenter.
- **`Response`:** `.activated()` (knapp tryckt, Enter i ett enradsfält, rad öppnad), `.changed()`
  (värdet ändrat), `.redraw()`.
- **Fasta negativa nod-id** för allt fokuserbart, i Tab-ordning: `TextBox::new(-1)`, `Button::new(-2)`.
  Fokus är värdens (SPEC-10.3.1); komponenten ritar ringen (SPEC-10.3.3).
- `List` och `ScrollView` har ingen fokuserbar roll (SPEC-10.3.2): de tar tangenter när de är
  *aktiva*, efter ett tryck i dem som inte gav någon nod fokus, eller efter `set_active(true)`.

En sida med ett fält och en knapp:

```rust
use vy_sdk::prelude::*;
use vy_sdk::ui::{Button, Input, Label, TextBox, Theme};

struct Page { window: Window, theme: Theme, name: TextBox, send: Button, sent: bool }

impl Page {
    fn layout(&self) -> (Rect, Rect) {
        let w = (self.window.width - 48.0).max(0.0);
        (Rect::new(24.0, 24.0, w, 40.0), Rect::new(24.0, 76.0, 120.0, 40.0))
    }
}

impl App for Page {
    fn init(window: Window, _: i32) -> Result<Self, VyError> {
        Ok(Page { window, theme: Theme::new()?, name: TextBox::new(-1), send: Button::new(-2), sent: false })
    }
    fn frame(&mut self, f: &mut Frame, _: i32) -> Result<(), VyError> {
        let (field, button) = self.layout();
        self.name.draw(f, field, &self.theme, Label::hidden("Namn"), "Ditt namn")?;
        self.send.draw(f, button, &self.theme, "Skicka")
    }
    fn event(&mut self, event: Event<'_>) -> bool {
        let (field, button) = self.layout();
        let mut ui = Input::new(event);
        let enter = self.name.event(&mut ui, field, &self.theme).activated();
        if self.send.event(&mut ui, button, &self.theme).activated() || enter {
            self.sent = true;
        }
        ui.redraw()
    }
    fn resize(&mut self, window: Window) { self.window = window; }
}
```

### TextBox

Ett textfält på en rad (`TextBox`) eller flera (`TextBox<Multiline>`): markör, markering med Skift
och pekaren, ord med Ctrl, Ctrl+A, Ctrl+C och Ctrl+X genom `clipboard_write` under
kopieringsgesten (SPEC-8.2.4; klipp tar bara bort texten om skrivningen lyckades), inklistring som
`TEXT_INPUT`, IME med `ime_begin`/`ime_end` när fältet får och tappar fokus. Manifestet måste begära
`ime` (SPEC-10.4.2). Noden är en `textbox` med värdet. Fältets namn är en `text`-nod som `draw`
registrerar precis före fältet, så att det finns i trädet också när fältet är tomt (platshållaren
ritas bara): `Label::shown(rektangel, "Namn")` för en etikett som sidan själv ritar i `rektangel`
med `f.text` (utan egen nod), `Label::hidden("Sök")` för ett namn som inte ritas någonstans; det
får då fältets hörn och ingen storlek. Fokuserat får fältet 2 px ram i accentfärgen.

```rust
name: TextBox::new(-1).with_max_len(200),        // byte; längre inmatning kapas
note: TextBox::multiline(-2),                     // fältets typ: TextBox<Multiline>

f.text(l.label.x, l.label.y, l.label.w, self.label, "Namn")?;   // etiketten ritar sidan
self.name.draw(f, l.name, &self.theme, Label::shown(l.label, "Namn"), "Förnamn Efternamn")?;
self.note.draw(f, l.note, &self.theme, Label::hidden("Anteckning"), "Skriv här")?;   // platshållare när tomt
let r = self.name.event(&mut ui, l.name, &self.theme);
if r.activated() { /* Enter i ett enradsfält */ }
let value: &str = self.name.text();               // set_text, select_all, selection, last_copy
```

Höjden `ui::CONTROL_H` (40) passar en rad. Ett flerradsfält bryter vid ordgränser, Enter ger ny rad,
och hjulet rullar det när pekaren är över.

### Button

```rust
save: Button::new(-3),                            // fylld; Button::secondary(-4) för en kontur
self.save.draw(f, l.save, &self.theme, "Spara")?;
if self.save.event(&mut ui, l.save, &self.theme).activated() { self.save(); }
```

Aktiveras av ett tryck med primärknappen (på `POINTER_DOWN`, som referenssidorna) och av Enter
eller mellanslag med fokus.

### Checkbox

```rust
news: Checkbox::new(-5, false),
self.news.draw(f, l.news, &self.theme, "Skicka nyhetsbrevet")?;  // rutan, etiketten till höger
if self.news.event(&mut ui, l.news, &self.theme).changed() { /* self.news.checked */ }
```

Ett tryck var som helst i rektangeln eller mellanslag med fokus växlar. Nodens text får
`Words::checked` (" (markerad)") när rutan är kryssad.

### RadioGroup

```rust
const SIZES: [&str; 3] = ["Liten", "Mellan", "Stor"];
size: RadioGroup::new(-6, SIZES.len(), 1),       // id -6, -7, -8; alternativ 1 valt
self.size.draw(f, l.sizes, &self.theme, &SIZES)?; // rektangeln delas i lika höga rader
if self.size.event(&mut ui, l.sizes, &self.theme).changed() { /* self.size.selected */ }
```

Varje alternativ är en egen `radio`-nod och ett eget Tab-stopp; tryck eller mellanslag väljer.

### Slider

```rust
volume: Slider::new(-9, 0, 100, 40),              // .with_step(5) för större steg
self.volume.draw(f, l.volume, &self.theme, "Volym")?;  // nodens text "Volym: 40"
if self.volume.event(&mut ui, l.volume, &self.theme).changed() { /* self.volume.value() */ }
```

Dras med pekaren; med fokus flyttar pilarna ett steg, Page Up/Down tio, Home/End till ändarna.

### List

```rust
const CITIES: [&str; 3] = ["Stockholm", "Göteborg", "Malmö"];
list: List::new(36.0),                            // radhöjd
self.list.draw(f, l.list, &self.theme, &CITIES)?;
let r = self.list.event(&mut ui, l.list, &self.theme, &CITIES);
if r.activated() { /* Enter, eller tryck på den markerade raden: self.list.selected() */ }
```

Ritar bara raderna i bild men registrerar en `listitem` per rad (SPEC-10.1.1). Rader är allt som
implementerar `Rows` (`len`, `label` och, om en textrad inte räcker, `draw_row`); `[&str]`,
`[&str; N]` och `[String]` gör det redan (`&items[..]`). Pilar, Page Up/Down, Home/End och Enter när
listan är aktiv; hjulet och rullningslisten alltid.

### ScrollView

```rust
main: ScrollView::new(),
// layout(): lägg ut innehållet med förskjutningen avdragen och räkna dess höjd
let top = 24.0 - self.main.offset();
// frame: rita innehållet i draw (ScrollView är Copy, så closuren kan låna self)
let mut main = self.main;
main.draw(f, view, &self.theme, l.content_h, |f| self.draw_content(f, &l))?;
self.main = main;
// event: sist, efter komponenterna inuti
self.main.event(&mut ui, view, &self.theme, l.content_h);
```

Klipper till vyn och ritar en dragbar rullningslist; hjulet rullar ytan under pekaren och lämnas åt
en yttre yta vid ändarna. När värden flyttar tangentbordsfokus till en kontroll utanför bild:
`self.main.reveal(rektangel, view, l.content_h)`. I `resize`: `self.main.clamp(view, content_h)`.

### Column, Row och split

```rust
let mut c = Column::new(card, 20.0, 12.0);        // område, marginal, mellanrum
let title = c.next(32.0);
let field = c.next(ui::CONTROL_H);
let mut row = Row::new(c.next(ui::CONTROL_H), 0.0, 8.0);
let save = row.next(120.0);
let cancel = row.next(120.0);
let card = Rect::new(card.x, card.y, card.w, c.height());   // höjden kortet behöver
let [left, right] = ui::split::<2>(area, 16.0);            // lika breda kolumner
```

Hela galleriet finns i `modules/widgets`; en guide med en hel sida från början i `docs/SDK.md`.

## Testa nativt

```rust
vy_sdk::mock::reset();
let mut page = Page::init(Window { width: 800.0, height: 600.0, scale: 1.0 }, 0).unwrap();
assert_eq!(vy_sdk::mock::run_frame(&mut page, 16), Ok(6), "bakgrund, kort, två texter, knapp, linje");
vy_sdk::mock::with(|h| assert_eq!(h.sem.len(), 4, "main, heading, text, button"));
```

`Frame` kan bara skapas av exportlimmet (och av `mock::run_frame` nativt), så en display list
utanför `vy_frame` går inte att skriva från Rust (SPEC-5.3.2). Texten i en `TEXT_INPUT` lånas ur
`vy_alloc`-arenan bara för det `vy_event` som bär den (SPEC-5.1.6): `Event<'_>` kan inte sparas.

Mockens textmodell är fast (varje tecken `storlek × 0,6` brett, rader `storlek × 1,25` höga); pixlar
kommer bara från `vy-cli render` och konformanssviten. `mock::as_copy_gesture(|| …)` kör en händelse
så som värden levererar Ctrl+C: `clipboard_write` lyckas då utan capability (SPEC-8.2.4).
Mocken räknar inget bränsle, men `MockHost::text_bytes` räknar de byte text sidan lämnat till
`text_measure`, `text_caret` och `dl_text`, som värden debiterar per byte (SPEC-5.3.3): en layout
som mäter samma text om och om igen syns där, och ett test kan sätta ett tak för den.
