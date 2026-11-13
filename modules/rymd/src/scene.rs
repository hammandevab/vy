//! The scene behind the page: its own sine, cosine and square root,
//! three solids, the projection with back-face culling, flat shading and
//! depth sorting, and the star field.
//!
//! Angles are whole numbers, 65 536 to the turn, and everything else is
//! `f32` addition, multiplication and division in a fixed order, which
//! WebAssembly and every native target compute alike: the same angles
//! give the same triangles, bit for bit (SPEC-2.1.5). `core` has no
//! `sin`, `cos` or `sqrt` and the SDK brings no `libm`, so they are
//! written here: a polynomial on a quarter turn, and Newton's method.

use alloc::vec::Vec;

/// A point or direction.
pub type V3 = [f32; 3];

/// Angle units in a full turn.
pub const TURN: i32 = 65_536;
const QUARTER: i32 = TURN / 4;
/// How far the solid's centre is from the eye, in the solid's units.
pub const DISTANCE: f32 = 4.0;
/// The torus: the radius of the ring and of the tube around it.
pub const TORUS_RING: f32 = 0.82;
pub const TORUS_TUBE: f32 = 0.36;
/// Most stars the field has.
pub const MAX_STARS: usize = 800;
/// Towards the light, from the solid: up, to the left and towards the
/// eye. Unit length.
const LIGHT: V3 = [-0.451_753_9, 0.702_728_4, -0.552_143_7];
/// The light every face has, and what the direct light adds.
const AMBIENT: f32 = 0.26;
const DIRECT: f32 = 0.74;

/// The sine of `a` angle units, `0..=QUARTER`: the Taylor polynomial to
/// the ninth power, off by less than 4 × 10⁻⁶.
fn sin_quarter(a: i32) -> f32 {
    let x = a as f32 * (core::f32::consts::FRAC_PI_2 / QUARTER as f32);
    let x2 = x * x;
    let tail = 1.0 / 120.0 + x2 * (-1.0 / 5040.0 + x2 * (1.0 / 362_880.0));
    x * (1.0 + x2 * (-1.0 / 6.0 + x2 * tail))
}

/// The sine of `angle` units (65 536 to the turn).
pub fn sin(angle: i32) -> f32 {
    let a = angle.rem_euclid(TURN);
    let r = a % QUARTER;
    match a / QUARTER {
        0 => sin_quarter(r),
        1 => sin_quarter(QUARTER - r),
        2 => -sin_quarter(r),
        _ => -sin_quarter(QUARTER - r),
    }
}

/// The cosine of `angle` units.
pub fn cos(angle: i32) -> f32 {
    sin(angle.wrapping_add(QUARTER))
}

/// The square root: a first guess from the bits, then four steps of
/// Newton's method.
pub fn sqrt(v: f32) -> f32 {
    if v <= 0.0 {
        return 0.0;
    }
    let mut x = f32::from_bits((v.to_bits() >> 1) + 0x1FBD_1DF5);
    for _ in 0..4 {
        x = 0.5 * (x + v / x);
    }
    x
}

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: V3, b: V3) -> V3 {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn unit(a: V3) -> V3 {
    let len = sqrt(dot(a, a));
    if len == 0.0 {
        a
    } else {
        [a[0] / len, a[1] / len, a[2] / len]
    }
}

/// One flat face: three or four corners and its outward unit normal.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Face {
    /// Indices into the solid's corners; the first `n` count.
    pub corners: [u16; 4],
    /// Three or four.
    pub n: u8,
    /// Unit normal, pointing out of the solid.
    pub normal: V3,
}

/// A solid: corners and flat faces.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Solid {
    /// The corners, around the origin.
    pub corners: Vec<V3>,
    /// The faces.
    pub faces: Vec<Face>,
}

impl Solid {
    /// Add a face whose normal points the way `out` does.
    fn face(&mut self, corners: &[u16], out: V3) {
        let at = |i: usize| self.corners[usize::from(corners[i])];
        let mut normal = unit(cross(sub(at(1), at(0)), sub(at(2), at(0))));
        if dot(normal, out) < 0.0 {
            normal = [-normal[0], -normal[1], -normal[2]];
        }
        let mut four = [0u16; 4];
        four[..corners.len()].copy_from_slice(corners);
        self.faces.push(Face {
            corners: four,
            n: corners.len() as u8,
            normal,
        });
    }

    /// The middle of a face's corners.
    fn middle(&self, corners: &[u16]) -> V3 {
        let mut m = [0.0f32; 3];
        for &c in corners {
            let p = self.corners[usize::from(c)];
            m = [m[0] + p[0], m[1] + p[1], m[2] + p[2]];
        }
        let n = corners.len() as f32;
        [m[0] / n, m[1] / n, m[2] / n]
    }

    /// Twelve corners on the unit sphere and twenty triangles.
    pub fn icosahedron() -> Self {
        const T: f32 = 1.618_034;
        const CORNERS: [V3; 12] = [
            [-1.0, T, 0.0],
            [1.0, T, 0.0],
            [-1.0, -T, 0.0],
            [1.0, -T, 0.0],
            [0.0, -1.0, T],
            [0.0, 1.0, T],
            [0.0, -1.0, -T],
            [0.0, 1.0, -T],
            [T, 0.0, -1.0],
            [T, 0.0, 1.0],
            [-T, 0.0, -1.0],
            [-T, 0.0, 1.0],
        ];
        const FACES: [[u16; 3]; 20] = [
            [0, 11, 5],
            [0, 5, 1],
            [0, 1, 7],
            [0, 7, 10],
            [0, 10, 11],
            [1, 5, 9],
            [5, 11, 4],
            [11, 10, 2],
            [10, 7, 6],
            [7, 1, 8],
            [3, 9, 4],
            [3, 4, 2],
            [3, 2, 6],
            [3, 6, 8],
            [3, 8, 9],
            [4, 9, 5],
            [2, 4, 11],
            [6, 2, 10],
            [8, 6, 7],
            [9, 8, 1],
        ];
        let mut solid = Self {
            corners: CORNERS.iter().map(|c| unit(*c)).collect(),
            faces: Vec::with_capacity(FACES.len()),
        };
        for face in FACES {
            let out = solid.middle(&face);
            solid.face(&face, out);
        }
        solid
    }

    /// Eight corners and six squares.
    pub fn cube() -> Self {
        const H: f32 = 0.62;
        const FACES: [[u16; 4]; 6] = [
            [0, 1, 3, 2],
            [4, 6, 7, 5],
            [0, 4, 5, 1],
            [2, 3, 7, 6],
            [0, 2, 6, 4],
            [1, 5, 7, 3],
        ];
        // Corner i has bit 0 for x, bit 1 for y, bit 2 for z.
        let side = |i: usize, bit: usize| if (i >> bit) & 1 == 0 { -H } else { H };
        let mut solid = Self {
            corners: (0..8)
                .map(|i| [side(i, 0), side(i, 1), side(i, 2)])
                .collect(),
            faces: Vec::with_capacity(FACES.len()),
        };
        for face in FACES {
            let out = solid.middle(&face);
            solid.face(&face, out);
        }
        solid
    }

    /// A ring of 20 × 10 quadrilaterals, its hole towards the eye.
    pub fn torus() -> Self {
        const AROUND: usize = 20;
        const TUBE: usize = 10;
        const RING: f32 = TORUS_RING;
        const THICK: f32 = TORUS_TUBE;
        let mut solid = Self {
            corners: Vec::with_capacity(AROUND * TUBE),
            faces: Vec::with_capacity(AROUND * TUBE),
        };
        let angle = |i: usize, of: usize| (i as i32 * TURN) / of as i32;
        for i in 0..AROUND {
            let (su, cu) = (sin(angle(i, AROUND)), cos(angle(i, AROUND)));
            for j in 0..TUBE {
                let (sv, cv) = (sin(angle(j, TUBE)), cos(angle(j, TUBE)));
                let r = RING + THICK * cv;
                solid.corners.push([r * cu, r * su, THICK * sv]);
            }
        }
        let corner = |i: usize, j: usize| ((i % AROUND) * TUBE + j % TUBE) as u16;
        for i in 0..AROUND {
            // The middle of the tube where this face sits: out is away
            // from it.
            let mid = (2 * i as i32 + 1) * TURN / (2 * AROUND as i32);
            let core = [RING * cos(mid), RING * sin(mid), 0.0];
            for j in 0..TUBE {
                let face = [
                    corner(i, j),
                    corner(i + 1, j),
                    corner(i + 1, j + 1),
                    corner(i, j + 1),
                ];
                let out = sub(solid.middle(&face), core);
                solid.face(&face, out);
            }
        }
        solid
    }
}

/// How the solid is seen: turned by `yaw` around the upright axis, then
/// tilted by `pitch`, and projected around `centre` with focal length
/// `focal` in pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Turn around the upright axis, in angle units.
    pub yaw: i32,
    /// Tilt towards the eye, in angle units.
    pub pitch: i32,
    /// Pixels a unit at distance one covers.
    pub focal: f32,
    /// Where the solid's centre lands in the window.
    pub centre: (f32, f32),
}

impl View {
    /// `p` turned and tilted, in the eye's space: x right, y up, z away,
    /// the solid's centre at `DISTANCE`.
    fn turn(&self, p: V3, trig: &[f32; 4]) -> V3 {
        let [sy, cy, sp, cp] = *trig;
        let x = p[0] * cy + p[2] * sy;
        let z = p[2] * cy - p[0] * sy;
        [x, p[1] * cp - z * sp, p[1] * sp + z * cp]
    }
}

/// One face ready to draw: its corners in the window and how lit it is.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Drawn {
    /// Window coordinates of the corners; the first `n` count.
    pub at: [(f32, f32); 4],
    /// Three or four.
    pub n: u8,
    /// The face's light, `AMBIENT..=1`.
    pub shade: f32,
    /// True when the face is turned towards the eye.
    pub front: bool,
    /// Distance of its middle from the eye along the line of sight.
    pub depth: f32,
}

/// Projects a solid, keeping its buffers between frames so a frame
/// allocates nothing once they have grown to the largest solid.
#[derive(Clone, Debug, Default)]
pub struct Projector {
    /// The corners in the eye's space.
    seen: Vec<V3>,
    /// The faces that are drawn, in the solid's order.
    faces: Vec<Drawn>,
    /// (depth, index into `faces`), sorted the furthest first.
    order: Vec<(i32, u16)>,
}

impl Projector {
    /// Project `solid` as `view` sees it. Faces turned away from the eye
    /// are left out (back-face culling) unless `backs` asks for them, as
    /// a wireframe does. `drawn` then gives the faces the furthest
    /// first: the painter's algorithm, nearer faces over further ones.
    pub fn project(&mut self, solid: &Solid, view: &View, backs: bool) {
        let trig = [
            sin(view.yaw),
            cos(view.yaw),
            sin(view.pitch),
            cos(view.pitch),
        ];
        self.seen.clear();
        self.seen.extend(solid.corners.iter().map(|c| {
            let p = view.turn(*c, &trig);
            [p[0], p[1], p[2] + DISTANCE]
        }));
        self.faces.clear();
        self.order.clear();
        for face in &solid.faces {
            let corners = &face.corners[..usize::from(face.n)];
            let mut middle = [0.0f32; 3];
            for &c in corners {
                let p = self.seen[usize::from(c)];
                middle = [middle[0] + p[0], middle[1] + p[1], middle[2] + p[2]];
            }
            let n = f32::from(face.n);
            let middle = [middle[0] / n, middle[1] / n, middle[2] / n];
            let normal = view.turn(face.normal, &trig);
            // Towards the eye when the normal points back along the line
            // from the eye, which is the origin, to the face.
            let front = dot(normal, middle) < 0.0;
            if !front && !backs {
                continue;
            }
            let mut at = [(0.0f32, 0.0f32); 4];
            for (slot, &c) in at.iter_mut().zip(corners) {
                let p = self.seen[usize::from(c)];
                let k = view.focal / p[2];
                *slot = (view.centre.0 + p[0] * k, view.centre.1 - p[1] * k);
            }
            // Whole numbers to sort on, so the order is a total one.
            let key = (middle[2] * 65_536.0) as i32;
            self.order.push((key, self.faces.len() as u16));
            self.faces.push(Drawn {
                at,
                n: face.n,
                shade: AMBIENT + DIRECT * dot(normal, LIGHT).max(0.0),
                front,
                depth: middle[2],
            });
        }
        // Furthest first; faces at the same depth keep the solid's order.
        self.order
            .sort_unstable_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    }

    /// The projected faces in the order to draw them.
    pub fn drawn(&self) -> impl Iterator<Item = &Drawn> + '_ {
        self.order.iter().map(|&(_, i)| &self.faces[usize::from(i)])
    }
}

/// A star: where it is at rest, as fractions of the scene's width and
/// height in 65 536ths, and how near it is (0 far to 3 near).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Star {
    /// Fraction of the width.
    pub x: u16,
    /// Fraction of the height.
    pub y: u16,
    /// 0 far to 3 near.
    pub layer: u8,
}

/// The star field: the same every time, from a fixed linear congruential
/// sequence, not the host's random, so the sky is the same on every
/// machine.
pub fn stars() -> Vec<Star> {
    let mut seed: u32 = 0x9E37_79B9;
    let mut next = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 16) as u16
    };
    (0..MAX_STARS)
        .map(|_| {
            let (x, y, depth) = (next(), next(), next());
            // More far stars than near ones.
            let layer = match depth % 10 {
                0..=3 => 0,
                4..=6 => 1,
                7..=8 => 2,
                _ => 3,
            };
            Star { x, y, layer }
        })
        .collect()
}

impl Star {
    /// Where the star is for a view, as fractions of the scene in
    /// 65 536ths. The sky turns the other way from the solid's near
    /// side, as it does for an eye that circles the solid, and near
    /// stars move more than far ones (parallax). `yaw` may be any
    /// number of turns: the place wraps around the scene.
    pub fn at(&self, yaw: i32, pitch: i32) -> (u32, u32) {
        let near = i32::from(self.layer) + 1;
        let x = i32::from(self.x).wrapping_add(yaw.wrapping_mul(near) >> 2);
        let y = i32::from(self.y).wrapping_add(pitch.wrapping_mul(near) >> 2);
        (x.rem_euclid(TURN) as u32, y.rem_euclid(TURN) as u32)
    }
}
