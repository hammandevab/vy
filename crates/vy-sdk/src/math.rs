//! Rounding for `no_std` pages, without `libm`.
//!
//! `core` has no `floor`, `round` or `ceil` (they live in `std`), so a page
//! would otherwise truncate with `as i32` and get blurry 1 px lines at
//! scale 1.5. These are plain casts and comparisons on `f32`; every one is
//! exact for the magnitudes a layout uses (below 2²³, where every `f32`
//! above is already an integer and is returned as is). NaN and infinities
//! come back unchanged.

/// Largest magnitude below which an `f32` can have a fraction.
const INTEGRAL_ABOVE: f32 = 8_388_608.0;

/// `v` with its fraction dropped, towards zero.
fn trunc(v: f32) -> f32 {
    if v.abs() < INTEGRAL_ABOVE {
        (v as i32) as f32
    } else {
        v
    }
}

/// Largest integer not above `v`.
pub fn floor(v: f32) -> f32 {
    let t = trunc(v);
    if t > v {
        t - 1.0
    } else {
        t
    }
}

/// Smallest integer not below `v`.
pub fn ceil(v: f32) -> f32 {
    let t = trunc(v);
    if t < v {
        t + 1.0
    } else {
        t
    }
}

/// Nearest integer, halves away from zero (as `f32::round` in `std`).
pub fn round(v: f32) -> f32 {
    let t = trunc(v);
    let frac = v - t;
    if frac >= 0.5 {
        t + 1.0
    } else if frac <= -0.5 {
        t - 1.0
    } else {
        t
    }
}

/// `v` moved to the nearest physical-pixel boundary, in logical pixels
/// (SPEC-6.3.1): `round(v × scale) / scale`. A 1 px rule drawn at
/// `snap(y, scale)` with height `snap(1.0, scale)` covers whole device
/// pixels at every scale, so it stays crisp at 1.5 as well as at 1 and 2.
/// A scale that is not positive and finite leaves `v` unchanged.
pub fn snap(v: f32, scale: f32) -> f32 {
    if scale > 0.0 && scale.is_finite() {
        round(v * scale) / scale
    } else {
        v
    }
}
