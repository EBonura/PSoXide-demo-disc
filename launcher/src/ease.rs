//! Integer easing. Time is Q8 (0..=256); no floats, no 64-bit maths.

/// Exponential-style ease-out (quartic), the shell's enter/open/lift curve.
pub fn out(t: i32) -> i32 {
    let u = 256 - t.clamp(0, 256);
    let u2 = (u * u) >> 8;
    256 - ((u2 * u2) >> 8)
}

/// Ease-in-out (cubic), the shell's recede/slide curve.
pub fn in_out(t: i32) -> i32 {
    let t = t.clamp(0, 256);
    if t < 128 {
        (4 * t * t * t) >> 16
    } else {
        let u = 256 - t;
        256 - ((4 * u * u * u) >> 16)
    }
}

/// Quadratic ramp, for the disc's spin-up.
pub fn quad_in(t: i32) -> i32 {
    let t = t.clamp(0, 256);
    (t * t) >> 8
}

pub fn lerp(a: i32, b: i32, t: i32) -> i32 {
    a + (((b - a) * t) >> 8)
}

/// Progress of frame `t` through the window `[from, to)`, as Q8.
pub fn span(t: i32, from: i32, to: i32) -> i32 {
    (((t - from) * 256) / (to - from)).clamp(0, 256)
}

/// Move `x` toward `target` by `1/2^shift` of the gap, landing exactly.
pub fn follow(x: i32, target: i32, shift: u32) -> i32 {
    let d = target - x;
    if d.abs() < (1 << shift) {
        target
    } else {
        x + (d >> shift)
    }
}

/// Piecewise-linear lookup through knots `(x, y)` sorted by x; clamps past the ends.
pub fn knots(x: i32, k: &[(i32, i32)]) -> i32 {
    if x <= k[0].0 {
        return k[0].1;
    }
    for w in k.windows(2) {
        if x <= w[1].0 {
            let t = ((x - w[0].0) * 256) / (w[1].0 - w[0].0);
            return lerp(w[0].1, w[1].1, t);
        }
    }
    k[k.len() - 1].1
}
