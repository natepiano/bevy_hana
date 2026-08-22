//! Typestate builders chained from [`sprinkle_example`](crate::sprinkle_example).
//!
//! `SprinkleBuilder` is the main chain; `PrimitiveBuilder`, `CameraHomeBuilder`,
//! and `TitleBarBuilder` are nested builders that return to the main chain
//! when a non-self method is called.

mod camera_home;
mod primitive;
mod sprinkle;
mod studio_lighting;
mod title_bar;

pub use camera_home::CameraHomeBuilder;
pub use primitive::PrimitiveBuilder;
pub use sprinkle::AssetRootPending;
pub use sprinkle::BaselineInstalled;
pub use sprinkle::NoOrbitCam;
pub use sprinkle::SprinkleBuilder;
pub use sprinkle::WithOrbitCam;
pub use studio_lighting::StudioLightingBuilder;
pub use title_bar::TitleBarBuilder;
