use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::ReflectComponent;
use bevy_ecs::prelude::SystemSet;
use bevy_kana::Displacement;
use bevy_kana::Orientation;
use bevy_math::Vec3;
use bevy_platform::collections::HashMap;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;

use crate::AnchorSite;

/// Local-frame resolver input for an anchored entity.
///
/// `AnchorPose` is deliberately not `Transform`: animation systems write
/// `AnchorPose`, and resolver systems convert it into a `Transform` later.
/// Keeping those components separate prevents animation systems and resolver
/// systems from writing the same component for different meanings.
/// [`Hinge`](crate::Hinge) is an `AnchorPose` driver; remove `Hinge` when
/// another system should write `AnchorPose` directly.
#[derive(Component, Clone, Copy, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, PartialEq, Debug, Default, opaque)]
pub struct AnchorPose {
    /// Finite normalized local rotation around the resolved anchor.
    pub rotation:    Orientation,
    /// Local target-frame translation from the resolved anchor.
    pub translation: Displacement,
}

/// Optional per-entity cache of resolved world-space anchor points.
///
/// Resolver systems recompute `ResolvedAnchorWorld` every frame for entities
/// carrying it, so gizmos and UI can read cached points without owning their
/// lifetime.
#[derive(Component, Clone, Debug, Default, PartialEq, Reflect)]
#[reflect(Component, Default)]
pub struct ResolvedAnchorWorld {
    /// World-space anchor points keyed by provider-authored ids.
    pub points: HashMap<AnchorSite, Vec3>,
}

/// System sets used by anchor providers, animation drivers, and resolvers.
///
/// Consumers own anchor-system wiring. [`FoldPlugin`](crate::FoldPlugin) is the
/// folding-only exception.
#[derive(SystemSet, Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub enum AnchorSystems {
    /// Providers write [`ResolvedAnchorGeometry`](crate::ResolvedAnchorGeometry).
    FillGeometry,
    /// Drivers write [`AnchorPose`], hinge data, or source transforms.
    AnimatePose,
    /// Resolver systems read geometry, relations, and pose, then write transforms.
    Resolve,
}
