//! Native tests: the rules (shapes, rotation and kicks, the bag, gravity
//! and locking, line clears, hold, levels, the end) and the page (fixed
//! steps, keys and on-screen keys, pause when hidden, the tree, the
//! stored high score) against the SDK's mock host.

use super::*;
use game::{LEVEL_LINES, LOCK_MOVES, LOCK_TICKS};
use vy_sdk::mock;

/// A started game with the mock host's deterministic random source.
fn started() -> Game {
    mock::reset();
    let mut g = Game::new();
    g.start();
    g
}

/// The same game with `shape` falling, spawned afresh.
fn with_piece(g: &mut Game, shape: Shape) {
    g.piece = Some(Piece {
        shape,
        rot: 0,
        x: 3,
        y: 0,
    });
}

fn ticks(g: &mut Game, n: u32) {
    for _ in 0..n {
        g.tick();
    }
}

/// Fill row `row` of the well except the columns in `gaps`.
fn fill_row(g: &mut Game, row: usize, gaps: &[usize]) {
    for col in 0..COLS {
        g.well[row][col] = if gaps.contains(&col) { 0 } else { 1 };
    }
}

/// Every shape is four distinct cells in each rotation, and four quarter
/// turns in the open, either way, bring a piece back where it was.
#[test]
fn shapes_have_four_cells_and_turn_full_circle() {
    for shape in Shape::ALL {
        for rot in 0..4 {
            let cells = shape.cells(rot);
            for (i, a) in cells.iter().enumerate() {
                assert!((0..4).contains(&a.0) && (0..4).contains(&a.1));
                assert!(cells[i + 1..].iter().all(|b| b != a), "{shape:?} {rot}");
            }
        }
        let mut g = started();
        g.piece = Some(Piece {
            shape,
            rot: 0,
            x: 3,
            y: 8,
        });
        let home = g.piece;
        for clockwise in [true, false] {
            for turn in 1..=4 {
                let turned = g.rotate(clockwise);
                assert_eq!(turned, shape != Shape::O, "{shape:?}");
                if turn < 4 && shape != Shape::O {
                    assert_ne!(g.piece.unwrap().rot, 0);
                }
            }
            assert_eq!(g.piece, home, "{shape:?} clockwise {clockwise}");
        }
        assert!(g.rotate(true) == g.rotate(false));
        assert_eq!(g.piece, home, "a turn and back");
    }
    assert_eq!(Shape::T.cells(0), [(1, 0), (0, 1), (1, 1), (2, 1)]);
    assert_eq!(Shape::I.cells(1), [(2, 0), (2, 1), (2, 2), (2, 3)]);
    assert_eq!(Shape::L.name(), "L");
}

/// A turn that does not fit in place is shifted by the first kick that
/// does: off a wall, off the floor, and into a slot under an overhang.
#[test]
fn rotation_kicks_off_walls_floor_and_stack() {
    // The I upright against the left wall lies down one column in.
    let mut g = started();
    g.piece = Some(Piece {
        shape: Shape::I,
        rot: 1,
        x: -2,
        y: 5,
    });
    assert!(g.fits(&g.piece.unwrap()));
    assert!(g.rotate(true));
    let p = g.piece.unwrap();
    assert_eq!((p.rot, p.x), (2, 0));
    assert!(p.cells().iter().all(|c| c.0 >= 0));

    // And against the right wall, turning the other way.
    g.piece = Some(Piece {
        shape: Shape::I,
        rot: 1,
        x: 7,
        y: 5,
    });
    assert!(g.rotate(false));
    assert!(g.piece.unwrap().cells().iter().all(|c| c.0 < COLS as i8));

    // The T flat on the floor turns upright by rising a row.
    let mut g = started();
    g.piece = Some(Piece {
        shape: Shape::T,
        rot: 2,
        x: 3,
        y: ROWS as i8 - 3,
    });
    assert!(g.fits(&g.piece.unwrap()));
    assert!(g.rotate(true));
    let p = g.piece.unwrap();
    assert!(p.cells().iter().all(|c| c.1 < ROWS as i8), "{p:?}");

    // The T against the right wall turns back to flat one column in.
    g.piece = Some(Piece {
        shape: Shape::T,
        rot: 3,
        x: 8,
        y: 5,
    });
    assert!(g.fits(&g.piece.unwrap()), "upright, its stem on the wall");
    assert!(g.rotate(true));
    assert_eq!(g.piece.unwrap().x, 7);

    // Against the stack: the turn in place and the first kick are
    // blocked, so the T takes the second kick, one column right and one
    // row down.
    let mut g = started();
    g.well[16][1] = 1;
    g.well[16][4] = 1;
    g.piece = Some(Piece {
        shape: Shape::T,
        rot: 1,
        x: 1,
        y: 15,
    });
    assert!(g.fits(&g.piece.unwrap()));
    assert!(g.rotate(true));
    let p = g.piece.unwrap();
    assert_eq!((p.rot, p.x, p.y), (2, 2, 16), "kicked past the stack");
    // Turned back there, it fits in place and stays.
    assert!(g.rotate(false));
    let p = g.piece.unwrap();
    assert_eq!((p.rot, p.x, p.y), (1, 2, 16));
    // A piece boxed in on every side does not turn.
    let mut g = started();
    for row in 0..ROWS {
        fill_row(&mut g, row, &[4]);
    }
    g.piece = Some(Piece {
        shape: Shape::I,
        rot: 1,
        x: 2,
        y: 10,
    });
    assert!(!g.rotate(true) && !g.rotate(false));
    assert_eq!(g.piece.unwrap().rot, 1);
}

/// SPEC-5.2.11: the order of the pieces comes from the host's random
/// through a bag: every seven pieces hold each shape once, and the same
/// source gives the same order.
#[test]
fn the_bag_deals_each_shape_once_in_seven() {
    let deal = || {
        mock::reset();
        let mut g = Game::new();
        g.start();
        let mut dealt = vec![g.piece.unwrap().shape];
        while dealt.len() < 70 {
            assert!(g.hard_drop());
            ticks(&mut g, 2 * LOCK_TICKS);
            if g.state == State::Over {
                g.start();
                dealt.truncate(dealt.len() / 7 * 7);
            }
            dealt.push(g.piece.unwrap().shape);
            // Keep the well from filling: empty it now and then.
            if dealt.len() % 5 == 0 {
                g.well = [[0; COLS]; ROWS];
            }
        }
        dealt
    };
    let dealt = deal();
    for bag in dealt.chunks_exact(7) {
        for shape in Shape::ALL {
            assert_eq!(bag.iter().filter(|s| **s == shape).count(), 1, "{bag:?}");
        }
    }
    assert_ne!(dealt[..7], dealt[7..14], "the bags are shuffled");
    assert_eq!(deal(), dealt, "the same source, the same pieces");
    // The piece shown as next before the start is the first to fall.
    mock::reset();
    let mut g = Game::new();
    let first = g.next;
    g.start();
    assert_eq!(g.piece.unwrap().shape, first);
}

/// The piece falls a row when its time is up, faster with soft drop,
/// rests on the stack and locks after the delay, which a move restarts
/// a limited number of times.
#[test]
fn gravity_soft_drop_and_the_lock_delay() {
    let mut g = started();
    with_piece(&mut g, Shape::O);
    assert_eq!(g.gravity(), 48);
    ticks(&mut g, 47);
    assert_eq!(g.piece.unwrap().y, 0);
    g.tick();
    assert_eq!(g.piece.unwrap().y, 1);
    g.soft = true;
    ticks(&mut g, 10);
    assert_eq!(g.piece.unwrap().y, 6, "a row every second tick");
    assert_eq!(g.score, 5, "a point a row of soft drop");
    g.soft = false;

    // On the floor: it rests, then locks.
    let landing = g.landing().unwrap();
    assert_eq!(landing.y, ROWS as i8 - 2);
    g.piece = Some(landing);
    ticks(&mut g, LOCK_TICKS - 1);
    assert_eq!(g.piece, Some(landing), "still resting");
    assert_eq!(g.filled(), 0);
    // A move restarts the delay.
    assert!(g.shift(-1));
    ticks(&mut g, LOCK_TICKS - 1);
    assert_eq!(g.filled(), 0);
    g.tick();
    assert_eq!(g.filled(), 4, "locked");
    assert_eq!(g.well[19][3], Shape::O as u8 + 1);
    assert_eq!(g.piece.unwrap().y, 0, "the next piece is at the top");

    // Moves cannot keep a piece alive for ever.
    g.well = [[0; COLS]; ROWS];
    with_piece(&mut g, Shape::O);
    g.piece = g.landing();
    let mut moves = 0;
    loop {
        ticks(&mut g, LOCK_TICKS - 1);
        if g.filled() != 0 {
            break;
        }
        g.shift(if moves % 2 == 0 { 1 } else { -1 });
        moves += 1;
        assert!(moves < 100);
    }
    assert_eq!(moves, LOCK_MOVES + 1, "fifteen restarts, then it locks");
    g.well = [[0; COLS]; ROWS];

    // Walls stop a shift; hard drop lands and locks at once.
    with_piece(&mut g, Shape::I);
    for _ in 0..3 {
        assert!(g.shift(-1));
    }
    assert!(!g.shift(-1), "the wall");
    let before = g.score;
    let rows = g.landing().unwrap().y;
    assert!(g.hard_drop());
    assert_eq!(g.score, before + 2 * rows as u32, "two points a row");
    assert_eq!(g.well[ROWS - 1][..5], [1, 1, 1, 1, 0]);
}

/// Full rows flash for a moment, then go, and what was above comes down;
/// one to four lines score 100, 300, 500 and 800 times the level.
#[test]
fn full_rows_flash_collapse_and_score() {
    let mut g = started();
    fill_row(&mut g, 19, &[0]);
    fill_row(&mut g, 18, &[0]);
    g.well[17][5] = 3;
    g.piece = Some(Piece {
        shape: Shape::I,
        rot: 1,
        x: -2,
        y: 0,
    });
    assert!(g.hard_drop());
    // The I stands in column 0, rows 16..=19: rows 18 and 19 are full.
    assert_eq!(g.clearing, Some((0b11 << 18, CLEAR_TICKS)));
    assert_eq!(g.piece, None, "nothing falls while the rows flash");
    assert_eq!(g.lines, 2);
    let drop_points = 2 * 16;
    assert_eq!(g.score, 300 + drop_points);
    ticks(&mut g, CLEAR_TICKS - 1);
    assert!(g.clearing.is_some());
    g.tick();
    assert_eq!(g.clearing, None);
    assert!(g.piece.is_some());
    // The loose cell and the I's top came down two rows.
    assert_eq!(g.well[19][5], 3);
    assert_eq!((g.well[19][0], g.well[18][0]), (1, 1));
    assert_eq!(g.filled(), 3);

    // Four at once, on level 3.
    let mut g = started();
    g.lines = 2 * LEVEL_LINES + 6;
    assert_eq!(g.level(), 3);
    for row in 16..20 {
        fill_row(&mut g, row, &[9]);
    }
    g.piece = Some(Piece {
        shape: Shape::I,
        rot: 1,
        x: 7,
        y: 0,
    });
    g.score = 0;
    g.hard_drop();
    assert_eq!(g.score, 800 * 3 + 2 * 16);
    assert_eq!(g.lines, 30);
    assert_eq!(g.level(), 4, "ten lines a level");
    ticks(&mut g, CLEAR_TICKS);
    assert_eq!(g.filled(), 0);
    assert!(g.gravity() < 48, "and the pieces fall faster");
    let mut slow = started();
    slow.lines = 999;
    assert_eq!(slow.gravity(), 3, "the speed has a floor");
}

/// Hold puts the falling piece aside and brings the held one, or the
/// next; once for each piece.
#[test]
fn hold_swaps_once_for_each_piece() {
    let mut g = started();
    let first = g.piece.unwrap().shape;
    let second = g.next;
    assert!(g.hold());
    assert_eq!(g.hold, Some(first));
    assert_eq!(g.piece.unwrap().shape, second);
    assert_eq!(g.piece.unwrap().y, 0);
    assert!(!g.hold(), "not twice for the same piece");
    g.hard_drop();
    ticks(&mut g, 1);
    let third = g.piece.unwrap().shape;
    assert!(g.hold());
    assert_eq!(g.hold, Some(third));
    assert_eq!(g.piece.unwrap().shape, first, "the held piece is back");
}

/// The game ends when a new piece has no room, and starts over empty.
#[test]
fn the_game_ends_when_a_piece_has_no_room() {
    let mut g = started();
    for row in 1..ROWS {
        fill_row(&mut g, row, &[0]);
    }
    with_piece(&mut g, Shape::O);
    g.piece = Some(Piece {
        x: 4,
        ..g.piece.unwrap()
    });
    g.score = 1234;
    assert!(g.hard_drop());
    assert_eq!(g.state, State::Over);
    assert_eq!(g.piece, None);
    assert!(g.score >= 1234);
    assert!(!g.shift(1) && !g.rotate(true) && !g.hard_drop() && !g.hold());
    g.tick();
    g.start();
    assert_eq!(g.state, State::Playing);
    assert_eq!((g.score, g.lines, g.filled()), (0, 0, 0));
    // Paused, nothing moves.
    g.pause(true);
    let frozen = g.clone();
    ticks(&mut g, 500);
    assert!(!g.shift(1) && !g.hard_drop());
    assert_eq!(g, frozen);
    g.pause(false);
    assert_eq!(g.state, State::Playing);
}

// ---- the page ---------------------------------------------------------

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

fn press(p: &mut Page, code: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(0),
        repeat: false,
    })
}

fn down(p: &mut Page, k: usize) -> bool {
    let r = p.layout().keys[k];
    p.event(Event::PointerDown {
        x: r.x + r.w / 2.0,
        y: r.y + r.h / 2.0,
        button: 1,
        modifiers: Modifiers(0),
    })
}

fn up(p: &mut Page) -> bool {
    p.event(Event::PointerUp {
        x: 0.0,
        y: 0.0,
        button: 1,
        modifiers: Modifiers(0),
    })
}

fn frames(p: &mut Page, n: u32, dt: i32) {
    for _ in 0..n {
        mock::run_frame(p, dt).unwrap();
    }
}

fn sem_texts() -> Vec<String> {
    mock::with(|h| h.sem.iter().map(|n| n.text.clone()).collect())
}

/// SPEC-5.1.2, SPEC-5.2.2: the frame's time pays for whole ticks of
/// 16 ms, so the game is the same at any frame rate; a frame is asked
/// for only while the game is on (SPEC-6.2.2).
#[test]
fn spec_5_1_2_the_game_advances_in_fixed_steps() {
    let run = |dt: i32, n: u32| {
        let mut p = page(800.0, 600.0, true);
        mock::run_frame(&mut p, 0).unwrap();
        assert_eq!(
            mock::with(|h| h.frame_requests),
            0,
            "at rest before the start"
        );
        assert!(press(&mut p, key::ENTER));
        assert_eq!(p.game.state, State::Playing);
        // The first frame after the start plays no time.
        mock::run_frame(&mut p, 5000).unwrap();
        assert_eq!(p.game.piece.unwrap().y, 0);
        frames(&mut p, n, dt);
        assert_eq!(mock::with(|h| h.frame_requests), n + 1);
        p.game
    };
    // 96 ticks: 96 frames of 16 ms, 48 of 32 ms, 192 of 8 ms.
    let a = run(16, 96);
    assert_eq!(a.piece.unwrap().y, 2, "two rows at 48 ticks a row");
    assert_eq!(run(32, 48), a);
    assert_eq!(run(8, 192), a);
    // A long gap is not caught up: at most eight ticks a frame.
    let mut p = page(800.0, 600.0, true);
    press(&mut p, key::ENTER);
    frames(&mut p, 1, 0);
    frames(&mut p, 5, 60_000);
    assert_eq!(p.game.piece.unwrap().y, 0, "40 ticks, not an hour");
    frames(&mut p, 1, 16 * 8);
    assert_eq!(p.game.piece.unwrap().y, 1);
}

/// The keys play the game, and so do the on-screen keys, which repeat
/// while the pointer holds them.
#[test]
fn keys_and_on_screen_keys_play_the_game() {
    let mut p = page(800.0, 600.0, true);
    assert!(!press(&mut p, key::LEFT), "nothing moves before the start");
    assert!(press(&mut p, key::ENTER));
    let x = p.game.piece.unwrap().x;
    assert!(press(&mut p, key::LEFT));
    assert!(press(&mut p, key::RIGHT));
    assert!(press(&mut p, key::RIGHT));
    assert_eq!(p.game.piece.unwrap().x, x + 1);
    // Up and X turn clockwise, Z back; the O does not turn.
    p.game.piece = Some(Piece {
        shape: Shape::T,
        ..p.game.piece.unwrap()
    });
    assert!(press(&mut p, key::UP));
    assert!(press(&mut p, KEY_X));
    assert_eq!(p.game.piece.unwrap().rot, 2);
    assert!(press(&mut p, KEY_Z));
    assert_eq!(p.game.piece.unwrap().rot, 1);
    // Down is soft drop while it is held.
    press(&mut p, key::DOWN);
    assert!(p.game.soft);
    p.event(Event::KeyUp {
        code: key::DOWN,
        modifiers: Modifiers(0),
    });
    assert!(!p.game.soft);
    // C holds, Space drops.
    let falling = p.game.piece.unwrap().shape;
    assert!(press(&mut p, KEY_C));
    assert_eq!(p.game.hold, Some(falling));
    assert!(press(&mut p, key::SPACE));
    assert_eq!(p.game.filled(), 4);
    // An auto-repeated Space or turn does nothing.
    let repeat = Event::KeyDown {
        code: key::SPACE,
        modifiers: Modifiers(0),
        repeat: true,
    };
    frames(&mut p, 2, 16);
    assert!(!p.event(repeat));
    assert_eq!(p.game.filled(), 4);

    // The pointer: a press moves once, holding repeats after a moment.
    frames(&mut p, 1, 16);
    let x = p.game.piece.unwrap().x;
    assert!(down(&mut p, K_LEFT));
    assert_eq!(p.game.piece.unwrap().x, x - 1);
    frames(&mut p, REPEAT_AFTER, 16);
    assert_eq!(p.game.piece.unwrap().x, x - 1, "not yet");
    frames(&mut p, 2 * REPEAT_EVERY, 16);
    assert_eq!(p.game.piece.unwrap().x, x - 3, "two repeats");
    assert!(up(&mut p));
    frames(&mut p, 10, 16);
    assert_eq!(p.game.piece.unwrap().x, x - 3, "let go");
    assert!(down(&mut p, K_TURN));
    up(&mut p);
    assert!(down(&mut p, K_RIGHT));
    up(&mut p);
    down(&mut p, K_DOWN);
    assert!(p.game.soft);
    up(&mut p);
    assert!(!p.game.soft);
    assert!(down(&mut p, K_DROP));
    up(&mut p);
    assert_eq!(p.game.filled(), 8);
    // The main key pauses and goes on; P does too.
    assert!(down(&mut p, K_MAIN));
    assert_eq!(p.game.state, State::Paused);
    up(&mut p);
    assert!(!press(&mut p, key::LEFT));
    assert!(press(&mut p, KEY_P));
    assert_eq!(p.game.state, State::Playing);
    // With focus on a key, Enter presses it; Space still drops.
    p.event(Event::Focus {
        node: NodeId(ID_KEY - K_HOLD as i32),
        gained: true,
        keyboard: true,
    });
    frames(&mut p, 2, 16);
    let held = p.game.hold;
    assert!(press(&mut p, key::ENTER));
    assert_ne!(p.game.hold, held);
    assert!(press(&mut p, key::SPACE));
    assert_eq!(p.game.filled(), 12);
}

/// SPEC-A.2.1, SPEC-5.1.5: a hidden page pauses the game and asks for no
/// frame; shown again it waits for the player.
#[test]
fn a_hidden_page_pauses_the_game() {
    let mut p = page(800.0, 600.0, true);
    press(&mut p, key::ENTER);
    frames(&mut p, 3, 16);
    assert!(!p.event(Event::Visibility { visible: false }));
    assert_eq!(p.game.state, State::Paused);
    assert!(p.event(Event::Visibility { visible: true }));
    assert_eq!(p.game.state, State::Paused, "the player decides when");
    let requests = mock::with(|h| h.frame_requests);
    frames(&mut p, 3, 16);
    assert_eq!(mock::with(|h| h.frame_requests), requests);
    assert!(sem_texts().contains(&String::from("Pausat. Tryck Fortsätt eller P.")));
    press(&mut p, KEY_P);
    assert_eq!(p.game.state, State::Playing);
    assert_eq!(p.suspend(), 0);
    assert_eq!(p.game.state, State::Paused);
    // Before the start there is nothing to pause.
    let mut p = page(800.0, 600.0, true);
    p.event(Event::Visibility { visible: false });
    assert_eq!(p.game.state, State::Ready);
}

/// SPEC-10.1.1: the tree says the score, the level, the lines and the
/// state, describes the well, and has every control as a button.
#[test]
fn the_tree_states_the_game() {
    let mut p = page(800.0, 600.0, true);
    mock::run_frame(&mut p, 0).unwrap();
    let texts = sem_texts();
    for expected in [
        "Block",
        "Inte startat. Tryck Starta eller Enter.",
        "Spelplan 10 × 20, 0 rutor fyllda",
        "Håll: tom",
        "Poäng 0",
        "Nivå 1",
        "Rader 0",
        "Bästa 0",
        "Vänster",
        "Höger",
        "Rotera",
        "Ned",
        "Släpp",
        "Håll",
        "Starta",
    ] {
        assert!(texts.iter().any(|t| t == expected), "{expected}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t.starts_with("Nästa: ")));
    mock::with(|h| {
        let ids: Vec<i32> = h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect();
        assert_eq!(
            ids,
            (0..KEYS as i32).map(|i| ID_KEY - i).collect::<Vec<_>>()
        );
        assert!(h.sem.iter().any(|n| n.role == Role::Image as i32));
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
    });
    press(&mut p, key::ENTER);
    let shape = p.game.piece.unwrap().shape;
    press(&mut p, key::SPACE);
    p.game.lines = 23;
    frames(&mut p, 3, 16);
    let texts = sem_texts();
    assert!(texts.contains(&String::from("Spelet pågår.")));
    assert!(texts.contains(&String::from("Nivå 3")));
    assert!(texts.contains(&String::from("Rader 23")));
    assert!(texts.contains(&String::from("Paus")));
    let well = texts.iter().find(|t| t.starts_with("Spelplan")).unwrap();
    assert!(well.contains("4 rutor fyllda, faller: "), "{well}");
    assert!(p.game.score > 0 && texts.contains(&cat(&["Poäng ", &number(p.game.score)])));
    let _ = shape;

    // Every key is inside the window at every size, beside or below the well.
    for (w, h) in [
        (400.0, 600.0),
        (620.0, 480.0),
        (1280.0, 800.0),
        (1600.0, 1000.0),
    ] {
        let p = page(w, h, true);
        let l = p.layout();
        assert!(
            l.well.x >= 0.0 && l.side.right() <= w && l.well.bottom() <= h,
            "{w}×{h}"
        );
        for (i, r) in l.keys.iter().enumerate() {
            assert!(
                r.x >= 0.0 && r.right() <= w + 0.01 && r.bottom() <= h,
                "{w}×{h} key {i}"
            );
            assert!(r.w >= 30.0 && r.h >= 40.0, "{w}×{h} key {i}: {r:?}");
        }
        assert!(l.stats.bottom() <= l.keys[K_MAIN].y || l.narrow, "{w}×{h}");
    }
}

/// SPEC-5.2.9, SPEC-5.3.1: a finished game that beat the best score is
/// stored and comes back; without storage the page says "sparas inte".
#[test]
fn the_high_score_is_stored() {
    let end = |p: &mut Page, score: u32| {
        press(p, key::ENTER);
        for row in 1..ROWS {
            fill_row(&mut p.game, row, &[0]);
        }
        p.game.piece = Some(Piece {
            shape: Shape::O,
            rot: 0,
            x: 4,
            y: 0,
        });
        p.game.score = score;
        press(p, key::SPACE);
        assert_eq!(p.game.state, State::Over);
    };
    let mut p = page(800.0, 600.0, true);
    end(&mut p, 4200);
    assert_eq!(p.best, 4200);
    mock::with(|h| {
        assert_eq!(
            h.state.get(STORE.as_bytes()).map(Vec::as_slice),
            Some(&4200u32.to_le_bytes()[..])
        )
    });
    mock::run_frame(&mut p, 16).unwrap();
    assert!(sem_texts().contains(&String::from("Spelet är slut. Tryck Börja om eller Enter.")));
    assert!(sem_texts().contains(&String::from("Börja om")));
    assert_eq!(mock::with(|h| h.frame_requests), 0, "over: at rest");
    // A worse game leaves it; a new instance has it.
    end(&mut p, 100);
    assert_eq!(p.best, 4200);
    let again = init(800.0, 600.0);
    assert_eq!(again.best, 4200);

    let mut p = page(800.0, 600.0, false);
    end(&mut p, 900);
    assert_eq!(p.best, 900, "kept while the page lives");
    mock::run_frame(&mut p, 16).unwrap();
    assert!(sem_texts().contains(&String::from("Bästa 900 (sparas inte)")));
}
