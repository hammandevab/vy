//! Native tests against the SDK's mock host: the board's stored form, the
//! drag logic, the keyboard moves, editing, storage and the narrow layout.

use super::*;
use vy_sdk::mock;

fn page(width: f32, height: f32, storage: bool) -> Page {
    mock::reset();
    mock::with(|h| {
        h.caps.ime = true;
        h.caps.storage = storage;
    });
    init(width, height)
}

fn init(width: f32, height: f32) -> Page {
    Page::init(
        Window {
            width,
            height,
            scale: 1.0,
        },
        0,
    )
    .unwrap()
}

fn down(p: &mut Page, x: f32, y: f32) -> bool {
    p.event(Event::PointerDown {
        x,
        y,
        button: 1,
        modifiers: Modifiers(0),
    })
}

fn drag(p: &mut Page, x: f32, y: f32) -> bool {
    p.event(Event::PointerMove {
        x,
        y,
        buttons: 1,
        modifiers: Modifiers(0),
    })
}

fn up(p: &mut Page, x: f32, y: f32) -> bool {
    p.event(Event::PointerUp {
        x,
        y,
        button: 1,
        modifiers: Modifiers(0),
    })
}

fn press(p: &mut Page, code: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(0),
        repeat: false,
    })
}

/// The host moves focus to `node`, away from wherever it was.
fn focus(p: &mut Page, node: i32) {
    if let Some(old) = p.focus {
        p.event(Event::Focus {
            node: NodeId(old),
            gained: false,
            keyboard: true,
        });
    }
    p.event(Event::Focus {
        node: NodeId(node),
        gained: true,
        keyboard: true,
    });
}

fn titles(p: &Page, c: usize) -> Vec<&str> {
    p.board.cols[c].iter().map(|k| k.title.as_str()).collect()
}

fn mid(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn node(id: i32) -> Option<mock::SemNode> {
    mock::with(|h| h.sem.iter().find(|n| n.id == id).cloned())
}

/// SPEC-5.2.9: the stored form holds the board and nothing else, a few
/// bytes more than the titles, and anything that is not a board is refused.
#[test]
fn board_round_trips_through_its_stored_form() {
    let board = Board::sample();
    let bytes = board.encode();
    let text: usize = board.cols.iter().flatten().map(|k| k.title.len()).sum();
    assert_eq!(bytes.len(), 1 + COLS + board.count() + text);
    let back = Board::decode(&bytes).unwrap();
    for c in 0..COLS {
        let a: Vec<&str> = board.cols[c].iter().map(|k| k.title.as_str()).collect();
        let b: Vec<&str> = back.cols[c].iter().map(|k| k.title.as_str()).collect();
        assert_eq!(a, b);
    }
    assert_eq!(Board::decode(&[]), None);
    assert_eq!(Board::decode(&[9, 0, 0, 0]), None, "unknown format");
    assert_eq!(Board::decode(&bytes[..bytes.len() - 1]), None, "cut short");
    let mut long = bytes.clone();
    long.push(0);
    assert_eq!(Board::decode(&long), None, "trailing bytes");

    // A full board of the longest titles stays far below the 16 KB quota.
    let mut full = Board::empty();
    let title = "å".repeat(MAX_TITLE / 2);
    while full.add(0, &title).is_some() {}
    assert_eq!(full.count(), MAX_CARDS);
    assert!(full.encode().len() < 8 * 1024, "{}", full.encode().len());
}

/// A press that moves lifts the card; the layout opens the slot under it
/// in the column it is over, and the release puts it there.
#[test]
fn drag_moves_a_card_to_the_slot_under_it() {
    let mut p = page(900.0, 600.0, true);
    let l = p.layout();
    let first = l.lanes[0].cards[0];
    let (x, y) = mid(first);
    assert!(down(&mut p, x, y));
    assert!(matches!(p.press, Press::Down { .. }));
    assert!(!drag(&mut p, x + 1.0, y + 1.0), "not far enough to lift");
    assert!(p.layout().float.is_none());

    // Over the top of the third column: slot 0 there.
    let target = l.lanes[2].cards[0];
    assert!(drag(&mut p, target.x + 40.0, target.y - 10.0));
    let dragging = p.layout();
    assert_eq!(dragging.drop, Some((2, 0)));
    let slot = dragging.lanes[2].slot.unwrap();
    assert_eq!(slot.y, target.y);
    assert_eq!(dragging.lanes[2].cards[0].y, slot.bottom() + CARD_GAP);
    assert_eq!(dragging.lanes[0].cards[0], Rect::default(), "out of flow");
    assert_eq!(dragging.lanes[0].cards[1].y, first.y, "the column closes");
    mock::run_frame(&mut p, 16).unwrap();
    let text = node(node_card(0)).unwrap().text;
    assert_eq!(text, "Skriv utkast till nyhetsbrevet (flyttas)");

    // Below the last card of that column: slot 1.
    assert!(drag(&mut p, target.x + 40.0, target.bottom() + 60.0));
    assert_eq!(p.layout().drop, Some((2, 1)));
    assert!(up(&mut p, target.x + 40.0, target.bottom() + 60.0));
    assert_eq!(
        titles(&p, 2),
        ["Publicera version 0.2", "Skriv utkast till nyhetsbrevet"]
    );
    assert_eq!(titles(&p, 0).len(), 2);
    assert_eq!(p.press, Press::Idle);
    mock::with(|h| assert_eq!(h.state.get(KEY.as_bytes()), Some(&p.board.encode())));
}

/// The dropped card eases into its slot on requested frames and the page
/// is at rest afterwards (SPEC-5.2.2, SPEC-6.2.2).
#[test]
fn a_dropped_card_settles_and_the_page_rests() {
    let mut p = page(900.0, 600.0, true);
    let l = p.layout();
    let (x, y) = mid(l.lanes[0].cards[0]);
    let to = mid(l.lanes[1].panel);
    down(&mut p, x, y);
    drag(&mut p, to.0, to.1);
    up(&mut p, to.0, to.1);
    let settle = p.settle.expect("the card is on its way");
    let slot = p.card_rect(settle.id).unwrap().1;
    assert_ne!(settle.from, (slot.x, slot.y));

    // The first frame's dt is the time at rest and moves nothing.
    mock::run_frame(&mut p, 5000).unwrap();
    assert_eq!(p.settle.unwrap().ms, 0.0);
    let mut frames = 1;
    while p.settle.is_some() {
        mock::run_frame(&mut p, 16).unwrap();
        frames += 1;
        assert!(frames < 30);
    }
    assert_eq!(frames, 11, "160 ms at 16 ms a frame");
    assert_eq!(mock::with(|h| h.frame_requests), 10);
    mock::run_frame(&mut p, 16).unwrap();
    assert_eq!(mock::with(|h| h.frame_requests), 10, "at rest");
    assert_eq!(ease(0.0), 0.0);
    assert_eq!(ease(1.0), 1.0);
    assert!(ease(0.5) > 0.8, "most of the way early");
}

#[test]
fn drag_reorders_within_a_column() {
    let mut p = page(900.0, 600.0, true);
    let l = p.layout();
    let (x, y) = mid(l.lanes[0].cards[0]);
    let last = l.lanes[0].cards[2];
    down(&mut p, x, y);
    drag(&mut p, x, last.bottom() + 20.0);
    assert_eq!(p.layout().drop, Some((0, 2)));
    up(&mut p, x, last.bottom() + 20.0);
    assert_eq!(
        titles(&p, 0),
        [
            "Boka lokal för höstmötet",
            "Beställ nya visitkort",
            "Skriv utkast till nyhetsbrevet"
        ]
    );
    // Escape lets a lifted card fall back.
    let (x, y) = mid(p.layout().lanes[0].cards[0]);
    down(&mut p, x, y);
    drag(&mut p, x + 300.0, y);
    assert!(press(&mut p, key::ESCAPE));
    assert_eq!(p.press, Press::Idle);
    assert_eq!(titles(&p, 0).len(), 3);
}

/// SPEC-10.3.2: the card keeps its node id wherever it goes, so the host's
/// focus stays on it while the arrows move it.
#[test]
fn arrow_keys_move_the_focused_card() {
    let mut p = page(900.0, 600.0, true);
    let id = p.board.cols[0][1].id;
    focus(&mut p, node_card(id));
    assert!(press(&mut p, key::RIGHT));
    assert_eq!(p.board.find(id), Some((1, 1)));
    assert!(press(&mut p, key::DOWN));
    assert_eq!(p.board.find(id), Some((1, 2)));
    assert!(!press(&mut p, key::DOWN), "already last");
    assert!(press(&mut p, key::UP));
    assert!(press(&mut p, key::UP));
    assert_eq!(p.board.find(id), Some((1, 0)));
    assert!(press(&mut p, key::RIGHT));
    assert!(!press(&mut p, key::RIGHT), "no column to the right");
    assert_eq!(p.board.find(id), Some((2, 0)));
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.board.find(id), Some((1, 0)));
    mock::run_frame(&mut p, 16).unwrap();
    let n = node(node_card(id)).unwrap();
    assert_eq!(n.role, Role::Button as i32);
    assert_eq!(n.text, "Boka lokal för höstmötet");
    assert_eq!(p.focus, Some(node_card(id)));
    // Delete removes the focused card.
    assert!(press(&mut p, key::DELETE));
    assert_eq!(p.board.find(id), None);
}

/// Enter on a focused card, or a double click, turns its node into a
/// `textbox` with the same id; Enter commits, Escape cancels.
#[test]
fn enter_or_double_click_edits_the_title() {
    let mut p = page(900.0, 600.0, true);
    let id = p.board.cols[0][0].id;
    focus(&mut p, node_card(id));
    assert!(press(&mut p, key::ENTER));
    assert!(p.edit.is_some());
    mock::run_frame(&mut p, 16).unwrap();
    let n = node(node_card(id)).unwrap();
    assert_eq!(n.role, Role::Textbox as i32);
    assert_eq!(n.text, "Skriv utkast till nyhetsbrevet");
    assert!(mock::with(|h| h.ime_rect.is_some()), "composition is open");
    assert!(node(node_del(id)).is_none(), "no delete while editing");

    // The title is selected: typing replaces it.
    assert!(p.event(Event::TextInput {
        text: "Skicka brevet"
    }));
    assert!(press(&mut p, key::ENTER));
    assert!(p.edit.is_none());
    assert_eq!(p.board.cols[0][0].title, "Skicka brevet");
    assert!(mock::with(|h| h.ime_rect.is_none()));
    mock::run_frame(&mut p, 16).unwrap();
    assert_eq!(node(node_card(id)).unwrap().role, Role::Button as i32);

    // Escape keeps the old title; so does an empty one.
    press(&mut p, F2);
    p.event(Event::TextInput { text: "Annat" });
    assert!(press(&mut p, key::ESCAPE));
    assert_eq!(p.board.cols[0][0].title, "Skicka brevet");
    press(&mut p, key::ENTER);
    press(&mut p, key::BACKSPACE);
    press(&mut p, key::ENTER);
    assert_eq!(p.board.cols[0][0].title, "Skicka brevet");

    // Two presses within 400 ms edit; focus leaving the field commits.
    let other = p.board.cols[1][0].id;
    let (x, y) = mid(p.layout().lanes[1].cards[0]);
    focus(&mut p, node_card(other));
    down(&mut p, x, y);
    up(&mut p, x, y);
    assert!(p.edit.is_none());
    mock::with(|h| h.now_ms += 200);
    down(&mut p, x, y);
    up(&mut p, x, y);
    assert_eq!(p.edit.as_ref().map(|e| e.0), Some(other));
    p.event(Event::TextInput {
        text: "Bygg sidorna",
    });
    focus(&mut p, ID_NEW);
    assert!(p.edit.is_none());
    assert_eq!(p.board.cols[1][0].title, "Bygg sidorna");

    // Slow presses are two clicks.
    let (x, y) = mid(p.layout().lanes[2].cards[0]);
    down(&mut p, x, y);
    up(&mut p, x, y);
    mock::with(|h| h.now_ms += 900);
    down(&mut p, x, y);
    assert!(p.edit.is_none());
}

#[test]
fn add_with_the_field_and_delete_with_the_button() {
    let mut p = page(900.0, 600.0, true);
    focus(&mut p, ID_NEW - 1);
    assert!(p.event(Event::TextInput { text: "  Diska  " }));
    assert!(press(&mut p, key::ENTER));
    assert_eq!(titles(&p, 1).last(), Some(&"Diska"));
    assert_eq!(p.new[1].text(), "");
    press(&mut p, key::ENTER);
    assert_eq!(titles(&p, 1).len(), 3, "an empty field adds nothing");

    p.event(Event::TextInput { text: "Städa" });
    let (x, y) = mid(p.layout().lanes[1].add);
    assert!(down(&mut p, x, y));
    assert_eq!(titles(&p, 1).last(), Some(&"Städa"));

    // The × in the card's corner.
    let l = p.layout();
    let (x, y) = mid(del_rect(l.lanes[0].cards[1]));
    assert!(down(&mut p, x, y));
    assert_eq!(
        titles(&p, 0),
        ["Skriv utkast till nyhetsbrevet", "Beställ nya visitkort"]
    );
    // And with the keyboard: Enter on the focused delete button.
    let id = p.board.cols[0][0].id;
    focus(&mut p, node_del(id));
    assert!(press(&mut p, key::ENTER));
    assert_eq!(titles(&p, 0), ["Beställ nya visitkort"]);

    // The board is full at 60 cards and says so.
    while p.board.add(2, "x").is_some() {}
    p.new[0].set_text("En till");
    focus(&mut p, ID_NEW);
    press(&mut p, key::ENTER);
    assert_eq!(p.board.count(), MAX_CARDS);
    assert!(p.status().contains("tavlan är full"), "{}", p.status());
    mock::run_frame(&mut p, 16).unwrap();
    mock::with(|h| assert!(h.refusals.is_empty(), "{:?}", h.refusals));
}

/// SPEC-5.2.9, SPEC-5.3.1: with storage the board comes back in a new
/// instance; without it the page works in memory and says "sparas inte".
#[test]
fn storage_persists_and_its_absence_is_shown() {
    let mut p = page(900.0, 600.0, true);
    assert!(!p.status().contains("sparas inte"));
    focus(&mut p, ID_NEW);
    p.event(Event::TextInput { text: "Kom ihåg" });
    press(&mut p, key::ENTER);
    let saved = p.board.encode();
    let again = init(900.0, 600.0);
    assert_eq!(again.board.encode(), saved);
    assert_eq!(titles(&again, 0).last(), Some(&"Kom ihåg"));

    let mut p = page(900.0, 600.0, false);
    assert_eq!(p.board.count(), 6, "the sample board");
    assert_eq!(p.status(), "6 kort · sparas inte");
    focus(&mut p, ID_NEW);
    p.event(Event::TextInput { text: "Ändå" });
    assert!(press(&mut p, key::ENTER));
    assert_eq!(p.board.count(), 7);
    mock::run_frame(&mut p, 16).unwrap();
    mock::with(|h| assert!(h.sem.iter().any(|n| n.text == "7 kort · sparas inte")));
}

/// Below 640 px one column shows behind three `tab` nodes; a card dropped
/// on a tab moves to that column, and the arrows take the view along.
#[test]
fn narrow_window_shows_one_column_behind_tabs() {
    let mut p = page(400.0, 700.0, true);
    let l = p.layout();
    assert!(l.narrow);
    assert_eq!(
        [l.lanes[0].visible, l.lanes[1].visible, l.lanes[2].visible],
        [true, false, false]
    );
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let roles = |r: Role| h.sem.iter().filter(|n| n.role == r as i32).count();
        assert_eq!(roles(Role::Tab), 3);
        assert_eq!(roles(Role::TabPanel), 1);
        assert_eq!(roles(Role::Textbox), 1);
        assert!(h.sem.iter().any(|n| n.text == "Att göra, 3 kort (vald)"));
        assert!(!h.sem.iter().any(|n| n.text == "Bygg exempelsidorna"));
    });

    let (x, y) = mid(l.tabs[1]);
    assert!(down(&mut p, x, y));
    assert_eq!(p.active, 1);
    let (x, y) = mid(p.layout().lanes[1].cards[0]);
    down(&mut p, x, y);
    let (tx, ty) = mid(l.tabs[2]);
    drag(&mut p, tx, ty);
    assert_eq!(p.layout().drop, Some((2, 1)), "the end of the third column");
    up(&mut p, tx, ty);
    assert_eq!(
        titles(&p, 2),
        ["Publicera version 0.2", "Bygg exempelsidorna"]
    );
    assert_eq!(p.active, 1, "the view stays");

    let id = p.board.cols[1][0].id;
    focus(&mut p, node_card(id));
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.board.find(id), Some((0, 0)));
    assert_eq!(p.active, 0, "the view follows the card");
    mock::run_frame(&mut p, 16).unwrap();
    assert!(
        node(node_card(id)).is_some(),
        "still registered: focus stays"
    );
}

/// SPEC-10.1.1, SPEC-10.3.2: every card and control is a node, in Tab
/// order: per column the cards with their delete buttons, the field and
/// the add button; a long column scrolls the focused card into view.
#[test]
fn tree_lists_every_control_in_tab_order() {
    let mut p = page(900.0, 600.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    let focusable: Vec<i32> =
        mock::with(|h| h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect());
    let mut expected = Vec::new();
    for c in 0..COLS {
        for k in &p.board.cols[c] {
            expected.extend([node_card(k.id), node_del(k.id)]);
        }
        expected.extend([ID_NEW - c as i32, ID_ADD - c as i32]);
    }
    assert_eq!(focusable, expected);
    mock::with(|h| {
        // Each field's name is the `text` node right before it: the
        // placeholder "Nytt kort" is drawn, never registered.
        for (c, name) in NAMES.iter().enumerate() {
            let i = h
                .sem
                .iter()
                .position(|n| n.id == ID_NEW - c as i32)
                .unwrap();
            assert_eq!(
                (h.sem[i - 1].role, h.sem[i - 1].text.clone()),
                (Role::Text as i32, format!("Nytt kort i {name}"))
            );
            assert_eq!((h.sem[i - 1].w, h.sem[i - 1].h), (0.0, 0.0));
        }
        assert!(h.sem.iter().any(|n| n.text == "Lägg till kort i Pågår"));
        assert!(h
            .sem
            .iter()
            .any(|n| n.text == "Ta bort: Publicera version 0.2"));
        assert_eq!(
            h.sem
                .iter()
                .filter(|n| n.role == Role::Region as i32)
                .count(),
            3
        );
    });

    // Fill a column past its viewport and focus its last card.
    for i in 0..20 {
        p.board.add(0, &format!("Kort {i}"));
    }
    let last = p.board.cols[0].last().unwrap().id;
    let before = p.card_rect(last).unwrap().1;
    let view = p.layout().lanes[0].view;
    assert!(before.y > view.bottom(), "below the viewport");
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(
        node(node_card(last)).unwrap().h,
        0.0,
        "hidden: no pointer focus"
    );
    focus(&mut p, node_card(last));
    let after = p.card_rect(last).unwrap().1;
    assert!(after.bottom() <= view.bottom() && after.y >= view.y);
    assert!(p.scroll[0].offset() > 0.0);
    assert_eq!(
        p.scroll[1].offset(),
        0.0,
        "the columns scroll independently"
    );
    let l = p.layout();
    assert_eq!(
        l.card_at(after.x + 5.0, view.y - 5.0),
        None,
        "the header is no card"
    );
}
