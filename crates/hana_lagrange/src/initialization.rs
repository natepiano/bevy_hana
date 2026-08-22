//! Shared camera initialization lifecycle.

use bevy::prelude::Reflect;

/// How a camera's start pose is established on the first controller pass.
#[derive(Clone, PartialEq, Eq, Debug, Reflect, Copy, Default)]
pub enum Initialization {
    /// Derive the start pose from the entity's `Transform`.
    #[default]
    FromTransform,
    /// Use the pose already seeded into the camera operations, then write the
    /// `Transform` to match.
    FromPose,
    /// Initialized; the controller drives the camera operations directly.
    ///
    /// Spawn a camera already `Active` to skip init and smoothly animate in from
    /// the default pose to whatever targets you set.
    Active,
}
