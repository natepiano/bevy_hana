use core::fmt;
use core::fmt::Display;
use core::fmt::Formatter;

use bevy_kana::Position;

/// Successful fit output: camera orbit radius and centered focus point.
#[derive(Debug, Clone, Copy)]
pub struct FitSolution {
    /// The optimal orbital radius.
    pub radius: f32,
    /// The centered focus point.
    pub focus:  Position,
}

/// Explicit fit calculation failures.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FitError {
    /// Camera viewport size/aspect ratio is unavailable.
    NoViewport,
    /// All candidate fits projected points behind the camera.
    PointsBehindCamera,
    /// Projection variant is not supported (e.g. `Projection::Custom`).
    UnsupportedProjection,
}

impl Display for FitError {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoViewport => write!(f, "camera viewport size is unavailable"),
            Self::PointsBehindCamera => {
                write!(f, "all candidate fits project points behind camera")
            },
            Self::UnsupportedProjection => write!(f, "projection variant is not supported"),
        }
    }
}
