//! Where the rotating title ring and the ball of balls above it end up on
//! screen.
//!
//! Straight homage to the PlayStation demo discs: a carousel of glossy blue
//! pills you spin to pick a game, under a slowly turning sphere made of
//! smaller spheres, over a starfield. This crate works out the positions; the
//! launcher draws them, so the geometry stays testable off the console.
//!
//! All integer. Angles are Q12 turns (0..4096), `psx_math::sin_q12` returns
//! Q12, and the projection is a shift-and-divide rather than anything with a
//! decimal point in it.

#![no_std]

use psx_math::{cos_q12, sin_q12};

/// A full turn.
pub const TURN: i32 = 4096;

/// Camera distance from the ring's centre, world units. Well clear of the
/// ring radius: closer would swing the front pill up to several times the
/// size of the back one and off the screen.
const CAMERA_Z: i32 = 520;
/// Focal length. Bigger spreads the ring wider across the screen.
const FOCAL: i32 = 300;
/// Ring radius.
const RING_R: i32 = 205;
/// Screen row the ring's centre projects to.
const RING_Y: i16 = 180;
/// How far the ring's far side rides up the screen: the tilt that turns a
/// circle into an ellipse.
const RING_TILT: i32 = 30;

/// Pill size at the front of the ring, before perspective. Wide and deep
/// enough to carry a two-line title without the words hanging off the ends.
const PILL_RX: i32 = 68;
const PILL_RY: i32 = 19;

/// The ball of balls hangs centred above the ring.
const SPHERE_CENTRE_X: i16 = 160;
const SPHERE_CENTRE_Y: i16 = 66;
const SPHERE_R: i32 = 84;
/// Rings of latitude, and points around each. Poles are added separately.
/// Dense enough that the beads crowd each other, which is what stops the
/// cluster reading as scattered confetti.
const SPHERE_LAT: usize = 7;
const SPHERE_LON: usize = 10;
pub const SPHERE_POINTS: usize = SPHERE_LAT * SPHERE_LON + 2;

/// Where one carousel item landed after projection.
#[derive(Copy, Clone)]
pub struct Placed {
    pub x: i16,
    pub y: i16,
    pub rx: i16,
    pub ry: i16,
    /// Depth, larger is further away. Used to sort and to dim.
    pub z: i32,
    /// 0..=255, how much of the way to the front this item is.
    pub front: u8,
}

/// Perspective divide. Returns a Q8 scale factor for something at depth `z`.
fn perspective(z: i32) -> i32 {
    let z = if z < 40 { 40 } else { z };
    (FOCAL << 8) / z
}

/// Project one carousel slot. `angle` is where that slot sits on the ring,
/// measured so that zero is the front: the selected entry sits nearest the
/// camera, centred, which is the whole point of a carousel.
pub fn place(angle: i32) -> Placed {
    let a = ((angle + TURN / 2) & (TURN - 1)) as u16;
    let world_x = (RING_R * sin_q12(a)) >> 12;
    let world_z = (RING_R * cos_q12(a)) >> 12;
    let z = CAMERA_Z + world_z;
    let k = perspective(z);

    // cos is +1 at the back of the ring and -1 at the front.
    let front = (((-cos_q12(a) + 4096) * 255) / 8192) as u8;

    Placed {
        x: (160 + ((world_x * k) >> 8)) as i16,
        y: (RING_Y as i32 - ((world_z * RING_TILT) >> 8)) as i16,
        rx: ((PILL_RX * k) >> 8) as i16,
        ry: ((PILL_RY * k) >> 8) as i16,
        z,
        front,
    }
}

/// A projected sphere bead, before sorting.
#[derive(Copy, Clone, Default)]
pub struct Bead {
    pub x: i16,
    pub y: i16,
    pub r: i16,
    pub z: i32,
    pub lit: u8,
}

/// Project the ball of balls, spun by `angle`. Fills `out` and returns how
/// many beads it wrote (always [`SPHERE_POINTS`], but the caller sorts a
/// slice so the count is worth being explicit about).
pub fn sphere(angle: i32, out: &mut [Bead; SPHERE_POINTS]) -> usize {
    let a = (angle & (TURN - 1)) as u16;
    let (sin_a, cos_a) = (sin_q12(a), cos_q12(a));
    let mut n = 0;

    let mut emit = |x: i32, y: i32, z: i32| {
        // Spin about the vertical axis.
        let rx = ((x * cos_a) + (z * sin_a)) >> 12;
        let rz = ((z * cos_a) - (x * sin_a)) >> 12;
        let depth = CAMERA_Z + rz;
        let k = perspective(depth);
        // Front beads are bigger and brighter, back ones sink into the dark.
        // Beads round the back sink most of the way into the dark, which is
        // what makes a cloud of ellipses read as one sphere.
        let lit = (24 + ((SPHERE_R - rz).clamp(0, 2 * SPHERE_R) * 231) / (2 * SPHERE_R)) as u8;
        out[n] = Bead {
            x: (SPHERE_CENTRE_X as i32 + ((rx * k) >> 8)) as i16,
            y: (SPHERE_CENTRE_Y as i32 + ((y * k) >> 8)) as i16,
            r: (((SPHERE_R / 7) * k) >> 8) as i16,
            z: depth,
            lit,
        };
        n += 1;
    };

    emit(0, -SPHERE_R, 0);
    for lat in 0..SPHERE_LAT {
        // Latitudes spread between the poles, exclusive of both.
        let phi = (((lat as i32 + 1) * TURN) / (2 * (SPHERE_LAT as i32 + 1))) as u16;
        let y = -((SPHERE_R * cos_q12(phi)) >> 12);
        let ring = (SPHERE_R * sin_q12(phi)) >> 12;
        for lon in 0..SPHERE_LON {
            // Offset alternate rings so the beads sit in each other's gaps.
            let theta = (((lon as i32 * 2 + (lat & 1) as i32) * TURN)
                / (SPHERE_LON as i32 * 2)) as u16;
            emit(
                (ring * sin_q12(theta)) >> 12,
                y,
                (ring * cos_q12(theta)) >> 12,
            );
        }
    }
    emit(0, SPHERE_R, 0);
    n
}

/// Sort `items` back to front by depth. Insertion sort: these arrays are
/// tens of entries, and it is already nearly sorted between frames.
pub fn sort_by_depth<T: Copy, F: Fn(&T) -> i32>(items: &mut [T], depth: F) {
    for i in 1..items.len() {
        let mut j = i;
        while j > 0 && depth(&items[j - 1]) < depth(&items[j]) {
            items.swap(j - 1, j);
            j -= 1;
        }
    }
}

/// Bleed a browse-kick off the ball's spin rate, one frame's worth.
///
/// The shift alone stalls a few units short of `idle` once the gap is small
/// enough that it rounds to zero, so the last of it closes by hand. Without
/// that the ball would keep creeping after the player stopped browsing.
pub fn ease_spin(rate: i32, idle: i32, decay_shift: i32) -> i32 {
    let excess = idle - rate;
    rate + if excess.abs() < (1 << decay_shift) {
        excess.signum()
    } else {
        excess >> decay_shift
    }
}

/// Deterministic specks of starfield. No RNG on the guest: a cheap integer
/// hash of the index gives a fixed, evenly scattered sky.
pub fn star(index: u32) -> (i16, i16, u8) {
    let mut h = index.wrapping_mul(2_654_435_761);
    h ^= h >> 15;
    h = h.wrapping_mul(0x85EB_CA6B);
    h ^= h >> 13;
    let x = (h % 320) as i16;
    let y = ((h >> 9) % 240) as i16;
    let bright = 60 + ((h >> 20) % 150) as u8;
    (x, y, bright)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stars_stay_on_screen() {
        for i in 0..512 {
            let (x, y, b) = star(i);
            assert!((0..320).contains(&x), "star {i} x={x}");
            assert!((0..240).contains(&y), "star {i} y={y}");
            assert!(b >= 60);
        }
    }

    #[test]
    fn the_front_of_the_ring_is_lower_bigger_and_brighter_than_the_back() {
        // Zero is the front, where the selected entry sits; half a turn is
        // the far side of the ring.
        let front = place(0);
        let back = place(TURN / 2);
        assert!(front.z < back.z, "front is nearer");
        assert!(front.rx > back.rx, "front is bigger");
        assert!(front.y > back.y, "front sits lower on screen");
        assert!(front.front > back.front);
        assert_eq!(front.x, 160, "the selection is centred");
    }

    #[test]
    fn a_browse_kick_always_settles_back_to_the_idle_drift() {
        const IDLE: i32 = 5;
        for start in [IDLE + 110, IDLE - 110, IDLE, IDLE + 1, IDLE - 1] {
            let mut rate = start;
            for _ in 0..400 {
                rate = ease_spin(rate, IDLE, 4);
            }
            assert_eq!(rate, IDLE, "starting from {start}");
        }
    }

    #[test]
    fn a_kick_decays_rather_than_snapping() {
        let after_one_frame = ease_spin(115, 5, 4);
        assert!(after_one_frame < 115, "it slows");
        assert!(after_one_frame > 40, "but is still clearly spun up");
    }

    #[test]
    fn depth_sort_puts_the_far_ones_first() {
        let mut items = [3i32, 1, 4, 1, 5, 9, 2, 6];
        sort_by_depth(&mut items, |v| *v);
        assert_eq!(items, [9, 6, 5, 4, 3, 2, 1, 1]);
    }

    #[test]
    fn every_sphere_point_is_written() {
        let mut beads = [Bead::default(); SPHERE_POINTS];
        assert_eq!(sphere(0, &mut beads), SPHERE_POINTS);
        // The poles are the extremes; nothing should be outside them.
        let top = beads.iter().map(|b| b.y).min().unwrap();
        let bottom = beads.iter().map(|b| b.y).max().unwrap();
        assert!(bottom > top);
    }
}
