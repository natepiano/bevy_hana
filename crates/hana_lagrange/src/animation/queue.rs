//! Camera movement authoring for retained camera journeys.
//!
//! [`CameraMove`] values describe validated destinations.
//! [`CameraSequence`](crate::CameraSequence) retains their order and the shared evaluator applies
//! the selected native or driver-owned journey.

use std::time::Duration;

use bevy::prelude::Component;
use bevy::prelude::Quat;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::ReflectDefault;
use bevy::prelude::Vec3;
use bevy::prelude::warn;
use bevy_kana::Displacement;
use bevy_kana::Easing;
use thiserror::Error;

use crate::constants::MILLIS_PER_SECOND;
use crate::operation::Focus;
use crate::operation::OrbitAngles;
use crate::operation::Position;
use crate::operation::Radius;
use crate::operation::Roll;

/// Which roll a free-flight camera holds when a retained move reaches its destination.
///
/// `InheritPrevious` carries the camera's current roll into the authored pose.
/// `OrbitCam` has no roll axis and ignores this choice.
#[derive(Clone, Copy, Debug, PartialEq, Reflect)]
pub enum FreeCamRollTarget {
    /// Hold the roll the camera already carries.
    InheritPrevious,
    /// Roll to this angle.
    Explicit(Roll),
}

/// Which authored form a [`CameraMove`] uses to describe its destination.
///
/// Both forms resolve to the same retained orbit parameters. A free-flight
/// journey uses the authored form to preserve its intended look direction.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Reflect)]
pub enum CameraMoveDestination {
    /// A world-space camera position looking at a focus point.
    LookAt,
    /// Orbit angles and a radius around a focus point.
    OrbitalLookAt,
}

/// The destination pose one retained move names, in the form its author chose.
///
/// `OrbitalLookAt` preserves yaw at extreme pitch angles where decomposing a
/// world-space `LookAt` position loses that information.
#[derive(Clone, Copy, Debug, PartialEq)]
enum AuthoredPose {
    LookAt {
        position: Position,
        focus:    Focus,
    },
    OrbitalLookAt {
        focus:        Focus,
        orbit_angles: OrbitAngles,
        radius:       Radius,
    },
}

/// One validated step in a retained camera journey.
///
/// Every field is private and every coordinate is semantic, so a constructed
/// move has finite pose data and a non-negative orbit radius. Reflection is
/// opaque because construction preserves those invariants.
#[derive(Clone, Debug, PartialEq, Reflect)]
#[reflect(opaque)]
pub struct CameraMove {
    authored_pose:        AuthoredPose,
    free_cam_roll_target: FreeCamRollTarget,
    duration:             Duration,
    easing:               Easing,
}

impl CameraMove {
    /// Creates a retained journey step to a world-space camera `position`
    /// looking at `focus`.
    ///
    /// Any [`Duration`] is valid, including zero, which applies the destination
    /// when the evaluator traverses the step.
    ///
    /// # Errors
    ///
    /// Returns [`CameraMoveError::NonFinitePosition`],
    /// [`CameraMoveError::NonFiniteFocus`], or
    /// [`CameraMoveError::NonFiniteRoll`] when its corresponding authored
    /// coordinate is NaN or infinite.
    pub fn try_to_look_at(
        position: Position,
        focus: Focus,
        free_cam_roll_target: FreeCamRollTarget,
        duration: Duration,
        easing: impl Into<Easing>,
    ) -> Result<Self, CameraMoveError> {
        if !position.0.is_finite() {
            return Err(CameraMoveError::NonFinitePosition { position });
        }
        if !focus.0.is_finite() {
            return Err(CameraMoveError::NonFiniteFocus { focus });
        }
        validate_roll(free_cam_roll_target)?;
        Ok(Self {
            authored_pose: AuthoredPose::LookAt { position, focus },
            free_cam_roll_target,
            duration,
            easing: easing.into(),
        })
    }

    /// Creates a retained journey step to `orbit_angles` and `radius` around
    /// `focus`.
    ///
    /// A zero radius is valid and places the camera at its focus.
    ///
    /// # Errors
    ///
    /// Returns [`CameraMoveError::NonFiniteFocus`],
    /// [`CameraMoveError::NonFiniteOrbitAngles`],
    /// [`CameraMoveError::NonFiniteRadius`], or
    /// [`CameraMoveError::NonFiniteRoll`] for non-finite authoring data, and
    /// [`CameraMoveError::NegativeRadius`] when the authored radius is behind
    /// the focus.
    pub fn try_to_orbital_look_at(
        focus: Focus,
        orbit_angles: OrbitAngles,
        radius: Radius,
        free_cam_roll_target: FreeCamRollTarget,
        duration: Duration,
        easing: impl Into<Easing>,
    ) -> Result<Self, CameraMoveError> {
        if !focus.0.is_finite() {
            return Err(CameraMoveError::NonFiniteFocus { focus });
        }
        if !orbit_angles.yaw.is_finite() || !orbit_angles.pitch.is_finite() {
            return Err(CameraMoveError::NonFiniteOrbitAngles { orbit_angles });
        }
        if !radius.0.is_finite() {
            return Err(CameraMoveError::NonFiniteRadius { radius });
        }
        if radius.0 < 0.0 {
            return Err(CameraMoveError::NegativeRadius { radius });
        }
        validate_roll(free_cam_roll_target)?;
        Ok(Self {
            authored_pose: AuthoredPose::OrbitalLookAt {
                focus,
                orbit_angles,
                radius,
            },
            free_cam_roll_target,
            duration,
            easing: easing.into(),
        })
    }

    /// Returns which form authored this retained destination.
    #[must_use]
    pub const fn destination(&self) -> CameraMoveDestination {
        match self.authored_pose {
            AuthoredPose::LookAt { .. } => CameraMoveDestination::LookAt,
            AuthoredPose::OrbitalLookAt { .. } => CameraMoveDestination::OrbitalLookAt,
        }
    }

    /// Returns this retained step's traversal duration.
    #[must_use]
    pub const fn duration(&self) -> Duration { self.duration }

    /// Returns this retained step's traversal duration in milliseconds.
    #[must_use]
    pub const fn duration_ms(&self) -> f32 { self.duration.as_secs_f32() * MILLIS_PER_SECOND }

    /// Returns the easing selected for this retained step.
    #[must_use]
    pub const fn easing(&self) -> &Easing { &self.easing }

    /// Returns the free-flight roll target for this retained step.
    #[must_use]
    pub const fn free_cam_roll_target(&self) -> FreeCamRollTarget { self.free_cam_roll_target }

    /// Returns the focus point this retained step looks at.
    #[must_use]
    pub const fn focus(&self) -> Focus {
        match self.authored_pose {
            AuthoredPose::LookAt { focus, .. } | AuthoredPose::OrbitalLookAt { focus, .. } => focus,
        }
    }

    /// Returns the world-space camera position at this step's destination.
    #[must_use]
    pub fn position(&self) -> Position {
        match self.authored_pose {
            AuthoredPose::LookAt { position, .. } => position,
            AuthoredPose::OrbitalLookAt {
                focus,
                orbit_angles,
                radius,
            } => {
                let yaw_rotation = Quat::from_axis_angle(Vec3::Y, orbit_angles.yaw);
                let pitch_rotation = Quat::from_axis_angle(Vec3::X, -orbit_angles.pitch);
                Position(focus.0 + yaw_rotation * pitch_rotation * Vec3::new(0.0, 0.0, radius.0))
            },
        }
    }

    /// Returns the orbit angles at this step's destination.
    #[must_use]
    pub fn orbit_angles(&self) -> OrbitAngles { self.orbital_parameters().0 }

    /// Returns the orbit radius at this step's destination.
    #[must_use]
    pub fn radius(&self) -> Radius { self.orbital_parameters().1 }

    fn orbital_parameters(&self) -> (OrbitAngles, Radius) {
        match self.authored_pose {
            AuthoredPose::LookAt { position, focus } => {
                let (yaw, pitch, radius) =
                    orbital_parameters_from_offset(Displacement(position.0 - focus.0));
                (OrbitAngles { yaw, pitch }, Radius(radius))
            },
            AuthoredPose::OrbitalLookAt {
                orbit_angles,
                radius,
                ..
            } => (orbit_angles, radius),
        }
    }
}

/// Invalid explicit input supplied to [`CameraMove`] construction.
///
/// Every variant rejects retained authoring before a journey can be built.
#[derive(Clone, Copy, Debug, Error, PartialEq, Reflect)]
#[reflect(opaque)]
#[non_exhaustive]
pub enum CameraMoveError {
    /// A camera position coordinate was NaN or infinite.
    #[error("camera move position {position:?} must be finite")]
    NonFinitePosition {
        /// Position supplied to the rejected move.
        position: Position,
    },
    /// A focus coordinate was NaN or infinite.
    #[error("camera move focus {focus:?} must be finite")]
    NonFiniteFocus {
        /// Focus supplied to the rejected move.
        focus: Focus,
    },
    /// A yaw or pitch angle was NaN or infinite.
    #[error("camera move orbit angles {orbit_angles:?} must be finite")]
    NonFiniteOrbitAngles {
        /// Orbit angles supplied to the rejected move.
        orbit_angles: OrbitAngles,
    },
    /// An orbit radius was NaN or infinite.
    #[error("camera move radius {radius:?} must be finite")]
    NonFiniteRadius {
        /// Radius supplied to the rejected move.
        radius: Radius,
    },
    /// An orbit radius named a point behind the focus.
    #[error("camera move radius {radius:?} must not be negative")]
    NegativeRadius {
        /// Radius supplied to the rejected move.
        radius: Radius,
    },
    /// An explicit free-flight roll angle was NaN or infinite.
    #[error("camera move roll {roll:?} must be finite")]
    NonFiniteRoll {
        /// Roll supplied to the rejected move.
        roll: Roll,
    },
}

/// Logs rejected retained authoring so the caller can skip its request.
pub(crate) fn warn_rejected_camera_move(error: &CameraMoveError) {
    warn!("camera move rejected: {error}");
}

const fn validate_roll(free_cam_roll_target: FreeCamRollTarget) -> Result<(), CameraMoveError> {
    match free_cam_roll_target {
        FreeCamRollTarget::InheritPrevious => Ok(()),
        FreeCamRollTarget::Explicit(roll) if roll.0.is_finite() => Ok(()),
        FreeCamRollTarget::Explicit(roll) => Err(CameraMoveError::NonFiniteRoll { roll }),
    }
}

/// Decomposes a camera position offset into retained orbit parameters.
///
/// The returned `(yaw, pitch, radius)` may lose yaw at ±PI/2 pitch because it
/// uses `atan2` to reconstruct the authored look direction.
pub(crate) fn orbital_parameters_from_offset(offset: Displacement) -> (f32, f32, f32) {
    let radius = offset.length();
    let yaw = offset.x.atan2(offset.z);
    let horizontal_distance = offset.x.hypot(offset.z);
    let pitch = offset.y.atan2(horizontal_distance);
    (yaw, pitch, radius)
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::time::Duration;

    use bevy::math::curve::easing::EaseFunction;

    use super::*;
    use crate::Easing;
    use crate::EasingCurve;
    use crate::EasingInput;
    use crate::EasingOutput;

    const AUTHORED_FOCUS: Vec3 = Vec3::new(1.0, 2.0, 3.0);
    const AUTHORED_ORBIT_ANGLES: OrbitAngles = OrbitAngles {
        yaw:   0.5,
        pitch: -0.25,
    };
    const AUTHORED_POSITION: Vec3 = Vec3::new(4.0, 5.0, 6.0);
    const AUTHORED_RADIUS: f32 = 7.0;
    const AUTHORED_ROLL: Roll = Roll(0.125);
    const MOVE_DURATION: Duration = Duration::from_secs(1);
    const POSITIVE_MOVE_DURATION: Duration = Duration::from_hours(24);

    type TestResult = Result<(), &'static str>;

    fn look_at_move(
        position: Vec3,
        focus: Vec3,
        free_cam_roll_target: FreeCamRollTarget,
    ) -> Result<CameraMove, CameraMoveError> {
        CameraMove::try_to_look_at(
            Position(position),
            Focus(focus),
            free_cam_roll_target,
            MOVE_DURATION,
            EaseFunction::Linear,
        )
    }

    fn orbital_move(
        focus: Vec3,
        orbit_angles: OrbitAngles,
        radius: f32,
    ) -> Result<CameraMove, CameraMoveError> {
        CameraMove::try_to_orbital_look_at(
            Focus(focus),
            orbit_angles,
            Radius(radius),
            FreeCamRollTarget::InheritPrevious,
            MOVE_DURATION,
            EaseFunction::Linear,
        )
    }

    #[test]
    fn look_at_rejects_a_non_finite_position() -> TestResult {
        let position = Vec3::new(f32::NAN, 0.0, 0.0);

        let Err(CameraMoveError::NonFinitePosition { position: rejected }) =
            look_at_move(position, AUTHORED_FOCUS, FreeCamRollTarget::InheritPrevious)
        else {
            return Err("a NaN position is rejected as non-finite");
        };
        assert!(rejected.0.x.is_nan());
        Ok(())
    }

    #[test]
    fn look_at_rejects_a_non_finite_focus() {
        let focus = Vec3::new(0.0, f32::INFINITY, 0.0);

        assert_eq!(
            look_at_move(AUTHORED_POSITION, focus, FreeCamRollTarget::InheritPrevious),
            Err(CameraMoveError::NonFiniteFocus {
                focus: Focus(focus),
            })
        );
    }

    #[test]
    fn either_constructor_rejects_a_non_finite_explicit_roll() -> TestResult {
        let roll = Roll(f32::NAN);

        let Err(CameraMoveError::NonFiniteRoll { roll: from_look_at }) = look_at_move(
            AUTHORED_POSITION,
            AUTHORED_FOCUS,
            FreeCamRollTarget::Explicit(roll),
        ) else {
            return Err("look-at construction accepts a non-finite explicit roll");
        };
        let Err(CameraMoveError::NonFiniteRoll { roll: from_orbital }) =
            CameraMove::try_to_orbital_look_at(
                Focus(AUTHORED_FOCUS),
                AUTHORED_ORBIT_ANGLES,
                Radius(AUTHORED_RADIUS),
                FreeCamRollTarget::Explicit(roll),
                Duration::ZERO,
                EaseFunction::Linear,
            )
        else {
            return Err("orbital construction accepts a non-finite explicit roll");
        };
        assert!(from_look_at.0.is_nan());
        assert!(from_orbital.0.is_nan());
        Ok(())
    }

    #[test]
    fn orbital_look_at_rejects_a_non_finite_focus() {
        let focus = Vec3::new(0.0, 0.0, f32::NEG_INFINITY);

        assert_eq!(
            orbital_move(focus, AUTHORED_ORBIT_ANGLES, AUTHORED_RADIUS),
            Err(CameraMoveError::NonFiniteFocus {
                focus: Focus(focus),
            })
        );
    }

    #[test]
    fn orbital_look_at_rejects_non_finite_orbit_angles() -> TestResult {
        let orbit_angles = OrbitAngles {
            yaw:   f32::NAN,
            pitch: 0.0,
        };

        let Err(CameraMoveError::NonFiniteOrbitAngles {
            orbit_angles: rejected,
        }) = orbital_move(AUTHORED_FOCUS, orbit_angles, AUTHORED_RADIUS)
        else {
            return Err("NaN orbit angles are rejected as non-finite");
        };
        assert!(rejected.yaw.is_nan());
        Ok(())
    }

    #[test]
    fn orbital_look_at_rejects_a_non_finite_radius() {
        let radius = f32::INFINITY;

        assert_eq!(
            orbital_move(AUTHORED_FOCUS, AUTHORED_ORBIT_ANGLES, radius),
            Err(CameraMoveError::NonFiniteRadius {
                radius: Radius(radius),
            })
        );
    }

    #[test]
    fn orbital_look_at_rejects_a_negative_radius() {
        let radius = -0.001;

        assert_eq!(
            orbital_move(AUTHORED_FOCUS, AUTHORED_ORBIT_ANGLES, radius),
            Err(CameraMoveError::NegativeRadius {
                radius: Radius(radius),
            })
        );
    }

    #[test]
    fn orbital_look_at_accepts_zero_radius_at_the_focus() -> TestResult {
        let Ok(camera_move) = orbital_move(AUTHORED_FOCUS, AUTHORED_ORBIT_ANGLES, 0.0) else {
            return Err("a zero radius puts the camera at its focus");
        };

        assert_eq!(camera_move.radius(), Radius(0.0));
        assert_eq!(camera_move.position(), Position(AUTHORED_FOCUS));
        Ok(())
    }

    #[test]
    fn look_at_round_trips_its_authored_semantic_coordinates() -> TestResult {
        let Ok(camera_move) = look_at_move(
            AUTHORED_POSITION,
            AUTHORED_FOCUS,
            FreeCamRollTarget::Explicit(AUTHORED_ROLL),
        ) else {
            return Err("a finite look-at pose is accepted");
        };

        assert_eq!(camera_move.destination(), CameraMoveDestination::LookAt);
        assert_eq!(camera_move.position(), Position(AUTHORED_POSITION));
        assert_eq!(camera_move.focus(), Focus(AUTHORED_FOCUS));
        assert_eq!(
            camera_move.free_cam_roll_target(),
            FreeCamRollTarget::Explicit(AUTHORED_ROLL)
        );
        assert_eq!(camera_move.duration(), MOVE_DURATION);
        Ok(())
    }

    #[test]
    fn orbital_look_at_round_trips_its_authored_semantic_coordinates() -> TestResult {
        let Ok(camera_move) = orbital_move(AUTHORED_FOCUS, AUTHORED_ORBIT_ANGLES, AUTHORED_RADIUS)
        else {
            return Err("a finite orbital pose is accepted");
        };

        assert_eq!(
            camera_move.destination(),
            CameraMoveDestination::OrbitalLookAt
        );
        assert_eq!(camera_move.focus(), Focus(AUTHORED_FOCUS));
        assert_eq!(camera_move.orbit_angles(), AUTHORED_ORBIT_ANGLES);
        assert_eq!(camera_move.radius(), Radius(AUTHORED_RADIUS));
        assert_eq!(
            camera_move.free_cam_roll_target(),
            FreeCamRollTarget::InheritPrevious
        );
        Ok(())
    }

    #[test]
    fn zero_and_positive_durations_are_accepted() -> TestResult {
        let Ok(instant) = CameraMove::try_to_orbital_look_at(
            Focus(AUTHORED_FOCUS),
            AUTHORED_ORBIT_ANGLES,
            Radius(AUTHORED_RADIUS),
            FreeCamRollTarget::InheritPrevious,
            Duration::ZERO,
            EaseFunction::Linear,
        ) else {
            return Err("a zero-duration retained step is accepted");
        };
        let Ok(positive) = CameraMove::try_to_look_at(
            Position(AUTHORED_POSITION),
            Focus(AUTHORED_FOCUS),
            FreeCamRollTarget::InheritPrevious,
            POSITIVE_MOVE_DURATION,
            EaseFunction::Linear,
        ) else {
            return Err("a positive-duration retained step is accepted");
        };

        assert_eq!(instant.duration(), Duration::ZERO);
        assert_eq!(positive.duration(), POSITIVE_MOVE_DURATION);
        Ok(())
    }

    #[test]
    fn stock_easing_is_stored_as_authored() -> TestResult {
        let Ok(camera_move) = orbital_move(AUTHORED_FOCUS, AUTHORED_ORBIT_ANGLES, AUTHORED_RADIUS)
        else {
            return Err("a finite orbital pose is accepted");
        };

        assert_eq!(camera_move.easing(), &Easing::Bevy(EaseFunction::Linear));
        Ok(())
    }

    #[test]
    fn owned_lookup_easing_is_stored_as_authored() -> TestResult {
        let curve = authored_curve();
        let Ok(camera_move) = CameraMove::try_to_orbital_look_at(
            Focus(AUTHORED_FOCUS),
            AUTHORED_ORBIT_ANGLES,
            Radius(AUTHORED_RADIUS),
            FreeCamRollTarget::InheritPrevious,
            MOVE_DURATION,
            curve.clone(),
        ) else {
            return Err("a finite orbital pose is accepted");
        };

        assert_eq!(camera_move.easing(), &Easing::Curve(curve));
        Ok(())
    }

    fn authored_curve() -> EasingCurve {
        EasingCurve::builder()
            .linear()
            .knot(authored_input(0.0), authored_output(0.0))
            .knot(authored_input(0.5), authored_output(0.25))
            .knot(authored_input(1.0), authored_output(1.0))
            .try_build()
            .expect("the authored lookup knots are valid")
    }

    fn authored_input(value: f32) -> EasingInput {
        EasingInput::try_new(value).expect("the authored lookup input is normalized")
    }

    fn authored_output(value: f32) -> EasingOutput {
        EasingOutput::try_new(value).expect("the authored lookup output is finite")
    }
}

/// Controls how physical input affects an active retained camera journey.
///
/// This component is independent from [`AnimationConflictPolicy`](crate::AnimationConflictPolicy):
/// it governs physical camera input, while that policy governs a new retained
/// animation request.
#[derive(Component, Reflect, Default, Clone, Copy, Debug, PartialEq, Eq)]
#[reflect(Component, Default)]
pub enum CameraInputInterruptBehavior {
    /// Suppress camera input and continue the retained journey.
    #[default]
    Ignore,
    /// Stop at the current retained pose and emit cancellation lifecycle events.
    Cancel,
    /// Traverse to the retained sequence end and emit normal completion events.
    Complete,
}
