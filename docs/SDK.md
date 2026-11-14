# Skriva en Vy-sida med vy-sdk

En guide för den som ska bygga en sida – människa eller agent – och inte har sett Vy förut. Den
förutsätter Rust men inget om Vy. API-referensen är rustdoc för `vy-sdk` (`tools/build_docs.sh`
lägger den bredvid den här filen); den normativa texten är `SPEC.md`, och id:n som `SPEC-10.3.1`
pekar dit.

## 1. Tankemodellen

En Vy-sida är en WebAssembly-modul som ritar sig själv. Det finns ingen HTML, ingen CSS, ingen
layoutmotor och ingen DOM: sidan är en `struct` som implementerar `App`, och värden anropar den.

- **`layout()` räknar ut rektanglarna.** Varje sida har en funktion som ur fönstret (och sitt
  tillstånd) räknar fram var allt ligger. Det är den enda layouten som finns (SPEC-2.1.3).
- **`frame` ritar och registrerar trädet.** Värden anropar `App::frame` när den vill ha en bild.
  Sidan ritar allt från början (rektanglar, text, bilder, kurvor) i en *display list* och
  registrerar samtidigt det semantiska trädet – en nod med roll och text för varje synlig text och
  kontroll (SPEC-6.1.2, SPEC-10.1.1).
- **`event` ändrar tillstånd.** Värden levererar inmatning och systemhändelser till `App::event`.
  Sidan ändrar sitt tillstånd och svarar `true` om den vill ritas om (SPEC-5.1.3). Den träfftestar
  mot samma rektanglar som `frame` ritade, eftersom båda frågar `layout()`.

Inget händer mellan händelserna: en stillastående sida kostar ingen CPU (SPEC-6.2.2). En animation
ber om nästa frame med `request_frame()`.

## 2. De sex exporterna

Modulen exporterar exakt sex funktioner och sitt minne (SPEC-4.2.1). Du skriver dem aldrig själv:
`vy_sdk::export!(Page);` sist i `src/lib.rs` genererar dem runt din typ.

| Export | Anropas | `App`-metod |
| --- | --- | --- |
| `vy_init(abi_minor, w, h, scale)` | en gång efter laddning | `init(window, abi_minor) -> Result<Self, VyError>`; ett `Err` stoppar sidan |
| `vy_frame(dt_ms)` | när värden vill ha en bild | `frame(&mut self, f: &mut Frame, dt_ms)` |
| `vy_event(kind, a, b, c, d)` | vid inmatning och systemhändelser | `event(&mut self, event: Event) -> bool` |
| `vy_resize(w, h, scale)` | fönstret ändrade storlek eller DPI | `resize(&mut self, window)` |
| `vy_suspend()` | fliken göms | `suspend(&mut self) -> i32` (valfri; spara tillstånd här) |
| `vy_alloc(size)` | värden ska skriva text till sidan | sköts av SDK:t |

`Frame` finns bara under `frame`; det går inte att rita någon annanstans (SPEC-5.3.2). Texten i
`Event::TextInput` lånas bara under det anrop som bär den (SPEC-5.1.6).

## 3. Komponenterna (`vy_sdk::ui`)

Textfält, knappar och listor behöver du inte skriva själv. `vy_sdk::ui` har färdiga komponenter som
följer samma mönster som sidan: du räknar ut rektangeln i `layout()`, komponenten ritar sig och
registrerar sin nod i `draw`, och hanterar sina händelser i `event` mot samma rektangel.

| Komponent | Roll i trädet | Gör |
| --- | --- | --- |
| `TextBox::new(id)`, `TextBox::multiline(id)` | `text` (fältets namn) + `textbox` | markör, markering med Skift och pekaren, Ctrl+A/C/X, inklistring, IME, radbrytning |
| `Button::new(id)`, `Button::secondary(id)` | `button` | aktiveras av tryck, Enter och mellanslag |
| `Checkbox::new(id, checked)` | `checkbox` | växlar med tryck och mellanslag |
| `RadioGroup::new(first_id, n, selected)` | `radio` × n | ett tryck eller mellanslag väljer |
| `Slider::new(id, min, max, value)` | `slider` | dras med pekaren, pilar, Page Up/Down, Home/End; ritar sitt värde till höger om spåret, inte sitt namn |
| `List::new(row_h)` | `list` + `listitem` × n | virtualiserad, en markerad rad, pilar och Enter |
| `ScrollView::new()` | – | rullar innehåll med hjul, tangenter och list |
| `Column`, `Row`, `split` | – | staplar rektanglar med mellanrum och marginal |

Regler som gäller alla:

- **Ett tema.** `let theme = Theme::new()?;` i `init` registrerar textstilarna. `Theme::custom(palett,
  typsnitt, storlek)` för andra färger; `theme.words = Words::EN` för engelska tillståndsord.
- **Samma form överallt.** `komponent.draw(f, rektangel, &theme, …)` i `frame` och
  `komponent.event(&mut ui, rektangel, &theme, …)` i `event`, med samma rektangel.
- **Händelser genom `Input`.** I `App::event`: `let mut ui = Input::new(event);`, ge `&mut ui` till
  varje komponent (inre före yttre – en lista i en rullyta före rullytan), returnera `ui.redraw()`.
  En komponent som använder en pekar-, tangent- eller texthändelse *tar* den, så komponenter efter
  den ser den inte; en fokushändelse tas aldrig. Det ingen tog får sidan med `ui.unhandled()`.
- **Svaret** från `event` är en `Response`: `.activated()` (knapp tryckt, Enter i ett enradsfält,
  rad öppnad), `.changed()` (värdet ändrat), `.redraw()`.
- **Fasta negativa nod-id** för allt som kan få fokus: `TextBox::new(-1)`, `Button::new(-2)`. Fokus
  följer id:t mellan frames (SPEC-10.3.2), och Tab går i den ordning noderna registreras.
- **Ett textfält har alltid ett namn.** En `textbox`-nods text är fältets värde, och platshållaren
  ritas bara, så `TextBox::draw` tar fältets namn som en `Label` och registrerar det som en
  `text`-nod precis före fältet: `Label::shown(rektangel, "Namn")` för en etikett som sidan själv
  ritar i `rektangel` (med `f.text`, utan egen nod), `Label::hidden("Ny uppgift")` för ett namn som
  inte ritas någonstans. Utan namn vet en skärmläsare inte vad ett tomt fält är till för.
- **En lista eller rullyta har ingen fokuserbar roll** (SPEC-10.3.2). Den tar tangenter när den är
  aktiv: efter ett tryck i den som inte gav någon nod fokus, eller när sidan sagt `set_active(true)`.
- **Knappar aktiveras på `POINTER_DOWN`**, som i alla referenssidor.

Sidan `modules/widgets` visar alla komponenter tillsammans.

## 4. Capabilities och manifestet

Allt utöver beräkning och ritning är en *capability* som manifestet begär och användaren eller
policyn beviljar (SPEC-2.1.4, SPEC-8.2.1). Manifestutkastet (`manifest.json`) beskriver sidan:

```json
{
  "abi": "1.0",
  "name": "Att göra",
  "summary": "En lista med uppgifter.",
  "lang": "sv",
  "memory_max": 32,
  "published": "2026-11-06",
  "capabilities": {"ime": {}},
  "fallback": "# Att göra\n\nSidan kräver en Vy-klient."
}
```

| Capability | Ger | Standardpolicy (SPEC-8.2.3) |
| --- | --- | --- |
| `ime` | `ime_begin`/`ime_end`; krävs av varje sida med ett textfält (SPEC-10.4.2) | beviljas |
| `net` `{"origins": ["https://api.example"]}` | `fetch_begin` mot de listade origins (högst 16) | egen origin beviljas, annan efter fråga |
| `storage` `{"quota_kb": 64}` | `state_put`/`state_get`/`state_delete` per origin | upp till 1 MB beviljas |
| `clipboard` | `clipboard_read`, och `clipboard_write` utanför kopieringsgesten | efter fråga |
| `hires_time` | 10 µs upplösning i `now_ms` | nekas |
| `persist_frame` | frames medan fliken är gömd | efter fråga |

- Ett anrop utan beviljad capability ger `Err(VyError::Denied)`; sidan ska fortsätta och visa det,
  aldrig krascha (SPEC-5.3.1). `vy-cli render` beviljar bara det du ger med `--grant`.
- **Kopiering behöver ingen capability:** `clipboard_write` medan sidan hanterar Ctrl+C eller Ctrl+X
  lyckas ändå (SPEC-8.2.4) – `TextBox` gör det åt dig. Ctrl+V kommer som `Event::TextInput`.
- **Bilder** är resurser: `"resources": [{"path": "bild.png", "mime": "image/png", "w": 320, "h": 200}]`.
  `w` och `h` är obligatoriska och måste stämma med filen (SPEC-8.1.2). Resurs `i` i listan är
  `ResourceId(i + 1)` (SPEC-4.5.1).
- `memory_max` är i wasm-sidor à 64 KB och minst modulens initiala minne plus en (SPEC-8.1.3);
  `vy-cli check-abi` säger modulens.
- **Lagring** är nycklar och värden i byte, per origin. Läs i `init`, skriv när något ändrats:

  ```rust
  // i init: värdet från förra besöket, om det finns och går att läsa
  let mut buf = [0u8; 64];
  let saved = match state_get("val", &mut buf) {
      Ok(n) if n <= buf.len() => Some(&buf[..n]),   // n är värdets hela längd, även över buf
      _ => None,                                     // inget sparat, nekad lagring eller för långt
  };
  // efter en ändring
  let saved_now = state_put("val", &bytes).is_ok(); // Err(Denied) utan `storage`: visa "sparas inte"
  ```

  Värden skriver det sparade till disk inom ett par sekunder, och när fliken göms, laddas om eller
  stängs. Läs alltid tillbaka som indata (avsnitt 9, "Sparad data är indata").

## 5. Determinism

Samma modul, samma indata och samma fönster ger samma display list, byte för byte (SPEC-2.1.5).
Det är vad som gör pixeltesterna möjliga.

- Tid bara via `now_ms()` och `dt_ms`, slump bara via `random()`; headless är båda deterministiska
  (SPEC-5.2.11).
- Inga egna klockor, inga globala räknare utanför sidans tillstånd, ingen iteration över en
  `HashMap` (finns inte i `no_std` ändå; använd `BTreeMap` eller `Vec`).
- Mät text med `text_measure` eller använd storleken `f.text` returnerar; gissa aldrig en bredd.

## 6. Budgeten

`vy_frame` har 8 ms logisk tid, och värdens arbete i importerna (textformning) räknas in
(SPEC-5.3.3). Första överskridandet ger `Event::Budget`, andra i rad stoppar sidan tills vidare.
Rasteriseringen av din display list har samma budget, räknad i pixlar: en helskärmsrektangel per
rad kostar mer än du tror. `vy-dev build` mäter de första framen och varnar i byggloggen, och
`vy-publish build` publicerar inte en sida som bryter budgeten utan `--allow-slow` (SPEC-13.4.1).

- Rita bara det som syns: `List` ritar bara raderna i bild, och en `ScrollView` klipper.
- Mät inte samma text många gånger per frame; räkna ut layouten en gång per frame (eller per
  händelse) och återanvänd den.
- Allokera inte obegränsat per frame: bygg inte strängar för tusentals rader, sätt tak på listor
  och fält (`with_max_len`).

## 7. Tillgänglighet

- **Trädet är obligatoriskt i praktiken.** En sida utan träd märks *otillgänglig* (SPEC-10.4.1).
  Registrera en nod för varje synlig text och kontroll, varje frame, även det som ligger utanför
  fönstret (SPEC-10.1.1). Rubriker `Role::Heading`, brödtext `Role::Text`, grupper med
  `f.group(Role::Main | Region | Nav, …)`.
- **Etiketter är egna noder.** Ett textfälts nod bär värdet; rita etiketten ovanför med `f.text` och
  registrera den som `Role::Text` före fältet, så hörs den först.
- **Fokus ägs av värden** (SPEC-10.3.1): Tab når aldrig sidan. `Event::Focus { node, gained, .. }`
  säger vart fokus gick; sidan ritar ringen själv (SPEC-10.3.3). Komponenterna gör båda. En sida
  kan inte begära fokus.
- **Tangentbordet räcker.** Allt som går med pekaren ska gå med Tab, Enter, mellanslag och pilar.
  När Tab flyttar fokus till en kontroll utanför bild ska sidan rulla dit
  (`ScrollView::reveal`).
- **Tillstånd står i texten**, eftersom en nod bara har roll och text: "Nyheter (markerad)",
  "Volym: 40". Komponenterna lägger till det.

## 8. En hel sida från början

`vy-dev new att-gora` skriver projektet; byt `src/lib.rs` och `manifest.json` mot det här. Sidan är
en lista med uppgifter: skriv i fältet, Enter eller *Lägg till* lägger till, klicka på en rad och
*Ta bort* tar bort den. Testet `sdk_guide_example_builds` i `vy-dev` bygger exakt den här koden till
wasm och kör en frame, så den är aktuell.

Manifestet är det i avsnitt 4. Hela `src/lib.rs`:

<!-- exempel: src/lib.rs -->
```rust
//! Att göra: en lista med uppgifter. Skriv i fältet, Enter eller
//! "Lägg till" lägger till; klicka på en rad och "Ta bort" tar bort den.
#![cfg_attr(not(test), no_std)]

extern crate alloc;

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use vy_sdk::prelude::*;
use vy_sdk::ui::{self, Button, Column, Input, Label, List, Row, TextBox, Theme};

const BG: Color = Color::rgb(0xF2, 0xF3, 0xF5);
const CARD: Color = Color::rgb(0xFF, 0xFF, 0xFF);
const MAX_TASKS: usize = 100;

// Fasta, negativa nod-id för det som kan få fokus (Tab-ordning = ritordning).
const ID_TASK: i32 = -1;
const ID_ADD: i32 = -2;
const ID_REMOVE: i32 = -3;

/// Alla rektanglar, räknade ur fönstret. frame ritar dem, event träfftestar dem.
struct Layout {
    card: Rect,
    title: Rect,
    field: Rect,
    add: Rect,
    list: Rect,
    remove: Rect,
    count: Rect,
}

struct Page {
    window: Window,
    theme: Theme,
    title: StyleId,
    tasks: Vec<String>,
    field: TextBox,
    add: Button,
    remove: Button,
    list: List,
}

impl Page {
    fn layout(&self) -> Layout {
        let w = (self.window.width - 48.0).clamp(0.0, 560.0);
        let area = Rect::new((self.window.width - w) / 2.0, 24.0, w, 0.0);
        let mut c = Column::new(area, 20.0, 12.0);
        let title = c.next(32.0);
        let mut row = Row::new(c.next(ui::CONTROL_H), 0.0, 8.0);
        let add = Rect::new(0.0, 0.0, 120.0, 0.0);
        let field = row.next((row.rest().w - add.w - 8.0).max(0.0));
        let add = row.next(add.w);
        let list_h = (self.window.height - c.bottom() - 160.0).clamp(120.0, 400.0);
        let list = c.next(list_h);
        let mut row = Row::new(c.next(ui::CONTROL_H), 0.0, 12.0);
        let remove = row.next(160.0);
        let count = row.rest();
        let card = Rect::new(area.x, area.y, area.w, c.height());
        Layout { card, title, field, add, list, remove, count }
    }

    /// Lägg till fältets text som en uppgift; true om något ändrades.
    fn add_task(&mut self) -> bool {
        let text = self.field.text().trim();
        if text.is_empty() || self.tasks.len() >= MAX_TASKS {
            return false;
        }
        self.tasks.push(String::from(text));
        self.field.set_text("");
        true
    }

    fn remove_selected(&mut self, l: &Layout) -> bool {
        let Some(i) = self.list.selected() else {
            return false;
        };
        self.tasks.remove(i);
        let next = i.min(self.tasks.len().saturating_sub(1));
        let next = (!self.tasks.is_empty()).then_some(next);
        self.list.select(next, l.list, self.tasks.len());
        true
    }
}

impl App for Page {
    fn init(window: Window, _abi_minor: i32) -> Result<Self, VyError> {
        let theme = Theme::new()?;
        Ok(Self {
            window,
            title: text_style(Font::Sans, 26.0, 700, theme.palette.ink)?,
            theme,
            tasks: Vec::new(),
            field: TextBox::new(ID_TASK).with_max_len(200),
            add: Button::new(ID_ADD),
            remove: Button::secondary(ID_REMOVE),
            list: List::new(40.0),
        })
    }

    fn frame(&mut self, f: &mut Frame, _dt_ms: i32) -> Result<(), VyError> {
        let l = self.layout();
        let t = self.theme;
        f.rect(Rect::new(0.0, 0.0, self.window.width, self.window.height), 0.0, BG)?;
        f.rect(l.card, 12.0, CARD)?;
        f.group(Role::Main, l.card, "", |f| {
            f.text(l.title.x, l.title.y, l.title.w, self.title, "Att göra")?;
            f.sem.node(Role::Heading, l.title, "Att göra")?;
            // The field's name in the tree; nothing is drawn for it.
            let label = Label::hidden("Ny uppgift");
            self.field.draw(f, l.field, &t, label, "Ny uppgift")?;
            self.add.draw(f, l.add, &t, "Lägg till")?;
            self.list.draw(f, l.list, &t, &self.tasks[..])?;
            self.remove.draw(f, l.remove, &t, "Ta bort")?;
            let count = format!("{} av högst {MAX_TASKS} uppgifter", self.tasks.len());
            let y = l.count.y + (l.count.h - t.line_h) / 2.0;
            f.text(l.count.x, y, 0.0, t.text, &count)?;
            f.sem.node(Role::Text, l.count, &count)?;
            Ok(())
        })
    }

    fn event(&mut self, event: Event<'_>) -> bool {
        let l = self.layout();
        let t = self.theme;
        let mut ui = Input::new(event);
        let enter = self.field.event(&mut ui, l.field, &t).activated();
        let add = self.add.event(&mut ui, l.add, &t).activated();
        let remove = self.remove.event(&mut ui, l.remove, &t).activated();
        self.list.event(&mut ui, l.list, &t, &self.tasks[..]);
        let mut changed = false;
        if enter || add {
            changed |= self.add_task();
        }
        if remove {
            changed |= self.remove_selected(&l);
        }
        ui.redraw() || changed
    }

    fn resize(&mut self, window: Window) {
        self.window = window;
    }
}

vy_sdk::export!(Page);

#[cfg(test)]
mod tests {
    use super::*;
    use vy_sdk::event::key;
    use vy_sdk::mock;

    #[test]
    fn add_and_remove() {
        mock::reset();
        let window = Window { width: 800.0, height: 600.0, scale: 1.0 };
        let mut page = Page::init(window, 0).unwrap();
        let focus = Event::Focus { node: NodeId(ID_TASK), gained: true, keyboard: true };
        assert!(page.event(focus));
        assert!(page.event(Event::TextInput { text: "Handla" }));
        let enter = Event::KeyDown { code: key::ENTER, modifiers: Modifiers(0), repeat: false };
        assert!(page.event(enter));
        assert_eq!(page.tasks, ["Handla"]);
        assert_eq!(page.field.text(), "");
        assert!(mock::run_frame(&mut page, 0).unwrap() > 0);
        mock::with(|h| assert!(h.sem.iter().any(|n| n.text == "Handla")));
        let row = page.list.row_rect(page.layout().list, 0);
        let press = Event::PointerDown { x: row.x + 5.0, y: row.y + 5.0, button: 1, modifiers: Modifiers(0) };
        assert!(page.event(press));
        let remove = page.layout().remove;
        let press = Event::PointerDown { x: remove.x + 5.0, y: remove.y + 5.0, button: 1, modifiers: Modifiers(0) };
        assert!(page.event(press));
        assert!(page.tasks.is_empty());
    }
}
```

Bygg, kontrollera, titta och testa:

```sh
cargo build --release --target wasm32-unknown-unknown
vy-cli check-abi target/wasm32-unknown-unknown/release/att_gora.wasm
vy-cli render target/wasm32-unknown-unknown/release/att_gora.wasm --out preview.png --size 800x600 --grant ime --dump-semantic semantic.json
cargo test
vy-dev build
```

## 9. Vanliga misstag

| Misstag | Följd | Rätt |
| --- | --- | --- |
| Textfält utan `"ime": {}` i manifestet | konformanssviten avvisar sidan (SPEC-10.4.2); `ime_begin` nekas | begär `ime` så fort sidan har en `textbox` |
| Bild utan `w`/`h` i manifestet, eller fel storlek | manifestet valideras inte; värden avvisar bilden med `E_RESOURCE` (SPEC-8.1.2, SPEC-6.5.1) | ange bildens verkliga storlek i pixlar |
| Glömd semantisk nod | texten finns inte för skärmläsare, sök eller markering; Tab hoppar över kontrollen | `f.sem.node(…)` för varje text, komponenterna för kontroller |
| Textfält utan namn (bara en platshållare) | en skärmläsare möter ett tomt fält utan att veta vad det är | `Label::shown(…)` eller `Label::hidden(…)` i `TextBox::draw`; platshållaren når aldrig trädet |
| Ny nod före en fokuserad kontroll med automatiskt id | id:n flyttas, fokus hamnar på fel nod | fasta negativa id för allt fokuserbart |
| Obegränsad allokering per frame (en `format!` per rad i en lång lista, växande `Vec`) | minnet tar slut mot `memory_max`, budgeten spricker | tak på data, återanvänd buffertar, rita bara det som syns |
| `event` returnerar `false` efter en ändring | ingen ny frame; skärmen visar det gamla | `ui.redraw() \|\| ändrat` |
| `request_frame()` i varje frame utan animation | sidan bränner CPU i onödan (SPEC-6.2.2) | be om frames bara medan något rör sig |
| Panik (`unwrap`, indexering utanför) | fliken stoppas | returnera `Err`, hantera `Err` |
| Layout räknad på två ställen | träffytor och ritning glider isär | en `layout()` som både `frame` och `event` använder |
| Glömd `resize` | sidan ritar för det gamla fönstret | spara `window` i `resize` (och klampa rullningen) |
| `std`, trådar, WASI | modulen avvisas (SPEC-4.2.2, SPEC-4.2.3) | `#![cfg_attr(not(test), no_std)]` och `alloc` |

**Sparad data är indata.** En sida får aldrig kunna stoppa sig själv med data den har sparat. Det
`state_get` ger tillbaka är vad en tidigare version av sidan, eller ett fel i den, lade dit, och det
läses i `init` vid varje besök: en sida som inte tål värdet går inte att öppna igen förrän någon
rensar lagringen. Läs därför tillbaka som du läser indata utifrån. Kontrollera varje längd och
varje antal mot värdets slut; avvisa ett värde som har mer än sidan själv tar emot (fler kort, fler
punkter, mer bläck, en cell två gånger) i stället för att visa det; och gör det som kostar
(tolkning, radbrytning, omräkning) i steg över flera frames när värdet är stort, inte i `init` och
inte i en enda frame. Mät sedan värsta fallet och inte exemplet: ge sidan det största och det
besvärligaste värde kvoten rymmer i ett fall i `conformance/cases.toml` (`state = [{key = "…",
hex = "…"}]`) och läs den dyraste framen med `vy-cli conformance --stats` (bränsle och
rasterisering mot budgeten, SPEC-5.3.3 och SPEC-12.2.4). Exempelsidornas prov av det
slaget, 300 slumpade värden, avklippta värden och hela kvoten för var och en, står i
`crates/vy-cli/src/budget_tests.rs`.

## 10. Testa nativt

SDK:t har en mock av värden (`vy_sdk::mock`), så `cargo test` kör sidans logik utan wasm-motor:
`mock::reset()`, `mock::run_frame(&mut sida, 0)` för en frame, `mock::with(|h| h.sem …)` för
trädet och `h.frame_ops()` för display listen, `h.caps.ime = true` för att bevilja capabilities och
`mock::as_copy_gesture(|| …)` för en händelse under kopieringsgesten. Mockens text är fast: varje
tecken är 0,6 × storleken brett och en rad 1,25 × storleken hög. Pixlar kommer bara från
`vy-cli render`.

Mocken äger inget fokus. Flytta det som värden gör vid Tab: först
`Event::Focus { node: gamla, gained: false, .. }`, sedan `gained: true` för den nya noden. Annars
tar den förra kontrollen fortfarande tangenterna. Tangentkoder är `i32` (`vy_sdk::event::key`).

### Händelser utan fönster

`vy-cli conformance` kör sidan som klienten gör, utan fönster: den spelar upp händelser, låter
värden äga fokus (Tab går genom trädets fokuserbara noder), kan starta om sidan med den gamla
lagringen och jämför slutbilden och det semantiska trädet med sparade facit. Den vill ha en rot
med den här formen, som du lägger bredvid projektet:

```
prov/
  conformance/cases.toml             fallen
  conformance/manifests/min_sida.json   sidans manifest.json, under modulens namn
  target/wasm/min_sida.wasm          modulen, kopierad ur target/wasm32-unknown-unknown/release/
```

Ett fall som skriver i ett fält, trycker på en knapp, startar om sidan och ser att värdet finns
kvar:

```toml
[[case]]
name = "min-sida-sparar"
module = "min_sida"
width = 800
height = 600
semantic = "min-sida-sparar"
capabilities = ["ime", "storage"]
events = [
  {kind = "key_down", code = 43},                  # Tab: värdens fokus till första kontrollen
  {kind = "text_input", text = "450"},
  {kind = "key_down", code = 43},
  {kind = "key_down", code = 40},                  # Enter på knappen
  {kind = "pointer_down", x = 300, y = 250},
  {kind = "pointer_move", x = 390, y = 250, buttons = 1},   # buttons = 1: knappen hålls nere (krävs för att dra)
  {kind = "pointer_up", x = 390, y = 250},
  {kind = "suspend"},
  {kind = "reinit"},                               # ny instans med den gamla lagringen
]
```

`vy-cli conformance --root prov --update` skriver `prov/conformance/golden/<name>.png` och
`prov/conformance/semantic/<semantic>.json`: titta på bilden och läs trädet. Samma kommando utan
`--update` jämför mot dem och fäller ett fall som ändrats eller som har en frame över budgeten;
`--stats` visar vad varje frame kostade. Tangentkoderna är USB HID (43 Tab, 40 Enter, 79–82
pilarna). Alla händelser och fält står i huvudet på `conformance/cases.toml` i repot.
