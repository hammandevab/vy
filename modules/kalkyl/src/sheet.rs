//! The sheet behind the page: cells, formulas, dependency-ordered
//! recalculation and number formatting. Nothing here touches the host, so
//! the same code runs in the native tests.
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
/// Cells a recalculation may visit through ranges before it gives up:
/// 2 600 formulas that each sum the whole sheet would otherwise run for
/// seconds inside one event. A million visits is a couple of hundred
/// whole-sheet sums, about 30 ms of logical time.
const WORK_MAX: u32 = 1_000_000;
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
}

impl Fault {
    /// What the cell shows.
    pub fn text(self) -> &'static str {
        match self {
            Fault::Error => "#FEL",
            Fault::Cycle => "#CIRKEL",
            Fault::DivZero => "#DIV/0",
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

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Num(i64),
    Ref(u16),
    Neg(Box<Expr>),
    Bin(u8, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Arg>),
}

#[derive(Clone, Debug, PartialEq)]
enum Arg {
    Value(Expr),
    /// A rectangle of cells: top-left and bottom-right index.
    Range(u16, u16),
}

/// One non-empty cell.
#[derive(Clone, Debug)]
pub struct Cell {
    /// What was typed.
    pub src: String,
    /// The parsed formula of a source that starts with `=` and parses.
    formula: Option<Expr>,
    is_formula: bool,
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

    fn relabel(&mut self, index: usize) {
        self.label = name(index);
        self.label.push_str(": ");
        self.shown_at = self.label.len();
        match self.value {
            Value::Num(v) => self.label.push_str(&format_number(v)),
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
    let (col, row) = col_row(index);
    let mut s = String::new();
    s.push(col_name(col));
    push_u64(&mut s, row as u64 + 1);
    s
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
    let mut s = String::new();
    if v < 0 && (int != 0 || frac != 0) {
        s.push('-');
    }
    push_u64(&mut s, int);
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
    s
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

/// A recursive-descent parser over a formula's bytes. A source is at
/// most [`MAX_SRC`] bytes, which bounds the nesting and the recursion.
struct Parser<'a> {
    s: &'a [u8],
    i: usize,
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

    fn expr(&mut self) -> Option<Expr> {
        let mut left = self.term()?;
        loop {
            self.ws();
            let Some(op @ (b'+' | b'-')) = self.peek() else {
                return Some(left);
            };
            self.i += 1;
            left = Expr::Bin(op, Box::new(left), Box::new(self.term()?));
        }
    }

    fn term(&mut self) -> Option<Expr> {
        let mut left = self.unary()?;
        loop {
            self.ws();
            let Some(op @ (b'*' | b'/')) = self.peek() else {
                return Some(left);
            };
            self.i += 1;
            left = Expr::Bin(op, Box::new(left), Box::new(self.unary()?));
        }
    }

    fn unary(&mut self) -> Option<Expr> {
        self.ws();
        match self.peek()? {
            b'-' => {
                self.i += 1;
                Some(Expr::Neg(Box::new(self.unary()?)))
            }
            b'+' => {
                self.i += 1;
                self.unary()
            }
            _ => self.primary(),
        }
    }

    fn primary(&mut self) -> Option<Expr> {
        let b = self.peek()?;
        if b == b'(' {
            self.i += 1;
            let e = self.expr()?;
            return self.eat(b')').then_some(e);
        }
        if b.is_ascii_digit() {
            return self.number().map(Expr::Num);
        }
        let start = self.i;
        while self.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
            self.i += 1;
        }
        if self.i > start && self.peek() == Some(b'(') {
            let func = Func::named(&self.s[start..self.i])?;
            self.i += 1;
            return Some(Expr::Call(func, self.args()?));
        }
        self.i = start;
        self.cell().map(Expr::Ref)
    }

    /// The arguments of a call, after its `(`: values and ranges
    /// separated by `;` (or `,` where it is not a decimal comma).
    fn args(&mut self) -> Option<Vec<Arg>> {
        let mut args = Vec::new();
        if self.eat(b')') {
            return Some(args);
        }
        loop {
            self.ws();
            let start = self.i;
            let arg = match self.range() {
                Some(range) => range,
                None => {
                    self.i = start;
                    Arg::Value(self.expr()?)
                }
            };
            args.push(arg);
            if self.eat(b')') {
                return Some(args);
            }
            if !(self.eat(b';') || self.eat(b',')) {
                return None;
            }
        }
    }

    fn range(&mut self) -> Option<Arg> {
        let a = usize::from(self.cell()?);
        if !self.eat(b':') {
            return None;
        }
        self.ws();
        let b = usize::from(self.cell()?);
        let ((ac, ar), (bc, br)) = (col_row(a), col_row(b));
        Some(Arg::Range(
            index(ac.min(bc), ar.min(br)) as u16,
            index(ac.max(bc), ar.max(br)) as u16,
        ))
    }
}

/// The formula after the `=`, or `None` when it does not parse.
fn parse_formula(body: &str) -> Option<Expr> {
    let mut p = Parser {
        s: body.as_bytes(),
        i: 0,
    };
    let e = p.expr()?;
    p.ws();
    (p.i == p.s.len()).then_some(e)
}

/// The cells of a range, row by row.
fn range_cells(a: u16, b: u16) -> impl Iterator<Item = usize> {
    let ((c0, r0), (c1, r1)) = (col_row(usize::from(a)), col_row(usize::from(b)));
    (r0..=r1).flat_map(move |r| (c0..=c1).map(move |c| index(c, r)))
}

/// 26 × 100 cells, most of them empty.
#[derive(Clone, Debug)]
pub struct Sheet {
    cells: Vec<Option<Box<Cell>>>,
    /// True when the last recalculation gave up (see [`WORK_MAX`]); the
    /// formulas it did not get to show `#FEL`.
    pub overloaded: bool,
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
            overloaded: false,
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
    /// are stale until [`Sheet::recalc`].
    pub fn set(&mut self, index: usize, src: &str) {
        let src = src.trim();
        let src = &src[..src.floor_char_boundary(MAX_SRC)];
        if src.is_empty() || index >= CELLS {
            if let Some(cell) = self.cells.get_mut(index) {
                *cell = None;
            }
            return;
        }
        let (formula, is_formula, value) = match src.strip_prefix('=') {
            Some(body) => (parse_formula(body), true, Value::Fault(Fault::Error)),
            None => (
                None,
                false,
                parse_number(src).map_or(Value::Text, Value::Num),
            ),
        };
        let mut cell = Cell {
            src: String::from(src),
            formula,
            is_formula,
            value,
            label: String::new(),
            shown_at: 0,
            width: -1.0,
            decimals: 6,
        };
        cell.relabel(index);
        self.cells[index] = Some(Box::new(cell));
    }

    fn is_formula(&self, index: usize) -> bool {
        self.get(index).is_some_and(|c| c.is_formula)
    }

    /// The formula cells `e` reads, ranges included.
    fn deps(&self, e: &Expr, out: &mut Vec<u16>, work: &mut u32) {
        match e {
            Expr::Num(_) => {}
            Expr::Ref(i) => {
                if self.is_formula(usize::from(*i)) {
                    out.push(*i);
                }
            }
            Expr::Neg(a) => self.deps(a, out, work),
            Expr::Bin(_, a, b) => {
                self.deps(a, out, work);
                self.deps(b, out, work);
            }
            Expr::Call(_, args) => {
                for arg in args {
                    match arg {
                        Arg::Value(e) => self.deps(e, out, work),
                        Arg::Range(a, b) => {
                            for i in range_cells(*a, *b) {
                                *work += 1;
                                if self.is_formula(i) {
                                    out.push(i as u16);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    fn number_at(&self, index: usize) -> Result<Option<i64>, Fault> {
        match self.get(index).map(|c| c.value) {
            None | Some(Value::Text) => Ok(None),
            Some(Value::Num(v)) => Ok(Some(v)),
            Some(Value::Fault(f)) => Err(f),
        }
    }

    fn eval(&self, e: &Expr, work: &mut u32) -> Result<i64, Fault> {
        Ok(match e {
            Expr::Num(v) => *v,
            Expr::Ref(i) => match self.get(usize::from(*i)).map(|c| c.value) {
                None => 0,
                Some(Value::Num(v)) => v,
                Some(Value::Text) => return Err(Fault::Error),
                Some(Value::Fault(f)) => return Err(f),
            },
            Expr::Neg(a) => self.eval(a, work)?.checked_neg().ok_or(Fault::Error)?,
            Expr::Bin(op, a, b) => {
                let (a, b) = (self.eval(a, work)?, self.eval(b, work)?);
                match op {
                    b'+' => a.checked_add(b).ok_or(Fault::Error)?,
                    b'-' => a.checked_sub(b).ok_or(Fault::Error)?,
                    b'*' => narrow(div_round(i128::from(a) * i128::from(b), i128::from(ONE)))?,
                    _ if b == 0 => return Err(Fault::DivZero),
                    _ => narrow(div_round(i128::from(a) * i128::from(ONE), i128::from(b)))?,
                }
            }
            Expr::Call(func, args) => {
                let (mut sum, mut n) = (0i64, 0i64);
                let (mut min, mut max) = (i64::MAX, i64::MIN);
                let mut take = |v: i64| -> Result<(), Fault> {
                    sum = sum.checked_add(v).ok_or(Fault::Error)?;
                    n += 1;
                    min = min.min(v);
                    max = max.max(v);
                    Ok(())
                };
                for arg in args {
                    match arg {
                        Arg::Value(e) => take(self.eval(e, work)?)?,
                        Arg::Range(a, b) => {
                            for i in range_cells(*a, *b) {
                                *work += 1;
                                if *work > WORK_MAX {
                                    return Err(Fault::Error);
                                }
                                if let Some(v) = self.number_at(i)? {
                                    take(v)?;
                                }
                            }
                        }
                    }
                }
                match func {
                    Func::Sum => sum,
                    Func::Count => n.checked_mul(ONE).ok_or(Fault::Error)?,
                    Func::Avg if n == 0 => return Err(Fault::DivZero),
                    Func::Avg => narrow(div_round(i128::from(sum), i128::from(n)))?,
                    Func::Min | Func::Max if n == 0 => 0,
                    Func::Min => min,
                    Func::Max => max,
                }
            }
        })
    }

    /// Calculate every formula, each after the formulas it reads: a
    /// depth-first walk over the dependencies with its own stack (a chain
    /// of 2 600 cells must not overflow the module's), which also finds
    /// the cycles. A formula on a cycle is `#CIRKEL`, and so is one that
    /// reads it.
    pub fn recalc(&mut self) {
        const NEW: u8 = 0;
        const OPEN: u8 = 1;
        const DONE: u8 = 2;
        let mut state = vec![NEW; CELLS];
        let mut cyclic = vec![false; CELLS];
        let mut order: Vec<u16> = Vec::new();
        let mut work = 0u32;
        let deps_of = |sheet: &Sheet, i: u16, work: &mut u32| {
            let mut deps = Vec::new();
            if let Some(e) = sheet.get(usize::from(i)).and_then(|c| c.formula.as_ref()) {
                sheet.deps(e, &mut deps, work);
            }
            deps
        };
        let mut stack: Vec<(u16, Vec<u16>, usize)> = Vec::new();
        for start in 0..CELLS {
            if !self.is_formula(start) || state[start] != NEW || work > WORK_MAX {
                continue;
            }
            state[start] = OPEN;
            stack.push((start as u16, deps_of(self, start as u16, &mut work), 0));
            while let Some(top) = stack.last_mut() {
                let Some(&dep) = top.1.get(top.2) else {
                    state[usize::from(top.0)] = DONE;
                    order.push(top.0);
                    stack.pop();
                    continue;
                };
                top.2 += 1;
                match state[usize::from(dep)] {
                    NEW if work <= WORK_MAX => {
                        state[usize::from(dep)] = OPEN;
                        stack.push((dep, deps_of(self, dep, &mut work), 0));
                    }
                    OPEN => {
                        // Everything from `dep` up the stack is on the cycle.
                        let from = stack.iter().position(|s| s.0 == dep).unwrap_or(0);
                        for s in &stack[from..] {
                            cyclic[usize::from(s.0)] = true;
                        }
                    }
                    _ => {}
                }
            }
        }
        self.overloaded = work > WORK_MAX;
        for &i in &order {
            let index = usize::from(i);
            let formula = self.cells[index].as_mut().and_then(|c| c.formula.take());
            let value = match &formula {
                _ if self.overloaded => Value::Fault(Fault::Error),
                _ if cyclic[index] => Value::Fault(Fault::Cycle),
                None => Value::Fault(Fault::Error),
                Some(e) => match self.eval(e, &mut work) {
                    Ok(v) => Value::Num(v),
                    Err(f) => Value::Fault(f),
                },
            };
            self.overloaded |= work > WORK_MAX;
            if let Some(cell) = self.cells[index].as_mut() {
                cell.formula = formula;
                if cell.value != value {
                    cell.value = value;
                    cell.relabel(index);
                }
            }
        }
        // Formulas the walk never reached (it gave up) have no value.
        if self.overloaded {
            for (index, cell) in self.cells.iter_mut().enumerate() {
                match cell {
                    Some(cell) if cell.is_formula && state[index] != DONE => {
                        cell.value = Value::Fault(Fault::Error);
                        cell.relabel(index);
                    }
                    _ => {}
                }
            }
        }
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

    /// The sheet a stored value describes, calculated; `None` for
    /// anything else.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let (&format, mut rest) = bytes.split_first()?;
        if format != FORMAT {
            return None;
        }
        let mut sheet = Self::new();
        while !rest.is_empty() {
            let (head, tail) = rest.split_at_checked(3)?;
            let i = usize::from(u16::from_le_bytes([head[0], head[1]]));
            let (src, tail) = tail.split_at_checked(usize::from(head[2]))?;
            if i >= CELLS {
                return None;
            }
            sheet.set(i, core::str::from_utf8(src).ok()?);
            rest = tail;
        }
        sheet.recalc();
        Some(sheet)
    }
}
