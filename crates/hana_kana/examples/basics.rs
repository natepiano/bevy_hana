//! Demonstrates core `hana_kana` types and their compile-time safety.
//!
//! Run with: `cargo run --example basics`

use std::error::Error;
use std::f32::consts::FRAC_PI_2;
use std::f32::consts::TAU;

use bevy::math::Quat;
use bevy::math::Vec3;
use hana_kana::Angle;
use hana_kana::Displacement;
use hana_kana::Orientation;
use hana_kana::Position;
use hana_kana::ScreenPosition;
use hana_kana::Velocity;

// demo fixtures
const CENTROID_DIVISOR: f32 = 2.0;
const CURSOR_POSITION: ScreenPosition = ScreenPosition::new(640.0, 480.0);
const DEMO_DISPLACEMENT: Displacement = Displacement::new(5.0, 0.0, 0.0);
const DEMO_POSITION: Position = Position::new(10.0, 0.0, 5.0);
const DEMO_VELOCITY: Velocity = Velocity::new(1.0, 0.0, -0.5);
const END_POSITION: Position = Position::new(3.0, 4.0, 0.0);
const FRAME_TIME_DELTA: f32 = 0.016;
const MULTI_TURN_RADIANS: f32 = 2.5 * TAU;
const OFFSET_POSITION: ScreenPosition = ScreenPosition::new(10.0, -5.0);
const ROUNDTRIP_POSITION: Vec3 = Vec3::new(1.0, 2.0, 3.0);
const SLERP_FACTOR: f32 = 0.5;
const START_POSITION: Position = Position::new(1.0, 0.0, 0.0);
const STEP_VELOCITY: Velocity = Velocity::new(2.0, 0.0, 0.0);

fn main() -> Result<(), Box<dyn Error>> {
    println!("=== Semantic types: compile-time safety ===\n");

    // `Position` arithmetic stays type-safe.
    let start_position = START_POSITION;
    let end_position = END_POSITION;
    let centroid = (start_position + end_position) / CENTROID_DIVISOR;
    println!("Centroid of {start_position:?} and {end_position:?}: {centroid:?}");

    // Cross-type mixing is a compile error — uncomment to see:
    // let velocity = Velocity(Vec3::new(1.0, 0.0, 0.0));
    // let bad = start_position + velocity;  // ERROR: expected `Position`, found `Velocity`

    println!("\n=== Deref: transparent access to inner type ===\n");

    let position = DEMO_POSITION;
    let velocity = DEMO_VELOCITY;
    println!("position.x = {}", position.x);
    println!("velocity.length() = {}", velocity.length());

    // Use `Deref` to access `Vec3` methods like dot product
    println!("dot(position, velocity) = {}", position.dot(*velocity));

    println!("\n=== into_inner / From / Into: Bevy API interop ===\n");

    // Use `Position::into_inner` to pass a raw `Vec3` to Bevy APIs.
    let vec3: Vec3 = position.into_inner();
    println!("into_inner: {vec3:?}");

    // `From`/`Into` conversions work both ways
    let roundtrip_position: Position = ROUNDTRIP_POSITION.into();
    let vec3: Vec3 = roundtrip_position.into();
    println!("roundtrip: {vec3:?}");

    println!("\n=== Angle: signed, unwrapped angular displacement ===\n");

    let angle = Angle::from_radians(MULTI_TURN_RADIANS)?;
    println!("Preserved multi-turn radians: {}", angle.radians());

    println!("\n=== Orientation: normalized rotation wrapper ===\n");

    let quarter_turn_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
    let rotated = quarter_turn_orientation * Vec3::X;
    println!("X rotated 90° around Y: {rotated:?}");

    // `Orientation` composition.
    let double_orientation = quarter_turn_orientation * quarter_turn_orientation;
    let result = double_orientation * Vec3::X;
    println!("X rotated 180° around Y: {result:?}");

    // `Orientation::inverse` restores identity.
    let undone_orientation = quarter_turn_orientation * quarter_turn_orientation.inverse();
    let identity_result = undone_orientation * Vec3::X;
    println!("Rotation * inverse = identity: {identity_result:?}");

    // `Orientation::slerp` interpolation.
    let start_orientation = Orientation::try_from(Quat::IDENTITY)?;
    let end_orientation = Orientation::try_from(Quat::from_rotation_y(FRAC_PI_2))?;
    let halfway_orientation = start_orientation.slerp(end_orientation, SLERP_FACTOR)?;
    let slerp_result = halfway_orientation * Vec3::X;
    println!("Slerp halfway (0° to 90°): {slerp_result:?}");

    println!("\n=== Displacement and Velocity ===\n");

    let displacement = DEMO_DISPLACEMENT;
    let velocity = STEP_VELOCITY;

    let total = displacement + displacement;
    let combined = velocity + velocity;
    println!("Double displacement: {total:?}");
    println!("Combined velocity: {combined:?}");

    // `Velocity` scales by `FRAME_TIME_DELTA` for per-frame movement.
    let time_delta = FRAME_TIME_DELTA;
    let frame_velocity = velocity * time_delta;
    println!("Velocity * time_delta({time_delta}): {frame_velocity:?}");

    println!("\n=== ScreenPosition: 2D pixel-space ===\n");

    let cursor = CURSOR_POSITION;
    let offset = OFFSET_POSITION;
    let moved = cursor + offset;
    println!("Cursor {cursor:?} + offset {offset:?} = {moved:?}");

    Ok(())
}
