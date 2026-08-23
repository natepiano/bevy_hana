//! Diegetic cascade attribute value types and their root defaults.

use core::mem::size_of;

use bevy::asset::Handle;
use bevy::log::warn_once;
use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;
use hana_kana::CascadeRootResource;

use super::constants::CASCADE_ATTRIBUTE_BYTES;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::layout::Unit;
use crate::render::AntiAlias;
use crate::render::HairlineFade;
use crate::render::HairlineWidth;
use crate::widgets::WidgetInteractivity;

/// Implements [`hana_kana::CascadeRootResource`] for a `Copy` attribute type
/// that is its own root resource. The type must derive `Resource`.
macro_rules! cascade_root_resource {
    ($name:ident) => {
        impl hana_kana::CascadeRootResource<$name> for $name {
            fn root(&self) -> Self { *self }

            fn from_root(root: Self) -> Self { root }
        }
    };
}

macro_rules! cascade_attribute {
    // Joins an already-declared value type (one whose own name is the
    // attribute, e.g. `AntiAlias`) to the cascade instead of minting a wrapper
    // struct. The type must derive `Resource`, `Copy`, `PartialEq`, `Debug`,
    // and `Reflect`.
    (existing $name:ident, default = $default:expr) => {
        cascade_root_resource!($name);

        impl $crate::cascade::resolved::CascadeRoot for $name {
            type Root = Self;

            fn root_default() -> Self { $default }
        }
    };

    // Same, for an attribute whose root value is one field of a resource the
    // crate already exposes. That resource implements
    // `hana_kana::CascadeRootResource` where it is declared.
    (existing $name:ident, root = $root:ty, default = $default:expr) => {
        impl $crate::cascade::resolved::CascadeRoot for $name {
            type Root = $root;

            fn root_default() -> Self { $default }
        }
    };

    // Mints the wrapper struct, which doubles as its own root resource.
    ($(#[$meta:meta])* $name:ident($value:ty), default = $default:expr, eq) => {
        $(#[$meta])*
        ///
        /// Insert this as a resource to set the value every entity inherits
        /// unless something between it and the cascade root overrides it.
        #[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Reflect)]
        #[reflect(Resource)]
        pub struct $name(pub $value);

        cascade_root_resource!($name);

        impl $crate::cascade::resolved::CascadeRoot for $name {
            type Root = Self;

            fn root_default() -> Self { $name($default) }
        }
    };

    ($(#[$meta:meta])* $name:ident($value:ty), default = $default:expr) => {
        $(#[$meta])*
        ///
        /// Insert this as a resource to set the value every entity inherits
        /// unless something between it and the cascade root overrides it.
        #[derive(Resource, Clone, Copy, PartialEq, Debug, Reflect)]
        #[reflect(Resource)]
        pub struct $name(pub $value);

        cascade_root_resource!($name);

        impl $crate::cascade::resolved::CascadeRoot for $name {
            type Root = Self;

            fn root_default() -> Self { $name($default) }
        }
    };
}

cascade_attribute!(
    /// Text alpha-mode cascade attribute.
    TextAlpha(AlphaMode),
    default = AlphaMode::Blend,
    eq
);
cascade_attribute!(
    /// Font-unit cascade attribute.
    FontUnit(Unit),
    default = Unit::Meters
);
cascade_attribute!(
    /// HDR text coverage-bias cascade attribute.
    ///
    /// Analytic text automatically gains a bounded coverage adjustment as its
    /// projected em shrinks. `0.0` leaves that adjustment unchanged. Positive
    /// values make fractional glyph-edge pixels more opaque beyond it, which
    /// can compensate for dark text looking too thin when an HDR camera renders
    /// into a float target. Negative values make fractional edges thinner.
    HdrTextCoverageBias(f32),
    default = 0.0
);

const HDR_TEXT_COVERAGE_BIAS_MIN: f32 = -4.0;
const HDR_TEXT_COVERAGE_BIAS_MAX: f32 = 4.0;

impl HdrTextCoverageBias {
    /// No authored HDR compensation; the shader keeps its automatic
    /// screen-size adjustment.
    pub(crate) const NO_BIAS: Self = Self(0.0);

    /// Value sent to `PathRenderRecord::text_coverage_bias`.
    ///
    /// The public authored value is intentionally plain `f32` so it can be
    /// tuned live, including through reflection. The shader path clamps it to a
    /// bounded signed transfer and treats non-finite input as no compensation.
    #[must_use]
    pub(crate) fn shader_value(self) -> f32 {
        if self.0.is_finite() {
            self.0
                .clamp(HDR_TEXT_COVERAGE_BIAS_MIN, HDR_TEXT_COVERAGE_BIAS_MAX)
        } else {
            warn_once!(
                "HdrTextCoverageBias value {} is not finite; rendering text with automatic coverage adjustment only",
                self.0
            );
            0.0
        }
    }
}

/// Source-material handle cascade for SDF backgrounds, borders, and element surfaces.
///
/// `SdfMaterial` is authored source-material identity. It is not the batched
/// `SdfExtendedMaterial` render asset and not the migration-only
/// `LegacySdfExtendedMaterial` render asset.
///
/// Insert this as a resource to set the handle every entity inherits unless
/// something between it and the cascade root overrides it.
#[derive(Resource, Clone, PartialEq, Eq, Debug, Reflect)]
#[reflect(Resource)]
pub struct SdfMaterial(pub Handle<StandardMaterial>);

impl CascadeRootResource<Self> for SdfMaterial {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for SdfMaterial {
    type Root = Self;

    fn root_default() -> Self { Self(Handle::default()) }
}

const _: () = assert!(size_of::<SdfMaterial>() <= CASCADE_ATTRIBUTE_BYTES);

/// Source-material handle cascade for text runs.
///
/// `TextMaterial` resolves the authored `StandardMaterial` handle before
/// analytic text projection. It is not a Bevy render material asset type.
///
/// Insert this as a resource to set the handle every entity inherits unless
/// something between it and the cascade root overrides it.
#[derive(Resource, Clone, PartialEq, Eq, Debug, Reflect)]
#[reflect(Resource)]
pub struct TextMaterial(pub Handle<StandardMaterial>);

impl CascadeRootResource<Self> for TextMaterial {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for TextMaterial {
    type Root = Self;

    fn root_default() -> Self { Self(Handle::default()) }
}

const _: () = assert!(size_of::<TextMaterial>() <= CASCADE_ATTRIBUTE_BYTES);

/// Source-material handle cascade for panel-shape primitives.
///
/// `ShapeMaterial` resolves the authored `StandardMaterial` handle before
/// analytic panel-shape projection. It is not a Bevy render material asset type.
///
/// Insert this as a resource to set the handle every entity inherits unless
/// something between it and the cascade root overrides it.
#[derive(Resource, Clone, PartialEq, Eq, Debug, Reflect)]
#[reflect(Resource)]
pub struct ShapeMaterial(pub Handle<StandardMaterial>);

impl CascadeRootResource<Self> for ShapeMaterial {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for ShapeMaterial {
    type Root = Self;

    fn root_default() -> Self { Self(Handle::default()) }
}

const _: () = assert!(size_of::<ShapeMaterial>() <= CASCADE_ATTRIBUTE_BYTES);

// Lighting cascade attribute. Global default is `Lit` (world text); the
// screen-panel construction bridge overrides it to `Unlit`. Consumed by both
// glyph runs and panel lines.
cascade_attribute!(existing Lighting, default = Lighting::Lit);
// Diegetic shadow-casting cascade attribute. Global default follows Bevy mesh
// behavior: rendered content casts shadows unless a local override opts out.
cascade_attribute!(existing ShadowCasting, default = ShadowCasting::On);
// Glyph-shadow silhouette cascade attribute. Text casts its glyph silhouette
// when shadow casting is enabled unless a local override opts out.
cascade_attribute!(existing GlyphShadowMode, default = GlyphShadowMode::Cast);
// Sidedness cascade attribute. Global default is `BothSides` (world text);
// the screen-panel construction bridge overrides it to `FrontOnly`. Consumed by
// both glyph runs and panel lines.
cascade_attribute!(existing Sidedness, default = Sidedness::BothSides);
// Anti-alias mode cascade attribute. The `AntiAlias` resource is both the
// authored global and the cascade root default.
cascade_attribute!(existing AntiAlias, default = AntiAlias::Both);
// Hairline-fade cascade attribute. The root value is `HairlineWidth::fade`, so
// the one resource carries both the fade policy and the `logical_px` stroke
// floor that `sync_hairline_width` sends to `PathUniform::hairline_min_px`.
cascade_attribute!(existing HairlineFade, root = HairlineWidth, default = HairlineFade::Full);
// Widget interactivity defaults to enabled when no ECS or layout scope authors
// an override.
cascade_attribute!(
    existing WidgetInteractivity,
    default = WidgetInteractivity::Enabled
);

pub(crate) trait CascadeRoot: hana_kana::CascadeAttribute {
    /// Resource holding this attribute's app-wide root value.
    type Root: hana_kana::CascadeRootResource<Self>;

    fn root_default() -> Self;

    /// Combines a value authored lower in the chain with one authored above it.
    ///
    /// The default replaces the higher value outright.
    fn combine(lower: Self, _: &Self) -> Self { lower }
}

#[cfg(test)]
mod tests {
    use core::fmt::Debug;

    use bevy::prelude::App;
    use bevy::prelude::Assets;
    use bevy::prelude::Color;
    use bevy::prelude::MinimalPlugins;

    use super::*;
    use crate::cascade;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeFrom;
    use crate::cascade::Resolved;

    fn assert_replace_cascade<A>(panel_value: A, widget_value: A)
    where
        A: CascadeRoot + Debug,
    {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_plugins(cascade::cascade_plugin::<A>());

        let default_value = hana_kana::CascadeRootResource::root(app.world().resource::<A::Root>());
        let root = app.world_mut().spawn(Cascade::<A>::Inherit).id();
        let panel = app
            .world_mut()
            .spawn((Cascade::Override(panel_value), CascadeFrom::new(root)))
            .id();
        let widget = app
            .world_mut()
            .spawn((
                Cascade::Override(widget_value.clone()),
                CascadeFrom::new(panel),
            ))
            .id();

        let inheriting_root = app.world_mut().spawn(Cascade::<A>::Inherit).id();
        let inheriting_panel = app
            .world_mut()
            .spawn((Cascade::<A>::Inherit, CascadeFrom::new(inheriting_root)))
            .id();
        let inheriting_widget = app
            .world_mut()
            .spawn((Cascade::<A>::Inherit, CascadeFrom::new(inheriting_panel)))
            .id();

        app.update();

        assert_eq!(
            app.world()
                .get::<Resolved<A>>(widget)
                .map(|resolved| resolved.0.clone()),
            Some(widget_value),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<A>>(inheriting_widget)
                .map(|resolved| resolved.0.clone()),
            Some(default_value),
        );
    }

    #[test]
    fn replace_attributes_keep_first_override_and_fall_back_to_root_default() {
        assert_replace_cascade(TextAlpha(AlphaMode::Opaque), TextAlpha(AlphaMode::Blend));
        assert_replace_cascade(FontUnit(Unit::Points), FontUnit(Unit::Pixels));
        assert_replace_cascade(HdrTextCoverageBias(1.0), HdrTextCoverageBias(2.0));

        let mut materials = Assets::<StandardMaterial>::default();
        let panel_material = materials.add(StandardMaterial::from(Color::WHITE));
        let widget_material = materials.add(StandardMaterial::from(Color::BLACK));
        assert_replace_cascade(
            SdfMaterial(panel_material.clone()),
            SdfMaterial(widget_material.clone()),
        );
        assert_replace_cascade(
            TextMaterial(panel_material.clone()),
            TextMaterial(widget_material.clone()),
        );
        assert_replace_cascade(
            ShapeMaterial(panel_material),
            ShapeMaterial(widget_material),
        );

        assert_replace_cascade(Lighting::Lit, Lighting::Unlit);
        assert_replace_cascade(ShadowCasting::On, ShadowCasting::Off);
        assert_replace_cascade(GlyphShadowMode::Cast, GlyphShadowMode::None);
        assert_replace_cascade(Sidedness::BothSides, Sidedness::FrontOnly);
        assert_replace_cascade(AntiAlias::Both, AntiAlias::Off);
        assert_replace_cascade(HairlineFade::Full, HairlineFade::Fade { exponent: 2.0 });
        assert_replace_cascade(WidgetInteractivity::Enabled, WidgetInteractivity::Disabled);
    }
}
