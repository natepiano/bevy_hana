use core::mem::size_of;
use std::sync::Arc;
use std::sync::LazyLock;

use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Resource;
use hana_kana::CascadeRootResource;

use super::Appearance;
use super::IntoAppearance;
use crate::cascade::CASCADE_ATTRIBUTE_BYTES;
use crate::cascade::Cascade;
use crate::cascade::CascadeRoot;

static EMPTY_APPEARANCE: LazyLock<Arc<Appearance>> = LazyLock::new(|| Arc::new(Appearance::new()));

/// Hovered-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::hovered`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the hovered appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetHoveredAppearance(Arc<Appearance>);

impl WidgetHoveredAppearance {
    /// Wraps one appearance bundle as the hovered-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this hovered-state bundle.
    pub(in crate::widgets::presentation) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetHoveredAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetHoveredAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetHoveredAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetHoveredAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Pressed-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::pressed`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the pressed appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetPressedAppearance(Arc<Appearance>);

impl WidgetPressedAppearance {
    /// Wraps one appearance bundle as the pressed-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this pressed-state bundle.
    pub(in crate::widgets::presentation) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetPressedAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetPressedAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetPressedAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetPressedAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Focused-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::focused`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the focused appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetFocusedAppearance(Arc<Appearance>);

impl WidgetFocusedAppearance {
    /// Wraps one appearance bundle as the focused-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this focused-state bundle.
    pub(in crate::widgets::presentation) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetFocusedAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetFocusedAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetFocusedAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetFocusedAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Disabled-state cascade attribute for one [`Appearance`] bundle.
///
/// [`crate::El::disabled`] creates this opaque attribute from its bundle.
///
/// Insert this as a resource to set the disabled appearance every widget
/// inherits unless something between it and the cascade root overrides it.
#[derive(Resource, Clone, Debug, Reflect)]
#[reflect(Resource)]
pub struct WidgetDisabledAppearance(Arc<Appearance>);

impl WidgetDisabledAppearance {
    /// Wraps one appearance bundle as the disabled-state attribute value.
    #[must_use]
    pub fn new(appearance: impl IntoAppearance) -> Self {
        Self(Arc::new(appearance.into_appearance()))
    }

    /// Borrows this disabled-state bundle.
    pub(in crate::widgets::presentation) fn appearance(&self) -> &Appearance { &self.0 }
}

impl PartialEq for WidgetDisabledAppearance {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || self.0.as_ref() == other.0.as_ref()
    }
}

impl CascadeRootResource<Self> for WidgetDisabledAppearance {
    fn root(&self) -> Self { self.clone() }

    fn from_root(root: Self) -> Self { root }
}

impl CascadeRoot for WidgetDisabledAppearance {
    type Root = Self;

    fn root_default() -> Self { Self(Arc::clone(&EMPTY_APPEARANCE)) }

    fn combine(lower: Self, higher: &Self) -> Self {
        Self(Arc::new(lower.0.as_ref().merge_over(higher.0.as_ref())))
    }
}

const _: () = assert!(size_of::<WidgetDisabledAppearance>() <= CASCADE_ATTRIBUTE_BYTES);

/// Authored per-state [`Appearance`] bundles held on a layout element and in
/// [`ComputedWidgetRecord`](crate::widgets::ComputedWidgetRecord).
///
/// The four channels reach the widget entity as separate
/// `Cascade<WidgetHoveredAppearance>`, `Cascade<WidgetPressedAppearance>`,
/// `Cascade<WidgetFocusedAppearance>`, and `Cascade<WidgetDisabledAppearance>`
/// components.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct StateAppearance {
    pub(crate) hovered:  Cascade<WidgetHoveredAppearance>,
    pub(crate) pressed:  Cascade<WidgetPressedAppearance>,
    pub(crate) focused:  Cascade<WidgetFocusedAppearance>,
    pub(crate) disabled: Cascade<WidgetDisabledAppearance>,
}

impl StateAppearance {
    /// Borrows this appearance's four independently cascaded state layers.
    pub(crate) const fn cascades(&self) -> WidgetStateCascades<'_> {
        WidgetStateCascades::new(&self.hovered, &self.pressed, &self.focused, &self.disabled)
    }
}

/// Borrowed state-appearance cascades read from a widget entity or layout element.
pub(crate) struct WidgetStateCascades<'a> {
    hovered:  &'a Cascade<WidgetHoveredAppearance>,
    pressed:  &'a Cascade<WidgetPressedAppearance>,
    focused:  &'a Cascade<WidgetFocusedAppearance>,
    disabled: &'a Cascade<WidgetDisabledAppearance>,
}

impl<'a> WidgetStateCascades<'a> {
    /// Borrows the four independently cascaded widget-state appearance layers.
    const fn new(
        hovered: &'a Cascade<WidgetHoveredAppearance>,
        pressed: &'a Cascade<WidgetPressedAppearance>,
        focused: &'a Cascade<WidgetFocusedAppearance>,
        disabled: &'a Cascade<WidgetDisabledAppearance>,
    ) -> Self {
        Self {
            hovered,
            pressed,
            focused,
            disabled,
        }
    }

    /// Whether any state channel explicitly uses [`Cascade::Override`].
    pub(crate) const fn any_overridden(&self) -> bool {
        self.hovered.as_override().is_some()
            || self.pressed.as_override().is_some()
            || self.focused.as_override().is_some()
            || self.disabled.as_override().is_some()
    }

    /// Borrows this state channel's authored override, if it has one.
    pub(in crate::widgets::presentation) fn layer(
        &self,
        state: WidgetState,
    ) -> Option<&Appearance> {
        match state {
            WidgetState::Focused => self
                .focused
                .as_override()
                .map(WidgetFocusedAppearance::appearance),
            WidgetState::Hovered => self
                .hovered
                .as_override()
                .map(WidgetHoveredAppearance::appearance),
            WidgetState::Pressed => self
                .pressed
                .as_override()
                .map(WidgetPressedAppearance::appearance),
            WidgetState::Disabled => self
                .disabled
                .as_override()
                .map(WidgetDisabledAppearance::appearance),
        }
    }

    /// Whether any state layer authors the property `authored` selects.
    pub(crate) fn any(&self, authored: impl Fn(&Appearance) -> bool) -> bool {
        WidgetState::LAYER_ORDER
            .into_iter()
            .any(|state| self.layer(state).is_some_and(&authored))
    }
}

/// Borrowed resolved appearance bundles for the four widget states.
pub(crate) struct ResolvedWidgetStateAppearances<'a> {
    hovered:  &'a Appearance,
    pressed:  &'a Appearance,
    focused:  &'a Appearance,
    disabled: &'a Appearance,
}

impl<'a> ResolvedWidgetStateAppearances<'a> {
    /// Borrows each state bundle after global, panel, and widget cascade resolution.
    pub(crate) const fn new(
        hovered: &'a Appearance,
        pressed: &'a Appearance,
        focused: &'a Appearance,
        disabled: &'a Appearance,
    ) -> Self {
        Self {
            hovered,
            pressed,
            focused,
            disabled,
        }
    }

    /// Borrows one resolved state bundle.
    pub(crate) const fn layer(&self, state: WidgetState) -> &'a Appearance {
        match state {
            WidgetState::Focused => self.focused,
            WidgetState::Hovered => self.hovered,
            WidgetState::Pressed => self.pressed,
            WidgetState::Disabled => self.disabled,
        }
    }
}

/// One interaction state a widget element authors an [`Appearance`] for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WidgetState {
    /// The widget's keyboard focus indicator is visible.
    Focused,
    /// A pointer is over the widget.
    Hovered,
    /// The widget is held: a button press or a slider drag.
    Pressed,
    /// The widget carries [`WidgetDisabled`](crate::widgets::WidgetDisabled) and takes
    /// no interaction.
    Disabled,
}

impl WidgetState {
    /// Layering order for [`ResolvedWidgetStateAppearances::resolve`]: a later state replaces
    /// the properties it authors over an earlier one.
    pub(crate) const LAYER_ORDER: [Self; 4] =
        [Self::Focused, Self::Hovered, Self::Pressed, Self::Disabled];
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::MinimalPlugins;

    use crate::DiegeticPanel;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::cascade::Resolved;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::presentation::appearance::Appearance;
    use crate::widgets::presentation::appearance::WidgetDisabledAppearance;
    use crate::widgets::presentation::appearance::WidgetFocusedAppearance;
    use crate::widgets::presentation::appearance::WidgetHoveredAppearance;
    use crate::widgets::presentation::appearance::WidgetPressedAppearance;

    const HOVER_FILL: Color = Color::srgb(0.1, 0.2, 0.3);
    const FOCUS_BORDER: Color = Color::srgb(0.9, 0.8, 0.2);
    const PRESS_FILL: Color = Color::srgb(0.4, 0.5, 0.6);
    const HIGHER_BACKGROUND: Color = Color::srgb(0.1, 0.2, 0.3);
    const LOWER_BACKGROUND: Color = Color::srgb(0.3, 0.2, 0.1);
    const HIGHER_BORDER: Color = Color::srgb(0.2, 0.3, 0.4);
    const LOWER_BORDER: Color = Color::srgb(0.4, 0.3, 0.2);
    const HIGHER_TEXT: Color = Color::srgb(0.4, 0.5, 0.6);
    const DISABLED_BACKGROUND: Color = Color::srgb(0.2, 0.2, 0.2);
    const DISABLED_BORDER: Color = Color::srgb(0.8, 0.1, 0.1);

    macro_rules! assert_resolved_appearance {
        ($app:expr, $widget:expr, $attribute:ty, $expected:expr $(,)?) => {{
            let expected = $expected;
            assert_eq!(
                $app.world()
                    .get::<Resolved<$attribute>>($widget)
                    .map(|resolved| resolved.0.appearance()),
                Some(&expected),
            );
        }};
    }

    fn cascade_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn two_widget_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("first"), |_| {});
        builder.with(El::new().button("second"), |_| {});
        builder.build()
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        let Some(widget) = widget else {
            return Entity::PLACEHOLDER;
        };
        widget
    }

    #[test]
    fn global_state_appearance_defaults_reach_every_widget_without_state_authoring() {
        let mut app = cascade_test_app();
        let hovered = Appearance::new().background(HOVER_FILL);
        let pressed = Appearance::new().background(PRESS_FILL);
        let focused = Appearance::new().border_color(FOCUS_BORDER);
        let disabled = Appearance::new().background(DISABLED_BACKGROUND);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(hovered.clone());
        *app.world_mut().resource_mut::<WidgetPressedAppearance>() =
            WidgetPressedAppearance::new(pressed.clone());
        *app.world_mut().resource_mut::<WidgetFocusedAppearance>() =
            WidgetFocusedAppearance::new(focused.clone());
        *app.world_mut().resource_mut::<WidgetDisabledAppearance>() =
            WidgetDisabledAppearance::new(disabled.clone());
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        for id in ["first", "second"] {
            let widget = resolve_widget(&mut app, panel, id);
            assert_resolved_appearance!(&app, widget, WidgetHoveredAppearance, hovered.clone());
            assert_resolved_appearance!(&app, widget, WidgetPressedAppearance, pressed.clone());
            assert_resolved_appearance!(&app, widget, WidgetFocusedAppearance, focused.clone());
            assert_resolved_appearance!(&app, widget, WidgetDisabledAppearance, disabled.clone());
        }
    }

    #[test]
    fn panel_state_appearances_merge_with_globals_in_the_reification_frame() {
        let mut app = cascade_test_app();
        let global = Appearance::new()
            .background(HIGHER_BACKGROUND)
            .text_color(HIGHER_TEXT);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetPressedAppearance>() =
            WidgetPressedAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetFocusedAppearance>() =
            WidgetFocusedAppearance::new(global.clone());
        *app.world_mut().resource_mut::<WidgetDisabledAppearance>() =
            WidgetDisabledAppearance::new(global);
        let hovered = Appearance::new()
            .background(HOVER_FILL)
            .border_color(HIGHER_BORDER);
        let pressed = Appearance::new()
            .background(PRESS_FILL)
            .border_color(LOWER_BORDER);
        let focused = Appearance::new()
            .background(LOWER_BACKGROUND)
            .border_color(FOCUS_BORDER);
        let disabled = Appearance::new()
            .background(DISABLED_BACKGROUND)
            .border_color(DISABLED_BORDER);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(hovered)
            .widget_pressed_appearance(pressed)
            .widget_focused_appearance(focused)
            .widget_disabled_appearance(disabled)
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "first");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance::new()
                .background(HOVER_FILL)
                .border_color(HIGHER_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetPressedAppearance,
            Appearance::new()
                .background(PRESS_FILL)
                .border_color(LOWER_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetFocusedAppearance,
            Appearance::new()
                .background(LOWER_BACKGROUND)
                .border_color(FOCUS_BORDER)
                .text_color(HIGHER_TEXT),
        );
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetDisabledAppearance,
            Appearance::new()
                .background(DISABLED_BACKGROUND)
                .border_color(DISABLED_BORDER)
                .text_color(HIGHER_TEXT),
        );
    }

    #[test]
    fn panel_hovered_appearance_preserves_global_properties_through_the_cascade() {
        let mut app = cascade_test_app();
        let global = Appearance::new()
            .background(HIGHER_BACKGROUND)
            .text_color(HIGHER_TEXT);
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(global);
        let hovered = Appearance::new().border_color(HIGHER_BORDER);
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .widget_hovered_appearance(hovered)
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, "first");
        assert_resolved_appearance!(
            &app,
            widget,
            WidgetHoveredAppearance,
            Appearance::new()
                .background(HIGHER_BACKGROUND)
                .border_color(HIGHER_BORDER)
                .text_color(HIGHER_TEXT),
        );
    }

    #[test]
    fn unchanged_state_appearance_propagation_does_not_dirty_resolved_values() {
        let mut app = cascade_test_app();
        *app.world_mut().resource_mut::<WidgetHoveredAppearance>() =
            WidgetHoveredAppearance::new(Appearance::new().background(HOVER_FILL));
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(two_widget_tree())
            .build()
            .expect("a sized panel should build");
        let panel = app.world_mut().spawn(panel).id();
        app.update();
        let widget = resolve_widget(&mut app, panel, "first");
        let before = (
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetHoveredAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetPressedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetFocusedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetDisabledAppearance>>()
                .map(|resolved| resolved.last_changed()),
        );

        app.update();

        let after = (
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetHoveredAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetPressedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetFocusedAppearance>>()
                .map(|resolved| resolved.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Resolved<WidgetDisabledAppearance>>()
                .map(|resolved| resolved.last_changed()),
        );
        assert_eq!(after, before);
    }
}
