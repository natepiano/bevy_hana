use bevy::pbr::StandardMaterial;
use bevy::prelude::AlphaMode;
use bevy::prelude::Entity;
use bevy::prelude::EntityCommands;
use bevy::prelude::Handle;
use bevy::prelude::World;
use hana_kana::CascadeAttribute;
use hana_kana::CascadeEntityCommandsExt as _;

use super::CascadeRoot;
pub use super::resolved::FontUnit;
pub use super::resolved::HdrTextCoverageBias;
pub use super::resolved::SdfMaterial;
pub use super::resolved::ShapeMaterial;
pub use super::resolved::TextAlpha;
pub use super::resolved::TextMaterial;
use crate::layout::GlyphShadowMode;
use crate::layout::Lighting;
use crate::layout::ShadowCasting;
use crate::layout::Sidedness;
use crate::layout::Unit;
use crate::render::AntiAlias;
use crate::render::HairlineFade;
use crate::widgets::IntoAppearance;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetInteractivity;
use crate::widgets::WidgetPressedAppearance;

/// Typed cascade commands for entity-local authored values.
///
/// `override_*` writes this entity's authored value. `inherit_*` writes
/// `Cascade::Inherit` so the entity remains participating and resolves through
/// `CascadeFrom` or the attribute's root resource, such as
/// [`TextAlpha`]. Schedule writes before
/// [`CascadeSet::Propagate`](super::CascadeSet::Propagate) and reads after it
/// for same-frame observation.
pub trait CascadeEntityCommandsExt {
    /// Authors this entity's durable widget interactivity.
    ///
    /// Use this for panels and other ECS-authored ancestors. Reified widget
    /// entities are derived from their panel's [`LayoutTree`](crate::LayoutTree);
    /// use [`PanelWidgetWriter`](crate::PanelWidgetWriter) for durable
    /// widget-local edits.
    fn override_widget_interactivity(&mut self, value: WidgetInteractivity) -> &mut Self;

    /// Makes this entity durably inherit widget interactivity.
    ///
    /// Use this for panels and other ECS-authored ancestors. Reified widget
    /// entities are derived from their panel's [`LayoutTree`](crate::LayoutTree);
    /// use [`PanelWidgetWriter`](crate::PanelWidgetWriter) for durable
    /// widget-local edits.
    fn inherit_widget_interactivity(&mut self) -> &mut Self;

    /// Authors this entity's durable hovered widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn override_widget_hovered_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self;

    /// Makes this entity durably inherit the hovered widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn inherit_widget_hovered_appearance(&mut self) -> &mut Self;

    /// Authors this entity's durable pressed widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn override_widget_pressed_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self;

    /// Makes this entity durably inherit the pressed widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn inherit_widget_pressed_appearance(&mut self) -> &mut Self;

    /// Authors this entity's durable focused widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn override_widget_focused_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self;

    /// Makes this entity durably inherit the focused widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn inherit_widget_focused_appearance(&mut self) -> &mut Self;

    /// Authors this entity's durable disabled widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn override_widget_disabled_appearance(&mut self, appearance: impl IntoAppearance)
    -> &mut Self;

    /// Makes this entity durably inherit the disabled widget appearance.
    ///
    /// Use this for panels and other ECS-authored ancestors. A direct command
    /// on a reified widget entity may be replaced by reification; author a
    /// durable widget or part appearance in the panel's
    /// [`LayoutTree`](crate::LayoutTree).
    fn inherit_widget_disabled_appearance(&mut self) -> &mut Self;

    /// Author this entity's text alpha mode.
    fn override_text_alpha(&mut self, alpha_mode: AlphaMode) -> &mut Self;

    /// Make this entity inherit text alpha mode.
    fn inherit_text_alpha(&mut self) -> &mut Self;

    /// Author this entity's font unit.
    fn override_font_unit(&mut self, unit: Unit) -> &mut Self;

    /// Make this entity inherit font unit.
    fn inherit_font_unit(&mut self) -> &mut Self;

    /// Author this entity's HDR text coverage bias.
    ///
    /// `0.0` leaves analytic text coverage unchanged. Positive values make
    /// fractional glyph edges more opaque and are useful for dark text that
    /// looks too thin under HDR. Negative values make edges thinner.
    fn override_hdr_text_coverage_bias(&mut self, bias: f32) -> &mut Self;

    /// Make this entity inherit HDR text coverage bias.
    fn inherit_hdr_text_coverage_bias(&mut self) -> &mut Self;

    /// Author this entity's lighting mode.
    fn override_lighting(&mut self, lighting: Lighting) -> &mut Self;

    /// Make this entity inherit lighting mode.
    fn inherit_lighting(&mut self) -> &mut Self;

    /// Author this entity's shadow-casting policy.
    fn override_shadow_casting(&mut self, shadow_casting: ShadowCasting) -> &mut Self;

    /// Make this entity inherit shadow-casting policy.
    fn inherit_shadow_casting(&mut self) -> &mut Self;

    /// Author this entity's glyph shadow mode.
    fn override_glyph_shadow_mode(&mut self, mode: GlyphShadowMode) -> &mut Self;

    /// Make this entity inherit glyph shadow mode.
    fn inherit_glyph_shadow_mode(&mut self) -> &mut Self;

    /// Author this entity's sidedness.
    fn override_sidedness(&mut self, sidedness: Sidedness) -> &mut Self;

    /// Make this entity inherit sidedness.
    fn inherit_sidedness(&mut self) -> &mut Self;

    /// Author this entity's anti-alias mode.
    fn override_anti_alias(&mut self, anti_alias: AntiAlias) -> &mut Self;

    /// Make this entity inherit anti-alias mode.
    fn inherit_anti_alias(&mut self) -> &mut Self;

    /// Author this entity's hairline fade policy.
    fn override_hairline_fade(&mut self, fade: HairlineFade) -> &mut Self;

    /// Make this entity inherit hairline fade policy.
    fn inherit_hairline_fade(&mut self) -> &mut Self;

    /// Author this entity's SDF surface material source handle.
    fn override_sdf_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self;

    /// Make this entity inherit SDF surface material source handle.
    fn inherit_sdf_material(&mut self) -> &mut Self;

    /// Author this entity's text material source handle.
    fn override_text_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self;

    /// Make this entity inherit text material source handle.
    fn inherit_text_material(&mut self) -> &mut Self;

    /// Author this entity's panel-shape material source handle.
    fn override_shape_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self;

    /// Make this entity inherit panel primitive material source handle.
    fn inherit_shape_material(&mut self) -> &mut Self;
}

impl CascadeEntityCommandsExt for EntityCommands<'_> {
    fn override_widget_interactivity(&mut self, value: WidgetInteractivity) -> &mut Self {
        apply_cascade_override(self, value)
    }

    fn inherit_widget_interactivity(&mut self) -> &mut Self {
        remove_cascade_override::<WidgetInteractivity>(self)
    }

    fn override_widget_hovered_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self {
        apply_cascade_override(self, WidgetHoveredAppearance::new(appearance))
    }

    fn inherit_widget_hovered_appearance(&mut self) -> &mut Self {
        remove_cascade_override::<WidgetHoveredAppearance>(self)
    }

    fn override_widget_pressed_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self {
        apply_cascade_override(self, WidgetPressedAppearance::new(appearance))
    }

    fn inherit_widget_pressed_appearance(&mut self) -> &mut Self {
        remove_cascade_override::<WidgetPressedAppearance>(self)
    }

    fn override_widget_focused_appearance(&mut self, appearance: impl IntoAppearance) -> &mut Self {
        apply_cascade_override(self, WidgetFocusedAppearance::new(appearance))
    }

    fn inherit_widget_focused_appearance(&mut self) -> &mut Self {
        remove_cascade_override::<WidgetFocusedAppearance>(self)
    }

    fn override_widget_disabled_appearance(
        &mut self,
        appearance: impl IntoAppearance,
    ) -> &mut Self {
        apply_cascade_override(self, WidgetDisabledAppearance::new(appearance))
    }

    fn inherit_widget_disabled_appearance(&mut self) -> &mut Self {
        remove_cascade_override::<WidgetDisabledAppearance>(self)
    }

    fn override_text_alpha(&mut self, alpha_mode: AlphaMode) -> &mut Self {
        apply_cascade_override(self, TextAlpha(alpha_mode))
    }

    fn inherit_text_alpha(&mut self) -> &mut Self { remove_cascade_override::<TextAlpha>(self) }

    fn override_font_unit(&mut self, unit: Unit) -> &mut Self {
        apply_cascade_override(self, FontUnit(unit))
    }

    fn inherit_font_unit(&mut self) -> &mut Self { remove_cascade_override::<FontUnit>(self) }

    fn override_hdr_text_coverage_bias(&mut self, bias: f32) -> &mut Self {
        apply_cascade_override(self, HdrTextCoverageBias(bias))
    }

    fn inherit_hdr_text_coverage_bias(&mut self) -> &mut Self {
        remove_cascade_override::<HdrTextCoverageBias>(self)
    }

    fn override_lighting(&mut self, lighting: Lighting) -> &mut Self {
        apply_cascade_override(self, lighting)
    }

    fn inherit_lighting(&mut self) -> &mut Self { remove_cascade_override::<Lighting>(self) }

    fn override_shadow_casting(&mut self, shadow_casting: ShadowCasting) -> &mut Self {
        apply_cascade_override(self, shadow_casting)
    }

    fn inherit_shadow_casting(&mut self) -> &mut Self {
        remove_cascade_override::<ShadowCasting>(self)
    }

    fn override_glyph_shadow_mode(&mut self, mode: GlyphShadowMode) -> &mut Self {
        apply_cascade_override(self, mode)
    }

    fn inherit_glyph_shadow_mode(&mut self) -> &mut Self {
        remove_cascade_override::<GlyphShadowMode>(self)
    }

    fn override_sidedness(&mut self, sidedness: Sidedness) -> &mut Self {
        apply_cascade_override(self, sidedness)
    }

    fn inherit_sidedness(&mut self) -> &mut Self { remove_cascade_override::<Sidedness>(self) }

    fn override_anti_alias(&mut self, anti_alias: AntiAlias) -> &mut Self {
        apply_cascade_override(self, anti_alias)
    }

    fn inherit_anti_alias(&mut self) -> &mut Self { remove_cascade_override::<AntiAlias>(self) }

    fn override_hairline_fade(&mut self, fade: HairlineFade) -> &mut Self {
        apply_cascade_override(self, fade)
    }

    fn inherit_hairline_fade(&mut self) -> &mut Self {
        remove_cascade_override::<HairlineFade>(self)
    }

    fn override_sdf_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self {
        apply_cascade_override(self, SdfMaterial(material))
    }

    fn inherit_sdf_material(&mut self) -> &mut Self { remove_cascade_override::<SdfMaterial>(self) }

    fn override_text_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self {
        apply_cascade_override(self, TextMaterial(material))
    }

    fn inherit_text_material(&mut self) -> &mut Self {
        remove_cascade_override::<TextMaterial>(self)
    }

    fn override_shape_material(&mut self, material: Handle<StandardMaterial>) -> &mut Self {
        apply_cascade_override(self, ShapeMaterial(material))
    }

    fn inherit_shape_material(&mut self) -> &mut Self {
        remove_cascade_override::<ShapeMaterial>(self)
    }
}

/// Resolve an entity's current text alpha mode.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_text_alpha(world: &World, entity: Entity) -> AlphaMode {
    resolved_cascade::<TextAlpha>(world, entity).0
}

/// Resolve an entity's current font unit.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_font_unit(world: &World, entity: Entity) -> Unit {
    resolved_cascade::<FontUnit>(world, entity).0
}

/// Resolve an entity's current HDR text coverage bias.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_hdr_text_coverage_bias(world: &World, entity: Entity) -> f32 {
    resolved_cascade::<HdrTextCoverageBias>(world, entity).0
}

/// Resolve an entity's current lighting mode.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_lighting(world: &World, entity: Entity) -> Lighting {
    resolved_cascade::<Lighting>(world, entity)
}

/// Resolve an entity's current shadow-casting policy.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_shadow_casting(world: &World, entity: Entity) -> ShadowCasting {
    resolved_cascade::<ShadowCasting>(world, entity)
}

/// Resolve an entity's current glyph shadow mode.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_glyph_shadow_mode(world: &World, entity: Entity) -> GlyphShadowMode {
    resolved_cascade::<GlyphShadowMode>(world, entity)
}

/// Resolve an entity's current sidedness.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_sidedness(world: &World, entity: Entity) -> Sidedness {
    resolved_cascade::<Sidedness>(world, entity)
}

/// Resolve an entity's current anti-alias mode.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_anti_alias(world: &World, entity: Entity) -> AntiAlias {
    resolved_cascade::<AntiAlias>(world, entity)
}

/// Resolve an entity's current hairline fade policy.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_hairline_fade(world: &World, entity: Entity) -> HairlineFade {
    resolved_cascade::<HairlineFade>(world, entity)
}

/// Resolve an entity's current SDF surface material handle.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_sdf_material(world: &World, entity: Entity) -> Handle<StandardMaterial> {
    resolved_cascade::<SdfMaterial>(world, entity).0
}

/// Resolve an entity's current text material handle.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_text_material(world: &World, entity: Entity) -> Handle<StandardMaterial> {
    resolved_cascade::<TextMaterial>(world, entity).0
}

/// Resolve an entity's current panel-shape material handle.
///
/// Reads the cached resolved value when present. Before propagation seeds the cache,
/// this resolves through the shared `CascadeFrom` relationship walk.
#[must_use]
pub fn resolved_shape_material(world: &World, entity: Entity) -> Handle<StandardMaterial> {
    resolved_cascade::<ShapeMaterial>(world, entity).0
}

pub(crate) fn apply_cascade_override<'a, 'w, A>(
    entity: &'a mut EntityCommands<'w>,
    value: A,
) -> &'a mut EntityCommands<'w>
where
    A: CascadeAttribute,
{
    entity.override_cascade(value)
}

pub(crate) fn remove_cascade_override<'a, 'w, A>(
    entity: &'a mut EntityCommands<'w>,
) -> &'a mut EntityCommands<'w>
where
    A: CascadeAttribute,
{
    entity.inherit_cascade::<A>()
}

fn resolved_cascade<A>(world: &World, entity: Entity) -> A
where
    A: CascadeAttribute + CascadeRoot,
{
    if let Some(value) = hana_kana::resolved_cascade::<A>(world, entity) {
        return value.clone();
    }
    hana_kana::resolve_entity_cascade::<A, A::Root>(world, entity).unwrap_or_else(A::root_default)
}

#[cfg(test)]
mod tests {
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::MinimalPlugins;

    use super::CascadeEntityCommandsExt;
    use super::WidgetDisabledAppearance;
    use super::WidgetFocusedAppearance;
    use super::WidgetHoveredAppearance;
    use super::WidgetPressedAppearance;
    use crate::Appearance;
    use crate::DiegeticTextMeasurer;
    use crate::HeadlessLayoutPlugin;
    use crate::cascade::Cascade;
    use crate::cascade::Resolved;
    use crate::widgets::WidgetsPlugin;

    const HOVERED: Color = Color::srgb(0.1, 0.2, 0.3);
    const PRESSED: Color = Color::srgb(0.3, 0.2, 0.1);
    const FOCUSED: Color = Color::srgb(0.2, 0.3, 0.1);
    const DISABLED: Color = Color::srgb(0.1, 0.3, 0.2);

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    #[test]
    fn widget_appearance_commands_override_and_restore_inheritance() {
        let mut app = test_app();
        let entity = app
            .world_mut()
            .spawn((
                Cascade::<WidgetHoveredAppearance>::Inherit,
                Cascade::<WidgetPressedAppearance>::Inherit,
                Cascade::<WidgetFocusedAppearance>::Inherit,
                Cascade::<WidgetDisabledAppearance>::Inherit,
            ))
            .id();
        app.update();

        app.world_mut()
            .commands()
            .entity(entity)
            .override_widget_hovered_appearance(Appearance::new().background(HOVERED))
            .override_widget_pressed_appearance(Appearance::new().background(PRESSED))
            .override_widget_focused_appearance(Appearance::new().background(FOCUSED))
            .override_widget_disabled_appearance(Appearance::new().background(DISABLED));
        app.update();

        assert_eq!(
            app.world()
                .get::<Resolved<WidgetHoveredAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetHoveredAppearance::new(
                Appearance::new().background(HOVERED),
            )),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetPressedAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetPressedAppearance::new(
                Appearance::new().background(PRESSED),
            )),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetFocusedAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetFocusedAppearance::new(
                Appearance::new().background(FOCUSED),
            )),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetDisabledAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetDisabledAppearance::new(
                Appearance::new().background(DISABLED),
            )),
        );

        app.world_mut()
            .commands()
            .entity(entity)
            .inherit_widget_hovered_appearance()
            .inherit_widget_pressed_appearance()
            .inherit_widget_focused_appearance()
            .inherit_widget_disabled_appearance();
        app.update();

        assert_eq!(
            app.world()
                .get::<Resolved<WidgetHoveredAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetHoveredAppearance::new(Appearance::new())),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetPressedAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetPressedAppearance::new(Appearance::new())),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetFocusedAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetFocusedAppearance::new(Appearance::new())),
        );
        assert_eq!(
            app.world()
                .get::<Resolved<WidgetDisabledAppearance>>(entity)
                .map(|resolved| resolved.0.clone()),
            Some(WidgetDisabledAppearance::new(Appearance::new())),
        );
    }
}
