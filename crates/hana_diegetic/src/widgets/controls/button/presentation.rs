use bevy::picking::hover::PickingInteraction;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::With;

use super::press::ButtonCaptures;
use super::press::ButtonLatch;
use super::press::ButtonPointerInside;
use super::press::ButtonPress;
use crate::DiegeticPanel;
use crate::cascade::Cascade;
use crate::cascade::Resolved;
use crate::widgets::WidgetDimmed;
use crate::widgets::WidgetDisabled;
use crate::widgets::WidgetDisabledAppearance;
use crate::widgets::WidgetFocusVisible;
use crate::widgets::WidgetFocusedAppearance;
use crate::widgets::WidgetHoveredAppearance;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;
use crate::widgets::WidgetPressedAppearance;
use crate::widgets::WidgetState;
use crate::widgets::WidgetVisualOverrides;
use crate::widgets::WidgetVisualSlots;
use crate::widgets::presentation as visual;

/// Maps each changed button's live state onto its retained visual overrides.
///
/// Runs after `WidgetSystems::FocusCommandsApplied`, so application and
/// keyboard-traversal indicator commands, as well as pointer-driven indicator
/// removal, are visible in the same frame. Each `Changed` query and
/// [`RemovedComponents`] stream is consumed here, so a quiet frame never walks
/// the live buttons.
///
/// Hover reads the all-pointer [`PickingInteraction`] aggregate and pressed
/// reads the private [`ButtonPress`] marker. For pointer-held presses, the
/// owning pointer must remain over the button; leaving changes only the
/// appearance, retaining the press until release or cancellation. Semantic
/// presses and latched buttons do not depend on pointer hover.
/// Writes go through [`visual::write_widget_overrides`], which
/// compares immutably first, so an unchanged state never marks
/// [`WidgetVisualOverrides`] changed.
pub(in crate::widgets) fn present_button_state(
    changed: Query<
        (Entity, &WidgetKind),
        (
            With<WidgetOf>,
            Or<(
                Changed<Resolved<WidgetHoveredAppearance>>,
                Changed<Resolved<WidgetPressedAppearance>>,
                Changed<Resolved<WidgetFocusedAppearance>>,
                Changed<Resolved<WidgetDisabledAppearance>>,
                Changed<Cascade<WidgetDisabledAppearance>>,
                Changed<WidgetVisualSlots>,
                Changed<PickingInteraction>,
                Changed<WidgetFocusVisible>,
                Changed<WidgetDisabled>,
                Changed<WidgetDimmed>,
                Changed<ButtonPress>,
                Changed<ButtonLatch>,
                Changed<ButtonPointerInside>,
            )>,
        ),
    >,
    buttons: Query<
        (
            &Resolved<WidgetHoveredAppearance>,
            &Resolved<WidgetPressedAppearance>,
            &Resolved<WidgetFocusedAppearance>,
            &Resolved<WidgetDisabledAppearance>,
            &Cascade<WidgetHoveredAppearance>,
            &Cascade<WidgetDisabledAppearance>,
            &WidgetOf,
            &WidgetVisualSlots,
            Option<&PickingInteraction>,
            Has<WidgetDisabled>,
            Has<WidgetDimmed>,
            Has<WidgetFocusVisible>,
            Has<ButtonPress>,
            Option<&ButtonLatch>,
            Option<&ButtonPointerInside>,
        ),
        With<WidgetOf>,
    >,
    kinds: Query<&WidgetKind, With<WidgetOf>>,
    panels: Query<&DiegeticPanel>,
    captures: Res<ButtonCaptures>,
    mut removed_interactions: RemovedComponents<PickingInteraction>,
    mut removed_focus: RemovedComponents<WidgetFocusVisible>,
    mut removed_disabled: RemovedComponents<WidgetDisabled>,
    mut removed_dimmed: RemovedComponents<WidgetDimmed>,
    mut removed_presses: RemovedComponents<ButtonPress>,
    mut removed_latches: RemovedComponents<ButtonLatch>,
    mut overrides: Query<&mut WidgetVisualOverrides>,
    mut commands: Commands,
) {
    let dirty = visual::dirty_widgets(
        changed.iter().map(|(entity, kind)| (entity, *kind)),
        removed_interactions
            .read()
            .chain(removed_focus.read())
            .chain(removed_disabled.read())
            .chain(removed_dimmed.read())
            .chain(removed_presses.read())
            .chain(removed_latches.read()),
        &kinds,
        WidgetKind::Button,
    );
    for entity in dirty {
        let Ok((
            hovered,
            pressed_appearance,
            focused_appearance,
            disabled_appearance,
            authored_hovered,
            authored_disabled,
            widget_of,
            slots,
            interaction,
            disabled,
            dimmed,
            focused,
            pressed,
            latch,
            pointer_inside,
        )) = buttons.get(entity)
        else {
            continue;
        };
        let held_outside =
            captures.pointer(entity).is_some() && pointer_inside.is_some_and(|inside| !inside.0);
        let active = [
            focused.then_some(WidgetState::Focused),
            (!held_outside
                && matches!(
                    interaction,
                    Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
                ))
            .then_some(WidgetState::Hovered),
            ((pressed && !held_outside) || matches!(latch, Some(ButtonLatch::Pressed)))
                .then_some(WidgetState::Pressed),
            disabled.then_some(WidgetState::Disabled),
        ];
        let panel = panels.get(widget_of.panel()).ok();
        let mut desired = WidgetVisualOverrides::default();
        visual::resolve_part_overrides(
            &mut desired,
            slots,
            hovered,
            pressed_appearance,
            focused_appearance,
            disabled_appearance,
            authored_hovered,
            authored_disabled,
            &active,
            dimmed,
            panel,
        );
        visual::write_widget_overrides(entity, desired, &mut overrides, &mut commands);
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::change_detection::Tick;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::PickingPlugin;
    use bevy::picking::PickingSettings;
    use bevy::picking::backend::HitData;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::hover::PickingInteraction;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::PointerMap;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Color;
    use bevy::prelude::Commands;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::Handle;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::PointerButton;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;

    use crate::Appearance;
    use crate::Border;
    use crate::Button;
    use crate::CascadeEntityCommandsExt;
    use crate::ClearWidgetFocus;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::RequestWidgetFocus;
    use crate::Sizing;
    use crate::WidgetInput;
    use crate::WidgetInteractivity;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::SemanticWidgetIntent;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetDimmed;
    use crate::widgets::WidgetDisabled;
    use crate::widgets::WidgetFocusVisible;
    use crate::widgets::WidgetSystems;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::controls::button;
    use crate::widgets::controls::button::ButtonActivationSource;
    use crate::widgets::controls::button::ButtonCancelCause;
    use crate::widgets::controls::button::ButtonCanceled;
    use crate::widgets::controls::button::ButtonCaptures;
    use crate::widgets::controls::button::ButtonClicked;
    use crate::widgets::controls::button::ButtonLatch;
    use crate::widgets::controls::button::ButtonPress;
    use crate::widgets::controls::button::ButtonPressSource;
    use crate::widgets::controls::button::ButtonPressed;
    use crate::widgets::controls::button::ButtonReleased;
    use crate::widgets::controls::button::press::ButtonPressHolder;
    use crate::widgets::interaction::WidgetCaptures;

    const BUTTON_ID: &str = "action";
    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedButtonEvent {
        Pressed(PointerId),
        Released(PointerId),
        Clicked(ButtonActivationSource),
        Canceled(PointerId, ButtonCancelCause),
    }

    #[derive(Default, Resource)]
    struct RecordedButtonEvents(Vec<RecordedButtonEvent>);

    fn record_pressed(event: On<ButtonPressed>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Pressed(match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            }));
    }

    fn record_released(event: On<ButtonReleased>, mut events: ResMut<RecordedButtonEvents>) {
        events
            .0
            .push(RecordedButtonEvent::Released(match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            }));
    }

    fn record_clicked(event: On<ButtonClicked>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Clicked(event.source));
    }

    fn record_canceled(event: On<ButtonCanceled>, mut events: ResMut<RecordedButtonEvents>) {
        events.0.push(RecordedButtonEvent::Canceled(
            match event.source {
                ButtonPressSource::Pointer(pointer_id) => pointer_id,
                ButtonPressSource::Semantic => return,
            },
            event.cause,
        ));
    }

    fn add_pointer(app: &mut App, pointer_id: PointerId) {
        app.world_mut()
            .spawn((pointer_id, PointerLocation::new(location())));
    }

    fn integrated_test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<RecordedButtonEvents>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin))
            .add_observer(record_pressed)
            .add_observer(record_released)
            .add_observer(record_clicked)
            .add_observer(record_canceled);
        app
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Entity {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return Entity::PLACEHOLDER;
        };
        app.world_mut().spawn(panel).id()
    }

    fn resolve_widget(app: &mut App, panel: Entity) -> Entity {
        let id = PanelElementId::named(BUTTON_ID);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let Ok(widget) = result else {
            return Entity::PLACEHOLDER;
        };
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  1,
                height: 1,
            },
            position: Vec2::ZERO,
        }
    }

    #[test]
    fn latched_toggle_uses_pressed_appearance_until_released() {
        let mut app = integrated_test_app();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        tree.with(
            El::new()
                .widget(BUTTON_ID, Button::new().toggle())
                .pressed(Appearance::new().background(PRESS_FILL)),
            |_| {},
        );
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut().trigger(SemanticWidgetIntent::Activate {
            entity: widget,
            window: Entity::PLACEHOLDER,
        });
        app.update();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );
        assert_eq!(
            root_override(&app, widget).map(|override_| override_.fill_color),
            Some(Some(PRESS_FILL))
        );

        app.world_mut()
            .trigger(SemanticWidgetIntent::ActivateRelease {
                entity: widget,
                window: Entity::PLACEHOLDER,
            });
        app.update();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Pressed)
        );
        assert_eq!(
            root_override(&app, widget).map(|override_| override_.fill_color),
            Some(Some(PRESS_FILL))
        );

        assert!(
            app.world_mut()
                .run_system_once(move |mut commands: Commands| button::release_button(
                    &mut commands,
                    widget
                ))
                .is_ok()
        );
        app.update();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );
        assert_eq!(root_override(&app, widget), None);
    }

    #[test]
    fn keyboard_hold_keeps_pressed_appearance_until_release() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.pressed(Appearance::new().background(PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut()
            .write_message(WidgetInput::ActivatePress { window });
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert_eq!(
            root_override(&app, widget).map(|override_| override_.fill_color),
            Some(Some(PRESS_FILL)),
        );

        app.update();
        assert_eq!(
            root_override(&app, widget).map(|override_| override_.fill_color),
            Some(Some(PRESS_FILL)),
        );
        app.world_mut()
            .write_message(WidgetInput::ActivateRelease { window });
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_none());
        assert_eq!(root_override(&app, widget), None);
    }

    const NORMAL_FILL: Color = Color::srgb(0.05, 0.10, 0.20);
    const NORMAL_BORDER: Color = Color::srgb(0.30, 0.30, 0.30);
    const HOVER_FILL: Color = Color::srgb(0.15, 0.30, 0.60);
    const PRESS_FILL: Color = Color::srgb(0.60, 0.30, 0.10);
    const FOCUS_FILL: Color = Color::srgb(0.20, 0.20, 0.45);
    const FOCUS_BORDER: Color = Color::srgb(0.95, 0.85, 0.25);
    const DISABLED_BORDER: Color = Color::srgb(0.35, 0.35, 0.40);
    const DISABLED_FILL: Color = Color::srgb(0.12, 0.12, 0.14);
    const PEER_HOVER_FILL: Color = Color::srgb(0.10, 0.55, 0.25);

    /// A button element in the default row layout.
    type ButtonElement = El<crate::Row, crate::WidgetElement<Button>>;

    fn button_tree_with(
        id: &'static str,
        configure: impl FnOnce(ButtonElement) -> ButtonElement,
    ) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            configure(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(NORMAL_FILL)
                    .border(Border::all(1.0, NORMAL_BORDER))
                    .button(id),
            ),
            |_| {},
        );
        builder.build()
    }

    fn two_styled_buttons_tree(first_hover: Color, second_hover: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for (id, hover) in [("first", first_hover), ("second", second_hover)] {
            builder.with(
                El::new()
                    .width(Sizing::GROW)
                    .height(Sizing::GROW)
                    .background(NORMAL_FILL)
                    .border(Border::all(1.0, NORMAL_BORDER))
                    .button(id)
                    .hovered(Appearance::new().background(hover)),
                |_| {},
            );
        }
        builder.build()
    }

    fn widget_by_id(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let id = PanelElementId::named(id);
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id));
        assert!(result.is_ok());
        let widget = result.ok().flatten();
        assert!(widget.is_some());
        widget.unwrap_or(Entity::PLACEHOLDER)
    }

    fn root_override(app: &App, widget: Entity) -> Option<VisualSlotOverride> {
        let slots = app.world().get::<WidgetVisualSlots>(widget)?;
        let element_index = slots.element_index(VisualSlotId::BUTTON_ROOT)?;
        app.world()
            .get::<WidgetVisualOverrides>(widget)
            .and_then(|overrides| {
                overrides
                    .element_overrides()
                    .iter()
                    .find(|(index, _)| *index == element_index)
                    .map(|(_, value)| value.clone())
            })
    }

    fn root_element_index(app: &App, widget: Entity) -> Option<usize> {
        app.world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(VisualSlotId::BUTTON_ROOT))
    }

    fn indexed_root_override(
        app: &App,
        panel: Entity,
        widget: Entity,
    ) -> Option<VisualSlotOverride> {
        let element_index = root_element_index(app, widget)?;
        app.world()
            .resource::<VisualOverrideIndex>()
            .get(panel, element_index)
            .cloned()
    }

    fn computed_tick(app: &App, panel: Entity) -> Option<Tick> {
        app.world()
            .entity(panel)
            .get_ref::<ComputedDiegeticPanel>()
            .map(|computed| computed.last_changed())
    }

    fn override_tick(app: &App, widget: Entity) -> Option<Tick> {
        app.world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed())
    }

    fn assert_override_quiet(app: &mut App, widget: Entity, tick: Option<Tick>) {
        app.update();
        assert_eq!(
            override_tick(app, widget),
            tick,
            "a quiet frame must not write the button override component",
        );
    }

    #[test]
    fn hovered_background_patches_only_the_fill_and_reaches_dispatch_same_frame() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();

        let expected = VisualSlotOverride {
            fill_color: Some(HOVER_FILL),
            ..VisualSlotOverride::default()
        };
        assert_eq!(root_override(&app, widget), Some(expected.clone()));
        assert_eq!(
            indexed_root_override(&app, panel, widget),
            Some(expected),
            "the first insertion must reach dispatch in the same frame",
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::None);
        app.update();
        assert_eq!(root_override(&app, widget), None);
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn border_color_and_material_builders_patch_their_own_properties() {
        let mut app = integrated_test_app();
        let material: Handle<StandardMaterial> = Handle::default();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().border_color(FOCUS_BORDER))
                    .pressed(Appearance::new().material(material.clone()))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "a border-color builder must not touch fill or material",
        );

        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_color: Some(FOCUS_BORDER),
                material: Some(material),
                ..VisualSlotOverride::default()
            }),
            "a state material applies without touching state colors",
        );
    }

    #[test]
    fn zero_width_border_lets_a_borderless_button_grow_a_focus_border() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        tree.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(NORMAL_FILL)
                .border(Border::all(Px(0.0), FOCUS_BORDER))
                .button(BUTTON_ID)
                .focused(Appearance::new().border_width(Px(3.0))),
            |_| {},
        );
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        assert!(
            root_override(&app, widget)
                .and_then(|value| value.border_widths)
                .is_some_and(|widths| widths.iter().all(|width| *width > 0.0)),
            "an authored zero-width border is a real record a focus width can widen",
        );
    }

    #[test]
    fn focused_border_width_resolves_against_the_owning_panel() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.focused(Appearance::new().border_width(Px(3.0)))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();

        let points_to_world = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map_or(0.0, DiegeticPanel::points_to_world);
        // Three logical pixels resolve to points through the fixed 96 DPI
        // convention, then scale to the owning panel's world units.
        let expected = 3.0 * 0.75 * points_to_world;
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                border_widths: Some([expected; 4]),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "leaving focus restores the authored border width",
        );
    }

    #[test]
    fn state_precedence_layers_independently_per_property() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .focused(
                        Appearance::new()
                            .background(FOCUS_FILL)
                            .border_color(FOCUS_BORDER),
                    )
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .disabled(Appearance::new().border_color(DISABLED_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(FOCUS_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "hover replaces the focused fill while the border falls through to focus",
        );

        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "press replaces the hovered fill while the border still falls through",
        );

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                border_color: Some(DISABLED_BORDER),
                dim_fill: true,
                ..VisualSlotOverride::default()
            }),
            "disabled replaces the border while the fill falls through to press",
        );
    }

    #[test]
    fn dimmed_button_keeps_enabled_appearance_and_input() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.disabled(Appearance::new().border_color(DISABLED_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());
        assert_eq!(
            root_override(&app, widget).and_then(|visual| visual.border_color),
            None,
        );

        app.world_mut().entity_mut(widget).remove::<WidgetDimmed>();
        app.update();
        assert_eq!(root_override(&app, widget), None);
    }

    #[test]
    fn empty_disabled_appearance_dims_only_disabled_button_base() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| element.disabled(Appearance::new())),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.update();
        app.update();
        let disabled = root_override(&app, widget);
        assert!(disabled.is_some(), "disabled button visual override");
        let disabled = disabled.unwrap_or_default();
        assert!(disabled.dim_fill);
        assert!(disabled.dim_border);
        assert_eq!(disabled.fill_color, None);
        assert_eq!(disabled.border_color, None);

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Enabled);
        app.update();
        app.update();
        assert_eq!(root_override(&app, widget), None);

        app.world_mut().entity_mut(widget).insert(WidgetDimmed);
        app.update();
        assert_eq!(root_override(&app, widget), None);
    }

    #[test]
    fn set_tree_accepts_state_targets_without_ordinary_declarations() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(&mut app, button_tree_with(BUTTON_ID, |element| element));
        app.update();

        let mut background_only = LayoutBuilder::new(100.0, 50.0);
        background_only.with(
            El::new()
                .button(BUTTON_ID)
                .hovered(Appearance::new().background(HOVER_FILL)),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, background_only.build());
        assert!(result.is_ok());

        let mut border_only = LayoutBuilder::new(100.0, 50.0);
        border_only.with(
            El::new()
                .background(NORMAL_FILL)
                .button(BUTTON_ID)
                .hovered(Appearance::new().border_color(FOCUS_BORDER)),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, border_only.build());
        assert!(result.is_ok());

        let mut border_width_only = LayoutBuilder::new(100.0, 50.0);
        border_width_only.with(
            El::new()
                .background(NORMAL_FILL)
                .button(BUTTON_ID)
                .focused(Appearance::new().border_width(Px(2.0))),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, border_width_only.build());
        assert!(result.is_ok());

        let mut material_only = LayoutBuilder::new(100.0, 50.0);
        material_only.with(
            El::new()
                .button(BUTTON_ID)
                .disabled(Appearance::new().material(Handle::default())),
            |_| {},
        );
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, material_only.build());
        assert!(result.is_ok());

        app.update();
    }

    #[test]
    fn buttons_retain_distinct_state_values_through_computed_output_and_reify() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");
        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.world_mut()
            .entity_mut(second)
            .insert(PickingInteraction::Hovered);
        app.update();

        let fill =
            |app: &App, widget| root_override(app, widget).and_then(|value| value.fill_color);
        assert_eq!(fill(&app, first), Some(HOVER_FILL));
        assert_eq!(fill(&app, second), Some(PEER_HOVER_FILL));

        // Same-id replacement swaps the authored hover colors; the live
        // widgets keep their entities and adopt the new authored values.
        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, two_styled_buttons_tree(PEER_HOVER_FILL, HOVER_FILL));
        assert!(result.is_ok());
        app.update();
        assert_eq!(widget_by_id(&mut app, panel, "first"), first);
        assert_eq!(widget_by_id(&mut app, panel, "second"), second);
        app.update();
        assert_eq!(fill(&app, first), Some(PEER_HOVER_FILL));
        assert_eq!(fill(&app, second), Some(HOVER_FILL));
    }

    #[test]
    fn removing_state_builders_on_reify_clears_the_stale_override() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, button_tree_with(BUTTON_ID, |element| element));
        assert!(result.is_ok());
        app.update();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "a same-id button without state builders must drop the old override",
        );
    }

    #[test]
    fn pressed_presentation_reads_button_press_and_ignores_captures() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.pressed(Appearance::new().background(PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // A capture without the `ButtonPress` marker shows no pressed state.
        let captured = app
            .world_mut()
            .resource_mut::<WidgetCaptures>()
            .try_capture(PointerId::Mouse, widget, 1);
        assert!(captured);
        app.world_mut()
            .resource_mut::<ButtonCaptures>()
            .begin_press(
                widget,
                PanelElementId::named(BUTTON_ID),
                ButtonPressHolder::Pointer(PointerId::Mouse),
            );
        app.update();
        assert_eq!(root_override(&app, widget), None);

        // The marker alone drives pressed presentation.
        app.world_mut()
            .resource_mut::<WidgetCaptures>()
            .release_widget(widget);
        app.world_mut()
            .resource_mut::<ButtonCaptures>()
            .take(widget);
        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(PRESS_FILL),
                ..VisualSlotOverride::default()
            }),
        );
    }

    #[test]
    fn traversal_focus_is_visible_to_presentation_in_the_same_frame() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        for (id, focus_border) in [("first", None), ("second", Some(FOCUS_BORDER))] {
            let element = El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(NORMAL_FILL)
                .border(Border::all(1.0, NORMAL_BORDER))
                .button(id);
            let element = match focus_border {
                Some(color) => element.focused(Appearance::new().border_color(color)),
                None => element,
            };
            tree.with(element, |_| {});
        }
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let widget = widget_by_id(&mut app, panel, "second");
        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: first,
        });
        app.world_mut().flush();
        app.update();
        assert_eq!(root_override(&app, widget), None);

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        app.update();

        let expected = VisualSlotOverride {
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "semantic traversal focus must restyle in the frame it lands",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));
    }

    fn pointer_presentation_app(pointer_id: PointerId) -> App {
        let mut app = integrated_test_app();
        app.add_plugins(InteractionPlugin)
            .add_message::<PointerInput>()
            .add_message::<PointerHits>()
            .init_resource::<PointerMap>();
        add_pointer(&mut app, pointer_id);
        app
    }

    fn write_widget_hit(app: &mut App, pointer_id: PointerId, camera: Entity, widget: Entity) {
        let hit = HitData::new(camera, 0.0, None, None);
        app.world_mut()
            .write_message(PointerHits::new(pointer_id, vec![(widget, hit)], 0.0));
    }

    #[test]
    fn p35b_latched_toggle_keeps_selection_when_pointer_repress_leaves() {
        let pointer = PointerId::Mouse;
        let mut app = pointer_presentation_app(pointer);
        app.add_plugins(PickingPlugin);
        app.world_mut()
            .resource_mut::<PickingSettings>()
            .is_window_picking_enabled = false;
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let camera = app.world_mut().spawn(Camera::default()).id();
        let mut tree = LayoutBuilder::new(100.0, 50.0);
        tree.with(
            El::new()
                .widget(BUTTON_ID, Button::new().toggle())
                .pressed(Appearance::new().background(PRESS_FILL)),
            |_| {},
        );
        let panel = spawn_panel(&mut app, tree.build());
        app.update();
        let widget = resolve_widget(&mut app, panel);
        write_widget_hit(&mut app, pointer, camera, widget);
        app.update();
        for action in [
            PointerAction::Press(PointerButton::Primary),
            PointerAction::Release(PointerButton::Primary),
        ] {
            write_widget_hit(&mut app, pointer, camera, widget);
            app.world_mut()
                .write_message(PointerInput::new(pointer, location(), action));
            app.update();
        }
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Pressed)
        );
        let selected = root_override(&app, widget);
        assert_eq!(
            selected.as_ref().and_then(|part| part.fill_color),
            Some(PRESS_FILL)
        );
        app.world_mut()
            .resource_mut::<RecordedButtonEvents>()
            .0
            .clear();
        write_widget_hit(&mut app, pointer, camera, widget);
        app.world_mut().write_message(PointerInput::new(
            pointer,
            location(),
            PointerAction::Press(PointerButton::Primary),
        ));
        app.update();
        app.world_mut().write_message(PointerInput::new(
            pointer,
            location(),
            PointerAction::Move { delta: Vec2::ONE },
        ));
        app.update();
        assert_eq!(
            root_override(&app, widget),
            selected,
            "drag-off must retain the latched fill"
        );
        app.world_mut().write_message(PointerInput::new(
            pointer,
            location(),
            PointerAction::Release(PointerButton::Primary),
        ));
        app.update();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Pressed)
        );
        assert_eq!(
            root_override(&app, widget),
            selected,
            "off-release keeps selection"
        );
        assert!(
            !app.world()
                .resource::<RecordedButtonEvents>()
                .0
                .iter()
                .any(|event| { matches!(event, RecordedButtonEvent::Clicked(_)) }),
            "off-release must not activate the toggle"
        );
    }

    #[test]
    fn p35b_only_capture_owner_edges_dirty_held_button_presentation() {
        let owner = PointerId::Mouse;
        let other = PointerId::Touch(206);
        let mut app = pointer_presentation_app(owner);
        app.add_plugins(PickingPlugin);
        app.world_mut()
            .resource_mut::<PickingSettings>()
            .is_window_picking_enabled = false;
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        add_pointer(&mut app, other);
        let camera = app.world_mut().spawn(Camera::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.pressed(Appearance::new().background(PRESS_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        write_widget_hit(&mut app, owner, camera, widget);
        write_widget_hit(&mut app, other, camera, widget);
        app.world_mut().write_message(PointerInput::new(
            owner,
            location(),
            PointerAction::Press(PointerButton::Primary),
        ));
        app.update();
        assert_eq!(
            root_override(&app, widget).and_then(|part| part.fill_color),
            Some(PRESS_FILL)
        );
        // The other pointer stays over the button while the press owner leaves.
        write_widget_hit(&mut app, other, camera, widget);
        app.update();
        assert_eq!(
            app.world()
                .get::<super::ButtonPointerInside>(widget)
                .map(|state| state.0),
            Some(false)
        );
        assert_eq!(root_override(&app, widget), None);
        let tick = app
            .world()
            .entity(widget)
            .get_ref::<super::ButtonPointerInside>()
            .map(|state| state.last_changed());
        let visual_tick = override_tick(&app, widget);
        for _ in 0..5 {
            write_widget_hit(&mut app, other, camera, widget);
            app.update();
            assert_eq!(
                app.world()
                    .entity(widget)
                    .get_ref::<super::ButtonPointerInside>()
                    .map(|state| state.last_changed()),
                tick,
                "stationary holds have no containment writes"
            );
            assert_eq!(override_tick(&app, widget), visual_tick);
        }
        write_widget_hit(&mut app, owner, camera, widget);
        write_widget_hit(&mut app, other, camera, widget);
        app.update();
        assert_eq!(
            root_override(&app, widget).and_then(|part| part.fill_color),
            Some(PRESS_FILL)
        );
        // The non-owning pointer leaving cannot suppress the owner's press.
        write_widget_hit(&mut app, owner, camera, widget);
        app.update();
        assert_eq!(
            app.world()
                .get::<super::ButtonPointerInside>(widget)
                .map(|state| state.0),
            Some(true)
        );
        assert_eq!(
            root_override(&app, widget).and_then(|part| part.fill_color),
            Some(PRESS_FILL)
        );
    }

    #[test]
    fn pointer_input_retains_focus_without_showing_its_indicator() {
        let pointer_id = PointerId::Mouse;
        let mut app = pointer_presentation_app(pointer_id);
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app.world_mut().spawn(Camera::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .focused(Appearance::new().border_color(FOCUS_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        // A backend hit alone: `generate_hovermap` and `update_interactions`
        // produce the hovered `PickingInteraction`, and presentation reaches
        // the override in the same `app.update()`.
        write_widget_hit(&mut app, pointer_id, camera, widget);
        app.update();
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                ..VisualSlotOverride::default()
            }),
            "a real backend hover hit must restyle in the update it lands",
        );

        // A raw primary press through Bevy's real `pointer_events` dispatcher:
        // the press observers insert `ButtonPress` and retain pointer focus,
        // but pointer focus does not draw the keyboard focus indicator.
        write_widget_hit(&mut app, pointer_id, camera, widget);
        app.world_mut().write_message(PointerInput::new(
            pointer_id,
            location(),
            PointerAction::Press(PointerButton::Primary),
        ));
        app.update();
        assert!(app.world().get::<ButtonPress>(widget).is_some());
        assert!(app.world().get::<crate::WidgetFocused>(widget).is_some());
        assert!(app.world().get::<WidgetFocusVisible>(widget).is_none());
        let expected = VisualSlotOverride {
            fill_color: Some(PRESS_FILL),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "a pointer press must show pressed state without the keyboard focus border",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));

        app.world_mut()
            .write_message(WidgetInput::FocusNext { window });
        // Keep the held pointer over the button while testing focus alone.
        write_widget_hit(&mut app, pointer_id, camera, widget);
        app.update();

        assert!(app.world().get::<WidgetFocusVisible>(widget).is_some());
        let expected = VisualSlotOverride {
            fill_color: Some(PRESS_FILL),
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "keyboard traversal must reveal the indicator without changing the focused widget",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));
    }

    /// One application focus operation held until [`trigger_queued_focus_change`]
    /// takes it inside `Update`.
    #[derive(Clone, Copy)]
    enum FocusChange {
        Request { window: Entity, widget: Entity },
        Clear { window: Entity },
    }

    #[derive(Default, Resource)]
    struct QueuedFocusChange(Option<FocusChange>);

    /// Triggers the queued [`FocusChange`] from inside `Update`. Scheduled after
    /// `WidgetSystems::SemanticInput` and before
    /// `WidgetSystems::FocusCommandsApplied`, so the `RequestWidgetFocus` /
    /// `ClearWidgetFocus` observer commands are applied by the production focus
    /// fence rather than a test-side `World::flush`.
    fn trigger_queued_focus_change(
        mut queued_focus_change: ResMut<QueuedFocusChange>,
        mut commands: Commands,
    ) {
        match queued_focus_change.0.take() {
            Some(FocusChange::Request { window, widget }) => {
                commands.trigger(RequestWidgetFocus { window, widget });
            },
            Some(FocusChange::Clear { window }) => {
                commands.trigger(ClearWidgetFocus { window });
            },
            None => {},
        }
    }

    fn queue_focus_change(app: &mut App, focus_change: FocusChange) {
        app.world_mut().resource_mut::<QueuedFocusChange>().0 = Some(focus_change);
    }

    #[test]
    fn application_focus_request_is_visible_to_presentation_in_the_same_frame() {
        let mut app = integrated_test_app();
        app.init_resource::<QueuedFocusChange>().add_systems(
            Update,
            trigger_queued_focus_change
                .after(WidgetSystems::SemanticInput)
                .before(WidgetSystems::FocusCommandsApplied),
        );
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.focused(Appearance::new().border_color(FOCUS_BORDER))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        assert_eq!(root_override(&app, widget), None);

        queue_focus_change(&mut app, FocusChange::Request { window, widget });
        app.update();

        let expected = VisualSlotOverride {
            border_color: Some(FOCUS_BORDER),
            ..VisualSlotOverride::default()
        };
        assert_eq!(
            root_override(&app, widget),
            Some(expected.clone()),
            "application focus must restyle in the same update that triggers the request",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), Some(expected));

        queue_focus_change(&mut app, FocusChange::Clear { window });
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "application focus clear must restyle in the same update that triggers it",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn disabled_presentation_adds_no_second_panel_or_computed_change() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.disabled(
                    Appearance::new()
                        .background(DISABLED_FILL)
                        .border_color(DISABLED_BORDER),
                )
            }),
        );
        app.update();
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // A panel-level cascade edge restyles without touching the panel or
        // its computed output at all.
        let before = computed_tick(&app, panel);
        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            computed_tick(&app, panel),
            before,
            "a cascade disabled edge must not refresh the computed panel",
        );
        assert_eq!(
            root_override(&app, widget),
            Some(VisualSlotOverride {
                fill_color: Some(DISABLED_FILL),
                border_color: Some(DISABLED_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        // A widget-local authored edit performs its one visual-only computed
        // refresh; presentation then adds no second change.
        app.world_mut()
            .commands()
            .entity(panel)
            .inherit_widget_interactivity();
        app.world_mut().flush();
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_none());
        let before = computed_tick(&app, panel);
        let result =
            app.world_mut()
                .run_system_once(move |mut writer: crate::PanelWidgetWriter| {
                    writer.override_interactivity(widget, WidgetInteractivity::Disabled)
                });
        assert_eq!(result.ok(), Some(true));
        app.update();
        let after_edit = computed_tick(&app, panel);
        assert_ne!(
            after_edit, before,
            "the authored edit performs its Phase 2 visual-only refresh",
        );
        app.update();
        app.update();
        assert!(app.world().get::<WidgetDisabled>(widget).is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|value| value.fill_color),
            Some(DISABLED_FILL),
        );
        assert_eq!(
            computed_tick(&app, panel),
            after_edit,
            "presentation must not add another panel/computed change",
        );
    }

    #[test]
    fn repeated_identical_state_leaves_the_override_tick_unchanged() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let tick = app
            .world()
            .entity(widget)
            .get_ref::<WidgetVisualOverrides>()
            .map(|overrides| overrides.last_changed());
        assert!(tick.is_some());

        app.update();
        app.update();
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<WidgetVisualOverrides>()
                .map(|overrides| overrides.last_changed()),
            tick,
            "an unchanged state must not mark the override component changed",
        );
    }

    #[test]
    fn button_state_edges_update_overrides_once_then_leave_quiet_frames_unchanged() {
        let mut app = integrated_test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .pressed(Appearance::new().background(PRESS_FILL))
                    .focused(Appearance::new().background(FOCUS_FILL))
                    .disabled(Appearance::new().background(DISABLED_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);

        // Hover insertion writes the authored hovered layer, then the next
        // frame remains quiet.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        let hovered_tick = override_tick(&app, widget);
        assert!(hovered_tick.is_some());
        assert_override_quiet(&mut app, widget, hovered_tick);

        // Both aggregate values mean hovered in the state resolver, so this
        // input-value edge legitimately preserves the resolved override.
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Pressed);
        app.update();
        assert_eq!(override_tick(&app, widget), hovered_tick);
        assert_override_quiet(&mut app, widget, hovered_tick);

        // The private press marker supplies the distinct pressed appearance.
        app.world_mut().entity_mut(widget).insert(ButtonPress);
        app.update();
        let pressed_tick = override_tick(&app, widget);
        assert!(pressed_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(PRESS_FILL),
        );
        assert_ne!(pressed_tick, hovered_tick);
        assert_override_quiet(&mut app, widget, pressed_tick);

        app.world_mut().entity_mut(widget).remove::<ButtonPress>();
        app.update();
        let restored_hover_tick = override_tick(&app, widget);
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(HOVER_FILL),
        );
        assert_ne!(restored_hover_tick, pressed_tick);
        assert_override_quiet(&mut app, widget, restored_hover_tick);

        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);

        app.world_mut()
            .trigger(RequestWidgetFocus { window, widget });
        app.world_mut().flush();
        app.update();
        let focused_tick = override_tick(&app, widget);
        assert!(focused_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(FOCUS_FILL),
        );
        assert_override_quiet(&mut app, widget, focused_tick);

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);

        app.world_mut()
            .commands()
            .entity(panel)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.world_mut().flush();
        app.update();
        app.update();
        let disabled_tick = override_tick(&app, widget);
        assert!(disabled_tick.is_some());
        assert_eq!(
            root_override(&app, widget).and_then(|override_value| override_value.fill_color),
            Some(DISABLED_FILL),
        );
        assert_override_quiet(&mut app, widget, disabled_tick);

        app.world_mut()
            .commands()
            .entity(panel)
            .inherit_widget_interactivity();
        app.world_mut().flush();
        app.update();
        app.update();
        assert_eq!(override_tick(&app, widget), None);
        assert_override_quiet(&mut app, widget, None);
    }

    #[test]
    fn interaction_removal_presents_the_edge_back_to_normal() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            button_tree_with(BUTTON_ID, |element| {
                element.hovered(Appearance::new().background(HOVER_FILL))
            }),
        );
        app.update();
        let widget = resolve_widget(&mut app, panel);
        app.world_mut()
            .entity_mut(widget)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(root_override(&app, widget).is_some());

        app.world_mut()
            .entity_mut(widget)
            .remove::<PickingInteraction>();
        app.update();
        assert_eq!(
            root_override(&app, widget),
            None,
            "an aggregate removal must clear the stale hover override",
        );
        assert_eq!(indexed_root_override(&app, panel, widget), None);
    }

    #[test]
    fn state_overrides_leave_unrelated_slots_and_widgets_untouched() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");

        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert!(root_override(&app, first).is_some());
        assert_eq!(
            root_override(&app, second),
            None,
            "an unhovered peer button must stay unstyled",
        );
        assert_eq!(indexed_root_override(&app, panel, second), None);
        let first_index = root_element_index(&app, first);
        let second_index = root_element_index(&app, second);
        assert_ne!(first_index, second_index);
    }

    #[test]
    fn state_edges_do_not_rebuild_same_kind_peer_overrides() {
        let mut app = integrated_test_app();
        let panel = spawn_panel(
            &mut app,
            two_styled_buttons_tree(HOVER_FILL, PEER_HOVER_FILL),
        );
        app.update();
        let first = widget_by_id(&mut app, panel, "first");
        let second = widget_by_id(&mut app, panel, "second");

        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Hovered);
        app.world_mut()
            .entity_mut(second)
            .insert(PickingInteraction::Hovered);
        app.update();
        assert!(app.world().get::<WidgetVisualOverrides>(first).is_some());
        assert!(app.world().get::<WidgetVisualOverrides>(second).is_some());

        app.world_mut()
            .entity_mut(second)
            .remove::<WidgetVisualOverrides>();
        app.world_mut()
            .entity_mut(first)
            .insert(PickingInteraction::Pressed);
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(second).is_none(),
            "a changed interaction on one button must not rebuild its peer override",
        );

        app.world_mut()
            .entity_mut(first)
            .remove::<PickingInteraction>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(second).is_none(),
            "an interaction removal on one button must not rebuild its peer override",
        );
    }
}
