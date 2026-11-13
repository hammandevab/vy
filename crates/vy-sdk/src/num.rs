//! Numbers as text without `core::fmt`.
//!
//! `format!("{n}")` brings the formatting machinery of `core` into a
//! module: about 4 KB of wasm for a page that only writes a count or a
//! price. These append the digits to a `String` instead, with integer
//! arithmetic only, so the same number is the same text on every host
//! (SPEC-2.1.5).
//!
//! ```
//! use vy_sdk::num::{push_decimal, push_int};
//!
//! let mut s = String::from("Poäng: ");
//! push_int(&mut s, -1250);
//! assert_eq!(s, "Poäng: -1250");
//!
//! // 1 995 öre as kronor, with the decimal comma.
//! let mut s = String::new();
//! push_decimal(&mut s, 1995, 2, ',');
//! s.push_str(" kr");
//! assert_eq!(s, "19,95 kr");
//! ```

use alloc::string::String;

/// Append `v` in decimal: `-` for a negative number, no grouping.
pub fn push_int(s: &mut String, v: i64) {
    if v < 0 {
        s.push('-');
    }
    push_digits(s, v.unsigned_abs(), 1);
}

/// Append the fixed-point number `v` × 10^−`decimals`: `v` in decimal with
/// its last `decimals` digits after `point`, every one of them written
/// (`push_decimal(s, 1500, 2, ',')` gives `15,00`). A value below one
/// starts with `0`, and `decimals == 0` is [`push_int`]. At most 18
/// decimals are written; a larger `decimals` is taken as 18.
///
/// A page with an `f32` rounds it first:
/// `push_decimal(s, vy_sdk::math::round(v * 100.0) as i64, 2, '.')`.
pub fn push_decimal(s: &mut String, v: i64, decimals: u32, point: char) {
    let decimals = decimals.min(18);
    if decimals == 0 {
        return push_int(s, v);
    }
    let unit = 10u64.pow(decimals);
    let a = v.unsigned_abs();
    if v < 0 {
        s.push('-');
    }
    push_digits(s, a / unit, 1);
    s.push(point);
    push_digits(s, a % unit, decimals as usize);
}

/// `v` in decimal, padded with leading zeros to at least `width` digits.
fn push_digits(s: &mut String, mut v: u64, width: usize) {
    // u64::MAX has twenty digits.
    let mut digits = [b'0'; 20];
    let mut n = 0;
    while v != 0 || n < width.min(20) {
        digits[n] = b'0' + (v % 10) as u8;
        v /= 10;
        n += 1;
    }
    for d in digits[..n].iter().rev() {
        s.push(*d as char);
    }
}
