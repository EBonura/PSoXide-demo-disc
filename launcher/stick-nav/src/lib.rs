//! The left analog stick as d-pad presses, for a menu.
//!
//! [`StickNav::update`] turns each poll's stick bytes into a button mask in the
//! same bit layout as [`psx_pad::button`], so the launcher ORs it into the
//! d-pad's own bits and everything downstream (press edges, the browse chirp,
//! the idle timer, the unlock code) treats the stick as the d-pad. The d-pad
//! has no auto-repeat in the launcher, so neither does this: a push is one
//! press, and the stick has to come back near centre before it can press again.
//!
//! The dead region is [`psx_pad::Deadzone`], the radial one the SDK shares, so a
//! diagonal push is judged by how far the stick is from centre rather than per
//! axis. Two radii give the hysteresis: a push registers past [`ENTER`] and the
//! stick only re-arms once it is back inside [`EXIT`], so a stick resting near
//! the threshold cannot chatter into repeated presses.
#![no_std]

use psx_pad::{button, AnalogSticks, Deadzone, PadMode};

/// Radius (of the 127 counts a healthy stick reaches) a push must pass to
/// press. About half travel: deliberate, well clear of centre drift.
pub const ENTER: i16 = 64;

/// Radius the stick must come back inside before it can press again.
pub const EXIT: i16 = 32;

/// The four d-pad bits this produces.
const DPAD: u16 = button::LEFT | button::RIGHT | button::UP | button::DOWN;

/// Which way the stick is latched, if it is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Latch {
    Centre,
    Held(u16),
}

/// Stick-to-d-pad state. One per pad.
#[derive(Copy, Clone, Debug)]
pub struct StickNav {
    latch: Latch,
    enter: Deadzone,
    exit: Deadzone,
}

impl Default for StickNav {
    fn default() -> Self {
        Self::new()
    }
}

impl StickNav {
    /// A stick at rest.
    pub const fn new() -> Self {
        Self {
            latch: Latch::Centre,
            enter: Deadzone::new(ENTER),
            exit: Deadzone::new(EXIT),
        }
    }

    /// The d-pad bits the left stick is holding down this poll (zero or one
    /// of them). A pad that is not reporting sticks (`mode` is not analog)
    /// holds nothing and releases any latch.
    ///
    /// The direction is the dominant axis at the moment the push crosses
    /// [`ENTER`] and stays that way until the stick re-centres.
    pub fn update(&mut self, mode: PadMode, sticks: AnalogSticks) -> u16 {
        if mode != PadMode::Analog {
            self.latch = Latch::Centre;
            return 0;
        }
        let (x, y) = sticks.left_centered();
        match self.latch {
            Latch::Centre => {
                if self.enter.outside(x, y) {
                    let bit = dominant(x, y);
                    self.latch = Latch::Held(bit);
                    bit & DPAD
                } else {
                    0
                }
            }
            Latch::Held(bit) => {
                if self.exit.outside(x, y) {
                    bit
                } else {
                    self.latch = Latch::Centre;
                    0
                }
            }
        }
    }
}

/// The d-pad bit for the larger axis. On a tie horizontal wins, which is the
/// carousel's axis. Stick Y is 0 at the top, like the d-pad's UP.
fn dominant(x: i16, y: i16) -> u16 {
    if x.abs() >= y.abs() {
        if x < 0 {
            button::LEFT
        } else {
            button::RIGHT
        }
    } else if y < 0 {
        button::UP
    } else {
        button::DOWN
    }
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::*;
    use psx_pad::{ButtonState, STICK_CENTER};
    use std::vec::Vec;

    /// Left stick reading `dx`, `dy` counts from centre (negative = left/up).
    fn stick(dx: i16, dy: i16) -> AnalogSticks {
        AnalogSticks {
            left_x: (STICK_CENTER as i16 + dx) as u8,
            left_y: (STICK_CENTER as i16 + dy) as u8,
            ..AnalogSticks::CENTERED
        }
    }

    /// Run a sequence of stick positions and collect the press edges the way
    /// the launcher sees them: the mask ORed into the pad, then `pressed`.
    fn presses(seq: &[(i16, i16)]) -> Vec<u16> {
        let mut nav = StickNav::new();
        let mut prev = ButtonState::NONE;
        let mut out = Vec::new();
        for &(dx, dy) in seq {
            let now = ButtonState::from_bits(nav.update(PadMode::Analog, stick(dx, dy)));
            for b in [button::LEFT, button::RIGHT, button::UP, button::DOWN] {
                if now.pressed_since(prev, b) {
                    out.push(b);
                }
            }
            prev = now;
        }
        out
    }

    #[test]
    fn centre_and_drift_press_nothing() {
        assert!(presses(&[(0, 0), (10, -8), (-20, 15), (30, 0), (0, -31)]).is_empty());
    }

    #[test]
    fn a_push_left_or_right_is_one_press() {
        assert_eq!(presses(&[(0, 0), (-80, 0), (-127, 0), (-127, 0)]), [button::LEFT]);
        assert_eq!(presses(&[(0, 0), (90, 0), (127, 0)]), [button::RIGHT]);
    }

    #[test]
    fn holding_the_push_does_not_repeat() {
        let mut held = std::vec![(0, 0)];
        held.extend(std::iter::repeat((127, 0)).take(300));
        assert_eq!(presses(&held), [button::RIGHT]);
    }

    #[test]
    fn up_and_down_follow_the_stick_y_axis() {
        // PS1 sticks read 0x00 at the top.
        assert_eq!(presses(&[(0, -100)]), [button::UP]);
        assert_eq!(presses(&[(0, 100)]), [button::DOWN]);
    }

    #[test]
    fn hysteresis_needs_the_stick_back_near_centre() {
        // Past ENTER, then hovering between EXIT and ENTER: still the same push.
        let seq = [(70, 0), (50, 0), (40, 0), (66, 0), (50, 0)];
        assert_eq!(presses(&seq), [button::RIGHT]);
        // Back inside EXIT re-arms; the next push is a second press.
        let seq = [(70, 0), (50, 0), (20, 0), (70, 0)];
        assert_eq!(presses(&seq), [button::RIGHT, button::RIGHT]);
    }

    #[test]
    fn resting_just_under_the_threshold_never_chatters() {
        let seq: Vec<(i16, i16)> = (0..200).map(|i| (ENTER - 1 - (i % 2), 0)).collect();
        assert!(presses(&seq).is_empty());
        // Noise straddling ENTER after a real push cannot re-fire either.
        let mut seq = std::vec![(80, 0)];
        seq.extend((0..200).map(|i| (ENTER - 5 + (i % 10), 0)));
        assert_eq!(presses(&seq), [button::RIGHT]);
    }

    #[test]
    fn a_flick_through_centre_presses_each_way() {
        let seq = [(-100, 0), (0, 0), (100, 0), (0, 0), (-100, 0)];
        assert_eq!(presses(&seq), [button::LEFT, button::RIGHT, button::LEFT]);
    }

    #[test]
    fn a_diagonal_picks_its_dominant_axis_once() {
        // Mostly right, a little up: one RIGHT, no UP.
        assert_eq!(presses(&[(90, -40), (100, -50)]), [button::RIGHT]);
        // Mostly up, a little right: one UP.
        assert_eq!(presses(&[(30, -90)]), [button::UP]);
        // Radial, not per axis: 50,50 is 70 from centre, so it presses.
        assert_eq!(presses(&[(50, 50)]), [button::RIGHT]);
    }

    #[test]
    fn a_pad_that_is_not_analog_presses_nothing_and_unlatches() {
        let mut nav = StickNav::new();
        assert_eq!(nav.update(PadMode::Analog, stick(100, 0)), button::RIGHT);
        // The pad drops to digital: no bits, and the latch is gone.
        assert_eq!(nav.update(PadMode::Digital, stick(100, 0)), 0);
        assert_eq!(nav.update(PadMode::Disconnected, AnalogSticks::CENTERED), 0);
        // Back in analog with the stick still pushed is a fresh press.
        assert_eq!(nav.update(PadMode::Analog, stick(100, 0)), button::RIGHT);
    }

    #[test]
    fn the_mask_only_ever_carries_dpad_bits() {
        let mut nav = StickNav::new();
        for dx in (-127..=127).step_by(7) {
            for dy in (-127..=127).step_by(7) {
                let mask = nav.update(PadMode::Analog, stick(dx, dy));
                assert_eq!(mask & !DPAD, 0);
                assert!(mask.count_ones() <= 1);
            }
        }
    }
}
