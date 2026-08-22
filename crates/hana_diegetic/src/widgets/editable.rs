//! Editable-field widget presentation.

use std::collections::HashSet;

use bevy::picking::hover::PickingInteraction;
use bevy::prelude::Changed;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Has;
use bevy::prelude::Or;
use bevy::prelude::Query;
use bevy::prelude::RemovedComponents;
use bevy::prelude::With;

use super::WidgetDisabled;
use super::WidgetFocusVisible;
use super::WidgetKind;
use super::WidgetOf;
use super::WidgetState;
use super::WidgetVisualOverrides;
use super::WidgetVisualSlots;
use super::visual;
use crate::DiegeticPanel;
use crate::cascade::Resolved;

/// Maps each changed editable field's live state onto its retained visual overrides.
///
/// Focus reads [`WidgetFocusVisible`], hover reads the all-pointer
/// [`PickingInteraction`] aggregate, and disabled reads [`WidgetDisabled`]. A
/// field has no press, so [`WidgetState::Pressed`] never applies and its
/// element does not offer [`crate::El::pressed`]. Each `Changed` query and
/// [`RemovedComponents`] stream is consumed here, so a quiet frame never walks
/// the live fields.
pub(super) fn present_editable_state(
    changed: Query<
        (Entity, &WidgetKind),
        (
            With<WidgetOf>,
            Or<(
                Changed<Resolved<super::WidgetHoveredAppearance>>,
                Changed<Resolved<super::WidgetPressedAppearance>>,
                Changed<Resolved<super::WidgetFocusedAppearance>>,
                Changed<Resolved<super::WidgetDisabledAppearance>>,
                Changed<WidgetVisualSlots>,
                Changed<WidgetFocusVisible>,
                Changed<PickingInteraction>,
                Changed<WidgetDisabled>,
            )>,
        ),
    >,
    fields: Query<
        (
            Entity,
            &WidgetKind,
            &Resolved<super::WidgetHoveredAppearance>,
            &Resolved<super::WidgetPressedAppearance>,
            &Resolved<super::WidgetFocusedAppearance>,
            &Resolved<super::WidgetDisabledAppearance>,
            &WidgetOf,
            &WidgetVisualSlots,
            Option<&PickingInteraction>,
            Has<WidgetDisabled>,
            Has<WidgetFocusVisible>,
        ),
        With<WidgetOf>,
    >,
    kinds: Query<&WidgetKind, With<WidgetOf>>,
    panels: Query<&DiegeticPanel>,
    mut removed_focus: RemovedComponents<WidgetFocusVisible>,
    mut removed_interactions: RemovedComponents<PickingInteraction>,
    mut removed_disabled: RemovedComponents<WidgetDisabled>,
    mut overrides: Query<&mut WidgetVisualOverrides>,
    mut commands: Commands,
) {
    let mut dirty: HashSet<Entity> = changed
        .iter()
        .filter_map(|(entity, kind)| (*kind == WidgetKind::EditableField).then_some(entity))
        .collect();
    dirty.extend(
        removed_focus
            .read()
            .chain(removed_interactions.read())
            .chain(removed_disabled.read())
            .filter(|&entity| matches!(kinds.get(entity), Ok(WidgetKind::EditableField))),
    );
    for entity in dirty {
        let Ok((
            entity,
            kind,
            hovered,
            pressed,
            focused_appearance,
            disabled_appearance,
            widget_of,
            slots,
            interaction,
            disabled,
            focused,
        )) = fields.get(entity)
        else {
            continue;
        };
        if *kind != WidgetKind::EditableField {
            continue;
        }
        let active = [
            focused.then_some(WidgetState::Focused),
            matches!(
                interaction,
                Some(PickingInteraction::Hovered | PickingInteraction::Pressed)
            )
            .then_some(WidgetState::Hovered),
            disabled.then_some(WidgetState::Disabled),
        ];
        let panel = panels.get(widget_of.panel()).ok();
        let mut desired = WidgetVisualOverrides::default();
        visual::resolve_part_overrides(
            &mut desired,
            slots,
            hovered,
            pressed,
            focused_appearance,
            disabled_appearance,
            &active,
            panel,
        );
        visual::write_widget_overrides(entity, desired, &mut overrides, &mut commands);
    }
}

#[cfg(test)]
mod tests {
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Color;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Window;

    use super::*;
    use crate::Appearance;
    use crate::Border;
    use crate::CascadeEntityCommandsExt;
    use crate::ClearWidgetFocus;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::HeadlessLayoutPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeEditableFieldSpec;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::RequestWidgetFocus;
    use crate::WidgetInteractivity;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::VisualSlotId;
    use crate::widgets::VisualSlotOverride;
    use crate::widgets::WidgetsPlugin;

    const FIELD_ID: &str = "editable";
    const FOCUS_BORDER: Color = Color::srgb(0.95, 0.85, 0.25);
    const NORMAL_BORDER: Color = Color::srgb(0.30, 0.30, 0.30);
    const NORMAL_FILL: Color = Color::srgb(0.10, 0.10, 0.12);
    const HOVER_FILL: Color = Color::srgb(0.20, 0.40, 0.80);
    const DISABLED_BORDER: Color = Color::srgb(0.35, 0.35, 0.40);

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin));
        app
    }

    fn field_tree() -> crate::LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .border(Border::all(1.0, NORMAL_BORDER))
                .editable_field(FIELD_ID, field)
                .focused(Appearance::new().border_color(FOCUS_BORDER)),
            |_| {},
        );
        builder.build()
    }

    /// A field authoring every state layer its kind can reach: focus, hover,
    /// and disabled. A field has no press, so [`crate::El::pressed`] is not available on
    /// its element at all.
    fn state_layered_field_tree() -> crate::LayoutTree {
        let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(
            El::new()
                .background(NORMAL_FILL)
                .border(Border::all(1.0, NORMAL_BORDER))
                .editable_field(FIELD_ID, field)
                .focused(Appearance::new().border_color(FOCUS_BORDER))
                .hovered(Appearance::new().background(HOVER_FILL))
                .disabled(Appearance::new().border_color(DISABLED_BORDER)),
            |_| {},
        );
        builder.build()
    }

    fn two_state_layered_field_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        for id in ["first", "second"] {
            let field = ImeEditableFieldSpec::AppOwned(ImeAppOwnedFieldSpec::new("test"));
            builder.with(
                El::new()
                    .background(NORMAL_FILL)
                    .border(Border::all(1.0, NORMAL_BORDER))
                    .editable_field(id, field)
                    .focused(Appearance::new().border_color(FOCUS_BORDER))
                    .hovered(Appearance::new().background(HOVER_FILL))
                    .disabled(Appearance::new().border_color(DISABLED_BORDER)),
                |_| {},
            );
        }
        builder.build()
    }

    fn spawn_field(app: &mut App) -> Entity { spawn_field_tree(app, field_tree()) }

    fn spawn_field_panel(app: &mut App, tree: crate::LayoutTree) -> Entity {
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(tree)
            .build();
        assert!(panel.is_ok());
        let panel = panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        app.update();
        panel
    }

    fn field_by_id(app: &mut App, panel: Entity, id: &'static str) -> Entity {
        let result = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named(id))
            });
        assert!(result.is_ok());
        result.ok().flatten().unwrap_or(Entity::PLACEHOLDER)
    }

    fn spawn_field_tree(app: &mut App, tree: crate::LayoutTree) -> Entity {
        let panel = spawn_field_panel(app, tree);
        field_by_id(app, panel, FIELD_ID)
    }

    fn root_override(app: &App, widget: Entity) -> Option<VisualSlotOverride> {
        let slots = app.world().get::<WidgetVisualSlots>(widget)?;
        let element_index = slots.element_index(VisualSlotId::EDITABLE_ROOT)?;
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

    #[test]
    fn visible_focus_presents_and_clears_the_authored_field_border() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let field = spawn_field(&mut app);
        assert_ne!(field, Entity::PLACEHOLDER);
        assert_eq!(root_override(&app, field), None);

        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: field,
        });
        app.world_mut().flush();
        app.update();

        assert_eq!(
            root_override(&app, field),
            Some(VisualSlotOverride {
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut().trigger(ClearWidgetFocus { window });
        app.world_mut().flush();
        app.update();

        assert_eq!(root_override(&app, field), None);
    }

    #[test]
    fn hover_presents_and_clears_the_authored_field_background() {
        let mut app = test_app();
        let field = spawn_field_tree(&mut app, state_layered_field_tree());
        assert_ne!(field, Entity::PLACEHOLDER);
        assert_eq!(root_override(&app, field), None);

        app.world_mut()
            .entity_mut(field)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            root_override(&app, field),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut()
            .entity_mut(field)
            .remove::<PickingInteraction>();
        app.update();

        assert_eq!(root_override(&app, field), None);
    }

    #[test]
    fn disabled_presents_and_clears_the_authored_field_border() {
        let mut app = test_app();
        let field = spawn_field_tree(&mut app, state_layered_field_tree());
        assert_ne!(field, Entity::PLACEHOLDER);

        app.world_mut()
            .commands()
            .entity(field)
            .override_widget_interactivity(WidgetInteractivity::Disabled);
        app.update();

        assert_eq!(
            root_override(&app, field),
            Some(VisualSlotOverride {
                border_color: Some(DISABLED_BORDER),
                ..VisualSlotOverride::default()
            }),
        );

        app.world_mut()
            .commands()
            .entity(field)
            .override_widget_interactivity(WidgetInteractivity::Enabled);
        app.update();

        assert_eq!(root_override(&app, field), None);
    }

    #[test]
    fn state_edges_do_not_rebuild_same_kind_peer_overrides() {
        let mut app = test_app();
        let panel = spawn_field_panel(&mut app, two_state_layered_field_tree());
        let first = field_by_id(&mut app, panel, "first");
        let second = field_by_id(&mut app, panel, "second");

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
            "a changed interaction on one field must not rebuild its peer override",
        );

        app.world_mut()
            .entity_mut(first)
            .remove::<PickingInteraction>();
        app.update();
        assert!(
            app.world().get::<WidgetVisualOverrides>(second).is_none(),
            "an interaction removal on one field must not rebuild its peer override",
        );
    }

    #[test]
    fn focus_and_hover_layer_independently_on_a_field() {
        let mut app = test_app();
        let window = app.world_mut().spawn(Window::default()).id();
        let field = spawn_field_tree(&mut app, state_layered_field_tree());
        assert_ne!(field, Entity::PLACEHOLDER);

        app.world_mut().trigger(RequestWidgetFocus {
            window,
            widget: field,
        });
        app.world_mut().flush();
        app.world_mut()
            .entity_mut(field)
            .insert(PickingInteraction::Hovered);
        app.update();

        assert_eq!(
            root_override(&app, field),
            Some(VisualSlotOverride {
                fill_color: Some(HOVER_FILL),
                border_color: Some(FOCUS_BORDER),
                ..VisualSlotOverride::default()
            }),
            "focus authors only the border and hover only the fill, so both survive",
        );
    }
}
