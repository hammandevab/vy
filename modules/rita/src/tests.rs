//! Native tests: simplification, the path and the stored form of a
//! stroke, and the page's pen, eraser, history, limits, dialog, tree
//! and storage against the SDK's mock host.

use super::*;
use ink::{simplify, MAX_STROKE_POINTS, SUB};

const TOLERANCE: f32 = 1.0;
use vy_sdk::mock;

fn page(width: f32, height: f32, storage: bool) -> Page {
    mock::reset();
    mock::with(|h| h.caps.storage = storage);
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

fn chord(p: &mut Page, code: i32, modifiers: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(modifiers),
        repeat: false,
    })
}

/// A stroke through window points given relative to the paper's corner.
fn stroke(p: &mut Page, points: &[(f32, f32)]) {
    let paper = p.layout().paper;
    let at = |(x, y): (f32, f32)| (paper.x + x, paper.y + y);
    let (x, y) = at(points[0]);
    down(p, x, y);
    for &q in &points[1..] {
        let (x, y) = at(q);
        drag(p, x, y);
    }
    let (x, y) = at(points[points.len() - 1]);
    up(p, x, y);
}

fn tap(p: &mut Page, key: usize) -> bool {
    let r = p.layout().keys[key];
    down(p, r.x + r.w / 2.0, r.y + r.h / 2.0)
}

fn node_text(id: i32) -> Option<String> {
    mock::with(|h| h.sem.iter().find(|n| n.id == id).map(|n| n.text.clone()))
}

/// Ramer–Douglas–Peucker keeps the ends and the corners and drops what
/// lies within the tolerance of the line between its neighbours.
#[test]
fn simplification_keeps_corners_and_drops_jitter() {
    // A straight line with jitter below the tolerance: two points.
    let line: Vec<Point> = (0..200)
        .map(|i| (i as f32, 100.0 + if i % 2 == 0 { 0.3 } else { -0.3 }))
        .collect();
    assert_eq!(simplify(&line, TOLERANCE), [line[0], line[199]]);
    // An L: the corner stays.
    let mut corner: Vec<Point> = (0..=50).map(|i| (i as f32 * 2.0, 0.0)).collect();
    corner.extend((1..=50).map(|i| (100.0, i as f32 * 2.0)));
    assert_eq!(
        simplify(&corner, TOLERANCE),
        [(0.0, 0.0), (100.0, 0.0), (100.0, 100.0)]
    );
    // A parabola: every dropped point is within the tolerance of the
    // kept line, and a tighter tolerance keeps more.
    let curve: Vec<Point> = (0..=300)
        .map(|i| (i as f32, (i as f32 - 150.0) * (i as f32 - 150.0) / 100.0))
        .collect();
    let loose = simplify(&curve, 2.0);
    let tight = simplify(&curve, 0.25);
    assert!(
        loose.len() < tight.len() && tight.len() < 60,
        "{}",
        tight.len()
    );
    assert_eq!(loose[0], curve[0]);
    assert_eq!(*loose.last().unwrap(), curve[300]);
    for w in loose.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (vx, vy) = (b.0 - a.0, b.1 - a.1);
        let len = (vx * vx + vy * vy).sqrt();
        for p in curve.iter().filter(|p| p.0 > a.0 && p.0 < b.0) {
            let off = (vx * (a.1 - p.1) - (a.0 - p.0) * vy).abs() / len;
            assert!(off <= 2.0 + 1e-3, "{p:?} is {off} from the line");
        }
    }
    assert_eq!(simplify(&[], 1.0), []);
    assert_eq!(simplify(&[(1.0, 2.0)], 1.0), [(1.0, 2.0)]);
    // 100 000 points do not recurse.
    let long: Vec<Point> = (0..100_000)
        .map(|i| (i as f32 * 0.01, (i % 100) as f32))
        .collect();
    assert!(simplify(&long, 0.5).len() > 1000);
}

/// The kind of each command of a path (SPEC-C.1.2): 0 moveTo, 1 lineTo,
/// 2 quadTo (two commands), 3 close.
fn kinds(path: &Path) -> Vec<u8> {
    path.bytes().chunks(9).map(|c| c[0]).collect()
}

/// A stroke is one path, smooth where its points bend and sharp where
/// they turn, far below the cap of 65 535 commands (SPEC-9.3.2), and the
/// same bytes every time.
#[test]
fn a_stroke_is_one_path_smooth_in_bends_and_sharp_in_corners() {
    // A gentle arc: a quadratic curve (two commands) around each inner
    // point.
    let arc: Vec<Point> = (0..=8)
        .map(|i| (100.0 + i as f32 * 50.0, 300.0 - (i * (8 - i)) as f32 * 6.0))
        .collect();
    let s = Stroke::new(1, 2, &arc, 0.01).unwrap();
    assert_eq!(s.pts.len(), 9);
    assert_eq!(s.pts[0], (800, 2400), "eighths of a paper unit");
    let path = s.path(|p| p);
    assert_eq!(path.len(), 1 + 2 * 7);
    assert_eq!(path.bytes().len(), 9 * path.len());
    assert_eq!(kinds(&path), [0, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2]);
    assert_eq!(s.path(|p| p), path, "deterministic");
    // The curve ends on the last point.
    let last = &path.bytes()[path.bytes().len() - 18..];
    assert_eq!(f32::from_le_bytes(last[1..5].try_into().unwrap()), 500.0);

    // A square: straight lines to its corners.
    let square = [
        (100.0, 100.0),
        (300.0, 100.0),
        (300.0, 300.0),
        (100.0, 300.0),
    ];
    let s = Stroke::new(0, 0, &square, TOLERANCE).unwrap();
    assert_eq!(kinds(&s.path(|p| p)), [0, 1, 1, 1]);
    // A bend, then a corner, then a bend: curve, line, curve.
    let mixed = [
        (0.0, 0.0),
        (100.0, 10.0),
        (200.0, 0.0),
        (210.0, 200.0),
        (300.0, 210.0),
        (400.0, 200.0),
    ];
    let s = Stroke::new(0, 0, &mixed, 0.01).unwrap();
    assert_eq!(kinds(&s.path(|p| p)), [0, 2, 2, 1, 1, 2, 2]);

    // Two points are a line, one point no path (the page draws a dot).
    let two = Stroke::new(0, 0, &[(0.0, 0.0), (50.0, 50.0)], TOLERANCE).unwrap();
    assert_eq!(kinds(&two.path(|p| p)), [0, 1]);
    let one = Stroke::new(0, 0, &[(5.0, 5.0)], TOLERANCE).unwrap();
    assert!(one.path(|p| p).is_empty());
    assert_eq!(Stroke::new(0, 0, &[], TOLERANCE), None);
    // A stroke never has more points than the cap, wherever they lie.
    let wild: Vec<Point> = (0..5000)
        .map(|i| (((i * 37) % 1200) as f32, ((i * 91) % 800) as f32))
        .collect();
    let s = Stroke::new(0, 0, &wild, TOLERANCE).unwrap();
    assert_eq!(s.pts.len(), MAX_STROKE_POINTS);
    assert!(s.path(|p| p).len() <= 2 * MAX_STROKE_POINTS);
    // Points outside the paper are stored on its edge.
    let out = Stroke::new(0, 0, &[(-50.0, 900.0), (2000.0, -3.0)], TOLERANCE).unwrap();
    assert_eq!(
        out.pts,
        [(0, (PAPER_H * SUB) as i16), ((PAPER_W * SUB) as i16, 0)]
    );
}

#[test]
fn the_stored_form_round_trips_and_is_compact() {
    let a = Stroke::new(
        5,
        0,
        &[(10.0, 10.0), (400.0, 300.0), (1200.0, 800.0)],
        TOLERANCE,
    )
    .unwrap();
    let b = Stroke::new(7, 2, &[(600.5, 400.25)], TOLERANCE).unwrap();
    let strokes = vec![a, b];
    let bytes = encode(&strokes);
    assert_eq!(decode(&bytes), Some(strokes.clone()));
    assert!(bytes.len() <= 3 + 2 * 3 + 4 * 6, "{}", bytes.len());
    assert_eq!(decode(&[]), None);
    assert_eq!(decode(&[9, 0, 0]), None, "unknown format");
    assert_eq!(decode(&bytes[..bytes.len() - 1]), None, "cut short");
    assert_eq!(decode(&[1, 1, 0, 3, 1, 0, 0, 0]), None, "no such pen");
    assert_eq!(decode(&[1, 1, 0, 0, 1, 0, 1, 0]), None, "off the paper");
    assert_eq!(decode(&[1, 0, 0]), Some(Vec::new()));

    // The largest drawing the page keeps fits the 64 KB quota: 2 500
    // points as far apart as the paper allows.
    let mut big = Vec::new();
    for s in 0..MAX_POINTS / MAX_STROKE_POINTS {
        let raw: Vec<Point> = (0..MAX_STROKE_POINTS)
            .map(|i| {
                if (i + s) % 2 == 0 {
                    (0.0, (i % 800) as f32)
                } else {
                    (1200.0, 800.0 - (i % 800) as f32)
                }
            })
            .collect();
        big.push(Stroke::new(0, 2, &raw, TOLERANCE).unwrap());
    }
    let points: usize = big.iter().map(|s| s.pts.len()).sum();
    assert_eq!(points, MAX_POINTS);
    let bytes = encode(&big);
    assert!(bytes.len() < 32 * 1024, "{}", bytes.len());
    assert_eq!(decode(&bytes).map(|d| d.len()), Some(big.len()));
}

/// A drag draws one simplified stroke with the chosen colour and pen;
/// a tap is a dot; the tree counts the strokes.
#[test]
fn the_pen_draws_simplified_strokes() {
    let mut p = page(1280.0, 800.0, true);
    assert!(tap(&mut p, 1));
    assert!(tap(&mut p, K_PEN + 2));
    assert_eq!((p.color, p.pen), (1, 2));
    let wave: Vec<(f32, f32)> = (0..=120)
        .map(|i| (100.0 + i as f32 * 4.0, 200.0 + ((i / 20) % 2) as f32 * 60.0))
        .collect();
    stroke(&mut p, &wave);
    assert_eq!(p.strokes.len(), 1);
    let s = &p.strokes[0];
    assert_eq!((s.color, s.width), (1, 2));
    assert!(s.pts.len() < 40, "{} of 121 points kept", s.pts.len());
    assert_eq!(p.points, s.pts.len());
    assert!(p.ink > 0.0);
    stroke(&mut p, &[(300.0, 300.0)]);
    assert_eq!(p.strokes[1].pts.len(), 1, "a tap is a dot");
    assert_eq!(p.status(), "2 streck");

    let ops = mock::run_frame(&mut p, 0).unwrap();
    assert!(ops > 30);
    mock::with(|h| {
        let paths = h
            .frame_ops()
            .iter()
            .filter(|op| matches!(op, mock::Op::Path { .. }))
            .count();
        assert_eq!(paths, 1, "one path for the stroke, a dot for the tap");
        assert!(h
            .sem
            .iter()
            .any(|n| n.text == "Ritning med 2 streck" && n.role == Role::Image as i32));
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });
    // A press outside the paper draws nothing.
    let paper = p.layout().paper;
    down(&mut p, paper.x - 5.0, paper.y + 10.0);
    up(&mut p, paper.x - 5.0, paper.y + 10.0);
    assert_eq!(p.strokes.len(), 2);
    // While the pen is down the page shows the raw stroke.
    down(&mut p, paper.x + 10.0, paper.y + 10.0);
    drag(&mut p, paper.x + 60.0, paper.y + 40.0);
    assert!(p.drawing && p.raw.len() == 2);
    mock::run_frame(&mut p, 0).unwrap();
    // Released outside the window: a move without the button ends it.
    p.event(Event::PointerMove {
        x: -10.0,
        y: 0.0,
        buttons: 0,
        modifiers: Modifiers(0),
    });
    assert!(!p.drawing);
    assert_eq!(p.strokes.len(), 3);
}

/// The eraser removes every stroke it touches, whole, as one step; undo
/// puts them back where they were, redo takes them again.
#[test]
fn the_eraser_and_the_history() {
    let mut p = page(1280.0, 800.0, true);
    stroke(&mut p, &[(100.0, 100.0), (300.0, 100.0)]);
    stroke(&mut p, &[(100.0, 200.0), (300.0, 200.0)]);
    stroke(&mut p, &[(100.0, 300.0), (300.0, 300.0)]);
    let all = p.strokes.clone();
    assert!(tap(&mut p, K_ERASER));
    assert!(p.eraser);
    // Down through the first two strokes, not the third.
    stroke(&mut p, &[(200.0, 80.0), (200.0, 150.0), (200.0, 220.0)]);
    assert_eq!(p.strokes, [all[2].clone()]);
    assert_eq!(p.undo.len(), 4, "three strokes and one erasure");
    assert!(chord(&mut p, KEY_Z, 2));
    assert_eq!(p.strokes, all, "back in their places");
    assert!(chord(&mut p, KEY_Y, 2));
    assert_eq!(p.strokes.len(), 1);
    assert!(chord(&mut p, KEY_Z, 2));
    // Undo three times more: the strokes go, last first; Ctrl+Shift+Z
    // brings one back; a new stroke forgets what was undone.
    for left in [2, 1, 0] {
        assert!(tap(&mut p, K_UNDO));
        assert_eq!(p.strokes.len(), left);
    }
    tap(&mut p, K_UNDO);
    assert!(
        p.undo.is_empty() && p.redo.len() == 4,
        "nothing more to undo"
    );
    assert!(chord(&mut p, KEY_Z, 3));
    assert_eq!(p.strokes, [all[0].clone()]);
    assert_eq!(p.redo.len(), 3);
    assert!(tap(&mut p, 0), "a colour: the pen again");
    assert!(!p.eraser);
    stroke(&mut p, &[(500.0, 500.0), (520.0, 520.0)]);
    assert!(p.redo.is_empty());
    let n = p.strokes.len();
    tap(&mut p, K_REDO);
    assert_eq!(p.strokes.len(), n, "nothing to redo");
    // An eraser that touches nothing is no step.
    tap(&mut p, K_ERASER);
    let steps = p.undo.len();
    stroke(&mut p, &[(900.0, 600.0), (910.0, 610.0)]);
    assert_eq!(p.undo.len(), steps);
    assert_eq!(
        p.points,
        p.strokes.iter().map(|s| s.pts.len()).sum::<usize>()
    );
}

/// Clear asks once in a dialog; while it is open the tools are out of
/// the tree and the paper does not draw.
#[test]
fn clear_asks_once() {
    let mut p = page(800.0, 600.0, true);
    assert!(tap(&mut p, K_CLEAR));
    assert!(!p.confirm, "nothing to clear: no question");
    stroke(&mut p, &[(100.0, 100.0), (300.0, 100.0)]);
    stroke(&mut p, &[(100.0, 200.0), (300.0, 200.0)]);
    assert!(tap(&mut p, K_CLEAR));
    assert!(p.confirm);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| {
        let ids: Vec<i32> = h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect();
        assert_eq!(ids, [ID_YES, ID_NO], "only the dialog can have focus");
        assert!(h.sem.iter().any(|n| n.role == Role::Dialog as i32));
    });
    // The paper is behind the dialog.
    let before = p.strokes.len();
    stroke(&mut p, &[(20.0, 20.0), (60.0, 60.0)]);
    assert_eq!(p.strokes.len(), before);
    // Escape and "Avbryt" keep the drawing.
    assert!(chord(&mut p, key::ESCAPE, 0));
    assert!(!p.confirm && p.strokes.len() == 2);
    tap(&mut p, K_CLEAR);
    let no = p.layout().no;
    assert!(down(&mut p, no.x + 5.0, no.y + 5.0));
    assert!(!p.confirm && p.strokes.len() == 2);
    // "Rensa" clears; undo brings everything back.
    tap(&mut p, K_CLEAR);
    let yes = p.layout().yes;
    assert!(down(&mut p, yes.x + 5.0, yes.y + 5.0));
    assert!(!p.confirm && p.strokes.is_empty());
    assert_eq!(p.status(), "0 streck");
    assert!(chord(&mut p, KEY_Z, 2));
    assert_eq!(p.strokes.len(), 2);
    assert!(chord(&mut p, KEY_Y, 2));
    assert!(p.strokes.is_empty());
    // With the dialog closed its buttons take no press on the paper.
    let paper = p.layout().paper;
    stroke(&mut p, &[(yes.x - paper.x + 5.0, yes.y - paper.y + 5.0)]);
    assert_eq!(p.strokes.len(), 1);
}

/// SPEC-5.3.3, SPEC-9.3.2: the drawing stops at 2 500 points; the page
/// then refuses strokes with a message and the frame still draws.
#[test]
fn a_full_drawing_refuses_strokes_with_a_message() {
    let mut p = page(1280.0, 800.0, true);
    let zigzag: Vec<(f32, f32)> = (0..400)
        .map(|i| (20.0 + (i % 2) as f32 * 900.0, 10.0 + i as f32 * 1.5))
        .collect();
    let mut rounds = 0;
    while !p.full() {
        stroke(&mut p, &zigzag);
        rounds += 1;
        assert!(rounds < 100);
    }
    assert!(p.points <= MAX_POINTS);
    assert!(p.strokes.iter().all(|s| s.pts.len() <= MAX_STROKE_POINTS));
    let n = p.strokes.len();
    stroke(&mut p, &zigzag);
    assert_eq!(p.strokes.len(), n, "refused");
    assert!(p.refused);
    assert!(p.status().contains("fullt"));
    let ops = mock::run_frame(&mut p, 0).unwrap();
    assert!(ops < 65_536);
    mock::with(|h| {
        assert!(h
            .sem
            .iter()
            .any(|n| n.text.starts_with("Ritningen är full")));
        let bytes: usize = h
            .frame_ops()
            .iter()
            .map(|op| match op {
                mock::Op::Path { bytes, .. } => bytes.len(),
                _ => 0,
            })
            .sum();
        assert!(bytes <= MAX_POINTS * 18, "{bytes} path bytes a frame");
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });
    assert!(encode(&p.strokes).len() < 48 * 1024);
    // Undo makes room again.
    tap(&mut p, K_UNDO);
    assert!(!p.full());
    stroke(&mut p, &[(50.0, 50.0), (80.0, 80.0)]);
    assert_eq!(p.strokes.len(), n);

    // One endless stroke is cut into several, none over the cap.
    let mut p = page(1280.0, 800.0, true);
    let paper = p.layout().paper;
    down(&mut p, paper.x + 10.0, paper.y + 10.0);
    for i in 0..2500 {
        let x = paper.x + 10.0 + (i % 2) as f32 * 20.0;
        drag(&mut p, x, paper.y + 10.0 + (i % 600) as f32);
    }
    up(&mut p, paper.x + 10.0, paper.y + 10.0);
    assert!(p.strokes.len() >= 5, "{}", p.strokes.len());
    assert!(p.points > 2490, "{}: nothing was cut off", p.points);
    assert!(p.strokes.iter().all(|s| s.pts.len() <= MAX_STROKE_POINTS));
}

/// SPEC-10.1.1: every tool is a button whose text says its state, in
/// Tab order; a narrow window wraps the tools into two rows inside it.
#[test]
fn tools_are_buttons_with_their_state_in_the_text() {
    let mut p = page(1280.0, 800.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    let ids: Vec<i32> = mock::with(|h| h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect());
    assert_eq!(
        ids,
        (0..KEYS as i32).map(|i| ID_KEY - i).collect::<Vec<_>>()
    );
    assert_eq!(node_text(ID_KEY).as_deref(), Some("Färg: svart (vald)"));
    assert_eq!(node_text(ID_KEY - 5).as_deref(), Some("Färg: blå"));
    assert_eq!(
        node_text(ID_KEY - 9).as_deref(),
        Some("Penna: mellan (vald)")
    );
    assert_eq!(node_text(ID_KEY - 11).as_deref(), Some("Sudd (av)"));
    assert_eq!(node_text(ID_KEY - 12).as_deref(), Some("Ångra (inget)"));
    assert_eq!(node_text(ID_KEY - 14).as_deref(), Some("Rensa ritningen"));

    // With the keyboard: focus a key, Enter or Space presses it.
    let focus = |p: &mut Page, id: i32| {
        p.event(Event::Focus {
            node: NodeId(id),
            gained: true,
            keyboard: true,
        })
    };
    focus(&mut p, ID_KEY - 5);
    assert!(chord(&mut p, key::ENTER, 0));
    assert_eq!(p.color, 5);
    focus(&mut p, ID_KEY - 11);
    assert!(chord(&mut p, key::SPACE, 0));
    stroke(&mut p, &[(100.0, 100.0), (300.0, 100.0)]);
    assert!(p.strokes.is_empty(), "the eraser draws nothing");
    chord(&mut p, key::SPACE, 0);
    stroke(&mut p, &[(100.0, 100.0), (300.0, 100.0)]);
    mock::run_frame(&mut p, 0).unwrap();
    assert_eq!(node_text(ID_KEY).as_deref(), Some("Färg: svart"));
    assert_eq!(node_text(ID_KEY - 5).as_deref(), Some("Färg: blå (vald)"));
    assert_eq!(node_text(ID_KEY - 12).as_deref(), Some("Ångra (1 steg)"));

    for (w, h) in [(400.0, 600.0), (760.0, 500.0), (1600.0, 1000.0)] {
        let p = page(w, h, true);
        let l = p.layout();
        for (i, r) in l.keys.iter().enumerate() {
            assert!(
                r.x >= 0.0 && r.right() <= w && r.bottom() <= l.paper.y,
                "{w}: key {i}"
            );
            if i > 0 {
                let before = l.keys[i - 1];
                assert!(
                    r.x >= before.right() || r.y > before.y,
                    "{w}: key {i} overlaps"
                );
            }
        }
        assert!(l.paper.right() <= w && l.paper.bottom() <= h);
        assert!(
            (l.paper.w / l.paper.h - 1.5).abs() < 1e-3,
            "the paper keeps its shape"
        );
    }
}

/// SPEC-5.2.9, SPEC-5.3.1: the drawing comes back in a new instance at
/// any window size; without storage the page draws in memory and says
/// "sparas inte".
#[test]
fn storage_persists_and_its_absence_is_shown() {
    let mut p = page(1280.0, 800.0, true);
    tap(&mut p, 4);
    stroke(&mut p, &[(100.0, 100.0), (300.0, 150.0), (500.0, 100.0)]);
    stroke(&mut p, &[(200.0, 300.0)]);
    let drawn = p.strokes.clone();
    let again = init(400.0, 700.0);
    assert_eq!(again.strokes, drawn);
    assert_eq!(again.status(), "2 streck");
    assert!(again.layout().k < p.layout().k, "the same paper, smaller");

    let mut p = page(800.0, 600.0, false);
    assert_eq!(p.status(), "0 streck · sparas inte");
    stroke(&mut p, &[(100.0, 100.0), (300.0, 150.0)]);
    assert_eq!(p.strokes.len(), 1);
    mock::run_frame(&mut p, 0).unwrap();
    mock::with(|h| assert!(h.sem.iter().any(|n| n.text == "1 streck · sparas inte")));
}
