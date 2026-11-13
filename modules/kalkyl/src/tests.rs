//! Native tests: the sheet's numbers, formulas, cycles and stored form,
//! and the page's selection, editing, Tab ring, copy gesture and storage
//! against the SDK's mock host.

use super::*;
use sheet::{parse_number, Fault, CELLS, HEAD_CHARS, ONE};
use vy_sdk::mock;

/// The index of the cell named `name`, as `B3`.
fn at(name: &str) -> usize {
    let col = usize::from(name.as_bytes()[0] - b'A');
    let row: usize = name[1..].parse().unwrap();
    index(col, row - 1)
}

/// A sheet from `(cell, source)` pairs, calculated.
fn sheet(cells: &[(&str, &str)]) -> Sheet {
    let mut s = Sheet::new();
    for (name, src) in cells {
        s.set(at(name), src);
    }
    s.recalc();
    s
}

fn shown(s: &Sheet, name: &str) -> String {
    s.get(at(name)).map_or(String::new(), |c| c.shown().into())
}

/// Decimal input is exact and the page formats its own numbers: a comma,
/// at most six decimals, no trailing zeros, never "-0".
#[test]
fn numbers_parse_and_format_with_a_decimal_comma() {
    assert_eq!(parse_number("12,5"), Some(12 * ONE + ONE / 2));
    assert_eq!(parse_number("12.5"), parse_number("12,5"));
    assert_eq!(parse_number(" -3 "), Some(-3 * ONE));
    assert_eq!(parse_number("+7"), Some(7 * ONE));
    assert_eq!(
        parse_number("0,123456789"),
        Some(12_345_679),
        "rounded at 8"
    );
    for text in ["", "-", "1,", ",5", "1 000", "1e5", "12kr", "1,2,3"] {
        assert_eq!(parse_number(text), None, "{text:?}");
    }
    assert_eq!(parse_number("99999999999999"), None, "does not fit");

    assert_eq!(format_number(1250 * ONE), "1250");
    assert_eq!(format_number(0), "0");
    assert_eq!(format_number(-ONE / 2), "-0,5");
    assert_eq!(format_number(ONE / 3), "0,333333");
    assert_eq!(format_number(2 * ONE / 3), "0,666667");
    assert_eq!(format_number(421_050 * ONE / 100), "4210,5");
    assert_eq!(format_number(99_999_999), "1", "0,99999999 rounds up");
    assert_eq!(format_number(-10), "0", "below six decimals: no minus zero");
    assert_eq!(format_number(i64::MIN), "-92233720368,547758");

    // 0,1 + 0,2 is 0,3, which binary floating point does not give.
    let s = sheet(&[
        ("A1", "0,1"),
        ("A2", "0,2"),
        ("A3", "=A1+A2"),
        ("A4", "=1/3*3"),
    ]);
    assert_eq!(shown(&s, "A3"), "0,3");
    assert_eq!(shown(&s, "A4"), "1");
}

#[test]
fn formulas_follow_precedence_references_and_functions() {
    let s = sheet(&[
        ("A1", "2"),
        ("A2", "3,5"),
        ("A3", "10"),
        ("B1", "text"),
        ("C1", "=1+2*3"),
        ("C2", "=(1+2)*3"),
        ("C3", "=-A1+A3/4"),
        ("C4", "= a1 * A2 - 1 "),
        ("C5", "=--A1"),
        ("D1", "=SUMMA(A1:A3)"),
        ("D2", "=MEDEL(A1:A3)"),
        ("D3", "=MIN(A1:A3)"),
        ("D4", "=MAX(A1:A3;100)"),
        ("D5", "=ANTAL(A1:B3)"),
        ("E1", "=sum(a1:a3)"),
        ("E2", "=Average(A1:A3)"),
        ("E3", "=COUNT(A1:A3; 7)"),
        ("E4", "=SUMMA(A1;A2;2,5)"),
        ("E5", "=SUMMA(A1,A3)"),
        ("F1", "=SUMMA(A3:A1)"),
        ("F2", "=SUMMA(B1:B9)"),
        ("F3", "=MAX(G1:G9)"),
        ("F4", "=D1*2"),
    ]);
    let expect = [
        ("C1", "7"),
        ("C2", "9"),
        ("C3", "0,5"),
        ("C4", "6"),
        ("C5", "2"),
        ("D1", "15,5"),
        ("D2", "5,166667"),
        ("D3", "2"),
        ("D4", "100"),
        ("D5", "3"),
        ("E1", "15,5"),
        ("E2", "5,166667"),
        ("E3", "4"),
        ("E4", "8"),
        ("E5", "12"),
        ("F1", "15,5"),
        ("F2", "0"),
        ("F3", "0"),
        ("F4", "31"),
    ];
    for (name, value) in expect {
        assert_eq!(shown(&s, name), value, "{name}: {}", s.src(at(name)));
    }
    assert_eq!(s.get(at("B1")).unwrap().value, Value::Text);
    assert_eq!(s.get(at("D2")).unwrap().label, "D2: 5,166667");
}

/// A formula that cannot be calculated shows why; nothing fails.
#[test]
fn faults_are_shown_in_the_cell() {
    let s = sheet(&[
        ("A1", "5"),
        ("A2", "0"),
        ("B1", "ord"),
        ("C1", "=A1/A2"),
        ("C2", "=A1/0"),
        ("C3", "=MEDEL(E1:E5)"),
        ("C4", "=C1+1"),
        ("D1", "=A1+"),
        ("D2", "=FOO(A1)"),
        ("D3", "=A1:A2"),
        ("D4", "=(A1"),
        ("D5", "=B1*2"),
        ("D6", "=A101"),
        ("D7", "=AB1"),
        ("D8", "=90000000000*90000000000"),
        ("D9", "="),
        ("D10", "=SUMMA(A1 A2)"),
        ("D11", "=D1"),
    ]);
    for name in ["C1", "C2", "C3", "C4"] {
        assert_eq!(shown(&s, name), "#DIV/0", "{name}");
        assert_eq!(s.get(at(name)).unwrap().value, Value::Fault(Fault::DivZero));
    }
    for name in [
        "D1", "D2", "D3", "D4", "D5", "D6", "D7", "D8", "D9", "D10", "D11",
    ] {
        assert_eq!(shown(&s, name), "#FEL", "{name}: {}", s.src(at(name)));
    }
}

/// A formula on a cycle, and one that reads it, is `#CIRKEL`; breaking
/// the cycle brings the values back.
#[test]
fn cycles_are_found_and_shown() {
    let mut s = sheet(&[
        ("A1", "=B1+1"),
        ("B1", "=C1"),
        ("C1", "=A1*2"),
        ("D1", "=A1+1"),
        ("E1", "=E1"),
        ("F1", "=SUMMA(F1:F3)"),
        ("G1", "=7"),
        ("G2", "=G1+SUMMA(A1:C1)"),
    ]);
    for name in ["A1", "B1", "C1", "D1", "E1", "F1", "G2"] {
        assert_eq!(shown(&s, name), "#CIRKEL", "{name}");
    }
    assert_eq!(shown(&s, "G1"), "7");
    s.set(at("C1"), "4");
    s.recalc();
    assert_eq!(shown(&s, "B1"), "4");
    assert_eq!(shown(&s, "A1"), "5");
    assert_eq!(shown(&s, "D1"), "6");
    assert_eq!(shown(&s, "G2"), "20");
    assert_eq!(shown(&s, "E1"), "#CIRKEL");
}

/// Formulas are calculated after what they read, whatever order they
/// were entered in, and a chain through every cell does not recurse.
#[test]
fn recalculation_is_dependency_ordered() {
    // Each cell reads the next one; the last is a number.
    let mut s = Sheet::new();
    for i in 0..CELLS - 1 {
        s.set(i, &format!("={}+1", name(i + 1)));
    }
    s.set(CELLS - 1, "1");
    s.recalc();
    assert_eq!(s.get(0).unwrap().shown(), "2600");
    assert_eq!(s.get(CELLS - 2).unwrap().shown(), "2");
    assert!(!s.calculating());
    // One change at the far end reaches the first cell.
    s.set(CELLS - 1, "0,5");
    s.recalc();
    assert_eq!(s.get(0).unwrap().shown(), "2599,5");

    let s = Sheet::sample();
    assert_eq!(shown(&s, "E4"), "99000");
    assert_eq!(shown(&s, "B10"), "16400,5");
    assert_eq!(shown(&s, "E10"), "49386,25");
    assert_eq!(shown(&s, "E11"), "49613,75");
    assert_eq!(shown(&s, "B12"), "0,495369");
    assert_eq!(shown(&s, "B14"), "4215,416667");
    assert_eq!(shown(&s, "B15"), "1240");
}

/// SPEC-5.3.3 (granskning 0013:2): a recalculation is a pass that stops
/// after about as much work as a step is given and goes on in the next,
/// so no sheet takes more than a step's worth of a call into the page.
/// The work is every cell looked at and every byte of formula read: the
/// first reading of a new formula, the walk over what the formulas read
/// with what it does on a cycle, and their evaluation. A step does not
/// interrupt the evaluation of one formula, and a formula may read the
/// sheet twice over through its ranges, no more. However the pass is cut
/// into steps, the values are the ones a single pass gives.
#[test]
fn a_recalculation_goes_on_in_steps_of_bounded_work() {
    const STEP: u32 = RECALC_STEP;
    // Stepped to the end: the steps taken and the most work one did.
    let stepped = |s: &mut Sheet, budget: u32| -> (u32, u64) {
        let (mut steps, mut most) = (0, 0);
        loop {
            let before = s.work;
            let done = s.step(budget);
            steps += 1;
            most = most.max(s.work - before);
            assert!(steps < 1_000_000, "the pass does not end");
            if done {
                assert!(!s.calculating());
                return (steps, most);
            }
            assert!(s.calculating());
        }
    };
    let labels = |s: &Sheet| -> Vec<String> {
        s.filled()
            .map(|i| s.get(i).unwrap().label.clone())
            .collect()
    };
    // The most a step goes over its budget: one formula of 200 bytes
    // that reads the sheet twice.
    let over = u64::from(40 + 4 * MAX_SRC as u32 + sheet::MAX_RANGE_CELLS);

    // The review's first sheet: a hundred numbers and 2 500 sums of them.
    let mut sums = Sheet::new();
    for row in 0..ROWS {
        sums.set(index(0, row), "1,5");
        for col in 1..COLS {
            sums.set(index(col, row), "=SUMMA(A1:A100)");
        }
    }
    assert!(sums.calculating());
    assert_eq!(shown(&sums, "Z100"), "…", "no value before its pass");
    let (steps, most) = stepped(&mut sums, STEP);
    // Read through: 2 600 cells and 2 500 × 4 × 14 bytes. Walked: 2 500
    // × (100 cells + 4) and 2 600 cells. Evaluated: 2 500 × (40 + 4 × 14
    // + 100 cells).
    assert_eq!(sums.work, 142_600 + 262_600 + 490_000);
    assert_eq!(steps, 45);
    assert!(most <= u64::from(STEP) + over, "{most} units in one step");
    assert_eq!(shown(&sums, "B1"), "150");
    assert_eq!(shown(&sums, "Z100"), "150");
    let whole = labels(&sums);
    // One number changed: the pass again (the formulas are not new), the
    // new sums at its end and the old ones until then.
    let before = sums.work;
    sums.set(at("A1"), "101,5");
    assert_eq!(shown(&sums, "Z100"), "150", "the old value meanwhile");
    assert!(!sums.step(STEP));
    stepped(&mut sums, STEP);
    assert_eq!(sums.work - before, 2_600 + 262_600 + 490_000);
    assert_eq!(shown(&sums, "Z100"), "250");
    sums.set(at("A1"), "1,5");
    stepped(&mut sums, 7);
    assert_eq!(
        labels(&sums),
        whole,
        "seven units a step give the same sheet"
    );

    // The review's second sheet: 200 formulas that each sum the whole
    // sheet, themselves included. Every one is on a cycle, found without
    // a search of the stack for every formula read.
    let mut tangle = Sheet::new();
    for i in 0..200 {
        tangle.set(i, "=SUMMA(A1:Z100)");
    }
    let (steps, most) = stepped(&mut tangle, STEP);
    assert_eq!(
        tangle.work,
        (2_600 + 200 * 56) + (200 * 2_604 + 2_600) + 200 * 40
    );
    assert_eq!(steps, 28);
    assert!(most <= u64::from(STEP) + over, "{most}");
    assert!(tangle
        .filled()
        .all(|i| tangle.get(i).unwrap().shown() == "#CIRKEL"));
    // The whole sheet of them: 6.8 million cells looked at, a step at a
    // time, and the walk's stack is a few bytes a formula.
    let mut full = Sheet::new();
    for i in 0..CELLS {
        full.set(i, "=SUMMA(A1:Z100)");
    }
    let (steps, most) = stepped(&mut full, STEP);
    assert!((340..360).contains(&steps), "{steps}");
    assert!(most <= u64::from(STEP) + over, "{most}");
    assert_eq!(shown(&full, "M50"), "#CIRKEL");

    // One formula is evaluated whole, so it may read the sheet twice over
    // and no more: a third range makes it `#FEL`, and what reads it too.
    let mut wide = Sheet::new();
    wide.set(at("A1"), "=SUMMA(B1:Z100;B1:Z100)+SUMMA(C1:D100)");
    wide.set(at("A2"), "=SUMMA(B1:Z100;B1:Z100;C1:Z100)");
    wide.set(at("A3"), "=A2+1");
    wide.set(at("B1"), "2");
    wide.set(at("C7"), "3");
    let (steps, most) = stepped(&mut wide, STEP);
    assert_eq!(shown(&wide, "A1"), "13");
    assert_eq!(shown(&wide, "A2"), "#FEL");
    assert_eq!(shown(&wide, "A3"), "#FEL");
    // Read through, walked (5 200 cells of ranges) and evaluated (5 200
    // again) in one step.
    assert_eq!(steps, 1);
    assert!((2 * 5_200..u64::from(STEP)).contains(&most), "{most}");

    // A sheet of formulas that are all text to read: 1 260 of 200 bytes,
    // what the quota of 256 KB holds. Reading them through is counted.
    let mut dense = Sheet::new();
    let terms = format!("=1{}", "+1".repeat(99));
    for i in 0..1260 {
        dense.set(i, &terms);
    }
    let (steps, most) = stepped(&mut dense, STEP);
    assert_eq!(dense.work, 2 * (1260 * 4 * 199) + 2 * 2_600 + 1260 * 44);
    assert!((100..110).contains(&steps), "{steps}");
    assert!(most <= u64::from(STEP) + over, "{most}");
    assert_eq!(shown(&dense, "A1"), "100");

    // What the sample costs: one step, a small part of it.
    let mut sample = Sheet::sample();
    let before = sample.work;
    sample.set(at("B4"), "40000");
    assert!(sample.step(STEP));
    assert!(sample.work - before < 7_000, "{}", sample.work - before);
    assert_eq!(shown(&sample, "E4"), "106500");
}

#[test]
fn the_stored_form_round_trips() {
    let s = Sheet::sample();
    let bytes = s.encode();
    let sources: usize = s.filled().map(|i| s.src(i).len()).sum();
    assert_eq!(bytes.len(), 1 + 3 * s.count() + sources);
    let mut back = Sheet::decode(&bytes).unwrap();
    assert_eq!(back.count(), s.count());
    // A sheet that was read has its formulas still to calculate.
    assert!(back.calculating());
    assert_eq!(back.get(at("E4")).unwrap().label, "E4: …");
    back.recalc();
    for i in s.filled() {
        assert_eq!(back.src(i), s.src(i));
        assert_eq!(back.get(i).unwrap().label, s.get(i).unwrap().label);
    }
    assert!(Sheet::decode(&[]).is_none());
    assert!(Sheet::decode(&[7]).is_none(), "unknown format");
    assert!(Sheet::decode(&bytes[..bytes.len() - 1]).is_none());
    assert!(
        Sheet::decode(&[1, 0xFF, 0xFF, 1, b'x']).is_none(),
        "no such cell"
    );
    assert_eq!(Sheet::decode(&[1]).unwrap().count(), 0);
    // Each cell once and in order, as `encode` writes them: a value that
    // sets one cell 65 000 times is not a sheet, however little it holds.
    assert!(Sheet::decode(&[1, 7, 0, 1, b'1', 8, 0, 1, b'2']).is_some());
    assert!(
        Sheet::decode(&[1, 7, 0, 1, b'1', 7, 0, 1, b'2']).is_none(),
        "a cell twice"
    );
    assert!(
        Sheet::decode(&[1, 8, 0, 1, b'1', 7, 0, 1, b'2']).is_none(),
        "out of order"
    );
}

// ---- the page ---------------------------------------------------------

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

fn key_mod(p: &mut Page, code: i32, modifiers: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(modifiers),
        repeat: false,
    })
}

fn press(p: &mut Page, code: i32) -> bool {
    key_mod(p, code, 0)
}

fn type_text(p: &mut Page, text: &str) -> bool {
    p.event(Event::TextInput { text })
}

fn focus(p: &mut Page, node: i32, gained: bool) {
    p.event(Event::Focus {
        node: NodeId(node),
        gained,
        keyboard: true,
    });
}

/// Tab as the host does it: focus leaves the bar for the next node of
/// the ring (or, with Shift, the previous).
fn tab(p: &mut Page, shift: bool) {
    let from = p.ring;
    let to = if shift {
        (from + 2) % 3
    } else {
        (from + 1) % 3
    };
    focus(p, RING[from], false);
    focus(p, RING[to], true);
}

fn down(p: &mut Page, x: f32, y: f32, modifiers: i32) -> bool {
    p.event(Event::PointerDown {
        x,
        y,
        button: 1,
        modifiers: Modifiers(modifiers),
    })
}

fn value(p: &Page, name: &str) -> String {
    shown(&p.sheet, name)
}

/// The grid takes the keys while no node has focus: arrows move the
/// selection, Shift extends it, and the status line sums the range.
#[test]
fn arrows_select_and_shift_extends() {
    let mut p = page(800.0, 600.0, true);
    assert_eq!(p.range_name(), "A1");
    assert_eq!(p.bar.text(), "Hushållsbudget 2026");
    for _ in 0..3 {
        assert!(press(&mut p, key::DOWN));
    }
    assert!(press(&mut p, key::RIGHT));
    assert_eq!(p.range_name(), "B4");
    assert_eq!(p.bar.text(), "32500");
    assert!(key_mod(&mut p, key::RIGHT, 1));
    assert!(key_mod(&mut p, key::RIGHT, 1));
    assert!(key_mod(&mut p, key::DOWN, 1));
    assert_eq!(p.range_name(), "B4:D5");
    assert_eq!(p.status(), "B4:D5 · Summa 128400 · Medel 21400 · Antal 6");
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.range_name(), "C5", "an arrow without Shift collapses");
    assert!(press(&mut p, key::HOME));
    assert!(press(&mut p, key::END));
    assert_eq!(
        p.range_name(),
        "E5",
        "the last cell of the row with content"
    );
    press(&mut p, key::PAGE_DOWN);
    assert!(
        p.cur.1 > 15 && p.scroll.offset() > 0.0,
        "paged and scrolled"
    );
    for _ in 0..200 {
        press(&mut p, key::DOWN);
        press(&mut p, key::RIGHT);
    }
    assert_eq!(p.range_name(), "Z100", "stops at the last cell");
    let l = p.layout();
    let r = l.cell(p.cur.0, p.cur.1);
    assert!(r.right() <= l.view.right() && r.bottom() <= l.view.bottom());
    assert!(p.scroll_x > 0.0);
}

/// Typing with the keyboard on the grid edits in the cell: Enter commits
/// and moves down, Escape cancels, Delete empties the selection.
#[test]
fn enter_commits_and_moves_down_escape_cancels() {
    let mut p = page(800.0, 600.0, true);
    p.select((6, 0), false);
    assert!(type_text(&mut p, "=B4"));
    assert!(type_text(&mut p, "*2x"));
    assert!(p.editing);
    assert!(press(&mut p, key::BACKSPACE));
    assert_eq!(p.bar.text(), "=B4*2");
    assert!(press(&mut p, key::ENTER));
    assert_eq!(value(&p, "G1"), "65000");
    assert_eq!(p.range_name(), "G2");
    assert!(!p.editing);

    type_text(&mut p, "kasseras");
    assert!(press(&mut p, key::ESCAPE));
    assert!(!p.editing);
    assert_eq!(p.sheet.get(at("G2")).map(|c| c.shown()), None);
    assert_eq!(p.bar.text(), "");

    // An arrow commits too, and moves that way.
    type_text(&mut p, "7");
    assert!(press(&mut p, key::UP));
    assert_eq!(value(&p, "G2"), "7");
    assert_eq!(p.range_name(), "G1");
    assert!(key_mod(&mut p, key::DOWN, 1));
    assert!(press(&mut p, key::DELETE));
    assert_eq!(p.sheet.get(at("G1")).map(|c| c.shown()), None);
    assert_eq!(p.sheet.get(at("G2")).map(|c| c.shown()), None);
    // Shift+Enter moves up.
    p.select((6, 5), false);
    key_mod(&mut p, key::ENTER, 1);
    assert_eq!(p.range_name(), "G5");
}

/// SPEC-10.3.1: Tab is the host's. The bar sits between the bars of the
/// cells beside it in the tree, so Tab commits and moves right and
/// Shift+Tab left, and focus stays in the bar.
#[test]
fn tab_commits_and_moves_right_through_the_ring() {
    let mut p = page(800.0, 600.0, true);
    p.select((6, 2), false);
    // The first Tab of a visit lands on the first of the three nodes:
    // that only means the bar, the selection stays.
    focus(&mut p, RING[2], true);
    assert_eq!(p.ring, 2);
    assert_eq!(p.range_name(), "G3");
    assert!(p.bar.focused() && !p.grid_active);
    assert_eq!(p.bar.id(), NodeId(RING[2]));

    type_text(&mut p, "1,5");
    assert!(p.editing);
    tab(&mut p, false);
    assert_eq!(value(&p, "G3"), "1,5");
    assert_eq!(p.range_name(), "H3");
    assert!(p.bar.focused(), "focus is still in the bar");
    assert_eq!(p.bar.id(), NodeId(RING[0]));
    type_text(&mut p, "=G3*2");
    tab(&mut p, false);
    assert_eq!(value(&p, "H3"), "3");
    assert_eq!(p.range_name(), "I3");
    tab(&mut p, true);
    tab(&mut p, true);
    assert_eq!(p.range_name(), "G3");
    assert_eq!(p.bar.text(), "1,5");

    // The tree: the bar's node between its neighbours, in ring order.
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let boxes: Vec<(i32, &str)> = h
            .sem
            .iter()
            .filter(|n| n.role == Role::Textbox as i32)
            .map(|n| (n.id, n.text.as_str()))
            .collect();
        let k = p.ring;
        assert_eq!(
            boxes,
            [
                (RING[(k + 2) % 3], ""),
                (RING[k], "1,5"),
                (RING[(k + 1) % 3], "=G3*2")
            ]
        );
        assert!(h.sem.iter().any(|n| n.text == "Markerat: G3"));
    });

    // Enter in the focused bar commits and moves down; in the bar the
    // side arrows move the caret once an edit has begun.
    type_text(&mut p, "20");
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.range_name(), "G3", "the caret moved, not the selection");
    type_text(&mut p, "0");
    assert!(press(&mut p, key::ENTER));
    assert_eq!(value(&p, "G3"), "200");
    assert_eq!(p.range_name(), "G4");
    // Not editing, the arrows are the grid's even with focus in the bar.
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.range_name(), "F4");
    // F2 edits what is there.
    p.select((6, 2), false);
    assert!(press(&mut p, F2));
    type_text(&mut p, "1");
    press(&mut p, key::ENTER);
    assert_eq!(value(&p, "G3"), "2001");
    // Focus leaving the bar for good commits as well.
    type_text(&mut p, "9");
    let bar = RING[p.ring];
    focus(&mut p, bar, false);
    assert_eq!(value(&p, "G4"), "9");
}

#[test]
fn the_pointer_selects_a_range_and_a_double_click_edits() {
    let mut p = page(800.0, 600.0, true);
    let l = p.layout();
    let b4 = l.cell(1, 3);
    let d6 = l.cell(3, 5);
    assert!(down(&mut p, b4.x + 10.0, b4.y + 10.0, 0));
    assert_eq!(p.range_name(), "B4");
    assert!(p.grid_active && p.selecting);
    p.event(Event::PointerMove {
        x: d6.x + 10.0,
        y: d6.y + 10.0,
        buttons: 1,
        modifiers: Modifiers(0),
    });
    assert_eq!(p.range_name(), "B4:D6");
    p.event(Event::PointerUp {
        x: d6.x + 10.0,
        y: d6.y + 10.0,
        button: 1,
        modifiers: Modifiers(0),
    });
    assert!(!p.selecting);
    // Shift+press extends from the anchor.
    down(&mut p, b4.x + 10.0, b4.y + 10.0, 0);
    mock::with(|h| h.now_ms += 1000);
    down(&mut p, d6.x + 10.0, d6.y + 10.0, 1);
    assert_eq!(p.range_name(), "B4:D6");
    assert_eq!(
        l.cell_at(0.0, 0.0),
        (0, 0),
        "outside the cells: the nearest"
    );

    // Two presses within 400 ms edit what is in the cell.
    mock::with(|h| h.now_ms += 1000);
    down(&mut p, b4.x + 10.0, b4.y + 10.0, 0);
    assert!(!p.editing);
    mock::with(|h| h.now_ms += 150);
    down(&mut p, b4.x + 10.0, b4.y + 10.0, 0);
    assert!(p.editing);
    type_text(&mut p, "0");
    press(&mut p, key::ENTER);
    assert_eq!(value(&p, "B4"), "325000");
    assert_eq!(value(&p, "E4"), "391500", "the sum followed");

    // The horizontal bar: a press beside the thumb pages.
    let thumb = l.hthumb.unwrap();
    assert!(down(&mut p, thumb.right() + 30.0, thumb.y + 2.0, 0));
    assert_eq!(p.scroll_x, l.view.w);
    assert_eq!(p.layout().cols.0, (l.view.w / CELL_W) as usize);
}

/// SPEC-8.2.4: Ctrl+C writes the shown values while the page handles the
/// key; outside the gesture, and without the capability, the host refuses
/// and the page says so. Pasted text arrives as TEXT_INPUT in the bar.
#[test]
fn spec_8_2_4_copy_writes_the_shown_values() {
    let mut p = page(800.0, 600.0, true);
    p.select((0, 9), false);
    p.select((1, 10), true);
    let ctrl_c = Event::KeyDown {
        code: key::C,
        modifiers: Modifiers(2),
        repeat: false,
    };
    assert!(p.event(ctrl_c));
    assert_eq!(p.status(), "Urklippet nekades (E_DENIED).");
    assert!(mock::as_copy_gesture(|| p.event(ctrl_c)));
    assert_eq!(p.status(), "A10:B11 kopierat.");
    mock::with(|h| assert_eq!(h.clipboard, "Utgifter\t16400,5\nKvar\t16099,5"));
    press(&mut p, key::RIGHT);
    assert!(
        !p.status().contains("kopierat"),
        "gone when the selection moves"
    );

    // A paste into the focused bar: line breaks become spaces.
    p.select((6, 0), false);
    let bar = RING[p.ring];
    focus(&mut p, bar, true);
    assert!(type_text(&mut p, "två\nrader"));
    assert_eq!(p.bar.text(), "två rader");
    press(&mut p, key::ENTER);
    assert_eq!(value(&p, "G1"), "två rader");
}

/// Only the cells in view are drawn, but every non-empty cell is a node
/// (SPEC-10.1.1), and the headers follow the scroll.
#[test]
fn the_grid_is_virtualised_and_the_tree_is_whole() {
    let mut p = page(800.0, 600.0, true);
    let l = p.layout();
    assert_eq!(l.cols, (0, 8));
    assert_eq!(l.rows.0, 0);
    assert!(l.rows.1 < 20);
    mock::run_frame(&mut p, 0).unwrap();
    let texts = |h: &mock::MockHost| -> Vec<String> {
        h.frame_ops()
            .iter()
            .filter_map(|op| match op {
                mock::Op::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    let cells = p.sheet.count();
    mock::with(|h| {
        let drawn = texts(h);
        assert!(drawn.iter().any(|t| t == "99000"));
        assert!(drawn.iter().any(|t| t == "H") && !drawn.iter().any(|t| t == "I"));
        let nodes = |r: Role| h.sem.iter().filter(|n| n.role == r as i32).count();
        assert_eq!(nodes(Role::Cell), cells);
        assert_eq!(nodes(Role::Row), 12);
        assert_eq!(nodes(Role::Table), 1);
        assert!(h.sem.iter().any(|n| n.text == "E4: 99000"));
        // The bar and its two neighbours each have their name as the
        // `text` node right before them.
        let named: Vec<(&str, i32)> = h
            .sem
            .windows(2)
            .filter(|w| w[1].role == Role::Textbox as i32)
            .map(|w| (w[0].text.as_str(), w[0].role))
            .collect();
        let text = Role::Text as i32;
        assert_eq!(
            named,
            [
                ("Föregående cell", text),
                ("Innehåll i A1", text),
                ("Nästa cell", text)
            ]
        );
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });

    // Far down and to the right: other headers, no sample cell drawn,
    // the same tree.
    p.select((25, 99), false);
    let l = p.layout();
    assert_eq!(l.cols.1, COLS);
    assert_eq!(l.rows.1, ROWS);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let drawn = texts(h);
        assert!(!drawn.iter().any(|t| t == "99000"));
        assert!(drawn.iter().any(|t| t == "Z") && drawn.iter().any(|t| t == "100"));
        assert!(!drawn.iter().any(|t| t == "A"));
        let n = h.sem.iter().filter(|n| n.role == Role::Cell as i32).count();
        assert_eq!(n, cells);
    });
    // A sideways wheel scrolls the columns; the wheel scrolls the rows.
    p.event(Event::Scroll {
        dx: 0.0,
        dy: -3.0,
        lines: true,
        modifiers: Modifiers(1),
    });
    assert_eq!(p.scroll_x, Page::max_x(&l) - 3.0 * CELL_W);
    let before = p.scroll.offset();
    p.event(Event::Scroll {
        dx: 0.0,
        dy: -100.0,
        lines: false,
        modifiers: Modifiers(0),
    });
    assert_eq!(p.scroll.offset(), before - 100.0);
}

/// SPEC-5.3.3 (granskning 0013:3): a frame costs what its cells show, not
/// what they hold. Of a text that is cut at its cell the page sends the
/// start, as much as a cell's width can show, and measures nothing; the
/// tree has every cell, the long texts whole while their pool lasts and
/// cut after that. 2 600 cells of 200 bytes used to send 114 KB to draw
/// the cells in view and 535 KB for the tree, in every frame.
#[test]
fn spec_5_3_3_a_frame_sends_what_the_cells_show_of_long_texts() {
    let mut p = page(1600.0, 1000.0, true);
    let long = "Wi".repeat(100);
    for i in 0..CELLS {
        p.sheet.set(i, &long);
    }
    let l = p.layout();
    let in_view = (l.cols.1 - l.cols.0) * (l.rows.1 - l.rows.0);
    assert!(in_view > 500, "{in_view}");
    mock::with(|h| h.text_bytes = 0);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let drawn: Vec<&str> = h
            .frame_ops()
            .iter()
            .filter_map(|op| match op {
                mock::Op::Text { text, .. } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        let heads = drawn.iter().filter(|t| **t == &long[..HEAD_CHARS]).count();
        assert_eq!(heads, in_view, "every cell in view shows its start");
        let cut = |t: &&str| !t.starts_with("Wi") || t.len() <= HEAD_CHARS;
        assert!(drawn.iter().all(cut), "and no more than that");
        // Nothing but what was drawn went to the host: no text was
        // measured to find out that it does not fit.
        let cells = (in_view * HEAD_CHARS) as u64;
        assert!(h.text_bytes < cells + 1500, "{}", h.text_bytes);

        let labels: Vec<&str> = h
            .sem
            .iter()
            .filter(|n| n.role == Role::Cell as i32)
            .map(|n| n.text.as_str())
            .collect();
        assert_eq!(labels.len(), CELLS, "every cell is in the tree");
        assert_eq!(labels[0], format!("A1: {long}"));
        let whole = labels.iter().filter(|t| t.ends_with(&long)).count();
        assert_eq!(whole, LABEL_POOL / (4 + 200 - LABEL_SHORT), "the pool");
        assert_eq!(labels[CELLS - 1], "Z100: WiWiWiWiWiWiWi…");
        let bytes: usize = labels.iter().map(|t| t.len()).sum();
        assert!(bytes <= CELLS * (LABEL_SHORT + 3) + LABEL_POOL, "{bytes}");
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });
    // What is copied is still the whole text.
    mock::as_copy_gesture(|| key_mod(&mut p, key::C, 2));
    mock::with(|h| assert_eq!(h.clipboard, long));

    // A window with more such cells in view than a frame pays for whole:
    // each shows less of its text, and a column has one clip for them all.
    let mut p = page(2560.0, 1440.0, true);
    for i in 0..CELLS {
        p.sheet.set(i, &long);
    }
    let l = p.layout();
    let in_view = (l.cols.1 - l.cols.0) * (l.rows.1 - l.rows.0);
    let each = DRAW_POOL / in_view;
    assert!(in_view > 1200 && (HEAD_MIN..HEAD_CHARS).contains(&each));
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let ops = h.frame_ops();
        let is_head =
            |op: &&mock::Op| matches!(op, mock::Op::Text { text, .. } if *text == long[..each]);
        assert_eq!(ops.iter().filter(is_head).count(), in_view);
        let clips = ops
            .iter()
            .filter(|op| matches!(op, mock::Op::ClipPush { .. }))
            .count();
        assert!(clips <= COLS + 4, "{clips} clips");
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });

    // Characters of several bytes: 28 of them, or 40 bytes, whichever is
    // less, and never part of one.
    p.sheet.set(at("A1"), &"å".repeat(100));
    p.sheet.set(at("B1"), &"\u{4e2d}".repeat(66));
    p.sheet.set(at("C1"), "kort");
    assert_eq!(p.sheet.get(at("A1")).unwrap().head(), "å".repeat(20));
    assert_eq!(p.sheet.get(at("B1")).unwrap().head(), "\u{4e2d}".repeat(13));
    assert_eq!(p.sheet.get(at("C1")).unwrap().head(), "kort");
}

/// A text still runs over the empty cells to its right when it fits them
/// whole, measured once, and is cut at its own cell when it does not.
#[test]
fn a_text_runs_over_empty_cells_or_is_cut_at_its_own() {
    let mut p = page(800.0, 600.0, true);
    for i in p.sheet.filled().collect::<Vec<_>>() {
        p.sheet.set(i, "");
    }
    // 35 characters: 273 px in the mock, so the text needs three cells.
    let title = "En rubrik som är längre än sin cell";
    p.sheet.set(at("A1"), title);
    let frame = |p: &mut Page| -> (Vec<mock::Op>, u64) {
        mock::with(|h| h.text_bytes = 0);
        mock::run_frame(p, 0).unwrap();
        mock::with(|h| (h.frame_ops().to_vec(), h.text_bytes))
    };
    let at_cell = |ops: &[mock::Op], text: &str| -> Option<bool> {
        let i = ops
            .iter()
            .position(|op| matches!(op, mock::Op::Text { text: t, .. } if t == text))?;
        // Cut texts come after the whole ones, inside their column's clip.
        let open = |ops: &[mock::Op]| {
            let count = |push: bool| {
                ops.iter()
                    .filter(|op| match op {
                        mock::Op::ClipPush { .. } => push,
                        mock::Op::ClipPop => !push,
                        _ => false,
                    })
                    .count()
            };
            count(true) - count(false)
        };
        Some(open(&ops[..i]) > open(&ops[..1]))
    };
    let (ops, first) = frame(&mut p);
    assert_eq!(at_cell(&ops, title), Some(false), "whole and not clipped");
    let (_, second) = frame(&mut p);
    assert_eq!(first - second, title.len() as u64, "measured once");

    // With C1 taken it does not fit: its start, clipped at A1.
    p.sheet.set(at("C1"), "x");
    let (ops, _) = frame(&mut p);
    assert_eq!(at_cell(&ops, title), None);
    assert_eq!(
        at_cell(&ops, "En rubrik som är längre än s"),
        Some(true),
        "28 characters under the cell's clip"
    );
    // With only D1 taken it fits again.
    p.sheet.set(at("C1"), "");
    p.sheet.set(at("D1"), "x");
    let (ops, _) = frame(&mut p);
    assert_eq!(at_cell(&ops, title), Some(false));
    // A text of 29 one-byte characters and no empty cell beside it is
    // not measured at all.
    p.sheet.set(at("A2"), &"m".repeat(29));
    p.sheet.set(at("B2"), "x");
    frame(&mut p);
    let (_, before) = frame(&mut p);
    p.sheet.set(at("A2"), &"n".repeat(29));
    let (ops, after) = frame(&mut p);
    assert_eq!(after, before, "a new value, and nothing measured");
    assert_eq!(at_cell(&ops, &"n".repeat(28)), Some(true));
}

/// A frame measures 8 KB of text at the most. A view with more texts that
/// may fit than that has them measured over the frames that follow, and
/// each is cut at its cell until it has been.
#[test]
fn texts_in_view_are_measured_a_pool_a_frame() {
    let mut p = page(2560.0, 1440.0, true);
    let text = "m".repeat(HEAD_CHARS);
    for i in 0..CELLS {
        p.sheet.set(i, &text);
    }
    let l = p.layout();
    let in_view = (l.cols.1 - l.cols.0) * (l.rows.1 - l.rows.0);
    let unmeasured = |p: &Page| {
        (0..in_view)
            .filter(|i| p.sheet.get(*i).unwrap().width < 0.0)
            .count()
    };
    assert_eq!(unmeasured(&p), in_view);
    let mut frames = 0;
    while unmeasured(&p) > 0 {
        let (before, asked) = (unmeasured(&p), mock::with(|h| h.frame_requests));
        mock::with(|h| h.text_bytes = 0);
        mock::run_frame(&mut p, 0).unwrap();
        frames += 1;
        let measured = before - unmeasured(&p);
        assert_eq!(measured, (MEASURE_POOL / HEAD_CHARS).min(before));
        mock::with(|h| {
            // Every cell in view is drawn in every one of the frames.
            let drawn = h
                .frame_ops()
                .iter()
                .filter(|op| matches!(op, mock::Op::Text { text, .. } if text.starts_with("mm")))
                .count();
            assert_eq!(drawn, in_view);
            assert_eq!(h.frame_requests, asked + u32::from(unmeasured(&p) > 0));
        });
    }
    assert_eq!(frames, (in_view * HEAD_CHARS).div_ceil(MEASURE_POOL));
}

/// SPEC-5.2.9, SPEC-5.3.1: the sheet comes back in a new instance; without
/// storage the page works in memory and says "sparas inte".
#[test]
fn storage_persists_and_its_absence_is_shown() {
    let mut p = page(800.0, 600.0, true);
    assert_eq!(p.note(), "50 celler");
    p.select((6, 0), false);
    type_text(&mut p, "=SUMMA(B4:D4)/3");
    press(&mut p, key::ENTER);
    // The stored sheet is read by `init` and calculated by the first
    // frame, before it is drawn.
    let mut again = init(800.0, 600.0);
    assert_eq!(again.note(), "51 celler · räknar…");
    mock::run_frame(&mut again, 0).unwrap();
    assert_eq!(value(&again, "G1"), "33000");
    assert_eq!(again.note(), "51 celler");
    mock::with(|h| assert_eq!(h.frame_requests, 0, "done in one step"));

    let mut p = page(800.0, 600.0, false);
    assert_eq!(p.note(), "50 celler · sparas inte");
    p.select((6, 0), false);
    type_text(&mut p, "1");
    assert!(press(&mut p, key::ENTER));
    assert_eq!(value(&p, "G1"), "1");
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| assert!(h.sem.iter().any(|n| n.text == "51 celler · sparas inte")));
}
