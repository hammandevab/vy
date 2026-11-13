//! Native tests: the scene (sine, cosine and square root, the solids,
//! culling, depth order, projection, the stars) and the page (the first
//! frame and its tree, determinism, rotation from `dt_ms`, the pointer,
//! the wheel and the keys, the solids' buttons, the wireframe, the
//! slider, the layouts) against the SDK's mock host.

use super::*;
use scene::{cos, sin, sqrt, DISTANCE, TORUS_RING, TORUS_TUBE};
use std::collections::BTreeMap;
use vy_sdk::mock::{self, Op};

const QUARTER: i32 = TURN / 4;

fn view(yaw: i32, pitch: i32) -> View {
    View {
        yaw,
        pitch,
        focal: 400.0,
        centre: (300.0, 200.0),
    }
}

/// The faces of `solid` in the order they are drawn.
fn project(solid: &Solid, view: &View, backs: bool) -> Vec<Drawn> {
    let mut projector = Projector::default();
    projector.project(solid, view, backs);
    projector.drawn().copied().collect()
}

fn length(v: [f32; 3]) -> f32 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

/// The middle of a face's corners.
fn middle(solid: &Solid, face: &scene::Face) -> [f32; 3] {
    let n = usize::from(face.n);
    let mut m = [0.0f32; 3];
    for &c in &face.corners[..n] {
        for (sum, part) in m.iter_mut().zip(solid.corners[usize::from(c)]) {
            *sum += part / n as f32;
        }
    }
    m
}

/// The page's own sine and cosine are those of `std` to five decimals at
/// every angle, exact in their symmetries, and periodic.
#[test]
fn sine_and_cosine_match_std() {
    for angle in (-2 * TURN..=2 * TURN).step_by(37) {
        let radians = f64::from(angle) * std::f64::consts::TAU / f64::from(TURN);
        let (s, c) = (sin(angle), cos(angle));
        assert!((f64::from(s) - radians.sin()).abs() < 1e-5, "sin {angle}");
        assert!((f64::from(c) - radians.cos()).abs() < 1e-5, "cos {angle}");
        assert!((s * s + c * c - 1.0).abs() < 1e-5, "{angle}");
        assert_eq!(sin(-angle), -s, "odd at {angle}");
        assert_eq!(cos(-angle), c, "even at {angle}");
        assert_eq!(sin(angle + TURN), s, "periodic at {angle}");
        assert_eq!(c, sin(angle + QUARTER));
        assert_eq!(sin(TURN / 2 - angle), s, "mirrored at {angle}");
    }
    assert_eq!(sin(0), 0.0);
    assert_eq!(sin(TURN / 2), 0.0);
    assert_eq!(cos(QUARTER), 0.0);
    assert!((sin(QUARTER) - 1.0).abs() < 1e-5);
    assert!((sin(TURN / 12) - 0.5).abs() < 1e-4);
    // Far outside a turn, either way, without overflow.
    assert_eq!(sin(i32::MAX), sin(i32::MAX % TURN));
    assert_eq!(cos(i32::MIN), cos(0));
}

/// The square root is that of `std` to six digits; nothing below zero.
#[test]
fn square_root_matches_std() {
    for v in [1e-6f32, 0.01, 0.5, 1.0, 2.0, 3.0, 16.0, 1234.5, 9.9e8] {
        let (ours, std) = (sqrt(v), v.sqrt());
        assert!(((ours - std) / std).abs() < 1e-6, "{v}: {ours} {std}");
    }
    assert_eq!(sqrt(0.0), 0.0);
    assert_eq!(sqrt(-4.0), 0.0);
    assert_eq!(sqrt(4.0), 2.0);
}

/// Each solid is closed (every edge belongs to exactly two faces), has
/// the corners, edges and faces of its kind, and every face's normal has
/// length one, is square to the face and points out of the solid.
#[test]
fn solids_are_closed_with_outward_unit_normals() {
    let solids = [
        (Solid::icosahedron(), 12, 30, 20, 3),
        (Solid::torus(), 200, 400, 200, 4),
        (Solid::cube(), 8, 12, 6, 4),
    ];
    for (i, (solid, corners, edges, faces, sides)) in solids.iter().enumerate() {
        assert_eq!(solid.corners.len(), *corners, "solid {i}");
        assert_eq!(solid.faces.len(), *faces, "solid {i}");
        let mut shared: BTreeMap<(u16, u16), u32> = BTreeMap::new();
        for face in &solid.faces {
            let n = usize::from(face.n);
            assert_eq!(n, *sides);
            for k in 0..n {
                let (a, b) = (face.corners[k], face.corners[(k + 1) % n]);
                assert_ne!(a, b);
                *shared.entry((a.min(b), a.max(b))).or_default() += 1;
            }
            assert!((length(face.normal) - 1.0).abs() < 1e-4, "solid {i}");
            let at = |k: usize| solid.corners[usize::from(face.corners[k])];
            for k in 1..n {
                let along: f32 = (0..3).map(|d| face.normal[d] * (at(k)[d] - at(0)[d])).sum();
                assert!(along.abs() < 0.02, "solid {i}: normal not square, {along}");
            }
            // A step along the normal leads away from the solid's
            // inside: its centre, or for the torus the tube's core.
            let m = middle(solid, face);
            let from_inside = |p: [f32; 3]| {
                if i == 1 {
                    let ring = (p[0] * p[0] + p[1] * p[1]).sqrt() - TORUS_RING;
                    (ring * ring + p[2] * p[2]).sqrt()
                } else {
                    length(p)
                }
            };
            let out = [0, 1, 2].map(|d| m[d] + 0.05 * face.normal[d]);
            assert!(from_inside(out) > from_inside(m) + 0.04, "solid {i}");
        }
        assert_eq!(shared.len(), *edges, "solid {i}");
        assert!(shared.values().all(|n| *n == 2), "solid {i} is not closed");
    }
    // The icosahedron's corners are on the unit sphere; the torus reaches
    // the ring's radius plus the tube's.
    assert!(Solid::icosahedron()
        .corners
        .iter()
        .all(|c| (length(*c) - 1.0).abs() < 1e-5));
    let reach = Solid::torus()
        .corners
        .iter()
        .fold(0.0f32, |r, c| r.max(length(*c)));
    assert!((reach - (TORUS_RING + TORUS_TUBE)).abs() < 1e-4);
}

/// Back-face culling: only faces turned towards the eye are drawn, which
/// for a cube is at most three and for the icosahedron at most half; a
/// wireframe gets all of them, each marked.
#[test]
fn culling_keeps_only_faces_towards_the_eye() {
    let (cube, ico, torus) = (Solid::cube(), Solid::icosahedron(), Solid::torus());
    let mut seen_three = false;
    for yaw in (0..TURN).step_by(1499) {
        for pitch in [-PITCH_MAX, -5000, 0, 3333, PITCH_MAX] {
            let v = view(yaw, pitch);
            let faces = project(&cube, &v, false);
            assert!((1..=3).contains(&faces.len()), "cube {yaw} {pitch}");
            assert!(faces.iter().all(|d| d.front && d.n == 4));
            seen_three |= faces.len() == 3;

            let faces = project(&ico, &v, false);
            assert!((5..=10).contains(&faces.len()), "{} faces", faces.len());
            assert!(faces.iter().all(|d| d.front && d.n == 3));

            let visible = project(&torus, &v, false).len();
            assert!((60..140).contains(&visible), "torus: {visible}");

            // With the backs every face is there, and as many are marked
            // front as were drawn without.
            let all = project(&torus, &v, true);
            assert_eq!(all.len(), 200);
            assert_eq!(all.iter().filter(|d| d.front).count(), visible);
        }
    }
    assert!(seen_three);
    // Straight on, the cube shows one face: the others are edge-on or
    // turned away, and the one is the near one.
    let front = project(&cube, &view(0, 0), false);
    assert_eq!(front.len(), 1);
    assert!(front[0].depth < DISTANCE);
}

/// The painter's order: the furthest face first, at every view, with and
/// without the backs, and the same order every time.
#[test]
fn faces_are_drawn_far_to_near() {
    for solid in [Solid::icosahedron(), Solid::torus(), Solid::cube()] {
        for yaw in (0..TURN).step_by(2731) {
            for pitch in [-9000, 0, 4000] {
                for backs in [false, true] {
                    let v = view(yaw, pitch);
                    let faces = project(&solid, &v, backs);
                    // In the whole numbers they are sorted on.
                    let key = |d: &Drawn| (d.depth * 65_536.0) as i32;
                    assert!(
                        faces.windows(2).all(|w| key(&w[0]) >= key(&w[1])),
                        "not far to near at {yaw} {pitch}"
                    );
                    assert_eq!(faces, project(&solid, &v, backs));
                }
            }
        }
    }
    // Faces at one depth keep the solid's order: straight on, a
    // wireframe cube's four side faces are level.
    let faces = project(&Solid::cube(), &view(0, 0), true);
    assert_eq!(faces.len(), 6);
    assert!(!faces[0].front && faces[5].front);
    assert!(faces[1..5].iter().all(|d| d.depth == DISTANCE));
    // A projector used again gives what a new one does.
    let mut projector = Projector::default();
    projector.project(&Solid::torus(), &view(100, 200), true);
    projector.project(&Solid::cube(), &view(0, 0), true);
    assert_eq!(projector.drawn().copied().collect::<Vec<_>>(), faces);
}

/// The projection is a perspective around the view's centre: near things
/// are larger, up is up, and the light falls from the upper left.
#[test]
fn projection_is_a_lit_perspective() {
    let cube = Solid::cube();
    let half = cube.corners[7][0];
    let v = view(0, 0);
    let faces = project(&cube, &v, true);
    let (far, near) = (faces[0], faces[5]);
    // The near face is a square around the centre, focal × half / depth
    // to a side.
    let side = v.focal * half / (DISTANCE - half);
    for (x, y) in near.at {
        assert!(((x - v.centre.0).abs() - side).abs() < 1e-3, "{x}");
        assert!(((y - v.centre.1).abs() - side).abs() < 1e-3, "{y}");
    }
    let far_side = v.focal * half / (DISTANCE + half);
    assert!(far
        .at
        .iter()
        .all(|p| ((p.0 - 300.0).abs() - far_side).abs() < 1e-3));
    assert!(far_side < side);
    assert!((near.depth - (DISTANCE - half)).abs() < 1e-4);
    assert!((far.depth - (DISTANCE + half)).abs() < 1e-4);
    assert!(faces.iter().all(|d| (0.25..=1.0).contains(&d.shade)));
    // Seen a little from above and from one side, a cube shows its top
    // and two sides. Up in the solid is up in the window, and the light
    // falls from the upper left: the top is the brightest, and the side
    // towards the left is brighter than the one towards the right.
    let mut faces = project(&cube, &view(TURN / 12, -TURN / 18), false);
    assert_eq!(faces.len(), 3);
    let mid = |d: &Drawn| {
        let sum = d.at.iter().fold((0.0, 0.0), |s, p| (s.0 + p.0, s.1 + p.1));
        (sum.0 / 4.0, sum.1 / 4.0)
    };
    faces.sort_by(|a, b| mid(a).1.total_cmp(&mid(b).1));
    let top = faces[0];
    assert!(mid(&top).1 < v.centre.1 && mid(&faces[1]).1 > mid(&top).1 + 50.0);
    let (left, right) = if mid(&faces[1]).0 < mid(&faces[2]).0 {
        (faces[1], faces[2])
    } else {
        (faces[2], faces[1])
    };
    assert!(mid(&left).0 < v.centre.0 && mid(&right).0 > v.centre.0);
    assert!(
        top.shade > left.shade && left.shade > right.shade,
        "{} {} {}",
        top.shade,
        left.shade,
        right.shade
    );
}

/// The stars are the same every time, in four depths, and shift with the
/// view: near ones more than far ones, around the scene without a jump.
#[test]
fn stars_are_fixed_and_shift_with_parallax() {
    let stars = scene::stars();
    assert_eq!(stars.len(), MAX_STARS);
    assert_eq!(stars, scene::stars());
    let per_layer = |layer: u8| stars.iter().filter(|s| s.layer == layer).count();
    assert!(per_layer(0) > per_layer(1) && per_layer(1) > per_layer(2));
    assert!(per_layer(2) > per_layer(3) && per_layer(3) > 40);
    // Spread over the scene: every sixteenth of it has some.
    let mut cells = [0u32; 16];
    for s in &stars {
        cells[usize::from(s.x >> 14) + 4 * usize::from(s.y >> 14)] += 1;
    }
    assert!(cells.iter().all(|n| *n > 20), "{cells:?}");

    for star in &stars[..40] {
        let near = i32::from(star.layer) + 1;
        assert_eq!(
            star.at(0, 0),
            (u32::from(star.x), u32::from(star.y)),
            "at rest"
        );
        let (x, y) = star.at(4000, -2000);
        let wrap = |v: i32| v.rem_euclid(TURN) as u32;
        assert_eq!(x, wrap(i32::from(star.x) + 1000 * near));
        assert_eq!(y, wrap(i32::from(star.y) - 500 * near));
        // No jump anywhere, not where a turn ends nor where the angle
        // itself wraps.
        for yaw in [TURN - 2, 4 * TURN - 2, i32::MAX - 1, i32::MIN, -3] {
            let (a, _) = star.at(yaw, 0);
            let (b, _) = star.at(yaw.wrapping_add(4), 0);
            let step = (b as i32 - a as i32).rem_euclid(TURN);
            assert_eq!(step, near, "yaw {yaw}");
        }
        // Four turns of the solid bring the sky back.
        assert_eq!(star.at(4 * TURN + 77, 5), star.at(77, 5));
    }
}

fn init(width: f32, height: f32, scale: f32) -> Page {
    Page::init(
        Window {
            width,
            height,
            scale,
        },
        0,
    )
    .unwrap()
}

fn page(width: f32, height: f32) -> Page {
    mock::reset();
    init(width, height, 1.0)
}

fn frame(p: &mut Page, dt_ms: i32) -> u32 {
    mock::run_frame(p, dt_ms).unwrap()
}

fn press(p: &mut Page, code: i32) -> bool {
    p.event(Event::KeyDown {
        code,
        modifiers: Modifiers(0),
        repeat: false,
    })
}

fn down(p: &mut Page, x: f32, y: f32) -> bool {
    p.event(Event::PointerDown {
        x,
        y,
        button: 1,
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

fn drag(p: &mut Page, x: f32, y: f32) -> bool {
    p.event(Event::PointerMove {
        x,
        y,
        buttons: 1,
        modifiers: Modifiers(0),
    })
}

/// A press and release in the middle of `r`.
fn click(p: &mut Page, r: Rect) -> bool {
    let (x, y) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let redraw = down(p, x, y);
    up(p, x, y) || redraw
}

fn wheel(p: &mut Page, dy: f32, lines: bool) -> bool {
    p.event(Event::Scroll {
        dx: 0.0,
        dy,
        lines,
        modifiers: Modifiers(0),
    })
}

fn focus(p: &mut Page, id: i32, gained: bool) -> bool {
    p.event(Event::Focus {
        node: NodeId(id),
        gained,
        keyboard: true,
    })
}

/// The tree of the last frame as (role, text).
fn tree() -> Vec<(i32, String)> {
    mock::with(|h| h.sem.iter().map(|n| (n.role, n.text.clone())).collect())
}

fn has(role: Role, text: &str) -> bool {
    tree().iter().any(|(r, t)| *r == role as i32 && t == text)
}

/// The stars of the last frame: the rectangles between the sky and the
/// solid's first path.
fn stars_drawn() -> Vec<(f32, f32, f32)> {
    mock::with(|h| {
        h.frame_ops()[1..]
            .iter()
            .map_while(|op| match op {
                Op::Rect { x, y, w, .. } => Some((*x, *y, *w)),
                _ => None,
            })
            .collect()
    })
}

/// The solid's paths of the last frame as (commands, colour, stroke
/// width): those before the title, the first text. A checkbox's tick is
/// a path too, further on.
fn paths() -> Vec<(usize, u32, f32)> {
    mock::with(|h| {
        h.frame_ops()
            .iter()
            .take_while(|op| !matches!(op, Op::Text { .. }))
            .filter_map(|op| match op {
                Op::Path {
                    bytes,
                    color,
                    stroke_w,
                } => Some((bytes.len() / 9, *color, *stroke_w)),
                _ => None,
            })
            .collect()
    })
}

/// The first frame: the sky, 300 stars, the icosahedron's visible faces
/// each filled and stroked in one colour, and the whole page in the tree
/// in Tab order. The readout counts every op, itself included. Nothing
/// is asked of the host but the next frame.
#[test]
fn first_frame_draws_the_scene_and_the_tree() {
    let mut p = page(1280.0, 800.0);
    let ops = frame(&mut p, 0);
    let l = p.layout();
    mock::with(|h| {
        assert_eq!(h.frame_ops().len() as u32, ops);
        assert!(
            matches!(h.frame_ops()[0], Op::Rect { x, y, w, h, .. } if (x, y, w, h) == (0.0, 0.0, 1280.0, 800.0))
        );
        assert!(h.refusals.is_empty(), "{:?}", h.refusals);
        assert!(h.state.is_empty() && h.links.is_empty());
        assert_eq!(h.frame_requests, 1, "rotating: the next frame is asked for");
    });
    let stars = stars_drawn();
    assert_eq!(stars.len(), 300);
    // Each starts in the scene (its last pixel at most) and is a square
    // of one, two or three pixels.
    assert!(stars.iter().all(|(x, y, side)| {
        (0.0..=l.scene.right()).contains(x)
            && (0.0..=l.scene.bottom()).contains(y)
            && [1.0, 2.0, 3.0].contains(side)
    }));
    let faces = paths();
    assert!((10..=20).contains(&faces.len()), "{}", faces.len());
    for pair in faces.chunks(2) {
        assert_eq!(pair[0].0, 4, "a triangle: move, two lines, close");
        assert_eq!((pair[0].2, pair[1].2), (0.0, SEAM_W));
        assert_eq!(pair[0].1, pair[1].1, "fill and seam in one colour");
    }
    let mut shades: Vec<u32> = faces.iter().map(|f| f.1).collect();
    shades.dedup();
    assert!(shades.len() > 3, "flat shading: faces differ");

    let ops_text = format!("Ritoperationer: {ops}");
    let expected = [
        (Role::Main, ""),
        (Role::Heading, "Rymd"),
        (Role::Image, "3D-scen: ikosaeder med 20 ytor, 300 partiklar"),
        (Role::Region, "Inställningar"),
        (Role::Text, "Form"),
        (Role::Button, "Ikosaeder (vald)"),
        (Role::Button, "Torus"),
        (Role::Button, "Kub"),
        (Role::Checkbox, "Trådmodell"),
        (Role::Checkbox, "Rotera (markerad)"),
        (Role::Text, "Partiklar"),
        (Role::Slider, "Partiklar: 300"),
    ];
    let nodes = tree();
    for (node, (role, text)) in nodes.iter().zip(expected) {
        assert_eq!(*node, (role as i32, text.to_string()));
    }
    assert_eq!(nodes.len(), expected.len() + 3);
    assert_eq!(nodes[12], (Role::Text as i32, ops_text));
    assert_eq!(nodes[13], (Role::Text as i32, "Bildtid: –".to_string()));
    assert_eq!(nodes[14], (Role::Text as i32, HINT.to_string()));
    assert!(HINT.starts_with("Dra i scenen") && !HINT.contains("  "));
    // The controls have the ids the host's focus names, in Tab order.
    let ids: Vec<i32> = mock::with(|h| h.sem.iter().filter(|n| n.id < 0).map(|n| n.id).collect());
    assert_eq!(ids, [-10, -11, -12, ID_WIRE, ID_SPIN, ID_STARS]);
    // The last ops are the readout's two lines.
    mock::with(|h| {
        let last: Vec<&Op> = h.frame_ops().iter().rev().take(2).collect();
        assert!(matches!(last[0], Op::Text { text, .. } if text == "Bildtid: –"));
        assert!(matches!(last[1], Op::Text { text, .. } if text.starts_with("Ritop")));
    });
}

/// Determinism (SPEC-2.1.5): two pages given the same events and frame
/// times draw the same ops and the same tree at every frame; another
/// frame time gives another picture.
#[test]
fn same_inputs_give_the_same_display_list() {
    let run = |dts: &[i32]| {
        let mut p = page(900.0, 640.0);
        let mut frames = Vec::new();
        for (i, dt) in dts.iter().enumerate() {
            match i {
                2 => {
                    let torus = p.layout().shapes[1];
                    click(&mut p, torus);
                }
                4 => {
                    down(&mut p, 300.0, 300.0);
                    drag(&mut p, 341.0, 277.0);
                    up(&mut p, 341.0, 277.0);
                }
                6 => {
                    wheel(&mut p, -2.0, true);
                    let wire = p.layout().wire;
                    click(&mut p, wire);
                }
                _ => {}
            }
            frame(&mut p, *dt);
            frames.push(mock::with(|h| (h.frame_ops().to_vec(), h.sem.clone())));
        }
        frames
    };
    let dts = [0, 16, 16, 33, 16, 7, 16, 16, 120, 16];
    let (a, b) = (run(&dts), run(&dts));
    assert_eq!(a, b);
    // Every frame of the run differs from the one before: it moves.
    assert!(a.windows(2).all(|w| w[0].0 != w[1].0));
    let mut other = dts;
    other[3] = 34;
    let c = run(&other);
    assert_eq!(a[..3], c[..3]);
    assert_ne!(a[3].0, c[3].0);
}

/// "Rotera" turns the solid by the time that passed, twenty degrees a
/// second, and asks for the next frame; a stall is no jump; off, the
/// page is at rest and says so.
#[test]
fn rotation_follows_frame_time_and_rests_when_off() {
    let mut p = page(800.0, 600.0);
    frame(&mut p, 0);
    assert_eq!(p.yaw, HOME_YAW, "the first frame's time is not spent");
    assert!(has(Role::Text, "Bildtid: –"));
    for n in 1..=10 {
        frame(&mut p, 16);
        assert_eq!(p.yaw, HOME_YAW + n * (16 * SPIN / 100));
        assert_eq!(mock::with(|h| h.frame_requests), n as u32 + 1);
        assert!(has(Role::Text, "Bildtid: 16 ms"));
    }
    // A second of frames is twenty degrees.
    let before = p.yaw;
    for _ in 0..10 {
        frame(&mut p, 100);
    }
    let degrees = (p.yaw - before) as f32 * 360.0 / TURN as f32;
    assert!((degrees - 20.0).abs() < 0.1, "{degrees}");
    // A long gap turns it for a tenth of a second and no more.
    let before = p.yaw;
    frame(&mut p, 5000);
    assert_eq!(p.yaw - before, MAX_STEP_MS * SPIN / 100);
    assert!(has(Role::Text, "Bildtid: 5000 ms"));
    // A time below zero turns nothing back.
    let before = p.yaw;
    frame(&mut p, -40);
    assert_eq!(p.yaw, before);

    // Off: no frame is asked for, the solid stands still, and the time
    // since the last frame is not a frame time.
    let spin = p.layout().spin;
    assert!(click(&mut p, spin));
    let requests = mock::with(|h| h.frame_requests);
    let before = p.yaw;
    frame(&mut p, 16);
    frame(&mut p, 900);
    assert_eq!(p.yaw, before);
    assert_eq!(mock::with(|h| h.frame_requests), requests);
    assert!(has(Role::Checkbox, "Rotera"));
    assert!(has(Role::Text, "Bildtid: –"));
    // On again: the first frame's time is the rest, not spent.
    assert!(click(&mut p, spin));
    frame(&mut p, 2500);
    assert_eq!(p.yaw, before);
    assert_eq!(mock::with(|h| h.frame_requests), requests + 1);
    frame(&mut p, 16);
    assert_eq!(p.yaw, before + 16 * SPIN / 100);
    // A hidden tab gets no frames; shown again the page draws one and
    // goes on from there, the time away not spent.
    assert!(!p.event(Event::Visibility { visible: false }));
    assert!(p.event(Event::Visibility { visible: true }));
    let before = p.yaw;
    frame(&mut p, 0);
    assert!(has(Role::Text, "Bildtid: –"));
    frame(&mut p, 16);
    assert_eq!(p.yaw, before + 16 * SPIN / 100);
    // Space on the focused checkbox does what the pointer does.
    focus(&mut p, ID_SPIN, true);
    assert!(press(&mut p, key::SPACE));
    assert!(!p.spin.checked);
}

/// The pointer drags the solid around, half a degree a pixel, the tilt
/// stopping at eighty degrees; while it holds the solid nothing turns by
/// itself. A press outside the scene is no drag.
#[test]
fn pointer_drag_turns_the_solid() {
    let mut p = page(1280.0, 800.0);
    frame(&mut p, 0);
    assert!(down(&mut p, 500.0, 400.0));
    assert!(drag(&mut p, 540.0, 370.0));
    // Dragged right and up: the near side follows.
    assert_eq!(p.yaw, HOME_YAW - 40 * 91);
    assert_eq!(p.pitch, HOME_PITCH + 30 * 91);
    assert!(!drag(&mut p, 540.0, 370.0), "no move, no redraw");
    // Held: frames do not turn it and ask for none.
    let requests = mock::with(|h| h.frame_requests);
    let held = p.yaw;
    frame(&mut p, 16);
    frame(&mut p, 16);
    assert_eq!(p.yaw, held);
    assert_eq!(mock::with(|h| h.frame_requests), requests);
    // The tilt stops.
    assert!(drag(&mut p, 540.0, -400.0));
    assert_eq!(p.pitch, PITCH_MAX);
    assert!(drag(&mut p, 540.0, 2000.0));
    assert_eq!(p.pitch, -PITCH_MAX);
    // Any number of turns around.
    for _ in 0..40 {
        drag(&mut p, 0.0, 2000.0);
        drag(&mut p, 1000.0, 2000.0);
    }
    assert_eq!(p.pitch, -PITCH_MAX);
    frame(&mut p, 16);
    // Released: it turns by itself again, from the next frame on.
    assert!(up(&mut p, 1000.0, 2000.0));
    assert!(p.drag.is_none());
    let released = p.yaw;
    frame(&mut p, 16);
    assert_eq!(p.yaw, released);
    frame(&mut p, 16);
    assert_eq!(p.yaw, released.wrapping_add(16 * SPIN / 100));
    assert!(!drag(&mut p, 600.0, 300.0), "not held: a move is nothing");

    // A button let go outside the window ends the drag at the next move.
    assert!(down(&mut p, 500.0, 400.0));
    let before = (p.yaw, p.pitch);
    assert!(p.event(Event::PointerMove {
        x: 700.0,
        y: 100.0,
        buttons: 0,
        modifiers: Modifiers(0),
    }));
    assert!(p.drag.is_none());
    assert_eq!((p.yaw, p.pitch), before);
    // The pointer leaving ends it too.
    down(&mut p, 500.0, 400.0);
    assert!(p.event(Event::PointerLeave));
    assert!(p.drag.is_none());
    // A press on the panel, or with another button, is no drag.
    let l = p.layout();
    down(&mut p, l.panel.x + 8.0, l.panel.bottom() - 8.0);
    assert!(p.drag.is_none());
    p.event(Event::PointerDown {
        x: 500.0,
        y: 400.0,
        button: 2,
        modifiers: Modifiers(0),
    });
    assert!(p.drag.is_none());
}

/// The wheel zooms in steps of a tenth, a line or forty pixels a step,
/// six steps either way; the solid on screen grows by the step.
#[test]
fn wheel_zooms_in_bounded_steps() {
    let mut p = page(800.0, 600.0);
    let l = p.layout();
    let base = p.view(&l).focal;
    assert_eq!(base, FOCAL * l.scene.w.min(l.scene.h));
    assert!(wheel(&mut p, -1.0, true), "away from the hand: in");
    assert_eq!(p.zoom, 1);
    assert!((p.view(&l).focal / base - ZOOM_STEP).abs() < 1e-5);
    assert!(wheel(&mut p, 2.0, true));
    assert_eq!(p.zoom, -1);
    assert!((p.view(&l).focal * ZOOM_STEP / base - 1.0).abs() < 1e-5);
    // Pixels add up to steps.
    p.zoom = 0;
    assert!(!wheel(&mut p, -15.0, false));
    assert!(!wheel(&mut p, -15.0, false));
    assert!(wheel(&mut p, -15.0, false));
    assert_eq!((p.zoom, p.wheel), (1, -5.0));
    // Bounded both ways; at the end the wheel changes nothing.
    assert!(wheel(&mut p, -100.0, true));
    assert_eq!(p.zoom, ZOOM_MAX);
    assert!(!wheel(&mut p, -3.0, true));
    assert!(wheel(&mut p, 100.0, true));
    assert_eq!(p.zoom, -ZOOM_MAX);
    assert!(!wheel(&mut p, 3.0, true));
    // The solid's faces on screen grow with the zoom: the widest path
    // of the frame is wider zoomed in.
    let width = |p: &mut Page| {
        frame(p, 0);
        mock::with(|h| {
            let xs: Vec<f32> = h
                .frame_ops()
                .iter()
                .take_while(|op| !matches!(op, Op::Text { .. }))
                .filter_map(|op| match op {
                    Op::Path { bytes, .. } => Some(bytes.clone()),
                    _ => None,
                })
                .flat_map(|b| {
                    b.chunks(9)
                        .filter(|c| c[0] < 3)
                        .map(|c| f32::from_le_bytes([c[1], c[2], c[3], c[4]]))
                        .collect::<Vec<_>>()
                })
                .collect();
            let (min, max) = xs
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), x| (a.min(*x), b.max(*x)));
            max - min
        })
    };
    click(&mut p, l.spin);
    let small = width(&mut p);
    p.zoom = 0;
    let normal = width(&mut p);
    p.zoom = ZOOM_MAX;
    let large = width(&mut p);
    assert!(small < normal && normal < large);
    assert!((large / normal - ZOOM_STEP.powi(ZOOM_MAX)).abs() < 0.01);
    // At rest the solid fills about three fifths of the scene's shorter
    // side, whatever the window.
    let share = normal / l.scene.w.min(l.scene.h);
    assert!((0.5..0.7).contains(&share), "{share}");
}

/// The keyboard does what the pointer does: arrows turn, Page Up and
/// Page Down zoom, Home returns to the first view. Keys a focused
/// control uses are the control's.
#[test]
fn keys_turn_zoom_and_reset() {
    let mut p = page(800.0, 600.0);
    assert!(press(&mut p, key::LEFT));
    assert!(press(&mut p, key::UP));
    assert_eq!((p.yaw, p.pitch), (HOME_YAW + 910, HOME_PITCH + 910));
    assert!(press(&mut p, key::RIGHT));
    assert!(press(&mut p, key::DOWN));
    assert_eq!((p.yaw, p.pitch), (HOME_YAW, HOME_PITCH));
    // A held arrow goes on turning.
    assert!(p.event(Event::KeyDown {
        code: key::RIGHT,
        modifiers: Modifiers(0),
        repeat: true,
    }));
    for _ in 0..40 {
        press(&mut p, key::UP);
    }
    assert_eq!(p.pitch, PITCH_MAX);
    assert!(!press(&mut p, key::UP), "at the stop");
    assert!(press(&mut p, key::PAGE_UP));
    assert!(press(&mut p, key::PAGE_UP));
    assert!(press(&mut p, key::PAGE_DOWN));
    assert_eq!(p.zoom, 1);
    assert!(press(&mut p, key::HOME));
    assert_eq!((p.yaw, p.pitch, p.zoom), (HOME_YAW, HOME_PITCH, 0));
    assert!(!press(&mut p, key::HOME), "already home");
    // With Ctrl held the keys are the host's shortcuts.
    assert!(!p.event(Event::KeyDown {
        code: key::LEFT,
        modifiers: Modifiers(2),
        repeat: false,
    }));
    assert_eq!(p.yaw, HOME_YAW);
    // Other keys do nothing.
    assert!(!press(&mut p, key::ESCAPE));

    // On the slider the arrows and Home move the slider, not the solid.
    assert!(focus(&mut p, ID_STARS, true));
    assert!(press(&mut p, key::RIGHT));
    assert_eq!(p.count.value(), STARS_DEFAULT + STARS_STEP);
    assert!(press(&mut p, key::HOME));
    assert_eq!(p.count.value(), 0);
    assert_eq!((p.yaw, p.pitch, p.zoom), (HOME_YAW, HOME_PITCH, 0));
    assert!(focus(&mut p, ID_STARS, false));
    // On a button they turn the solid; Enter and Space press the button.
    assert!(focus(&mut p, ID_SHAPE - 2, true));
    assert!(press(&mut p, key::LEFT));
    assert_eq!(p.yaw, HOME_YAW + 910);
    assert_eq!(p.shape, 0);
    assert!(press(&mut p, key::ENTER));
    assert_eq!(p.shape, 2);
    focus(&mut p, ID_SHAPE - 2, false);
    focus(&mut p, ID_SHAPE - 1, true);
    assert!(press(&mut p, key::SPACE));
    assert_eq!(p.shape, 1);
    focus(&mut p, ID_SHAPE - 1, false);
    assert!(
        !press(&mut p, key::ENTER),
        "nothing focused, nothing pressed"
    );
    assert_eq!(p.shape, 1);
}

/// The three buttons switch the solid; the tree says which is chosen and
/// what the scene shows. The focused button has a ring.
#[test]
fn buttons_switch_the_solid() {
    let mut p = page(1280.0, 800.0);
    let l = p.layout();
    assert!(click(&mut p, l.spin));
    frame(&mut p, 0);
    let ico = paths().len();
    assert!(click(&mut p, l.shapes[1]));
    frame(&mut p, 0);
    assert!(has(Role::Button, "Torus (vald)") && has(Role::Button, "Ikosaeder"));
    assert!(has(
        Role::Image,
        "3D-scen: torus med 200 ytor, 300 partiklar"
    ));
    let torus = paths();
    assert!(torus.len() > 4 * ico, "{} paths", torus.len());
    assert!(torus.iter().all(|f| f.0 == 5), "quadrilaterals");
    assert_ne!(torus[0].1 >> 8, 0);
    assert!(click(&mut p, l.shapes[2]));
    frame(&mut p, 0);
    assert!(has(Role::Button, "Kub (vald)") && has(Role::Button, "Torus"));
    assert!(has(Role::Image, "3D-scen: kub med 6 ytor, 300 partiklar"));
    // Three faces of the cube, each filled and stroked.
    assert_eq!(paths().len(), 6);
    // The ring: one more rectangle around the focused button.
    let rects = |p: &mut Page| {
        frame(p, 0);
        mock::with(|h| {
            h.frame_ops()
                .iter()
                .filter(|op| matches!(op, Op::Rect { .. }))
                .count()
        })
    };
    let plain = rects(&mut p);
    focus(&mut p, ID_SHAPE, true);
    assert_eq!(rects(&mut p), plain + 1);
    let ring = PALETTE.ring.as_i32() as u32;
    let ringed = mock::with(|h| {
        h.frame_ops().iter().any(|op| {
            matches!(op, Op::Rect { x, w, color, .. }
                if *color == ring && *x == l.shapes[0].x - 3.0 && *w == l.shapes[0].w + 6.0)
        })
    });
    assert!(ringed);
    focus(&mut p, ID_SHAPE, false);
    assert_eq!(rects(&mut p), plain);
}

/// The wireframe: two stroked paths and nothing filled, the faces turned
/// away first and dimmer, every face of the solid in one or the other.
#[test]
fn wireframe_strokes_every_face() {
    let mut p = page(1280.0, 800.0);
    let l = p.layout();
    click(&mut p, l.spin);
    click(&mut p, l.shapes[1]);
    frame(&mut p, 0);
    let filled = paths();
    assert!(click(&mut p, l.wire));
    let ops = frame(&mut p, 0);
    let wire = paths();
    assert_eq!(wire.len(), 2);
    assert!(wire.iter().all(|w| w.2 == WIRE_W));
    assert_eq!(wire[0].0 + wire[1].0, 200 * 5, "every face, five commands");
    // The front path has the faces that were filled.
    assert_eq!(wire[1].0, filled.len() / 2 * 5);
    let (r, g, b) = TINTS[1];
    let bright = u32::from(r) << 24 | u32::from(g) << 16 | u32::from(b) << 8 | 0xFF;
    assert_eq!(wire[1].1, bright);
    assert!(wire[0].1 >> 24 < wire[1].1 >> 24, "the backs are dimmer");
    assert!(has(Role::Checkbox, "Trådmodell (markerad)"));
    assert!(has(
        Role::Image,
        "3D-scen: torus med 200 ytor som trådmodell, 300 partiklar"
    ));
    assert!(has(Role::Text, &format!("Ritoperationer: {ops}")));
    // The cube straight on: its far face and four sides in one path,
    // the near face in the other.
    click(&mut p, l.shapes[2]);
    (p.yaw, p.pitch) = (0, 0);
    frame(&mut p, 0);
    assert_eq!(paths().iter().map(|w| w.0).collect::<Vec<_>>(), [5 * 5, 5]);
    // Off again: filled.
    assert!(click(&mut p, l.wire));
    frame(&mut p, 0);
    assert_eq!(paths(), [(5, paths()[0].1, 0.0), (5, paths()[0].1, SEAM_W)]);
}

/// The slider sets how many stars are drawn, none to eight hundred in
/// fifties, by pointer and by keys; the readout and the tree follow.
#[test]
fn slider_sets_the_number_of_stars() {
    let mut p = page(1280.0, 800.0);
    let l = p.layout();
    click(&mut p, l.spin);
    let default_ops = frame(&mut p, 0);
    assert_eq!(stars_drawn().len(), 300);
    // The right end of the track: all of them.
    let track = Slider::track(l.stars);
    let y = l.stars.y + l.stars.h / 2.0;
    assert!(down(&mut p, track.right(), y));
    up(&mut p, track.right(), y);
    assert_eq!(p.count.value(), 800);
    assert!(p.drag.is_none(), "the slider's press is not the scene's");
    let ops = frame(&mut p, 0);
    assert_eq!(stars_drawn().len(), 800);
    assert_eq!(ops, default_ops + 500);
    assert!(has(Role::Slider, "Partiklar: 800"));
    assert!(has(Role::Text, &format!("Ritoperationer: {ops}")));
    assert!(has(
        Role::Image,
        "3D-scen: ikosaeder med 20 ytor, 800 partiklar"
    ));
    // Dragged to the left end: none, and the solid is still there.
    down(&mut p, track.right(), y);
    assert!(drag(&mut p, track.x - 50.0, y));
    up(&mut p, track.x - 50.0, y);
    assert_eq!(p.count.value(), 0);
    assert_eq!((p.yaw, p.pitch), (HOME_YAW, HOME_PITCH));
    assert_eq!(frame(&mut p, 0), default_ops - 300);
    assert!(stars_drawn().is_empty());
    assert!(!paths().is_empty());
    // By keys, a step at a time.
    focus(&mut p, ID_STARS, true);
    press(&mut p, key::RIGHT);
    press(&mut p, key::RIGHT);
    press(&mut p, key::RIGHT);
    frame(&mut p, 0);
    assert_eq!(stars_drawn().len(), 150);
    assert!(has(Role::Slider, "Partiklar: 150"));
    // The same stars as before, fewer of them: the first 150 of 300.
    let few = stars_drawn();
    press(&mut p, key::PAGE_UP);
    frame(&mut p, 0);
    assert_eq!(stars_drawn()[..150], few[..]);
}

/// The stars move with the view, the near ones more, and stay inside
/// the scene on whole device pixels at every scale.
#[test]
fn stars_follow_the_view_on_whole_pixels() {
    for scale in [1.0f32, 1.5, 2.0] {
        mock::reset();
        let mut p = init(1000.0, 700.0, scale);
        let l = p.layout();
        click(&mut p, l.spin);
        frame(&mut p, 0);
        let rest = stars_drawn();
        for (x, y, side) in &rest {
            for v in [x, y, side] {
                let device = v * scale;
                assert!((device - device.round()).abs() < 1e-3, "{v} at {scale}");
            }
            assert!(*x >= l.scene.x && *x <= l.scene.right());
            assert!(*y >= l.scene.y && *y <= l.scene.bottom());
            assert!(*side * scale >= 1.0);
        }
        // Dragged 100 px to the right: the sky goes left, the near
        // stars four times as far as the far ones.
        down(&mut p, 300.0, 300.0);
        drag(&mut p, 400.0, 300.0);
        up(&mut p, 400.0, 300.0);
        frame(&mut p, 0);
        let moved = stars_drawn();
        let shift = |layer: u8| {
            let i = p.stars.iter().position(|s| s.layer == layer).unwrap();
            (rest[i].0 - moved[i].0).rem_euclid(l.scene.w)
        };
        let unit = 100.0 * 91.0 / 4.0 * l.scene.w / TURN as f32;
        for layer in 0..4u8 {
            let expected = unit * f32::from(layer + 1);
            assert!((shift(layer) - expected).abs() < 1.5, "layer {layer}");
        }
    }
}

/// Every window from 400 × 600 to 1600 × 1000: the scene and the panel
/// share the window, every control is inside the panel and none touches
/// another; a narrow window has the panel below, a wide one beside.
#[test]
fn layouts_fit_every_window() {
    let sizes = [
        (400.0, 600.0),
        (400.0, 700.0),
        (560.0, 900.0),
        (719.0, 600.0),
        (720.0, 600.0),
        (800.0, 600.0),
        (1280.0, 800.0),
        (1600.0, 1000.0),
    ];
    for (w, h) in sizes {
        let p = page(w, h);
        let l = p.layout();
        assert_eq!(l.narrow, w < NARROW);
        let window = Rect::new(0.0, 0.0, w, h);
        let inside = |r: Rect, outer: Rect| {
            r.x >= outer.x
                && r.y >= outer.y
                && r.right() <= outer.right()
                && r.bottom() <= outer.bottom()
        };
        assert!(inside(l.scene, window) && inside(l.panel, window));
        assert_eq!(l.scene.w * l.scene.h + l.panel.w * l.panel.h, w * h);
        assert!(l.scene.w >= 400.0 && l.scene.h >= 400.0, "{w} × {h}");
        if l.narrow {
            assert_eq!((l.panel.y, l.panel.w), (l.scene.bottom(), w));
            assert!(l.form.is_none() && l.hint.is_none());
        } else {
            assert_eq!((l.panel.x, l.panel.h), (l.scene.right(), h));
            assert!(l.form.is_some() && l.hint.is_some());
        }
        let mut parts = vec![l.wire, l.spin, l.stars, l.readout[0], l.readout[1]];
        parts.extend(l.shapes);
        parts.extend(l.form);
        parts.extend(l.hint);
        if !l.narrow {
            parts.push(l.stars_label);
        }
        for (i, a) in parts.iter().enumerate() {
            assert!(inside(*a, l.panel), "{w} × {h}: part {i} {a:?}");
            assert!(a.w > 60.0 && a.h >= 20.0, "{w} × {h}: part {i} {a:?}");
            for b in &parts[i + 1..] {
                let apart =
                    a.right() <= b.x || b.right() <= a.x || a.bottom() <= b.y || b.bottom() <= a.y;
                assert!(apart, "{w} × {h}: {a:?} over {b:?}");
            }
        }
        // The slider's label is beside it in the strip.
        if l.narrow {
            assert!(l.stars_label.right() <= l.stars.x);
            assert_eq!(l.stars_label.y, l.stars.y);
        }
        // What is drawn is what is hit: the nodes have the layout's
        // rectangles.
        let mut p = p;
        frame(&mut p, 0);
        mock::with(|h| {
            let rect_of = |id: i32| {
                let n = h.sem.iter().find(|n| n.id == id).unwrap();
                Rect::new(n.x, n.y, n.w, n.h)
            };
            for (i, r) in l.shapes.iter().enumerate() {
                assert_eq!(rect_of(ID_SHAPE - i as i32), *r);
            }
            assert_eq!(rect_of(ID_WIRE), l.wire);
            assert_eq!(rect_of(ID_SPIN), l.spin);
            assert_eq!(rect_of(ID_STARS), l.stars);
            let image = h.sem.iter().find(|n| n.role == Role::Image as i32).unwrap();
            assert_eq!(
                (image.x, image.y, image.w, image.h),
                (0.0, 0.0, l.scene.w, l.scene.h)
            );
        });
    }
    // A resize takes effect at once.
    let mut p = page(1280.0, 800.0);
    p.resize(Window {
        width: 400.0,
        height: 700.0,
        scale: 2.0,
    });
    assert!(p.layout().narrow);
}
