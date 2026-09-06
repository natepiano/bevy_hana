use bevy_ecs::prelude::Component;
use bevy_ecs::prelude::ReflectComponent;
use bevy_ecs::prelude::SystemSet;
use bevy_math::Vec3;
use bevy_platform::collections::HashMap;
use bevy_reflect::Reflect;
use bevy_reflect::std_traits::ReflectDefault;
use hana_kana::Displacement;
use hana_kana::Orientation;

use crate::AnchorSite;

/// Local-frame resolver input for an anchored entity.
///
/// `AnchorPose` is a separate component from `Transform`: animation systems
/// write `AnchorPose`, and resolver systems convert it into a `Transform`
/// later. Two components keep animation writes and resolver writes off the same
/// component, where the stored value would mean two different things.
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
    /// The hinge driver turns each [`Hinge`](crate::Hinge)
    /// into an [`AnchorPose`].
    ///
    /// [`HingePlugin`](crate::HingePlugin) nests this set inside
    /// [`Self::AnimatePose`]. Order pose-writing systems against this set rather
    /// than against the system itself: an ordering against a set holds whether
    /// or not the set is populated, while an ordering against a system that no
    /// plugin registered is silently discarded.
    HingeToPose,
    /// Resolver systems read geometry, relations, and pose, then write transforms.
    Resolve,
}
