//! The rules of the game: a well of 10 × 20 cells, seven shapes of four
//! cells, rotation with wall kicks, a seven-piece bag, hold, soft and
//! hard drop, line clears, levels and score.
//!
//! Time is counted in ticks of [`TICK_MS`]; the page calls [`Game::tick`]
//! once for every tick that has passed, so the game is the same for the
//! same events at the same ticks whatever the frame rate
//! (SPEC-2.1.5). The only chance is the order of the pieces, drawn from
//! the host's `random` (SPEC-5.2.11).

use vy_sdk::random;

/// Columns of the well.
pub const COLS: usize = 10;
/// Rows of the well.
pub const ROWS: usize = 20;
/// Milliseconds a tick: sixty a second.
pub const TICK_MS: i32 = 16;
/// Ticks a piece may rest on the stack before it locks.
pub const LOCK_TICKS: u32 = 30;
/// Moves that restart the lock delay before the piece locks anyway.
pub const LOCK_MOVES: u32 = 15;
/// Ticks the cleared rows flash before they go.
pub const CLEAR_TICKS: u32 = 18;
/// Lines a level.
pub const LEVEL_LINES: u32 = 10;
/// Ticks a row falls at each level; the last entry goes on.
const GRAVITY: [u32; 15] = [48, 40, 33, 27, 22, 18, 15, 12, 10, 8, 7, 6, 5, 4, 3];
/// Points for one to four lines at once, times the level.
const POINTS: [u32; 5] = [0, 100, 300, 500, 800];

/// The seven shapes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shape {
    /// Four in a row.
    I,
    /// The square.
    O,
    /// The T.
    T,
    /// The S.
    S,
    /// The Z.
    Z,
    /// The J.
    J,
    /// The L.
    L,
}

impl Shape {
    /// All seven, in bag order.
    pub const ALL: [Shape; 7] = [
        Shape::I,
        Shape::O,
        Shape::T,
        Shape::S,
        Shape::Z,
        Shape::J,
        Shape::L,
    ];

    /// The shape's letter.
    pub fn name(self) -> &'static str {
        ["I", "O", "T", "S", "Z", "J", "L"][self as usize]
    }

    /// The four cells of the shape in rotation `rot` (0 as spawned, then
    /// clockwise), as columns and rows inside its box.
    pub fn cells(self, rot: u8) -> [(i8, i8); 4] {
        CELLS[self as usize][usize::from(rot & 3)]
    }
}

/// The cells of each shape in each rotation, the standard layouts: I and
/// O in a box of four, the others in a box of three.
const CELLS: [[[(i8, i8); 4]; 4]; 7] = [
    [
        [(0, 1), (1, 1), (2, 1), (3, 1)],
        [(2, 0), (2, 1), (2, 2), (2, 3)],
        [(0, 2), (1, 2), (2, 2), (3, 2)],
        [(1, 0), (1, 1), (1, 2), (1, 3)],
    ],
    [[(1, 0), (2, 0), (1, 1), (2, 1)]; 4],
    [
        [(1, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (1, 1), (2, 1), (1, 2)],
        [(0, 1), (1, 1), (2, 1), (1, 2)],
        [(1, 0), (0, 1), (1, 1), (1, 2)],
    ],
    [
        [(1, 0), (2, 0), (0, 1), (1, 1)],
        [(1, 0), (1, 1), (2, 1), (2, 2)],
        [(1, 1), (2, 1), (0, 2), (1, 2)],
        [(0, 0), (0, 1), (1, 1), (1, 2)],
    ],
    [
        [(0, 0), (1, 0), (1, 1), (2, 1)],
        [(2, 0), (1, 1), (2, 1), (1, 2)],
        [(0, 1), (1, 1), (1, 2), (2, 2)],
        [(1, 0), (0, 1), (1, 1), (0, 2)],
    ],
    [
        [(0, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (2, 0), (1, 1), (1, 2)],
        [(0, 1), (1, 1), (2, 1), (2, 2)],
        [(1, 0), (1, 1), (0, 2), (1, 2)],
    ],
    [
        [(2, 0), (0, 1), (1, 1), (2, 1)],
        [(1, 0), (1, 1), (1, 2), (2, 2)],
        [(0, 1), (1, 1), (2, 1), (0, 2)],
        [(0, 0), (1, 0), (1, 1), (1, 2)],
    ],
];

/// Where a rotation may shift the piece when it does not fit in place,
/// tried in order: for a turn from rotation `r` clockwise `KICKS[r]`,
/// anticlockwise from `r` the same shifts of the turn back, negated.
/// Columns to the right and rows down are positive.
const KICKS: [[(i8, i8); 5]; 4] = [
    [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
    [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)],
    [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
    [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)],
];
/// The same for the I, which turns around the middle of its box of four.
const KICKS_I: [[(i8, i8); 5]; 4] = [
    [(0, 0), (-2, 0), (1, 0), (-2, 1), (1, -2)],
    [(0, 0), (-1, 0), (2, 0), (-1, -2), (2, 1)],
    [(0, 0), (2, 0), (-1, 0), (2, -1), (-1, 2)],
    [(0, 0), (1, 0), (-2, 0), (1, 2), (-2, -1)],
];

/// The falling piece: its shape, rotation and the corner of its box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Piece {
    /// The shape.
    pub shape: Shape,
    /// 0 as spawned, then clockwise.
    pub rot: u8,
    /// Column of the box's left edge; may be outside the well.
    pub x: i8,
    /// Row of the box's top edge.
    pub y: i8,
}

impl Piece {
    /// The four cells in the well: column, row.
    pub fn cells(&self) -> [(i8, i8); 4] {
        self.shape
            .cells(self.rot)
            .map(|(cx, cy)| (self.x + cx, self.y + cy))
    }
}

/// What the game is doing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    /// Before the first piece.
    Ready,
    /// Pieces fall.
    Playing,
    /// Stopped by the player or because the page was hidden.
    Paused,
    /// A new piece had no room.
    Over,
}

/// A game.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Game {
    /// The locked cells: 0 empty, else the shape's number plus one.
    pub well: [[u8; COLS]; ROWS],
    /// The falling piece; none while rows flash or before the start.
    pub piece: Option<Piece>,
    /// The piece after this one.
    pub next: Shape,
    /// The piece put aside.
    pub hold: Option<Shape>,
    /// True once hold was used for the falling piece.
    pub held: bool,
    /// Points.
    pub score: u32,
    /// Lines cleared.
    pub lines: u32,
    /// What the game is doing.
    pub state: State,
    /// Rows that are flashing before they go (a bit a row) and the ticks
    /// left.
    pub clearing: Option<(u32, u32)>,
    /// True while soft drop is held.
    pub soft: bool,
    bag: [Shape; 7],
    /// Pieces taken from `bag`.
    drawn: usize,
    /// Ticks since the piece last fell a row.
    fall: u32,
    /// Ticks the piece has rested, and the moves that restarted the count.
    rest: u32,
    moves: u32,
}

impl Default for Game {
    fn default() -> Self {
        Self::new()
    }
}

impl Game {
    /// A game before its start: an empty well and the first bag shuffled.
    pub fn new() -> Self {
        let mut game = Self {
            well: [[0; COLS]; ROWS],
            piece: None,
            next: Shape::I,
            hold: None,
            held: false,
            score: 0,
            lines: 0,
            state: State::Ready,
            clearing: None,
            soft: false,
            bag: Shape::ALL,
            drawn: 7,
            fall: 0,
            rest: 0,
            moves: 0,
        };
        game.next = game.draw();
        game
    }

    /// The level: one more for every ten lines.
    pub fn level(&self) -> u32 {
        1 + self.lines / LEVEL_LINES
    }

    /// Ticks a row at the current level.
    pub fn gravity(&self) -> u32 {
        GRAVITY[(self.level() as usize - 1).min(GRAVITY.len() - 1)]
    }

    /// The next piece of the bag; a spent bag is shuffled anew with seven
    /// bytes from the host (Fisher–Yates), so every seven pieces hold
    /// each shape once.
    fn draw(&mut self) -> Shape {
        if self.drawn >= 7 {
            let mut bytes = [0u8; 8];
            let _ = random(&mut bytes);
            self.bag = Shape::ALL;
            for i in (1..7).rev() {
                self.bag.swap(i, usize::from(bytes[i]) % (i + 1));
            }
            self.drawn = 0;
        }
        self.drawn += 1;
        self.bag[self.drawn - 1]
    }

    /// True when every cell of `p` is inside the well and free. Rows
    /// above the well count as free.
    pub fn fits(&self, p: &Piece) -> bool {
        p.cells().iter().all(|&(x, y)| {
            (0..COLS as i8).contains(&x)
                && y < ROWS as i8
                && (y < 0 || self.well[y as usize][x as usize] == 0)
        })
    }

    /// Put `shape` at the top of the well; the game is over when it has
    /// no room.
    fn spawn(&mut self, shape: Shape) {
        let piece = Piece {
            shape,
            rot: 0,
            x: 3,
            y: 0,
        };
        self.fall = 0;
        self.rest = 0;
        self.moves = 0;
        if self.fits(&piece) {
            self.piece = Some(piece);
        } else {
            self.piece = None;
            self.state = State::Over;
        }
    }

    fn spawn_next(&mut self) {
        let shape = self.next;
        self.next = self.draw();
        self.held = false;
        self.spawn(shape);
    }

    /// Start, or start over after the end with a new bag.
    pub fn start(&mut self) {
        if self.state != State::Ready {
            *self = Self::new();
        }
        self.state = State::Playing;
        self.spawn_next();
    }

    /// Pause or go on; nothing before the start or after the end.
    pub fn pause(&mut self, paused: bool) {
        self.state = match (self.state, paused) {
            (State::Playing, true) => State::Paused,
            (State::Paused, false) => State::Playing,
            (state, _) => state,
        };
        self.soft = false;
    }

    /// A move or turn succeeded: a resting piece gets its delay again, a
    /// limited number of times.
    fn moved(&mut self, piece: Piece) {
        self.piece = Some(piece);
        if self.rest > 0 && self.moves < LOCK_MOVES {
            self.rest = 0;
            self.moves += 1;
        }
    }

    /// The falling piece while the game is on.
    fn active(&self) -> Option<Piece> {
        self.piece.filter(|_| self.state == State::Playing)
    }

    /// One column left (`dx` −1) or right (+1); true when the piece moved.
    pub fn shift(&mut self, dx: i8) -> bool {
        let Some(mut p) = self.active() else {
            return false;
        };
        p.x += dx;
        let fits = self.fits(&p);
        if fits {
            self.moved(p);
        }
        fits
    }

    /// A quarter turn, clockwise or not, shifted by the first kick that
    /// fits; true when the piece turned.
    pub fn rotate(&mut self, clockwise: bool) -> bool {
        let Some(p) = self.active() else {
            return false;
        };
        if p.shape == Shape::O {
            return false;
        }
        let rot = (if clockwise { p.rot + 1 } else { p.rot + 3 }) & 3;
        let table = if p.shape == Shape::I {
            &KICKS_I
        } else {
            &KICKS
        };
        // Anticlockwise from r undoes the clockwise turn from r − 1.
        let (row, sign) = if clockwise {
            (usize::from(p.rot), 1)
        } else {
            (usize::from(rot), -1)
        };
        for (dx, dy) in table[row] {
            let turned = Piece {
                rot,
                x: p.x + sign * dx,
                y: p.y + sign * dy,
                ..p
            };
            if self.fits(&turned) {
                self.moved(turned);
                return true;
            }
        }
        false
    }

    /// The row the piece would rest on if it fell now.
    pub fn landing(&self) -> Option<Piece> {
        let mut p = self.piece?;
        while self.fits(&Piece { y: p.y + 1, ..p }) {
            p.y += 1;
        }
        Some(p)
    }

    /// Drop the piece to where it lands and lock it: two points a row.
    pub fn hard_drop(&mut self) -> bool {
        let (Some(p), Some(to)) = (self.active(), self.landing()) else {
            return false;
        };
        self.score += 2 * (to.y - p.y) as u32;
        self.piece = Some(to);
        self.lock();
        true
    }

    /// Put the falling piece aside and take the held one, or the next;
    /// once for each piece.
    pub fn hold(&mut self) -> bool {
        let Some(p) = self.active() else {
            return false;
        };
        if self.held {
            return false;
        }
        match self.hold.replace(p.shape) {
            Some(shape) => self.spawn(shape),
            None => self.spawn_next(),
        }
        self.held = true;
        true
    }

    /// The piece becomes part of the well; full rows start to flash.
    fn lock(&mut self) {
        let Some(p) = self.piece.take() else {
            return;
        };
        for (x, y) in p.cells() {
            if y >= 0 {
                self.well[y as usize][x as usize] = p.shape as u8 + 1;
            }
        }
        let full = (0..ROWS)
            .filter(|&r| self.well[r].iter().all(|c| *c != 0))
            .fold(0u32, |mask, r| mask | 1 << r);
        if full == 0 {
            self.spawn_next();
            return;
        }
        let n = full.count_ones();
        self.score += POINTS[n as usize] * self.level();
        self.lines += n;
        self.clearing = Some((full, CLEAR_TICKS));
    }

    /// Take the flashing rows out and let what is above fall.
    fn collapse(&mut self, full: u32) {
        let mut to = ROWS;
        for from in (0..ROWS).rev() {
            if full & 1 << from == 0 {
                to -= 1;
                self.well[to] = self.well[from];
            }
        }
        for row in &mut self.well[..to] {
            *row = [0; COLS];
        }
    }

    /// One tick of the game: rows flash and go, the piece falls a row
    /// when its time is up, and locks when it has rested long enough.
    pub fn tick(&mut self) {
        if self.state != State::Playing {
            return;
        }
        if let Some((full, left)) = self.clearing {
            if left > 1 {
                self.clearing = Some((full, left - 1));
            } else {
                self.clearing = None;
                self.collapse(full);
                self.spawn_next();
            }
            return;
        }
        let Some(p) = self.piece else {
            return;
        };
        let below = Piece { y: p.y + 1, ..p };
        if !self.fits(&below) {
            self.rest += 1;
            if self.rest >= LOCK_TICKS {
                self.lock();
            }
            return;
        }
        self.rest = 0;
        self.fall += 1;
        let rate = if self.soft {
            self.gravity().min(2)
        } else {
            self.gravity()
        };
        if self.fall >= rate {
            self.fall = 0;
            self.piece = Some(below);
            if self.soft {
                self.score += 1;
            }
        }
    }

    /// Locked cells in the well.
    pub fn filled(&self) -> usize {
        self.well.iter().flatten().filter(|c| **c != 0).count()
    }
}
