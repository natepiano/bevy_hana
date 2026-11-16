//! Zero-cost newtype wrappers around Bevy math primitives, plus the lossy
//! numeric cast traits.
//!
//! `Position`, `Velocity`, `Displacement`, `ScreenPosition`, and
//! `Orientation` `Deref` to their inner type for field and method access.
//! `Angle` wraps `f32` and reads through `Angle::radians` instead.

mod cast;
#[cfg(feature = "math")]
mod screen_position;
#[cfg(feature = "math")]
mod space;

pub use cast::ToF32;
pub use cast::ToF64;
pub use cast::ToI32;
pub use cast::ToI64;
pub use cast::ToU8;
pub use cast::ToU16;
pub use cast::ToU32;
pub use cast::ToUsize;
#[cfg(feature = "math")]
pub use screen_position::ScreenPosition;
#[cfg(feature = "math")]
pub use space::Angle;
#[cfg(feature = "math")]
pub use space::AngleError;
#[cfg(feature = "math")]
pub use space::Displacement;
#[cfg(feature = "math")]
pub use space::Orientation;
#[cfg(feature = "math")]
pub use space::OrientationError;
#[cfg(feature = "math")]
pub use space::Position;
#[cfg(feature = "math")]
pub use space::Velocity;
