//! Zero-cost newtype wrappers around Bevy math primitives, plus the lossy
//! numeric cast traits.
//!
//! [`Position`], [`Velocity`], [`Displacement`], [`ScreenPosition`], and
//! [`Orientation`] `Deref` to their inner type for field and method access.
//! [`Angle`] wraps `f32` and reads through [`Angle::radians`] instead.

mod cast;
mod screen_position;
mod space;

pub use cast::ToF32;
pub use cast::ToF64;
pub use cast::ToI32;
pub use cast::ToU8;
pub use cast::ToU16;
pub use cast::ToU32;
pub use cast::ToUsize;
pub use screen_position::ScreenPosition;
pub use space::Angle;
pub use space::AngleError;
pub use space::Displacement;
pub use space::Orientation;
pub use space::OrientationError;
pub use space::Position;
pub use space::Velocity;
