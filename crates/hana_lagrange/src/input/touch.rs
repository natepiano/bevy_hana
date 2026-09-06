use bevy::input::touch::Touch;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Touches;
use bevy::prelude::Vec2;

/// The touch gesture recognized for the current frame.
#[derive(Debug, Clone)]
pub(crate) enum TouchGestures {
    /// No touch gesture this frame.
    None,
    /// One-finger touch gesture.
    OneFinger(OneFingerGestures),
    /// Two-finger touch gesture.
    TwoFinger(TwoFingerGestures),
}

/// Holds information pertaining to one finger gestures
#[derive(Debug, Clone, Copy)]
pub(crate) struct OneFingerGestures {
    /// Movement of the touch since the previous frame, in logical pixels.
    pub motion: Vec2,
}

/// Holds information pertaining to two finger gestures
#[derive(Debug, Clone, Copy)]
pub(crate) struct TwoFingerGestures {
    /// Movement of both touches since the previous frame, measured as the
    /// change in their midpoint. When the midpoint holds still this is zero (or
    /// near zero), as it is during a pinch.
    pub motion:   Vec2,
    /// Change in the distance between the two touches since the previous
    /// frame. Use this to implement pinch gestures.
    pub pinch:    f32,
    /// Change in the angle of the line joining the two touches since the
    /// previous frame. Positive values are clockwise.
    #[allow(
        dead_code,
        reason = "computed but not yet wired — planned for touch-based camera roll"
    )]
    pub rotation: f32,
}

/// The pressed touches from the current and previous frames, which
/// [`TouchTracker::get_touch_gestures`] differences into a gesture.
#[derive(Resource, Default, Debug)]
pub(crate) struct TouchTracker {
    current_pressed:  (Option<Touch>, Option<Touch>),
    previous_pressed: (Option<Touch>, Option<Touch>),
}

impl TouchTracker {
    /// Returns the touch gesture for this frame.
    pub(crate) fn get_touch_gestures(&self) -> TouchGestures {
        // The arms below match only when the previous and current frames hold the same number
        // of touches, so the frame on which the touch count changes returns
        // `TouchGestures::None`.
        match (self.current_pressed, self.previous_pressed) {
            // One finger
            ((Some(curr), None), (Some(prev), None)) => {
                let current_position = curr.position();
                let previous_position = prev.position();

                let motion = current_position - previous_position;

                TouchGestures::OneFinger(OneFingerGestures { motion })
            },
            // Two fingers
            ((Some(curr1), Some(curr2)), (Some(prev1), Some(prev2))) => {
                let current_first_position = curr1.position();
                let current_second_position = curr2.position();
                let previous_first_position = prev1.position();
                let previous_second_position = prev2.position();

                // Move
                let current_midpoint = current_first_position.midpoint(current_second_position);
                let previous_midpoint = previous_first_position.midpoint(previous_second_position);
                let motion = current_midpoint - previous_midpoint;

                // Pinch
                let current_distance = current_first_position.distance(current_second_position);
                let previous_distance = previous_first_position.distance(previous_second_position);
                let pinch = current_distance - previous_distance;

                // Rotate
                let previous_vector = previous_second_position - previous_first_position;
                let current_vector = current_second_position - current_first_position;
                let previous_angle_from_negative_y = previous_vector.angle_to(Vec2::NEG_Y);
                let current_angle_from_negative_y = current_vector.angle_to(Vec2::NEG_Y);
                let previous_angle_from_positive_y = previous_vector.angle_to(Vec2::Y);
                let current_angle_from_positive_y = current_vector.angle_to(Vec2::Y);
                let rotation_from_negative_y =
                    current_angle_from_negative_y - previous_angle_from_negative_y;
                let rotation_from_positive_y =
                    current_angle_from_positive_y - previous_angle_from_positive_y;
                // Vec2::angle_between reports the angle between -1deg and +1deg as 358deg, where
                // the wanted answer is +2deg (or -2deg if swapped). So two angles are computed,
                // one from UP and one from DOWN, and the one with the smaller absolute value is
                // used. That keeps the result continuous when the two touches swap sides (touch
                // 1's X position going from less than touch 2's to greater).
                let rotation = if rotation_from_negative_y.abs() < rotation_from_positive_y.abs() {
                    rotation_from_negative_y
                } else {
                    rotation_from_positive_y
                };

                TouchGestures::TwoFinger(TwoFingerGestures {
                    motion,
                    pinch,
                    rotation,
                })
            },
            // Zero fingers, three+ fingers, or mismatched counts
            _ => TouchGestures::None,
        }
    }
}

/// Moves the previously pressed touches into `previous_pressed` and records
/// this frame's pressed touches in the [`TouchTracker`] resource. Frames with
/// three or more touches leave the tracker unchanged.
pub(super) fn touch_tracker(touches: Res<Touches>, mut touch_tracker: ResMut<TouchTracker>) {
    let pressed: Vec<&Touch> = touches.iter().collect();

    match pressed.len() {
        0 => {
            touch_tracker.current_pressed = (None, None);
            touch_tracker.previous_pressed = (None, None);
        },
        1 => {
            touch_tracker.previous_pressed = touch_tracker.current_pressed;
            touch_tracker.current_pressed = (Some(*pressed[0]), None);
        },
        2 => {
            touch_tracker.previous_pressed = touch_tracker.current_pressed;
            touch_tracker.current_pressed = (Some(*pressed[0]), Some(*pressed[1]));
        },
        _ => {},
    }
}
