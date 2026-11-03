use std::collections::HashMap;

use bevy::ecs::system::EntityCommands;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Transform;
use bevy::prelude::World;
use bevy::prelude::error;

use super::ComputedWidgetRecord;
use super::PanelWidget;
use super::PanelWidgetIndex;
use super::PanelWidgets;
use super::SliderCancelCause;
use super::SliderCaptures;
use super::SliderState;
use super::StateAppearance;
use super::TooltipBoundary;
use super::WidgetAnchorRect;
use super::WidgetFocusable;
use super::WidgetKind;
use super::WidgetOf;
use super::WidgetSpec;
use super::WidgetVisualSlots;
use super::controls as button;
use super::controls as slider;
use super::controls::ButtonCallback;
use super::controls::ButtonCallbackHandle;
use super::controls::ButtonCallbackPolicy;
use super::controls::ButtonCancelCause;
use super::controls::ButtonCaptures;
use super::controls::ButtonLatch;
use super::controls::ButtonMode;
use crate::PanelElementId;
use crate::cascade::Cascade;
use crate::cascade::CascadeFrom;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelOwned;

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(super) struct WidgetPreorder(usize);

/// Reifies widget entities for every changed computed panel.
pub(super) fn reify_widgets(
    mut changed_panels: Query<
        (
            Entity,
            &DiegeticPanel,
            &GlobalTransform,
            &ComputedDiegeticPanel,
            Option<&PanelWidgets>,
            &mut PanelWidgetIndex,
        ),
        Changed<ComputedDiegeticPanel>,
    >,
    existing_widgets: Query<(
        &PanelWidget,
        &WidgetKind,
        &WidgetSpec,
        &Cascade<super::WidgetHoveredAppearance>,
        &Cascade<super::WidgetPressedAppearance>,
        &Cascade<super::WidgetFocusedAppearance>,
        &Cascade<super::WidgetDisabledAppearance>,
        &WidgetPreorder,
        &Transform,
        &WidgetAnchorRect,
        Option<&Cascade<super::WidgetInteractivity>>,
        Option<&CascadeFrom>,
        Option<&WidgetVisualSlots>,
        Option<&SliderState>,
        (Option<&ButtonLatch>, Option<&TooltipBoundary>),
    )>,
    mut button_captures: ResMut<ButtonCaptures>,
    mut slider_captures: ResMut<SliderCaptures>,
    mut commands: Commands,
) {
    for (panel_entity, panel, panel_global, computed, panel_widgets, mut widget_index) in
        &mut changed_panels
    {
        let existing_entities: &[Entity] = panel_widgets.map_or(&[], |widgets| &**widgets);
        let mut existing_by_id: HashMap<&PanelElementId, Entity> = existing_entities
            .iter()
            .filter_map(|entity| {
                existing_widgets
                    .get(*entity)
                    .ok()
                    .map(|(widget, ..)| (widget.id(), *entity))
            })
            .collect();

        let index_changed = existing_entities.len() != computed.widget_records().len()
            || computed.widget_records().iter().any(|record| {
                existing_by_id
                    .get(record.id())
                    .is_none_or(|entity| !widget_index.maps_to(record.id(), *entity))
            });
        let mut next_widget_index = HashMap::with_capacity(if index_changed {
            computed.widget_records().len()
        } else {
            0
        });
        for record in computed.widget_records() {
            let anchor_rect = WidgetAnchorRect::new(panel, record.rect());
            let visual_slots = WidgetVisualSlots::new(record.visual_slots().to_vec())
                .with_elements(record.visual_elements().to_vec())
                .with_generated_editor_elements(record.generated_editor_elements().to_vec())
                .with_part_appearances(record.part_appearances().to_vec());
            let entity = match existing_by_id.remove(record.id()) {
                None => spawn_widget(
                    &mut commands,
                    panel_entity,
                    panel_global,
                    record,
                    anchor_rect,
                    visual_slots,
                ),
                Some(entity) => {
                    update_widget(
                        &mut commands,
                        entity,
                        record.kind(),
                        record.authored(),
                        record.appearance(),
                        record.preorder(),
                        record.interactivity(),
                        anchor_rect,
                        visual_slots,
                        record.tooltip_boundary(),
                        panel_entity,
                        &existing_widgets,
                        &mut button_captures,
                        &mut slider_captures,
                    );
                    entity
                },
            };
            if index_changed {
                next_widget_index.insert(record.id().clone(), entity);
            }
        }

        for &entity in existing_entities {
            if existing_widgets.get(entity).map_or(true, |(widget, ..)| {
                existing_by_id.contains_key(widget.id())
            }) {
                button::cancel_button_press(
                    entity,
                    ButtonCancelCause::WidgetRemoved,
                    &mut button_captures,
                    &mut commands,
                );
                slider::cancel_slider_drag(
                    entity,
                    SliderCancelCause::WidgetRemoved,
                    &mut slider_captures,
                    &mut commands,
                );
                commands.entity(entity).despawn();
            }
        }

        if index_changed {
            widget_index.replace(next_widget_index);
        }
    }
}

fn spawn_widget(
    commands: &mut Commands<'_, '_>,
    panel: Entity,
    panel_global: &GlobalTransform,
    record: &ComputedWidgetRecord,
    anchor_rect: WidgetAnchorRect,
    visual_slots: WidgetVisualSlots,
) -> Entity {
    let authored = record.authored().clone();
    let transform = anchor_rect.transform();
    let global_transform = panel_global.mul_transform(transform);
    let callback = widget_callback(&authored).cloned();
    let toggle =
        matches!(&authored, WidgetSpec::Button(button) if button.mode() == ButtonMode::Toggle);
    let slider_state = match &authored {
        WidgetSpec::Slider(slider) => Some(slider.initial_state()),
        WidgetSpec::Button(_) | WidgetSpec::EditableField(_) => None,
    };
    let StateAppearance {
        hovered,
        pressed,
        focused,
        disabled,
    } = record.appearance().clone();
    let mut spawned = Entity::PLACEHOLDER;
    commands.entity(panel).with_children(|children| {
        spawned = children
            .spawn((
                PanelWidget::new(record.id().clone()),
                WidgetFocusable,
                WidgetOf::new(panel),
                record.kind(),
                authored,
                (hovered, pressed, focused, disabled),
                WidgetPreorder(record.preorder()),
                transform,
                global_transform,
                anchor_rect,
                record.interactivity(),
                CascadeFrom::new(panel),
                PanelOwned::from(panel),
                (visual_slots, record.tooltip_boundary()),
            ))
            .id();
    });
    if let Some(state) = slider_state {
        commands.entity(spawned).insert(state);
    }
    if toggle {
        commands.entity(spawned).insert(ButtonLatch::Released);
    }
    if let Some(callback) = callback {
        install_callback_handle(commands, spawned, callback);
    }
    spawned
}

fn update_widget(
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    kind: WidgetKind,
    authored: &WidgetSpec,
    appearance: &StateAppearance,
    preorder: usize,
    interactivity: Cascade<super::WidgetInteractivity>,
    anchor_rect: WidgetAnchorRect,
    visual_slots: WidgetVisualSlots,
    tooltip_boundary: TooltipBoundary,
    panel: Entity,
    existing_widgets: &Query<(
        &PanelWidget,
        &WidgetKind,
        &WidgetSpec,
        &Cascade<super::WidgetHoveredAppearance>,
        &Cascade<super::WidgetPressedAppearance>,
        &Cascade<super::WidgetFocusedAppearance>,
        &Cascade<super::WidgetDisabledAppearance>,
        &WidgetPreorder,
        &Transform,
        &WidgetAnchorRect,
        Option<&Cascade<super::WidgetInteractivity>>,
        Option<&CascadeFrom>,
        Option<&WidgetVisualSlots>,
        Option<&SliderState>,
        (Option<&ButtonLatch>, Option<&TooltipBoundary>),
    )>,
    button_captures: &mut ButtonCaptures,
    slider_captures: &mut SliderCaptures,
) {
    let Ok((
        _,
        existing_kind,
        existing_authored,
        existing_hovered,
        existing_pressed,
        existing_focused,
        existing_disabled,
        existing_preorder,
        existing_transform,
        existing_anchor_rect,
        existing_interactivity,
        existing_cascade_from,
        existing_visual_slots,
        existing_slider_state,
        (existing_latch, existing_tooltip_boundary),
    )) = existing_widgets.get(entity)
    else {
        return;
    };
    if *existing_kind != kind {
        match *existing_kind {
            WidgetKind::Button => {
                button::cancel_button_press(
                    entity,
                    ButtonCancelCause::WidgetKindChanged,
                    button_captures,
                    commands,
                );
            },
            WidgetKind::EditableField => {},
            WidgetKind::Slider => {
                slider::cancel_slider_drag(
                    entity,
                    SliderCancelCause::WidgetKindChanged,
                    slider_captures,
                    commands,
                );
            },
        }
        commands.entity(entity).insert(kind);
    }
    let mut widget = commands.entity(entity);
    if existing_authored != authored {
        widget.insert(authored.clone());
    }
    update_widget_appearance(
        &mut widget,
        existing_hovered,
        existing_pressed,
        existing_focused,
        existing_disabled,
        appearance,
    );
    if existing_preorder.0 != preorder {
        widget.insert(WidgetPreorder(preorder));
    }
    let transform = anchor_rect.transform();
    if *existing_transform != transform {
        widget.insert(transform);
    }
    if *existing_anchor_rect != anchor_rect {
        widget.insert(anchor_rect);
    }
    if existing_visual_slots != Some(&visual_slots) {
        widget.insert(visual_slots);
    }
    if existing_tooltip_boundary != Some(&tooltip_boundary) {
        widget.insert(tooltip_boundary);
    }
    if existing_interactivity != Some(&interactivity) {
        widget.insert(interactivity);
    }
    if existing_cascade_from.is_none_or(|relationship| relationship.target() != panel) {
        widget.insert(CascadeFrom::new(panel));
    }
    reconcile_slider_state(&mut widget, authored, existing_slider_state);
    reconcile_button_latch(&mut widget, authored, existing_latch);
    let existing_callback = widget_callback(existing_authored);
    let next_callback = widget_callback(authored);
    if existing_callback != next_callback {
        match next_callback.cloned() {
            Some(callback) => install_callback_handle(commands, entity, callback),
            None => {
                commands.entity(entity).remove::<ButtonCallbackHandle>();
            },
        }
    }
}

fn reconcile_slider_state(
    widget: &mut EntityCommands<'_>,
    authored: &WidgetSpec,
    existing_state: Option<&SliderState>,
) {
    match authored {
        WidgetSpec::Slider(slider) => match existing_state {
            None => {
                widget.insert(slider.initial_state());
            },
            Some(state) if !state.matches_configuration(slider) => {
                widget.insert(SliderState::with_configuration(slider, state.value()));
            },
            Some(_) => {},
        },
        WidgetSpec::Button(_) | WidgetSpec::EditableField(_) => {
            if existing_state.is_some() {
                widget.remove::<SliderState>();
            }
        },
    }
}

fn reconcile_button_latch(
    widget: &mut EntityCommands<'_>,
    authored: &WidgetSpec,
    existing_latch: Option<&ButtonLatch>,
) {
    if matches!(authored, WidgetSpec::Button(button) if button.mode() == ButtonMode::Toggle) {
        if existing_latch.is_none() {
            widget.insert(ButtonLatch::Released);
        }
    } else if existing_latch.is_some() {
        widget.remove::<ButtonLatch>();
    }
}

fn update_widget_appearance(
    widget: &mut EntityCommands<'_>,
    existing_hovered: &Cascade<super::WidgetHoveredAppearance>,
    existing_pressed: &Cascade<super::WidgetPressedAppearance>,
    existing_focused: &Cascade<super::WidgetFocusedAppearance>,
    existing_disabled: &Cascade<super::WidgetDisabledAppearance>,
    appearance: &StateAppearance,
) {
    if existing_hovered != &appearance.hovered {
        widget.insert(appearance.hovered.clone());
    }
    if existing_pressed != &appearance.pressed {
        widget.insert(appearance.pressed.clone());
    }
    if existing_focused != &appearance.focused {
        widget.insert(appearance.focused.clone());
    }
    if existing_disabled != &appearance.disabled {
        widget.insert(appearance.disabled.clone());
    }
}

const fn widget_callback(authored: &WidgetSpec) -> Option<&ButtonCallback> {
    match authored {
        WidgetSpec::Button(button) => match button.callback() {
            ButtonCallbackPolicy::NotifyObserversOnly => None,
            ButtonCallbackPolicy::AlsoRun(callback) => Some(callback),
        },
        WidgetSpec::EditableField(_) | WidgetSpec::Slider(_) => None,
    }
}

/// Builds the tracked handle for `callback` on the world and stores it on the
/// widget, replacing (and thereby releasing) any prior tracked handle.
fn install_callback_handle(
    commands: &mut Commands<'_, '_>,
    entity: Entity,
    callback: ButtonCallback,
) {
    commands.queue(move |world: &mut World| {
        let Ok(mut widget) = world.get_entity_mut(entity) else {
            return;
        };
        match callback.build_handle(&mut widget) {
            Ok(handle) => {
                widget.insert(ButtonCallbackHandle::new(handle));
            },
            Err(error) => error!(target: DIAGNOSTIC_LOG_TARGET, "failed to register button click callback: {error}"),
        }
    });
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;

    use bevy::camera::NormalizedRenderTarget;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::ecs::system::SystemIdMarker;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::App;
    use bevy::prelude::ChildOf;
    use bevy::prelude::Color;
    use bevy::prelude::Component;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::DetectChangesMut;
    use bevy::prelude::Entity;
    use bevy::prelude::In;
    use bevy::prelude::Mesh3d;
    use bevy::prelude::MeshMaterial3d;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Pointer;
    use bevy::prelude::PointerButton;
    use bevy::prelude::PointerClick;
    use bevy::prelude::PointerPress;
    use bevy::prelude::PointerRelease;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Window;
    use bevy::window::PrimaryWindow;

    use super::StateAppearance;
    use super::WidgetPreorder;
    use crate::Anchor;
    use crate::Appearance;
    use crate::Button;
    use crate::ButtonActivationSource;
    use crate::ButtonClicked;
    use crate::ButtonLatch;
    use crate::ComputedDiegeticPanel;
    use crate::DiegeticPanel;
    use crate::DiegeticPanelCommands;
    use crate::El;
    use crate::Fit;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidget;
    use crate::PanelWidgetReader;
    use crate::PanelWidgets;
    use crate::Slider;
    use crate::TextStyle;
    use crate::WidgetDisabledAppearance;
    use crate::WidgetFocusedAppearance;
    use crate::WidgetHoveredAppearance;
    use crate::WidgetInteractivity;
    use crate::WidgetOf;
    use crate::WidgetPressedAppearance;
    use crate::cascade::Cascade;
    use crate::cascade::CascadeFrom;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::PanelWidgetIndex;
    use crate::widgets::TooltipBoundary;
    use crate::widgets::VisualOverrideIndex;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetKind;
    use crate::widgets::WidgetSpec;
    use crate::widgets::WidgetVisualOverrides;
    use crate::widgets::WidgetVisualSlots;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::controls::ButtonCallbackHandle;
    use crate::widgets::controls::ButtonPress;

    const SCREEN_FIT_SPACER_WIDTH: f32 = 30.0;
    const SCREEN_FIT_WIDGET_HEIGHT: f32 = 10.0;
    const SCREEN_FIT_WIDGET_WIDTH: f32 = 20.0;

    #[derive(Component)]
    struct ApplicationData;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum CallbackSource {
        Authored,
        Replacement,
    }

    #[derive(Default, Resource)]
    struct CallbackRuns(Vec<(CallbackSource, ButtonActivationSource)>);

    fn record_authored_callback(click: In<ButtonClicked>, mut runs: ResMut<CallbackRuns>) {
        runs.0.push((CallbackSource::Authored, click.source));
    }

    fn record_replacement_callback(click: In<ButtonClicked>, mut runs: ResMut<CallbackRuns>) {
        runs.0.push((CallbackSource::Replacement, click.source));
    }

    fn callback_tree(button: Button) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().widget("action", button), |_| {});
        builder.build()
    }

    #[test]
    fn a_panel_tree_rebuild_keeps_a_latched_toggle_button_latched() {
        let mut app = test_app();
        let tree = callback_tree(Button::new().toggle());
        let panel = spawn_panel(&mut app, tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("toggle should be reified");
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );
        app.world_mut()
            .entity_mut(widget)
            .insert(ButtonLatch::Pressed);

        app.world_mut()
            .commands()
            .set_tree(panel, tree)
            .expect("rebuild should be accepted");
        app.update();

        assert_eq!(
            resolve_widget(&mut app, panel, PanelElementId::named("action")),
            Some(widget)
        );
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Pressed)
        );
    }

    #[test]
    fn changing_a_toggle_button_to_momentary_clears_its_latch() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, callback_tree(Button::new().toggle()))
            .expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("toggle should be reified");
        app.world_mut()
            .entity_mut(widget)
            .insert(ButtonLatch::Pressed);

        app.world_mut()
            .commands()
            .set_tree(panel, callback_tree(Button::new()))
            .expect("mode change should be accepted");
        app.update();

        assert_eq!(
            resolve_widget(&mut app, panel, PanelElementId::named("action")),
            Some(widget)
        );
        assert!(app.world().get::<ButtonLatch>(widget).is_none());

        app.world_mut()
            .commands()
            .set_tree(panel, callback_tree(Button::new().toggle()))
            .expect("return to toggle should be accepted");
        app.update();
        assert_eq!(
            app.world().get::<ButtonLatch>(widget),
            Some(&ButtonLatch::Released)
        );
    }

    fn widget_tree(ids: &[&str]) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for id in ids {
            builder.with(El::new().button(*id), |_| {});
        }
        builder.build()
    }

    fn appearance_tree(color: Color) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(Color::WHITE)
                .button("action")
                .hovered(Appearance::new().background(color)),
            |_| {},
        );
        builder.build()
    }

    fn hovered_empty_appearance() -> StateAppearance {
        StateAppearance {
            hovered: Cascade::Override(WidgetHoveredAppearance::new(Appearance::new())),
            ..StateAppearance::default()
        }
    }

    fn part_appearance_tree(include_leading_sibling: bool) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().button("action"), |builder| {
            if include_leading_sibling {
                builder.with(El::new(), |_| {});
            }
            builder.with(El::new(), |_| {});
        });
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn editable_display_tree() -> LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new().editable_field("gain", field, crate::TextStyle::new(10.0)),
            |builder| {
                builder.text(("display", TextStyle::new(10.0)));
                builder.with(El::new(), |_| {});
            },
        );
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn inline_editor_content_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::with_root(El::new());
        builder.with(El::new(), |_| {});
        builder.with(El::new(), |_| {});
        let mut tree = builder.build();
        let part = tree.len() - 1;
        assert!(tree.set_element_state_appearance(part, hovered_empty_appearance()));
        tree
    }

    fn part_appearance_keys(app: &App, widget: Entity) -> Option<Vec<usize>> {
        app.world().get::<WidgetVisualSlots>(widget).map(|slots| {
            slots
                .part_appearances()
                .iter()
                .map(|(element_index, _)| *element_index)
                .collect()
        })
    }

    fn slider_tree(id: &str, initial_value: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().widget(id, slider(initial_value)), |_| {});
        builder.build()
    }

    fn ranked_slider_tree(initial_value: f32, z_index: i8) -> LayoutTree {
        let slider = slider(initial_value);
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .size(20.0, 10.0)
                .widget("level", slider)
                .widget_interactivity(WidgetInteractivity::Disabled)
                .z_index(z_index),
            |_| {},
        );
        builder.with(El::new().size(20.0, 10.0).button("peer"), |_| {});
        builder.build()
    }

    fn slider(initial_value: f32) -> Slider { Slider::new(0.0..=10.0).value(initial_value) }

    fn slider_spec(initial_value: f32) -> WidgetSpec { WidgetSpec::Slider(slider(initial_value)) }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn spawn_panel(app: &mut App, tree: LayoutTree) -> Option<Entity> {
        let result = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(result.is_ok());
        let Ok(panel) = result else {
            return None;
        };
        Some(app.world_mut().spawn(panel).id())
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: PanelElementId) -> Option<Entity> {
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .ok()
            .flatten()
    }

    fn callback_system_entity(app: &App, widget: Entity) -> Option<Entity> {
        app.world()
            .get::<ButtonCallbackHandle>(widget)
            .map(ButtonCallbackHandle::system_entity)
    }

    fn system_marker_count(app: &mut App) -> usize {
        app.world_mut()
            .query::<&SystemIdMarker>()
            .iter(app.world())
            .count()
    }

    fn pointer_location() -> Location {
        Location {
            target:   NormalizedRenderTarget::None {
                width:  1,
                height: 1,
            },
            position: Vec2::ZERO,
        }
    }

    fn pointer_hit() -> HitData { HitData::new(Entity::PLACEHOLDER, 0.0, None, None) }

    fn press_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerPress {
            entity:  widget,
            pointer: Pointer::new(pointer_id, pointer_location()),
            button:  PointerButton::Primary,
            hit:     pointer_hit(),
            count:   1,
        });
        app.world_mut().flush();
    }

    fn click_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerClick {
            entity:   widget,
            pointer:  Pointer::new(pointer_id, pointer_location()),
            button:   PointerButton::Primary,
            hit:      pointer_hit(),
            duration: std::time::Duration::ZERO,
            count:    1,
        });
        app.world_mut().flush();
    }

    fn release_widget(app: &mut App, widget: Entity, pointer_id: PointerId) {
        app.world_mut().trigger(PointerRelease {
            entity:  widget,
            pointer: Pointer::new(pointer_id, pointer_location()),
            button:  PointerButton::Primary,
            hit:     pointer_hit(),
        });
        app.world_mut().flush();
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < 1e-6,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn reify_creates_child_relationship_and_lookup() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };

        assert!(
            resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none(),
            "a widget must not resolve before its first computed reification",
        );
        app.update();

        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        assert_eq!(
            app.world().get::<PanelWidget>(widget).map(PanelWidget::id),
            Some(&PanelElementId::named("action"))
        );
        assert_eq!(
            app.world().get::<WidgetOf>(widget).map(WidgetOf::panel),
            Some(panel)
        );
        assert_eq!(
            app.world().get::<ChildOf>(widget).map(ChildOf::parent),
            Some(panel)
        );
        assert!(
            app.world()
                .get::<PanelWidgets>(panel)
                .is_some_and(|widgets| widgets.contains(&widget))
        );
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("missing")).is_none());
    }

    #[test]
    fn reify_places_widget_transform_at_its_solved_panel_offset() {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::row().size(100.0, 50.0), |builder| {
            builder.with(El::new().size(30.0, 10.0), |_| {});
            builder.with(El::new().size(20.0, 10.0).button("offset"), |_| {});
        });
        let mut app = test_app();
        let panel = spawn_panel(&mut app, builder.build()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("offset"))
            .expect("widget should be reified");
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should remain live");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("widget should carry a transform");
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        let scale = panel_component.points_to_world();
        let rect = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .and_then(|computed| {
                computed
                    .widget_records()
                    .iter()
                    .find(|record| record.id() == &PanelElementId::named("offset"))
            })
            .map(crate::widgets::ComputedWidgetRecord::rect);
        let rect = rect.expect("widget should have computed bounds");
        let (center_x, center_y) = rect.center();
        assert_close(transform.translation.x, center_x.mul_add(scale, -anchor_x));
        assert_close(
            transform.translation.y,
            (-center_y).mul_add(scale, anchor_y),
        );
        assert!(app.world().get::<Mesh3d>(widget).is_none());
        assert!(
            app.world()
                .get::<MeshMaterial3d<StandardMaterial>>(widget)
                .is_none()
        );
    }

    #[test]
    fn centered_fit_panel_reifies_from_final_dimensions() {
        let mut tree = LayoutBuilder::with_root(El::column());
        tree.with(El::new().size(20.0, 10.0).button("centered"), |_| {});
        let panel = DiegeticPanel::world()
            .size(Fit, Fit)
            .anchor(Anchor::Center)
            .with_tree(tree.build())
            .build()
            .expect("fit panel should build");
        let mut app = test_app();
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let widget = resolve_widget(&mut app, panel, PanelElementId::named("centered"))
            .expect("centered widget should be reified");
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should remain live");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("widget should carry a transform");
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        assert!(anchor_x > 0.0);
        assert!(anchor_y > 0.0);
        assert_close(transform.translation.x, 0.0);
        assert_close(transform.translation.y, 0.0);
    }

    #[test]
    fn centered_screen_fit_reifies_off_origin_widget_from_final_dimensions() {
        let mut tree = LayoutBuilder::with_root(El::row());
        tree.with(
            El::new().size(SCREEN_FIT_SPACER_WIDTH, SCREEN_FIT_WIDGET_HEIGHT),
            |_| {},
        );
        tree.with(
            El::new()
                .size(SCREEN_FIT_WIDGET_WIDTH, SCREEN_FIT_WIDGET_HEIGHT)
                .button("screen-fit"),
            |_| {},
        );
        let panel = DiegeticPanel::screen()
            .size(Fit, Fit)
            .anchor(Anchor::Center)
            .with_tree(tree.build())
            .build()
            .expect("screen Fit panel should build");
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        let panel = app.world_mut().spawn(panel).id();

        app.update();

        let (scale, anchor_x, anchor_y, rect) = {
            let panel_component = app
                .world()
                .get::<DiegeticPanel>(panel)
                .expect("screen Fit panel should remain live");
            assert_close(
                panel_component.width(),
                SCREEN_FIT_SPACER_WIDTH + SCREEN_FIT_WIDGET_WIDTH,
            );
            assert_close(panel_component.height(), SCREEN_FIT_WIDGET_HEIGHT);
            let computed = app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("screen Fit panel should have computed output");
            computed
                .layout()
                .solved()
                .expect("screen Fit panel should have a computed layout result");
            let record = computed
                .widget_records()
                .iter()
                .find(|record| record.id() == &PanelElementId::named("screen-fit"))
                .expect("screen Fit widget should have a computed record");
            assert!(
                record.rect().x > 0.0,
                "widget should be off the panel origin"
            );
            let (anchor_x, anchor_y) = panel_component.anchor_offsets();
            assert_close(anchor_x, panel_component.width() * 0.5);
            assert_close(anchor_y, panel_component.height() * 0.5);
            (
                panel_component.points_to_world(),
                anchor_x,
                anchor_y,
                record.rect(),
            )
        };
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("screen-fit"))
            .expect("screen Fit widget should be reified");
        let transform = app
            .world()
            .get::<Transform>(widget)
            .expect("screen Fit widget should carry a transform");
        let (center_x, center_y) = rect.center();
        assert_close(transform.translation.x, center_x.mul_add(scale, -anchor_x));
        assert_close(
            transform.translation.y,
            (-center_y).mul_add(scale, anchor_y),
        );
    }

    #[test]
    fn removing_panel_role_despawns_owned_children_and_preserves_application_state() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(&["action"])).expect("panel should build");
        app.world_mut().entity_mut(panel).insert(ApplicationData);
        let application_child = app
            .world_mut()
            .spawn((ApplicationData, ChildOf(panel)))
            .id();
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");

        app.world_mut().entity_mut(panel).remove::<DiegeticPanel>();
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(app.world().get::<ApplicationData>(panel).is_some());
        assert!(app.world().get_entity(application_child).is_ok());
        assert!(app.world().get::<PanelWidgetIndex>(panel).is_none());
        assert!(app.world().get::<ComputedDiegeticPanel>(panel).is_none());
        assert!(app.world().get::<PanelWidgets>(panel).is_none());
        assert!(app.world().get_entity(widget).is_err());
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none());
    }

    #[test]
    fn identical_ids_resolve_independently_per_panel() {
        let mut app = test_app();
        let Some(first_panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        let Some(second_panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        app.update();

        let first = resolve_widget(&mut app, first_panel, PanelElementId::named("action"));
        let second = resolve_widget(&mut app, second_panel, PanelElementId::named("action"));
        assert!(first.is_some());
        assert!(second.is_some());
        assert_ne!(first, second);
    }

    #[test]
    fn identical_tree_replacement_preserves_widget_lookup() {
        let mut app = test_app();
        let tree = widget_tree(&["action"]);
        let Some(panel) = spawn_panel(&mut app, tree.clone()) else {
            return;
        };
        app.update();

        let before = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        assert!(before.is_some());
        let revision = app
            .world()
            .get::<DiegeticPanel>(panel)
            .map(DiegeticPanel::tree_revision)
            .map(u64::from);

        let result = app.world_mut().commands().set_tree(panel, tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        assert_eq!(after, before);
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(panel)
                .map(DiegeticPanel::tree_revision)
                .map(u64::from),
            revision.map(|revision| revision + 1)
        );
    }

    #[test]
    fn retained_widget_does_not_republish_unchanged_bounds_or_lookup() {
        let mut app = test_app();
        let tree = widget_tree(&["action"]);
        let panel = spawn_panel(&mut app, tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should exist");
        let before_boundary = app
            .world()
            .entity(widget)
            .get_ref::<TooltipBoundary>()
            .expect("tooltip bounds should exist")
            .last_changed();
        let before_index = app
            .world()
            .entity(panel)
            .get_ref::<PanelWidgetIndex>()
            .expect("lookup should exist")
            .last_changed();
        let before_slots = app
            .world()
            .entity(widget)
            .get_ref::<WidgetVisualSlots>()
            .expect("visual slots should exist")
            .last_changed();
        app.world_mut()
            .commands()
            .set_tree(panel, tree)
            .expect("identical tree should build");
        app.world_mut()
            .get_mut::<ComputedDiegeticPanel>(panel)
            .expect("computed panel should exist")
            .set_changed();
        app.update();
        assert_eq!(
            resolve_widget(&mut app, panel, PanelElementId::named("action")),
            Some(widget)
        );
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<TooltipBoundary>()
                .expect("tooltip bounds remain")
                .last_changed(),
            before_boundary
        );
        assert_eq!(
            app.world()
                .entity(panel)
                .get_ref::<PanelWidgetIndex>()
                .expect("lookup remains")
                .last_changed(),
            before_index
        );
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<WidgetVisualSlots>()
                .expect("visual slots remain")
                .last_changed(),
            before_slots
        );
    }

    #[test]
    fn reorder_reuses_entities_and_refreshes_preorder() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["first", "second"])) else {
            return;
        };
        app.update();
        let first_before = resolve_widget(&mut app, panel, PanelElementId::named("first"));
        let second_before = resolve_widget(&mut app, panel, PanelElementId::named("second"));

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["second", "first"]));
        assert!(result.is_ok());
        app.update();

        let first_after = resolve_widget(&mut app, panel, PanelElementId::named("first"));
        let second_after = resolve_widget(&mut app, panel, PanelElementId::named("second"));
        assert_eq!(first_before, first_after);
        assert_eq!(second_before, second_after);
        let Some(first) = first_after else {
            return;
        };
        let Some(second) = second_after else {
            return;
        };
        assert!(
            app.world()
                .get::<WidgetPreorder>(second)
                .zip(app.world().get::<WidgetPreorder>(first))
                .is_some_and(|(second_order, first_order)| second_order.0 < first_order.0)
        );
    }

    #[test]
    fn reified_widget_carries_four_appearance_cascades_without_state_appearance() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, appearance_tree(Color::srgb(1.0, 0.0, 0.0))) else {
            return;
        };
        app.update();
        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        let entity = app.world().entity(widget);

        assert!(matches!(
            entity.get::<Cascade<WidgetHoveredAppearance>>(),
            Some(Cascade::Override(_))
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetPressedAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetFocusedAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(matches!(
            entity.get::<Cascade<WidgetDisabledAppearance>>(),
            Some(Cascade::Inherit)
        ));
        assert!(!entity.contains_type_id(TypeId::of::<StateAppearance>()));
    }

    #[test]
    fn reauthoring_one_state_reinserts_only_its_cascade_channel() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, appearance_tree(Color::srgb(1.0, 0.0, 0.0))) else {
            return;
        };
        app.update();
        let Some(widget) = resolve_widget(&mut app, panel, PanelElementId::named("action")) else {
            return;
        };
        let before = (
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetHoveredAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetPressedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetFocusedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetDisabledAppearance>>()
                .map(|cascade| cascade.last_changed()),
        );
        let (
            Some(before_hovered),
            Some(before_pressed),
            Some(before_focused),
            Some(before_disabled),
        ) = before
        else {
            return;
        };

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, appearance_tree(Color::srgb(0.0, 0.0, 1.0)))
                .is_ok()
        );
        app.update();

        let after = (
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetHoveredAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetPressedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetFocusedAppearance>>()
                .map(|cascade| cascade.last_changed()),
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetDisabledAppearance>>()
                .map(|cascade| cascade.last_changed()),
        );
        let (Some(after_hovered), Some(after_pressed), Some(after_focused), Some(after_disabled)) =
            after
        else {
            return;
        };

        assert_ne!(after_hovered, before_hovered);
        assert_eq!(after_pressed, before_pressed);
        assert_eq!(after_focused, before_focused);
        assert_eq!(after_disabled, before_disabled);
    }

    #[test]
    fn reifying_structure_changes_replaces_part_appearance_indices() {
        let mut app = test_app();
        let tree = part_appearance_tree(false);
        let panel = spawn_panel(&mut app, tree).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let before = part_appearance_keys(&app, widget)
            .expect("widget should carry reified part appearances");
        assert_eq!(before.len(), 1);

        let tree = part_appearance_tree(true);
        assert!(app.world_mut().commands().set_tree(panel, tree).is_ok());
        app.update();

        let after = part_appearance_keys(&app, widget)
            .expect("widget should replace its reified part appearances");
        assert_eq!(after.len(), 1);
        assert_ne!(after, before);
        assert!(before.iter().all(|index| !after.contains(index)));
    }

    #[test]
    fn inline_editor_round_trip_replaces_part_appearance_indices() {
        let mut app = test_app();
        let mut authoritative_tree = editable_display_tree();
        let panel = spawn_panel(&mut app, authoritative_tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("gain"))
            .expect("field should be reified");
        let display_before = part_appearance_keys(&app, widget)
            .expect("field should carry reified part appearances");
        assert_eq!(display_before.len(), 1);

        let editor_content = inline_editor_content_tree();
        let mut editor_tree = authoritative_tree.clone();
        assert_eq!(
            editor_tree.set_field_editing_content(&PanelElementId::named("gain"), &editor_content),
            crate::layout::FieldDisplayTextUpdate::Updated,
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, editor_tree)
                .is_ok()
        );
        app.update();

        let editor = part_appearance_keys(&app, widget)
            .expect("field should replace its reified part appearances");
        assert_eq!(editor.len(), 1);
        assert_ne!(editor, display_before);
        assert!(display_before.iter().all(|index| !editor.contains(index)));

        assert_eq!(
            authoritative_tree.set_field_display_text(&PanelElementId::named("gain"), "edited"),
            crate::layout::FieldDisplayTextUpdate::Updated,
        );
        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, authoritative_tree)
                .is_ok()
        );
        app.update();

        let display_after = part_appearance_keys(&app, widget)
            .expect("field should restore its reified part appearances");
        assert_eq!(display_after, display_before);
        assert!(editor.iter().all(|index| !display_after.contains(index)));
    }

    #[test]
    fn removal_sweeps_widget_and_stale_lookup_returns_none() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["keep", "remove"])) else {
            return;
        };
        app.update();
        let removed = resolve_widget(&mut app, panel, PanelElementId::named("remove"));
        assert!(removed.is_some());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["keep"]));
        assert!(result.is_ok());
        app.update();

        assert!(app.world().get_entity(panel).is_ok());
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("remove")).is_none());
        assert!(removed.is_some_and(|entity| app.world().get_entity(entity).is_err()));
    }

    #[test]
    fn kind_replacement_retains_entity_and_replaces_authored_snapshot() {
        let mut app = test_app();
        let panel = spawn_panel(&mut app, widget_tree(&["control"]));
        assert!(panel.is_some());
        let Some(panel) = panel else {
            return;
        };
        app.update();
        let before = resolve_widget(&mut app, panel, PanelElementId::named("control"));
        assert!(before.is_some());
        let Some(before) = before else {
            return;
        };
        let interactivity_tick = app
            .world()
            .entity(before)
            .get_ref::<Cascade<WidgetInteractivity>>()
            .map(|authored| authored.last_changed());
        let relationship_tick = app
            .world()
            .entity(before)
            .get_ref::<CascadeFrom>()
            .map(|relationship| relationship.last_changed());
        let tree = slider_tree("control", 4.0);
        let expected_authored = slider_spec(4.0);

        let result = app.world_mut().commands().set_tree(panel, tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("control"));
        assert_eq!(after, Some(before));
        let Some(widget) = after else {
            return;
        };
        assert_eq!(
            app.world().get::<WidgetKind>(widget),
            Some(&WidgetKind::Slider)
        );
        assert_eq!(
            app.world().get::<WidgetSpec>(widget),
            Some(&expected_authored)
        );
        assert!(matches!(expected_authored, WidgetSpec::Slider(_)));
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<Cascade<WidgetInteractivity>>()
                .map(|authored| authored.last_changed()),
            interactivity_tick
        );
        assert_eq!(
            app.world()
                .entity(widget)
                .get_ref::<CascadeFrom>()
                .map(|relationship| relationship.last_changed()),
            relationship_tick
        );
    }

    #[test]
    fn visual_only_slider_edit_refreshes_snapshot_without_replacing_entity() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, slider_tree("level", 2.0)) else {
            return;
        };
        app.update();
        let before = resolve_widget(&mut app, panel, PanelElementId::named("level"));
        let next_tree = slider_tree("level", 8.0);

        let result = app.world_mut().commands().set_tree(panel, next_tree);
        assert!(result.is_ok());
        app.update();

        let after = resolve_widget(&mut app, panel, PanelElementId::named("level"));
        assert_eq!(before, after);
        let Some(widget) = after else {
            return;
        };
        let expected = slider_spec(8.0);
        assert_eq!(app.world().get::<WidgetSpec>(widget), Some(&expected));
    }

    #[test]
    fn visual_only_refresh_preserves_geometry_and_updates_rank_without_layout() {
        let mut app = test_app();
        let first_tree = ranked_slider_tree(2.0, -1);
        let panel = spawn_panel(&mut app, first_tree).expect("panel should build");
        app.update();
        let before_entity = resolve_widget(&mut app, panel, PanelElementId::named("level"))
            .expect("slider should be reified");
        let (rect, hit_area, rank, interactivity, layout_solves) = {
            let computed = app
                .world()
                .get::<ComputedDiegeticPanel>(panel)
                .expect("panel should have computed output");
            let record = computed
                .widget_records()
                .iter()
                .find(|record| record.id() == &PanelElementId::named("level"))
                .expect("slider should have a computed record");
            (
                record.rect(),
                record.hit_area(),
                record.interaction_rank(),
                record.interactivity(),
                computed.layout_solves(),
            )
        };
        let next_tree = ranked_slider_tree(8.0, 1);

        app.world_mut()
            .commands()
            .set_tree(panel, next_tree)
            .expect("visual-only replacement should be accepted");
        app.update();

        let after_entity = resolve_widget(&mut app, panel, PanelElementId::named("level"))
            .expect("slider should remain reified");
        assert_eq!(after_entity, before_entity);
        let expected = slider_spec(8.0);
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("panel should retain computed output");
        let record = computed
            .widget_records()
            .iter()
            .find(|record| record.id() == &PanelElementId::named("level"))
            .expect("slider should retain its computed record");
        assert_eq!(record.rect(), rect);
        assert_eq!(record.hit_area(), hit_area);
        assert_ne!(record.interaction_rank(), rank);
        assert_eq!(record.authored(), &expected);
        assert_eq!(record.interactivity(), interactivity);
        assert_eq!(computed.layout_solves(), layout_solves);
    }

    #[test]
    fn stale_entity_and_panel_despawn_are_safe() {
        let mut app = test_app();
        let Some(panel) = spawn_panel(&mut app, widget_tree(&["action"])) else {
            return;
        };
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"));
        let Some(widget) = widget else {
            return;
        };
        app.world_mut().entity_mut(widget).despawn();
        assert!(resolve_widget(&mut app, panel, PanelElementId::named("action")).is_none());

        let result = app
            .world_mut()
            .commands()
            .set_tree(panel, widget_tree(&["replacement"]));
        assert!(result.is_ok());
        app.update();
        let replacement = resolve_widget(&mut app, panel, PanelElementId::named("replacement"));
        assert!(replacement.is_some());
        app.world_mut().entity_mut(panel).despawn();
        assert!(replacement.is_some_and(|entity| app.world().get_entity(entity).is_err()));
    }

    #[test]
    fn identical_tree_replacement_reuses_the_tracked_callback_without_reregistering() {
        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let tree = callback_tree(Button::new().on_click(record_authored_callback));
        let panel = spawn_panel(&mut app, tree.clone()).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let system_entity = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");

        // The first replacement also registers `set_tree`'s own cached command
        // system, so the registered-system count is measured after it.
        app.world_mut()
            .commands()
            .set_tree(panel, tree.clone())
            .expect("identical tree should be accepted");
        app.update();
        assert_eq!(callback_system_entity(&app, widget), Some(system_entity));
        let registered_systems = system_marker_count(&mut app);

        app.world_mut()
            .commands()
            .set_tree(panel, tree)
            .expect("repeated identical tree should be accepted");
        app.update();

        assert_eq!(callback_system_entity(&app, widget), Some(system_entity));
        assert_eq!(system_marker_count(&mut app), registered_systems);

        let pointer_id = PointerId::Mouse;
        press_widget(&mut app, widget, pointer_id);
        click_widget(&mut app, widget, pointer_id);
        release_widget(&mut app, widget, pointer_id);
        assert_eq!(
            app.world().resource::<CallbackRuns>().0,
            [(
                CallbackSource::Authored,
                ButtonActivationSource::Pointer(pointer_id)
            )]
        );
    }

    #[test]
    fn callback_replacement_during_live_press_swaps_the_handle_and_keeps_the_press() {
        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let panel = spawn_panel(
            &mut app,
            callback_tree(Button::new().on_click(record_authored_callback)),
        )
        .expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let authored_system = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");

        let pointer_id = PointerId::Mouse;
        press_widget(&mut app, widget, pointer_id);
        assert!(app.world().get::<ButtonPress>(widget).is_some());

        app.world_mut()
            .commands()
            .set_tree(
                panel,
                callback_tree(Button::new().on_click(record_replacement_callback)),
            )
            .expect("replacement tree should be accepted");
        app.update();
        app.update();

        let replacement_system = callback_system_entity(&app, widget)
            .expect("replacement should install exactly one tracked handle");
        assert_ne!(replacement_system, authored_system);
        assert!(
            app.world().get_entity(authored_system).is_err(),
            "replacement must release only the prior tracked handle",
        );
        assert!(app.world().get_entity(replacement_system).is_ok());
        assert!(
            app.world().get::<ButtonPress>(widget).is_some(),
            "callback replacement must not disturb the live press",
        );

        click_widget(&mut app, widget, pointer_id);
        release_widget(&mut app, widget, pointer_id);
        assert_eq!(
            app.world().resource::<CallbackRuns>().0,
            [(
                CallbackSource::Replacement,
                ButtonActivationSource::Pointer(pointer_id)
            )]
        );
    }

    #[test]
    fn widget_removal_releases_the_final_tracked_callback() {
        let mut app = test_app();
        let panel = spawn_panel(
            &mut app,
            callback_tree(Button::new().on_click(record_authored_callback)),
        )
        .expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let system_entity = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");
        assert!(app.world().get_entity(system_entity).is_ok());

        app.world_mut()
            .commands()
            .set_tree(panel, LayoutBuilder::new(100.0, 50.0).build())
            .expect("empty tree should be accepted");
        app.update();
        app.update();

        assert!(app.world().get_entity(widget).is_err());
        assert!(
            app.world().get_entity(system_entity).is_err(),
            "final tracked-handle drop must release the registered system",
        );
    }

    #[test]
    fn slot_attachment_and_overrides_preserve_authored_spec_and_callback() {
        const TEST_SLOT: VisualSlotId = VisualSlotId::new(3);

        fn slotted_callback_tree(button: Button) -> LayoutTree {
            let mut builder = LayoutBuilder::new(100.0, 50.0);
            builder.with(
                El::new()
                    .background(Color::WHITE)
                    .widget("action", button)
                    .visual_slot(TEST_SLOT),
                |_| {},
            );
            builder.build()
        }

        let mut app = test_app();
        app.init_resource::<CallbackRuns>();
        let button = Button::new().on_click(record_authored_callback);
        let panel =
            spawn_panel(&mut app, callback_tree(button.clone())).expect("panel should build");
        app.update();
        let widget = resolve_widget(&mut app, panel, PanelElementId::named("action"))
            .expect("widget should be reified");
        let callback_system = callback_system_entity(&app, widget)
            .expect("reify should install a tracked callback handle");
        let authored_spec = app
            .world()
            .get::<WidgetSpec>(widget)
            .cloned()
            .expect("widget should carry its authored spec");
        assert!(
            app.world()
                .get::<WidgetVisualSlots>(widget)
                .is_some_and(|slots| slots.element_index(TEST_SLOT).is_none()),
            "an unslotted tree reifies empty slot references",
        );

        app.world_mut()
            .commands()
            .set_tree(panel, slotted_callback_tree(button))
            .expect("slotted tree should be accepted");
        app.update();

        let element_index = app
            .world()
            .get::<WidgetVisualSlots>(widget)
            .and_then(|slots| slots.element_index(TEST_SLOT))
            .expect("slot attachment should reify onto the widget");
        assert_eq!(
            callback_system_entity(&app, widget),
            Some(callback_system),
            "slot attachment must preserve the tracked callback handle",
        );
        assert_eq!(
            app.world().get::<WidgetSpec>(widget),
            Some(&authored_spec),
            "slot attachment must preserve the authored spec",
        );

        let mut overrides = WidgetVisualOverrides::default();
        overrides.set(
            TEST_SLOT,
            VisualSlotOverride::default().with_fill_color(Color::BLACK),
        );
        app.world_mut().entity_mut(widget).insert(overrides);
        app.update();

        assert!(
            app.world()
                .resource::<VisualOverrideIndex>()
                .get(panel, element_index)
                .is_some(),
            "the dispatched override should resolve through the reified slot",
        );
        assert_eq!(
            callback_system_entity(&app, widget),
            Some(callback_system),
            "slot updates must retain the unchanged button's callback system",
        );
        assert_eq!(app.world().get::<WidgetSpec>(widget), Some(&authored_spec));
    }
}
