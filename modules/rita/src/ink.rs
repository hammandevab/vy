//! The drawing behind the page: strokes on a paper of fixed size, their
//! simplification, hit testing, stored form and the path a stroke is
//! drawn with. Plain arithmetic on `f32` and integers, so the same input
//! gives the same strokes and the same path bytes everywhere
//! (SPEC-2.1.5).

use alloc::vec;
use alloc::vec::Vec;
use vy_sdk::Path;

/// The paper, in its own units; the page scales it to the window.
pub const PAPER_W: f32 = 1200.0;
/// Height of the paper.
pub const PAPER_H: f32 = 800.0;
/// Stored coordinates are whole eighths of a paper unit.
pub const SUB: f32 = 8.0;
/// The three pens, in paper units.
pub const WIDTHS: [f32; 3] = [3.0, 7.0, 14.0];
/// How far a point may lie from the simplified stroke, in window pixels
/// where it was drawn: the jitter of a hand and of whole-pixel pointer
/// positions goes, and the curve rounds what is left.
pub const TOLERANCE_PX: f32 = 1.2;
/// Most points in the whole drawing. Every frame sends every stroke
/// again, 18 bytes a point, and the host charges for every byte it
/// copies and every byte it encodes: 2 500 points are about 3 ms of the
/// 8 ms a frame has (SPEC-5.3.3).
pub const MAX_POINTS: usize = 2500;
/// Most points in one stroke; a longer one is ended and a new one begun.
pub const MAX_STROKE_POINTS: usize = 500;
/// Most ink in the drawing: the sum of length × width over the strokes,
/// in square paper units. Rasterizing a stroke costs its length times
/// its width, so this bounds the frame's raster work (SPEC-5.3.3).
pub const MAX_INK: f32 = 2_000_000.0;
const FORMAT: u8 = 1;

/// A point on the paper, in paper units.
pub type Point = (f32, f32);

/// One stroke: a colour, a pen and its points in stored coordinates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stroke {
    /// Index into the page's colours.
    pub color: u8,
    /// Index into [`WIDTHS`].
    pub width: u8,
    /// The simplified points, in eighths of a paper unit.
    pub pts: Vec<(i16, i16)>,
}

impl Stroke {
    /// A stroke through `raw`, simplified to within `tolerance` paper
    /// units and rounded to stored coordinates; `None` without a point.
    pub fn new(color: u8, width: u8, raw: &[Point], tolerance: f32) -> Option<Self> {
        let mut pts: Vec<(i16, i16)> = simplify(raw, tolerance)
            .into_iter()
            .map(|(x, y)| (store(x, PAPER_W), store(y, PAPER_H)))
            .collect();
        pts.dedup();
        pts.truncate(MAX_STROKE_POINTS);
        (!pts.is_empty()).then_some(Self { color, width, pts })
    }

    /// Point `i` in paper units.
    pub fn point(&self, i: usize) -> Point {
        let (x, y) = self.pts[i];
        (f32::from(x) / SUB, f32::from(y) / SUB)
    }

    /// The pen's width in paper units.
    pub fn pen(&self) -> f32 {
        WIDTHS[usize::from(self.width) % WIDTHS.len()]
    }

    /// Length times width (a dot counts as its own square): what the
    /// stroke costs to rasterize, in square paper units.
    pub fn ink(&self) -> f32 {
        let length: f32 = (1..self.pts.len())
            .map(|i| distance(self.point(i - 1), self.point(i)))
            .sum();
        (length + self.pen()) * self.pen()
    }

    /// True when the stroke passes within `radius` of `p`.
    pub fn touches(&self, p: Point, radius: f32) -> bool {
        let reach = radius + self.pen() / 2.0;
        if self.pts.len() == 1 {
            return distance2(p, self.point(0)) <= reach * reach;
        }
        (1..self.pts.len())
            .any(|i| segment_distance2(p, self.point(i - 1), self.point(i)) <= reach * reach)
    }

    /// The path of the stroke with every point mapped by `map` (paper
    /// units to window coordinates); see [`curve`]. Empty for a single
    /// point, which the page draws as a dot.
    pub fn path(&self, map: impl Fn(Point) -> Point) -> Path {
        curve(self.pts.len(), |i| map(self.point(i)))
    }
}

/// True when the line turns by more than 45° at `b` on its way from `a`
/// to `c`: a corner the hand meant, not a bend of a curve.
fn corner(a: Point, b: Point, c: Point) -> bool {
    let (ux, uy) = (b.0 - a.0, b.1 - a.1);
    let (vx, vy) = (c.0 - b.0, c.1 - b.1);
    let dot = ux * vx + uy * vy;
    dot <= 0.0 || dot * dot < 0.5 * (ux * ux + uy * uy) * (vx * vx + vy * vy)
}

/// A path through `n` points that is smooth where the points bend and
/// sharp where they turn: a quadratic curve around each inner point to
/// the midpoint after it, which rounds what the simplification left,
/// but a straight line to a point that is a [`corner`], so a drawn
/// square keeps its corners.
pub fn curve(n: usize, at: impl Fn(usize) -> Point) -> Path {
    let mut path = Path::new();
    if n < 2 {
        return path;
    }
    let (x, y) = at(0);
    path.move_to(x, y);
    let mut before = (x, y);
    let mut here = at(1);
    for i in 1..n - 1 {
        let after = at(i + 1);
        if corner(before, here, after) {
            path.line_to(here.0, here.1);
        } else if i + 2 == n {
            path.quad_to(here.0, here.1, after.0, after.1);
            return path;
        } else {
            let mid = ((here.0 + after.0) / 2.0, (here.1 + after.1) / 2.0);
            path.quad_to(here.0, here.1, mid.0, mid.1);
        }
        before = here;
        here = after;
    }
    path.line_to(here.0, here.1);
    path
}

fn store(v: f32, max: f32) -> i16 {
    vy_sdk::math::round(v.clamp(0.0, max) * SUB) as i16
}

fn distance2(a: Point, b: Point) -> f32 {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    dx * dx + dy * dy
}

/// The distance between two points to within 4 %, without a square root
/// (`core` has none): the longer side plus 0.41 of the shorter.
pub fn distance(a: Point, b: Point) -> f32 {
    let (dx, dy) = ((a.0 - b.0).abs(), (a.1 - b.1).abs());
    dx.max(dy) + 0.41 * dx.min(dy)
}

/// Squared distance from `p` to the segment `a`–`b`.
fn segment_distance2(p: Point, a: Point, b: Point) -> f32 {
    let (vx, vy) = (b.0 - a.0, b.1 - a.1);
    let len2 = vx * vx + vy * vy;
    if len2 == 0.0 {
        return distance2(p, a);
    }
    let t = (((p.0 - a.0) * vx + (p.1 - a.1) * vy) / len2).clamp(0.0, 1.0);
    distance2(p, (a.0 + t * vx, a.1 + t * vy))
}

/// Ramer–Douglas–Peucker: the fewest of `pts`, in order, such that no
/// dropped point lies further than `tolerance` from the line through
/// its kept neighbours. Iterative, with its own stack.
pub fn simplify(pts: &[Point], tolerance: f32) -> Vec<Point> {
    let n = pts.len();
    if n <= 2 {
        return pts.to_vec();
    }
    let mut keep = vec![false; n];
    keep[0] = true;
    keep[n - 1] = true;
    let mut spans = vec![(0usize, n - 1)];
    while let Some((a, b)) = spans.pop() {
        let mut far = 0.0f32;
        let mut at = a;
        for i in a + 1..b {
            let d = segment_distance2(pts[i], pts[a], pts[b]);
            if d > far {
                far = d;
                at = i;
            }
        }
        if far > tolerance * tolerance {
            keep[at] = true;
            spans.push((a, at));
            spans.push((at, b));
        }
    }
    pts.iter()
        .zip(&keep)
        .filter_map(|(p, k)| k.then_some(*p))
        .collect()
}

fn put_varint(out: &mut Vec<u8>, delta: i32) {
    // Zigzag, then seven bits a byte.
    let mut v = ((delta << 1) ^ (delta >> 31)) as u32;
    while v >= 0x80 {
        out.push((v & 0x7F) as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn take_varint(bytes: &mut &[u8]) -> Option<i32> {
    let mut v = 0u32;
    for shift in [0, 7, 14, 21] {
        let (&b, rest) = bytes.split_first()?;
        *bytes = rest;
        v |= u32::from(b & 0x7F) << shift;
        if b & 0x80 == 0 {
            return Some((v >> 1) as i32 ^ -((v & 1) as i32));
        }
    }
    None
}

/// The stored form: a format byte, the number of strokes, and per
/// stroke a byte of colour and pen, the number of points, the first
/// point and then each point as the difference from the one before, in
/// one to three bytes a coordinate.
pub fn encode(strokes: &[Stroke]) -> Vec<u8> {
    let mut out = vec![FORMAT];
    out.extend_from_slice(&(strokes.len() as u16).to_le_bytes());
    for s in strokes {
        out.push(s.color << 2 | s.width);
        out.extend_from_slice(&(s.pts.len() as u16).to_le_bytes());
        let mut last = (0i32, 0i32);
        for &(x, y) in &s.pts {
            let p = (i32::from(x), i32::from(y));
            put_varint(&mut out, p.0 - last.0);
            put_varint(&mut out, p.1 - last.1);
            last = p;
        }
    }
    out
}

/// The strokes a stored value describes; `None` for anything else, or
/// for a drawing over the limits.
pub fn decode(mut bytes: &[u8]) -> Option<Vec<Stroke>> {
    let u16_at = |b: &mut &[u8]| -> Option<usize> {
        let (head, rest) = b.split_at_checked(2)?;
        *b = rest;
        Some(usize::from(u16::from_le_bytes([head[0], head[1]])))
    };
    let (&format, rest) = bytes.split_first()?;
    bytes = rest;
    if format != FORMAT {
        return None;
    }
    let count = u16_at(&mut bytes)?;
    let mut strokes = Vec::new();
    let mut total = 0;
    for _ in 0..count {
        let (&tool, rest) = bytes.split_first()?;
        bytes = rest;
        let n = u16_at(&mut bytes)?;
        total += n;
        if n == 0 || n > MAX_STROKE_POINTS || total > MAX_POINTS || tool & 3 == 3 {
            return None;
        }
        let mut pts = Vec::with_capacity(n);
        let mut last = (0i32, 0i32);
        for _ in 0..n {
            last.0 += take_varint(&mut bytes)?;
            last.1 += take_varint(&mut bytes)?;
            let inside = (0..=(PAPER_W * SUB) as i32).contains(&last.0)
                && (0..=(PAPER_H * SUB) as i32).contains(&last.1);
            if !inside {
                return None;
            }
            pts.push((last.0 as i16, last.1 as i16));
        }
        strokes.push(Stroke {
            color: tool >> 2,
            width: tool & 3,
            pts,
        });
    }
    bytes.is_empty().then_some(strokes)
}
