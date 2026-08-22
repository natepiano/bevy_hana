use bevy::prelude::Reflect;

/// Screen edge identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub(in crate::fit::overlay) enum Edge {
    /// Left screen edge.
    Left,
    /// Right screen edge.
    Right,
    /// Top screen edge.
    Top,
    /// Bottom screen edge.
    Bottom,
}
