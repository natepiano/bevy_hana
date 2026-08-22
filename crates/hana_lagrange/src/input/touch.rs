use bevy::input::touch::Touch;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Touches;
use bevy::prelude::Vec2;

/// Holds information about current mobile gestures
#[derive(Debug, Clone)]
pub(crate) enum TouchGestures {
    /// No mobile gestures
    None,
    /// One finger mobile gestures
    OneFinger(OneFingerGestures),
    /// Two finger mobile gestures
    TwoFinger(TwoFingerGestures),
}

/// Holds information pertaining to one finger gestures
#[derive(Debug, Clone, Copy)]
pub(crate) struct OneFingerGestures {
    /// The delta movement of the mobile
    pub motion: Vec2,
}

/// Holds information pertaining to two finger gestures
#[derive(Debug, Clone, Copy)]
pub(crate) struct TwoFingerGestures {
    /// The delta movement of both touches.
    /// Uses the midpoint between the touches to calculate movement. Thus, if the midpoint doesn't
    /// move then this will be zero (or close to zero), like when pinching.
    pub motion:   Vec2,
    /// The delta distance between both touches.
    /// Use this to implement pinch gestures.
    pub pinch:    f32,
    /// The delta angle of the two touches.
    /// Positive values correspond to rotating clockwise.
    #[allow(
        dead_code,
        reason = "computed but not yet wired — planned for touch-based camera roll"
    )]
    pub rotation: f32,
}

/// Stores current and previous frame mobile data, and provides a method to get mobile gestures
#[derive(Resource, Default, Debug)]
pub(crate) struct TouchTracker {
    current_pressed:  (Option<Touch>, Option<Touch>),
    previous_pressed: (Option<Touch>, Option<Touch>),
}

impl TouchTracker {
    /// Calculate and return mobile gesture data for this frame
    pub(crate) fn get_touch_gestures(&self) -> TouchGestures {
        // The below matches only match when the previous and current frames have the same number
        // of touches. This means that when the number of touches changes, there's one frame
        // where this will return `TouchGestures::None`. From my testing, this does not result
        // in any adverse effects.
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
                // The angle between -1deg and +1deg is 358deg according to Vec2::angle_between,
                // but we want the answer to be +2deg (or -2deg if swapped). Therefore, we calculate
                // two angles - one from UP and one from DOWN, and use the one with the smallest
                // absolute value. This is necessary to get a predictable result when the two
                // touches swap sides (i.e. mobile 1's X position being less than
                // the other, to the other way round).
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

/// Read touch input and save it in `TouchTracker` resource for easy consumption by the main system
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
