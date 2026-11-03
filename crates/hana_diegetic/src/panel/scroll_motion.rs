//! Frame-rate-independent wheel easing and direct gesture edge motion.

use bevy::input::touch::TouchPhase;

use super::constants::COAST_DECAY;
use super::constants::OVERSCROLL_RELEASE_SECONDS;
use super::constants::SCROLL_REST_DISTANCE;
use super::constants::SCROLL_REST_SPEED;
use super::constants::SPRING_STIFFNESS;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum ScrollPhase {
    #[default]
    AtRest,
    Tracking,
    Coasting,
    SpringingBack,
}

pub(crate) enum ScrollInput {
    Notches { distance: f32 },
    Pixels { distance: f32, phase: TouchPhase },
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(super) struct ScrollMotion {
    phase:                 ScrollPhase,
    velocity:              f32,
    pub(super) overscroll: f32,
    since_input:           f32,
}

#[derive(Clone, Copy)]
pub(super) struct ScrollBounds {
    pub(super) offset:   f32,
    pub(super) max:      f32,
    pub(super) viewport: f32,
}

impl ScrollMotion {
    pub(super) fn input(&mut self, input: ScrollInput, at: ScrollBounds) -> f32 {
        if at.max <= 0.0 {
            *self = Self::default();
            return 0.0;
        }
        self.since_input = 0.0;
        match input {
            ScrollInput::Notches { distance } => {
                if self.phase != ScrollPhase::Coasting {
                    self.velocity = 0.0;
                }
                self.overscroll = 0.0;
                self.velocity = distance.mul_add(COAST_DECAY, self.velocity);
                self.phase = if self.velocity == 0.0 {
                    ScrollPhase::AtRest
                } else {
                    ScrollPhase::Coasting
                };
                0.0
            },
            ScrollInput::Pixels { distance, phase } => {
                self.velocity = 0.0;
                // Undo the band before accumulating another gesture event:
                // b(a) + b(b) is not b(a + b).
                let raw = unband(self.overscroll, at.viewport);
                let requested = at.offset + raw + distance;
                let applied = requested.clamp(0.0, at.max);
                self.overscroll = rubber_band(requested - applied, at.viewport);
                self.phase = ScrollPhase::Tracking;
                if matches!(phase, TouchPhase::Ended | TouchPhase::Canceled) {
                    self.release();
                }
                applied - at.offset
            },
        }
    }

    pub(super) fn step(&mut self, dt: f32, at: ScrollBounds) -> f32 {
        if at.max <= 0.0 {
            *self = Self::default();
            return 0.0;
        }
        let dt = dt.max(0.0);
        match self.phase {
            ScrollPhase::AtRest => 0.0,
            ScrollPhase::Tracking => {
                self.since_input += dt;
                if self.since_input >= OVERSCROLL_RELEASE_SECONDS {
                    let released_dt = self.since_input - OVERSCROLL_RELEASE_SECONDS;
                    self.release();
                    self.spring(released_dt);
                }
                0.0
            },
            ScrollPhase::Coasting => {
                let decay = (-COAST_DECAY * dt).exp();
                let mut distance = self.velocity * (1.0 - decay) / COAST_DECAY;
                self.velocity *= decay;
                // Include the tiny remaining integral when coming to rest,
                // so one notch travels its entire declared distance.
                if self.velocity.abs() < SCROLL_REST_SPEED
                    && (self.velocity / COAST_DECAY).abs() < SCROLL_REST_DISTANCE
                {
                    distance += self.velocity / COAST_DECAY;
                    self.velocity = 0.0;
                    self.phase = ScrollPhase::AtRest;
                }
                let requested = at.offset + distance;
                let applied = requested.clamp(0.0, at.max);
                if requested < 0.0 || requested > at.max {
                    self.velocity = 0.0;
                    self.phase = ScrollPhase::AtRest;
                }
                applied - at.offset
            },
            ScrollPhase::SpringingBack => {
                self.spring(dt);
                0.0
            },
        }
    }

    pub(super) const fn is_at_rest(&self) -> bool { matches!(self.phase, ScrollPhase::AtRest) }

    const fn release(&mut self) {
        self.velocity = 0.0;
        self.phase = if self.overscroll == 0.0 {
            ScrollPhase::AtRest
        } else {
            ScrollPhase::SpringingBack
        };
    }

    fn spring(&mut self, dt: f32) {
        if self.phase != ScrollPhase::SpringingBack {
            return;
        }
        // Exact critically damped solution, including its velocity. An Euler
        // step would bounce or diverge on a long frame.
        let frequency = SPRING_STIFFNESS.sqrt();
        let decay = (-frequency * dt).exp();
        let slope = frequency.mul_add(self.overscroll, self.velocity);
        self.overscroll = slope.mul_add(dt, self.overscroll) * decay;
        self.velocity = (-frequency * slope).mul_add(dt, self.velocity) * decay;
        if self.overscroll.abs() < SCROLL_REST_DISTANCE && self.velocity.abs() < SCROLL_REST_SPEED {
            self.overscroll = 0.0;
            self.velocity = 0.0;
            self.phase = ScrollPhase::AtRest;
        }
    }
}

fn rubber_band(distance: f32, viewport: f32) -> f32 {
    if viewport <= 0.0 {
        return 0.0;
    }
    distance.signum() * viewport * (1.0 - 1.0 / (distance.abs() / viewport).mul_add(0.55, 1.0))
}

fn unband(distance: f32, viewport: f32) -> f32 {
    if viewport <= 0.0 || distance == 0.0 {
        return 0.0;
    }
    distance.signum() * distance.abs() / (0.55 * (1.0 - distance.abs() / viewport))
}

#[cfg(test)]
mod tests {
    use hana_kana::ToF32;

    use super::*;

    const fn bounds(offset: f32) -> ScrollBounds {
        ScrollBounds {
            offset,
            max: 1000.0,
            viewport: 100.0,
        }
    }

    fn near(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < 0.01, "{actual} != {expected}");
    }

    #[test]
    fn scroll_notches_travel_the_whole_distance_at_different_frame_rates() {
        for dt in [1.0 / 30.0, 1.0 / 60.0, 1.0 / 120.0, 0.4] {
            let mut motion = ScrollMotion::default();
            near(
                motion.input(ScrollInput::Notches { distance: 45.0 }, bounds(0.0)),
                0.0,
            );
            let mut offset = 0.0;
            for _ in 0..1000 {
                offset += motion.step(dt, bounds(offset));
                if motion.is_at_rest() {
                    break;
                }
            }
            near(offset, 45.0);
            assert!(motion.is_at_rest());
        }
    }

    #[test]
    fn scroll_coasting_uses_the_exact_decay_integral() {
        let mut motion = ScrollMotion::default();
        motion.input(ScrollInput::Notches { distance: 45.0 }, bounds(0.0));
        let mut offset = 0.0;
        for frame in 1..=60 {
            offset += motion.step(1.0 / 120.0, bounds(offset));
            if [1, 10, 60].contains(&frame) {
                near(
                    offset,
                    45.0 * (1.0 - (-12.0 * frame.to_f32() / 120.0).exp()),
                );
            }
        }
    }

    #[test]
    fn scroll_notch_coasts_stop_at_both_edges_without_bouncing() {
        for (offset, distance, expected) in [(990.0, 45.0, 1000.0), (10.0, -45.0, 0.0)] {
            let mut motion = ScrollMotion::default();
            motion.input(ScrollInput::Notches { distance }, bounds(offset));
            near(offset + motion.step(1.0, bounds(offset)), expected);
            assert!(motion.is_at_rest());
            assert_eq!(motion.overscroll, 0.0);
        }
    }

    #[test]
    fn scroll_pixels_move_one_to_one_without_library_momentum() {
        let mut motion = ScrollMotion::default();
        near(
            motion.input(
                ScrollInput::Pixels {
                    distance: 20.0,
                    phase:    TouchPhase::Moved,
                },
                bounds(50.0),
            ),
            20.0,
        );
        near(motion.step(0.2, bounds(70.0)), 0.0);
        assert!(motion.is_at_rest());
    }

    #[test]
    fn scroll_pixel_events_accumulate_the_rubber_band_and_can_reverse_it() {
        let mut motion = ScrollMotion::default();
        for _ in 0..2 {
            motion.input(
                ScrollInput::Pixels {
                    distance: -25.0,
                    phase:    TouchPhase::Moved,
                },
                bounds(0.0),
            );
        }
        near(motion.overscroll, rubber_band(-50.0, 100.0));
        let change = motion.input(
            ScrollInput::Pixels {
                distance: 60.0,
                phase:    TouchPhase::Moved,
            },
            bounds(0.0),
        );
        near(change, 10.0);
        near(motion.overscroll, 0.0);
    }

    #[test]
    fn scroll_pixel_overscroll_springs_on_release_or_idle() {
        for phase in [TouchPhase::Ended, TouchPhase::Canceled, TouchPhase::Moved] {
            let mut motion = ScrollMotion::default();
            motion.input(
                ScrollInput::Pixels {
                    distance: -50.0,
                    phase,
                },
                bounds(0.0),
            );
            let initial = motion.overscroll;
            if phase == TouchPhase::Moved {
                motion.step(0.09, bounds(0.0));
                near(motion.overscroll, initial);
                motion.step(0.02, bounds(0.0));
            }
            for _ in 0..240 {
                let previous = motion.overscroll;
                near(motion.step(1.0 / 120.0, bounds(0.0)), 0.0);
                assert!(motion.overscroll >= previous && motion.overscroll <= 0.0);
            }
            assert!(motion.is_at_rest());
            assert_eq!(motion.overscroll, 0.0);
        }
    }

    #[test]
    fn scroll_motion_drops_everything_when_content_fits() {
        let mut motion = ScrollMotion::default();
        motion.input(
            ScrollInput::Pixels {
                distance: -50.0,
                phase:    TouchPhase::Moved,
            },
            bounds(0.0),
        );
        motion.step(
            0.01,
            ScrollBounds {
                offset:   0.0,
                max:      0.0,
                viewport: 100.0,
            },
        );
        assert_eq!(motion, ScrollMotion::default());
    }
}
