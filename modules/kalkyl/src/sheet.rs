//! The sheet behind the page: cells, formulas, dependency-ordered
//! recalculation and number formatting. Nothing here touches the host, so
//! the same code runs in the native tests.
//!
//! Recalculation is a pass that can stop anywhere and go on later
//! ([`Sheet::step`]): a sheet is saved before it is calculated, so one
//! that took longer to calculate than a call into the page may run would
//! stop the page at every visit (granskning 0013:2). Every cell a pass
//! looks at is a unit of work, a step does about as many units as it is
//! given, and until the pass is done the cells show the values they had.
//!
//! A formula is kept as its text and evaluated straight from it, with the
//! rectangles it reads noted beside it for the walk: 256 KB of stored
//! formulas parsed into trees would be more memory than the page may have,
//! and a page out of memory stops as surely as one out of time.
//!
//! Numbers are fixed-point, a whole number of 10⁻⁸ in an `i64`: decimal
//! input is exact (`0,1 + 0,2` is `0,3`), every operation is integer
//! arithmetic, and a number is formatted by this module, so no result
//! depends on a platform's floating point or its printing (SPEC-2.1.5).

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec;
use alloc::vec::Vec;

/// Columns A to Z.
pub const COLS: usize = 26;
/// Rows 1 to 100.
pub const ROWS: usize = 100;
/// Cells of the sheet.
pub const CELLS: usize = COLS * ROWS;
/// The number 1 in fixed point: eight decimals.
pub const ONE: i64 = 100_000_000;
/// Longest source of a cell, in bytes.
pub const MAX_SRC: usize = 200;
/// Most cells one formula may read through its ranges: the sheet twice
/// over. A formula is evaluated in one go, so this is what bounds the
/// longest stretch of a recalculation that cannot stop; 200 bytes could
/// otherwise name the whole sheet 24 times. A formula that asks for more
/// is `#FEL`.
pub const MAX_RANGE_CELLS: u32 = 2 * CELLS as u32;
const FORMAT: u8 = 1;

/// Why a formula has no value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The formula does not parse, uses a text as a number, or overflows.
    Error,
    /// The formula depends on itself.
    Cycle,
    /// Division by zero.
    DivZero,
    /// A formula no recalculation has reached yet.
    Pending,
}

impl Fault {
    /// What the cell shows.
    pub fn text(self) -> &'static str {
        match self {
            Fault::Error => "#FEL",
            Fault::Cycle => "#CIRKEL",
            Fault::DivZero => "#DIV/0",
            Fault::Pending => "…",
        }
    }
}

/// What a cell holds once the sheet is calculated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Value {
    /// A number in fixed point.
    Num(i64),
    /// A text: the source itself.
    Text,
    /// A formula without a value.
    Fault(Fault),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Func {
    Sum,
    Avg,
    Min,
    Max,
    Count,
}

impl Func {
    /// The function a name means; Swedish and English, any case.
    fn named(name: &[u8]) -> Option<Self> {
        const NAMES: [(&str, Func); 8] = [
            ("SUMMA", Func::Sum),
            ("SUM", Func::Sum),
            ("MEDEL", Func::Avg),
            ("AVERAGE", Func::Avg),
            ("MIN", Func::Min),
            ("MAX", Func::Max),
            ("ANTAL", Func::Count),
            ("COUNT", Func::Count),
        ];
        NAMES
            .iter()
            .find(|(n, _)| n.as_bytes().eq_ignore_ascii_case(name))
            .map(|(_, f)| *f)
    }
}

/// One non-empty cell.
#[derive(Clone, Debug)]
pub struct Cell {
    /// What was typed.
    pub src: String,
    /// True for a source that starts with `=`.
    is_formula: bool,
    /// True once a recalculation has read the formula through: `parses`,
    /// `reads` and `cost` say what it found.
    checked: bool,
    /// True for a formula that parses; one that does not is `#FEL`.
    parses: bool,
    /// The rectangles of cells the formula reads, in the order it reads
    /// them; a single cell is a rectangle of one.
    reads: Vec<(u16, u16)>,
    /// What evaluating the formula costs, in units of work.
    cost: u32,
    /// The value after the last recalculation.
    pub value: Value,
    /// The cell's name and what it shows, `B3: 1250`: the text of its
    /// semantic node.
    pub label: String,
    shown_at: usize,
    /// Width of the text the page draws, as it last measured it; negative
    /// when it has to be measured again.
    pub width: f32,
    /// Decimals of a number that fit the cell's width; set with `width`.
    pub decimals: u8,
}

impl Cell {
    /// What the cell shows: the formatted number, the text or the fault.
    pub fn shown(&self) -> &str {
        &self.label[self.shown_at..]
    }

    /// Read a formula through without evaluating it: whether it parses,
    /// which rectangles it reads and what evaluating it will cost.
    /// Returns the work that was.
    fn check(&mut self) -> u32 {
        let body = self.src.as_bytes().get(1..).unwrap_or(&[]);
        let read_through = PARSE_COST * body.len() as u32;
        let (mut reads, mut cost) = (Vec::new(), EVAL_COST + read_through);
        let mut check = Parser {
            s: body,
            i: 0,
            from: From::Check {
                reads: &mut reads,
                cost: &mut cost,
            },
        };
        let in_ranges: u32 = {
            let parsed = check.formula().is_some();
            let cells = |&(a, b): &(u16, u16)| Rectangle::of(a, b).cells();
            self.parses = parsed;
            reads.iter().filter(|r| r.0 != r.1).map(cells).sum()
        };
        if in_ranges > MAX_RANGE_CELLS {
            self.parses = false;
        }
        if !self.parses {
            reads.clear();
            cost = EVAL_COST;
        }
        self.reads = reads;
        self.cost = cost;
        self.checked = true;
        read_through
    }

    /// Write the label again, in the string it already has. A
    /// recalculation relabels thousands of cells; a string that had to
    /// move for each would leave as many holes in the heap, and the SDK's
    /// allocator looks through the holes below a block every time it
    /// frees one.
    fn relabel(&mut self, index: usize) {
        self.label.clear();
        if let Value::Text = self.value {
            self.label.reserve_exact(self.src.len() + 8);
        }
        push_name(&mut self.label, index);
        self.label.push_str(": ");
        self.shown_at = self.label.len();
        match self.value {
            Value::Num(v) => push_decimals(&mut self.label, v, 6),
            Value::Text => self.label.push_str(&self.src),
            Value::Fault(f) => self.label.push_str(f.text()),
        }
        self.width = -1.0;
    }
}

/// Index of the cell in column `col` (0 = A) and row `row` (0 = row 1).
pub fn index(col: usize, row: usize) -> usize {
    row * COLS + col
}

/// Column and row of cell `index`.
pub fn col_row(index: usize) -> (usize, usize) {
    (index % COLS, index / COLS)
}

/// The letter of column `col`.
pub fn col_name(col: usize) -> char {
    (b'A' + col as u8) as char
}

/// A cell's name, `B3`.
pub fn name(index: usize) -> String {
    let mut s = String::new();
    push_name(&mut s, index);
    s
}

/// Append a cell's name.
fn push_name(s: &mut String, index: usize) {
    let (col, row) = col_row(index);
    s.push(col_name(col));
    push_u64(s, row as u64 + 1);
}

/// Append `v` in decimal.
pub fn push_u64(s: &mut String, mut v: u64) {
    let mut digits = [0u8; 20];
    let mut n = 0;
    loop {
        digits[n] = b'0' + (v % 10) as u8;
        n += 1;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    for d in digits[..n].iter().rev() {
        s.push(*d as char);
    }
}

/// A fixed-point number as the sheet shows it: a decimal comma, at most
/// six decimals (rounded half away from zero), no trailing zeros.
pub fn format_number(v: i64) -> String {
    format_decimals(v, 6)
}

/// [`format_number`] with at most `decimals` decimals (0 to 6), for a
/// cell too narrow for all six.
pub fn format_decimals(v: i64, decimals: u32) -> String {
    let mut s = String::new();
    push_decimals(&mut s, v, decimals);
    s
}

/// Append what [`format_decimals`] gives.
fn push_decimals(s: &mut String, v: i64, decimals: u32) {
    let decimals = decimals.min(6);
    let a = v.unsigned_abs();
    let one = ONE as u64;
    let unit = 10u64.pow(8 - decimals);
    let limit = 10u64.pow(decimals);
    let mut int = a / one;
    let mut frac = (a % one + unit / 2) / unit;
    if frac >= limit {
        int += 1;
        frac -= limit;
    }
    if v < 0 && (int != 0 || frac != 0) {
        s.push('-');
    }
    push_u64(s, int);
    if frac != 0 {
        s.push(',');
        let mut digits = [b'0'; 6];
        let digits = &mut digits[..decimals as usize];
        for d in digits.iter_mut().rev() {
            *d = b'0' + (frac % 10) as u8;
            frac /= 10;
        }
        let used = digits.iter().rposition(|d| *d != b'0').map_or(0, |i| i + 1);
        for d in &digits[..used] {
            s.push(*d as char);
        }
    }
}

/// The number a cell's text is, when it is one: digits with an optional
/// sign and an optional decimal part after a comma or a point.
pub fn parse_number(text: &str) -> Option<i64> {
    let text = text.trim();
    let (negative, digits) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text.strip_prefix('+').unwrap_or(text)),
    };
    let mut p = Parser {
        s: digits.as_bytes(),
        i: 0,
        from: From::Cells(&[]),
    };
    let v = p.number()?;
    (p.i == p.s.len()).then_some(if negative { -v } else { v })
}

/// `n / d` rounded half away from zero.
fn div_round(n: i128, d: i128) -> i128 {
    let (q, r) = (n / d, n % d);
    if 2 * r.abs() >= d.abs() {
        q + if (n < 0) == (d < 0) { 1 } else { -1 }
    } else {
        q
    }
}

fn narrow(v: i128) -> Result<i64, Fault> {
    i64::try_from(v).map_err(|_| Fault::Error)
}

/// What a part of a formula comes to: a number, or the first fault met.
type Val = Result<i64, Fault>;

/// Where a formula's cells come from.
enum From<'a> {
    /// The sheet: the formula is being evaluated.
    Cells(&'a [Option<Box<Cell>>]),
    /// Nowhere: the formula is being checked when it is put into its
    /// cell. Every cell reads as 0, and the rectangles it names are noted
    /// with what reading them will cost.
    Check {
        reads: &'a mut Vec<(u16, u16)>,
        cost: &'a mut u32,
    },
}

/// The arguments of a call so far.
struct Agg {
    sum: i64,
    n: i64,
    min: i64,
    max: i64,
    /// The first fault among them, which is then the call's value.
    fault: Option<Fault>,
}

impl Agg {
    fn take(&mut self, v: Val) {
        if self.fault.is_some() {
            return;
        }
        match v.and_then(|v| Ok((v, self.sum.checked_add(v).ok_or(Fault::Error)?))) {
            Ok((v, sum)) => {
                self.sum = sum;
                self.n += 1;
                self.min = self.min.min(v);
                self.max = self.max.max(v);
            }
            Err(f) => self.fault = Some(f),
        }
    }

    fn finish(self, func: Func) -> Val {
        if let Some(f) = self.fault {
            return Err(f);
        }
        Ok(match func {
            Func::Sum => self.sum,
            Func::Count => self.n.checked_mul(ONE).ok_or(Fault::Error)?,
            Func::Avg if self.n == 0 => return Err(Fault::DivZero),
            Func::Avg => narrow(div_round(i128::from(self.sum), i128::from(self.n)))?,
            Func::Min | Func::Max if self.n == 0 => 0,
            Func::Min => self.min,
            Func::Max => self.max,
        })
    }
}

/// `a op b` in fixed point; a fault on the left comes before one on the
/// right.
fn bin(op: u8, a: Val, b: Val) -> Val {
    let (a, b) = (a?, b?);
    match op {
        b'+' => a.checked_add(b).ok_or(Fault::Error),
        b'-' => a.checked_sub(b).ok_or(Fault::Error),
        b'*' => narrow(div_round(i128::from(a) * i128::from(b), i128::from(ONE))),
        _ if b == 0 => Err(Fault::DivZero),
        _ => narrow(div_round(i128::from(a) * i128::from(ONE), i128::from(b))),
    }
}

/// A recursive-descent parser over a formula's bytes that evaluates as it
/// goes: `None` for a source that is no formula, else its value. A source
/// is at most [`MAX_SRC`] bytes, which bounds the nesting and the
/// recursion, and nothing is built on the heap.
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    from: From<'a>,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn ws(&mut self) {
        while self.peek() == Some(b' ') {
            self.i += 1;
        }
    }

    fn eat(&mut self, b: u8) -> bool {
        self.ws();
        let hit = self.peek() == Some(b);
        if hit {
            self.i += 1;
        }
        hit
    }

    /// Digits with an optional decimal part; `None` when there is no
    /// digit or the number does not fit.
    fn number(&mut self) -> Option<i64> {
        let start = self.i;
        let mut int: i64 = 0;
        while let Some(b @ b'0'..=b'9') = self.peek() {
            int = int.checked_mul(10)?.checked_add(i64::from(b - b'0'))?;
            self.i += 1;
        }
        if self.i == start {
            return None;
        }
        let mut v = int.checked_mul(ONE)?;
        let decimal = matches!(self.peek(), Some(b'.' | b','))
            && self.s.get(self.i + 1).is_some_and(u8::is_ascii_digit);
        if decimal {
            self.i += 1;
            let mut unit = ONE / 10;
            let mut round_up = false;
            let mut first_dropped = true;
            while let Some(b @ b'0'..=b'9') = self.peek() {
                let d = i64::from(b - b'0');
                if unit > 0 {
                    v = v.checked_add(d * unit)?;
                    unit /= 10;
                } else if first_dropped {
                    round_up = d >= 5;
                    first_dropped = false;
                }
                self.i += 1;
            }
            if round_up {
                v = v.checked_add(1)?;
            }
        }
        Some(v)
    }

    /// A cell reference, `B3` in any case.
    fn cell(&mut self) -> Option<u16> {
        let col = self.peek().filter(u8::is_ascii_alphabetic)?;
        let mut i = self.i + 1;
        let mut row = 0usize;
        while let Some(b @ b'0'..=b'9') = self.s.get(i).copied() {
            row = row * 10 + usize::from(b - b'0');
            if row > ROWS {
                return None;
            }
            i += 1;
        }
        if row == 0 || i == self.i + 1 {
            return None;
        }
        self.i = i;
        Some(index(usize::from(col.to_ascii_uppercase() - b'A'), row - 1) as u16)
    }

    /// The whole formula: an expression and nothing after it.
    fn formula(&mut self) -> Option<Val> {
        let v = self.expr()?;
        self.ws();
        (self.i == self.s.len()).then_some(v)
    }

    fn expr(&mut self) -> Option<Val> {
        let mut left = self.term()?;
        loop {
            self.ws();
            let Some(op @ (b'+' | b'-')) = self.peek() else {
                return Some(left);
            };
            self.i += 1;
            left = bin(op, left, self.term()?);
        }
    }

    fn term(&mut self) -> Option<Val> {
        let mut left = self.unary()?;
        loop {
            self.ws();
            let Some(op @ (b'*' | b'/')) = self.peek() else {
                return Some(left);
            };
            self.i += 1;
            left = bin(op, left, self.unary()?);
        }
    }

    fn unary(&mut self) -> Option<Val> {
        self.ws();
        match self.peek()? {
            b'-' => {
                self.i += 1;
                let v = self.unary()?;
                Some(v.and_then(|v| v.checked_neg().ok_or(Fault::Error)))
            }
            b'+' => {
                self.i += 1;
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<Val> {
        let b = self.peek()?;
        if b == b'(' {
            self.i += 1;
            let v = self.expr()?;
            return self.eat(b')').then_some(v);
        }
        if b.is_ascii_digit() {
            return self.number().map(Ok);
        }
        let start = self.i;
        while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i > start && self.peek() == Some(b'(') {
            let func = Func::named(&self.s[start..self.i])?;
            self.i += 1;
            return self.call(func);
        }
        self.i = start;
        let cell = self.cell()?;
        Some(self.read(cell))
    }

    /// The value of the cell a formula names: an empty cell is 0, a text
    /// is no number, a fault is the formula's fault too.
    fn read(&mut self, i: u16) -> Val {
        match &mut self.from {
            From::Check { reads, cost } => {
                reads.push((i, i));
                **cost += 1;
                Ok(0)
            }
            From::Cells(cells) => match cells.get(usize::from(i)).and_then(|c| c.as_deref()) {
                None => Ok(0),
                Some(cell) => match cell.value {
                    Value::Num(v) => Ok(v),
                    Value::Text => Err(Fault::Error),
                    Value::Fault(f) => Err(f),
                },
            },
        }
    }

    /// A call, after its `(`: values and ranges separated by `;` (or `,`
    /// where it is not a decimal comma), taken in as they are read.
    fn call(&mut self, func: Func) -> Option<Val> {
        let mut agg = Agg {
            sum: 0,
            n: 0,
            min: i64::MAX,
            max: i64::MIN,
            fault: None,
        };
        if self.eat(b')') {
            return Some(agg.finish(func));
        }
        loop {
            self.ws();
            let start = self.i;
            match self.range() {
                Some((a, b)) => self.read_range(&mut agg, a, b),
                None => {
                    self.i = start;
                    let v = self.expr()?;
                    agg.take(v);
                }
            }
            if self.eat(b')') {
                return Some(agg.finish(func));
            }
            if !(self.eat(b';') || self.eat(b',')) {
                return None;
            }
        }
    }

    /// A rectangle of cells, `B4:D8`: its top-left and bottom-right.
    fn range(&mut self) -> Option<(u16, u16)> {
        let a = usize::from(self.cell()?);
        if !self.eat(b':') {
            return None;
        }
        self.ws();
        let b = usize::from(self.cell()?);
        let ((ac, ar), (bc, br)) = (col_row(a), col_row(b));
        Some((
            index(ac.min(bc), ar.min(br)) as u16,
            index(ac.max(bc), ar.max(br)) as u16,
        ))
    }

    /// The numbers of a rectangle into a call; texts and empty cells are
    /// passed over.
    fn read_range(&mut self, agg: &mut Agg, a: u16, b: u16) {
        let r = Rectangle::of(a, b);
        match &mut self.from {
            From::Check { reads, cost } => {
                reads.push((a, b));
                **cost += r.cells();
            }
            From::Cells(cells) => {
                if agg.fault.is_some() {
                    return;
                }
                // The loop every sum spends its time in: plain variables.
                let (mut sum, mut n, mut min, mut max) = (agg.sum, agg.n, agg.min, agg.max);
                'rows: for row in r.r0..r.r0 + r.h {
                    let first = index(r.c0, row);
                    let Some(line) = cells.get(first..first + r.w) else {
                        break;
                    };
                    for cell in line {
                        let Some(cell) = cell.as_deref() else {
                            continue;
                        };
                        match cell.value {
                            Value::Text => {}
                            Value::Num(v) => {
                                let Some(with) = sum.checked_add(v) else {
                                    agg.fault = Some(Fault::Error);
                                    break 'rows;
                                };
                                sum = with;
                                n += 1;
                                min = min.min(v);
                                max = max.max(v);
                            }
                            Value::Fault(f) => {
                                agg.fault = Some(f);
                                break 'rows;
                            }
                        }
                    }
                }
                (agg.sum, agg.n, agg.min, agg.max) = (sum, n, min, max);
            }
        }
    }
}

/// A range as columns and rows.
#[derive(Clone, Copy)]
struct Rectangle {
    c0: usize,
    r0: usize,
    /// Columns across.
    w: usize,
    /// Rows down.
    h: usize,
}

impl Rectangle {
    fn of(a: u16, b: u16) -> Self {
        let ((c0, r0), (c1, r1)) = (col_row(usize::from(a)), col_row(usize::from(b)));
        Self {
            c0,
            r0,
            w: c1 + 1 - c0,
            h: r1 + 1 - r0,
        }
    }

    fn cells(self) -> u32 {
        (self.w * self.h) as u32
    }

    /// The cell at `pos`, counting row by row.
    fn at(self, pos: usize) -> usize {
        index(self.c0 + pos % self.w, self.r0 + pos / self.w)
    }
}

/// Units of work the evaluation of any formula costs before its text and
/// its ranges are counted: finding it, and writing its value and label.
const EVAL_COST: u32 = 40;
/// Units of work for a byte of a formula's text, read through once when
/// the formula is new and once for every evaluation.
const PARSE_COST: u32 = 4;
/// Units of work for the walk to enter and leave a formula.
const WALK_COST: u32 = 4;

const NEW: u8 = 0;
const OPEN: u8 = 1;
const DONE: u8 = 2;
/// Set with `DONE` for a formula on a cycle.
const CYCLIC: u8 = 4;

/// A formula the walk is inside: which of its rectangles it has come to
/// and how far into it.
#[derive(Clone, Copy, Debug)]
struct Walk {
    cell: u16,
    /// Index into the cell's `reads`.
    item: u16,
    /// Position in that rectangle.
    pos: u16,
    /// The shallowest formula still open that this one, or one it reads,
    /// reads back to: at most this formula's own depth when it is on a
    /// cycle.
    reach: u16,
}

/// A recalculation under way: a depth-first walk over what the formulas
/// read, which puts them in an order where each comes after what it reads
/// and finds the cycles, then their evaluation in that order.
#[derive(Clone, Debug)]
struct Pass {
    /// `NEW`, `OPEN` or `DONE` per cell, with `CYCLIC`.
    state: Vec<u8>,
    /// The depth in `stack` of each `OPEN` cell.
    depth: Vec<u16>,
    stack: Vec<Walk>,
    /// The next cell to look for a formula not yet read through.
    unchecked: usize,
    /// The next cell a walk may start from.
    start: usize,
    order: Vec<u16>,
    /// How many of `order` have their value.
    evaluated: usize,
}

/// 26 × 100 cells, most of them empty.
#[derive(Clone, Debug)]
pub struct Sheet {
    cells: Vec<Option<Box<Cell>>>,
    /// True when a cell changed since the last pass began.
    stale: bool,
    pass: Option<Box<Pass>>,
    /// Units of work every [`Sheet::step`] so far has done.
    pub work: u64,
}

impl Default for Sheet {
    fn default() -> Self {
        Self::new()
    }
}

impl Sheet {
    /// An empty sheet.
    pub fn new() -> Self {
        Self {
            cells: vec![None; CELLS],
            stale: false,
            pass: None,
            work: 0,
        }
    }

    /// The small budget a first visit shows.
    pub fn sample() -> Self {
        const SAMPLE: [(usize, [&str; 5]); 12] = [
            (1, ["Hushållsbudget 2026", "", "", "", ""]),
            (3, ["Post", "Jan", "Feb", "Mar", "Kvartal"]),
            (4, ["Lön", "32500", "32500", "34000", "=SUMMA(B4:D4)"]),
            (5, ["Hyra", "9800", "9800", "9800", "=SUMMA(B5:D5)"]),
            (6, ["Mat", "4210,5", "3980", "4455,75", "=SUMMA(B6:D6)"]),
            (7, ["Resor", "890", "1240", "760", "=SUMMA(B7:D7)"]),
            (8, ["Nöjen", "1500", "800", "2150", "=SUMMA(B8:D8)"]),
            (
                10,
                [
                    "Utgifter",
                    "=SUMMA(B5:B8)",
                    "=SUMMA(C5:C8)",
                    "=SUMMA(D5:D8)",
                    "=SUMMA(B10:D10)",
                ],
            ),
            (11, ["Kvar", "=B4-B10", "=C4-C10", "=D4-D10", "=E4-E10"]),
            (12, ["Sparkvot", "=B11/B4", "=C11/C4", "=D11/D4", "=E11/E4"]),
            (14, ["Snitt mat", "=MEDEL(B6:D6)", "", "", ""]),
            (15, ["Dyraste resa", "=MAX(B7:D7)", "", "", ""]),
        ];
        let mut sheet = Self::new();
        for (row, cells) in SAMPLE {
            for (col, src) in cells.iter().enumerate() {
                sheet.set(index(col, row - 1), src);
            }
        }
        sheet.recalc();
        sheet
    }

    /// The cell at `index`, if it is not empty.
    pub fn get(&self, index: usize) -> Option<&Cell> {
        self.cells.get(index)?.as_deref()
    }

    /// The cell at `index` for the page to note its measured width.
    pub fn get_mut(&mut self, index: usize) -> Option<&mut Cell> {
        self.cells.get_mut(index)?.as_deref_mut()
    }

    /// What was typed into the cell; empty for an empty cell.
    pub fn src(&self, index: usize) -> &str {
        self.get(index).map_or("", |c| c.src.as_str())
    }

    /// Non-empty cells.
    pub fn count(&self) -> usize {
        self.cells.iter().flatten().count()
    }

    /// Indices of the non-empty cells, in order.
    pub fn filled(&self) -> impl Iterator<Item = usize> + '_ {
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|_| i))
    }

    /// Put `src` into a cell: empty clears it, a leading `=` makes it a
    /// formula, a number is a number and anything else a text. The values
    /// are stale until the recalculation this starts is done
    /// ([`Sheet::step`]); a new formula shows `…` until then.
    pub fn set(&mut self, index: usize, src: &str) {
        let src = src.trim();
        let src = &src[..src.floor_char_boundary(MAX_SRC)];
        self.stale = true;
        if src.is_empty() || index >= CELLS {
            if let Some(cell) = self.cells.get_mut(index) {
                *cell = None;
            }
            return;
        }
        let (is_formula, value) = if src.starts_with('=') {
            (true, Value::Fault(Fault::Pending))
        } else {
            (false, parse_number(src).map_or(Value::Text, Value::Num))
        };
        let mut cell = Cell {
            src: String::from(src),
            is_formula,
            // A formula is read through by the recalculation, a step at a
            // time: a stored sheet may hold 256 KB of them.
            checked: false,
            parses: false,
            reads: Vec::new(),
            cost: EVAL_COST,
            value,
            // Room for a formula's name and any value it usually gets
            // (`Z100: #CIRKEL`), so that relabelling does not move it.
            label: String::with_capacity(if is_formula { 16 } else { 0 }),
            shown_at: 0,
            width: -1.0,
            decimals: 6,
        };
        cell.relabel(index);
        self.cells[index] = Some(Box::new(cell));
    }

    /// True while the cells may show values from before the last change:
    /// a recalculation is due or under way.
    pub fn calculating(&self) -> bool {
        self.stale || self.pass.is_some()
    }

    /// Calculate every formula now, however long it takes: for a sheet
    /// known to be small, and for tests. The page calls [`Sheet::step`].
    pub fn recalc(&mut self) {
        while !self.step(u32::MAX) {}
    }

    /// Carry the recalculation on by about `budget` units of work; true
    /// when every formula has its value.
    ///
    /// A pass first reads through the formulas that are new, then walks
    /// what the formulas read, depth first, with its own stack (a chain through 2 600 cells must not overflow the
    /// module's): that puts each formula after the formulas it reads and
    /// finds the cycles. Then it evaluates them in that order. A formula
    /// on a cycle is `#CIRKEL`, and so is one that reads it. Every cell
    /// the walk looks at, every formula it enters or leaves and every
    /// part and range cell of a formula it evaluates is a unit of work,
    /// so a step costs in proportion to its budget whatever the sheet is;
    /// only the evaluation of one formula is not interrupted (at most the
    /// ranges 200 bytes of source can name: 24 times the sheet). A change
    /// to the sheet starts the pass over.
    pub fn step(&mut self, budget: u32) -> bool {
        if self.stale {
            self.stale = false;
            self.pass = Some(Box::new(Pass {
                state: vec![NEW; CELLS],
                depth: vec![0; CELLS],
                stack: Vec::new(),
                unchecked: 0,
                start: 0,
                order: Vec::new(),
                evaluated: 0,
            }));
        }
        let Some(mut pass) = self.pass.take() else {
            return true;
        };
        let mut work = 0u32;
        let done = self.check(&mut pass, budget, &mut work)
            && self.walk(&mut pass, budget, &mut work)
            && self.evaluate(&mut pass, budget, &mut work);
        self.work += u64::from(work);
        if !done {
            self.pass = Some(pass);
        }
        done
    }

    /// The start of a pass: every formula put into its cell since the
    /// last one is read through; true when none is left.
    fn check(&mut self, pass: &mut Pass, budget: u32, work: &mut u32) -> bool {
        while pass.unchecked < CELLS {
            if *work >= budget {
                return false;
            }
            *work += 1;
            if let Some(cell) = self.cells[pass.unchecked].as_deref_mut() {
                if cell.is_formula && !cell.checked {
                    *work += cell.check();
                }
            }
            pass.unchecked += 1;
        }
        true
    }

    /// The walk of a pass; true when every formula is in `order`.
    fn walk(&self, pass: &mut Pass, budget: u32, work: &mut u32) -> bool {
        let formula_at = |i: usize| self.cells[i].as_deref().filter(|c| c.is_formula);
        loop {
            if *work >= budget {
                return false;
            }
            let depth = pass.stack.len();
            let Some(top) = pass.stack.last_mut() else {
                // The next formula no walk has reached.
                while pass.start < CELLS
                    && (pass.state[pass.start] != NEW || formula_at(pass.start).is_none())
                {
                    pass.start += 1;
                    *work += 1;
                    if *work >= budget {
                        return false;
                    }
                }
                if pass.start == CELLS {
                    return true;
                }
                pass.state[pass.start] = OPEN;
                pass.depth[pass.start] = 0;
                pass.stack.push(Walk {
                    cell: pass.start as u16,
                    item: 0,
                    pos: 0,
                    reach: u16::MAX,
                });
                *work += WALK_COST;
                continue;
            };
            let own = depth - 1;
            let reads = formula_at(usize::from(top.cell)).map_or(&[][..], |c| &c.reads);
            let Some(&(a, b)) = reads.get(usize::from(top.item)) else {
                // Everything it reads is in order: so is the formula.
                let cyclic = usize::from(top.reach) <= own;
                let (cell, reach) = (usize::from(top.cell), top.reach);
                pass.state[cell] = if cyclic { DONE | CYCLIC } else { DONE };
                pass.order.push(cell as u16);
                pass.stack.pop();
                if let Some(parent) = pass.stack.last_mut() {
                    parent.reach = parent.reach.min(reach);
                }
                continue;
            };
            // On through the rectangle to the next formula in it.
            let r = Rectangle::of(a, b);
            let mut entered = None;
            while u32::from(top.pos) < r.cells() && *work < budget {
                let i = r.at(usize::from(top.pos));
                top.pos += 1;
                *work += 1;
                if formula_at(i).is_none() {
                    continue;
                }
                match pass.state[i] {
                    NEW => {
                        entered = Some(i);
                        break;
                    }
                    // It reads a formula that is still open: both, and
                    // everything between them on the stack, are on a cycle.
                    OPEN => top.reach = top.reach.min(pass.depth[i]),
                    _ => {}
                }
            }
            if u32::from(top.pos) >= r.cells() {
                top.item += 1;
                top.pos = 0;
            }
            if let Some(i) = entered {
                *work += WALK_COST;
                pass.state[i] = OPEN;
                pass.depth[i] = depth as u16;
                pass.stack.push(Walk {
                    cell: i as u16,
                    item: 0,
                    pos: 0,
                    reach: u16::MAX,
                });
            }
        }
    }

    /// The end of a pass; true when every formula has its value.
    fn evaluate(&mut self, pass: &mut Pass, budget: u32, work: &mut u32) -> bool {
        while let Some(&i) = pass.order.get(pass.evaluated) {
            let index = usize::from(i);
            let cyclic = pass.state[index] & CYCLIC != 0;
            // A formula on a cycle is not evaluated, only labelled.
            let cost = match self.cells[index].as_deref() {
                Some(cell) if !cyclic => cell.cost,
                _ => EVAL_COST,
            };
            // One formula is never cut short, but it does not begin in a
            // step that has no room for it unless the step is empty.
            if *work > 0 && work.saturating_add(cost) > budget {
                return false;
            }
            *work = work.saturating_add(cost);
            pass.evaluated += 1;
            let value = match self.cells[index].as_deref() {
                _ if cyclic => Value::Fault(Fault::Cycle),
                Some(cell) if cell.parses => {
                    let mut eval = Parser {
                        s: &cell.src.as_bytes()[1..],
                        i: 0,
                        from: From::Cells(&self.cells),
                    };
                    match eval.formula() {
                        Some(Ok(v)) => Value::Num(v),
                        Some(Err(f)) => Value::Fault(f),
                        None => Value::Fault(Fault::Error),
                    }
                }
                _ => Value::Fault(Fault::Error),
            };
            if let Some(cell) = self.cells[index].as_mut() {
                if cell.value != value {
                    cell.value = value;
                    cell.relabel(index);
                }
            }
        }
        true
    }

    /// The stored form: a format byte, then per non-empty cell its index
    /// (two bytes), the length of its source (one byte) and the source.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = vec![FORMAT];
        for (i, cell) in self.cells.iter().enumerate() {
            if let Some(cell) = cell {
                out.extend_from_slice(&(i as u16).to_le_bytes());
                out.push(cell.src.len() as u8);
                out.extend_from_slice(cell.src.as_bytes());
            }
        }
        out
    }

    /// The sheet a stored value describes, its formulas waiting for a
    /// recalculation ([`Sheet::step`]): reading a sheet costs in proportion
    /// to its bytes, whatever its formulas are. `None` for anything else:
    /// the cells come in the order [`Sheet::encode`] writes them, each
    /// once, so a value of any length sets at most every cell.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&format, mut rest) = bytes.split_first()?;
        if format != FORMAT {
            return None;
        }
        let mut sheet = Self::new();
        let mut next = 0;
        while !rest.is_empty() {
            let (head, tail) = rest.split_at_checked(3)?;
            let i = usize::from(u16::from_le_bytes([head[0], head[1]]));
            let (src, tail) = tail.split_at_checked(usize::from(head[2]))?;
            if i >= CELLS || i < next {
                return None;
            }
            next = i + 1;
            sheet.set(i, core::str::from_utf8(src).ok()?);
            rest = tail;
        }
        Some(sheet)
    }
}
