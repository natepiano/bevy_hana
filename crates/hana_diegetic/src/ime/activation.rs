//! Panel picking activation for authored editable fields.

use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::Camera;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Has;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::window::PrimaryWindow;

use super::ImeAuthoredFieldText;
use super::ImeCancelCause;
use super::ImeOpenSession;
use super::ImeTarget;
use super::PanelElementId;
use super::editor::PendingImePanelAnchor;
use super::field::FirstActivation;
use super::session::ActiveImeSession;
use crate::ComputedDiegeticPanel;
use crate::DiegeticPanel;
use crate::widgets;
use crate::widgets::PanelWidget;
use crate::widgets::SemanticWidgetIntent;
use crate::widgets::WidgetFocusAuthority;
use crate::widgets::WidgetKind;
use crate::widgets::WidgetOf;

/// Activation history belongs to the owner, so replacing a field's layout cannot rearm it.
#[derive(Component, Default)]
pub(super) struct ActivatedImeFields(Vec<PanelElementId>);

#[derive(Clone, Copy, Eq, PartialEq)]
pub(super) enum ActivationSelection {
    /// Preserve the target's usual opening selection and any pointer edit.
    DefaultSelection,
    /// Start with a caret and apply any pointer edit.
    PlaceCaret,
    /// Select the whole buffer, overriding the opening pointer edit.
    SelectAll,
}

pub(super) fn first_activation_selection(
    request: &ImeOpenSession,
    owners: &mut Query<Option<&mut ActivatedImeFields>>,
    commands: &mut Commands,
) -> ActivationSelection {
    let initial_text = match request.field_spec.first_activation() {
        FirstActivation::DefaultSelection => return ActivationSelection::DefaultSelection,
        FirstActivation::SelectUntouched(initial_text) => initial_text,
    };
    let (owner, field_id) = match &request.target {
        ImeTarget::WorldPanelField { panel, field_id }
        | ImeTarget::ScreenPanelField { panel, field_id } => (*panel, field_id),
        ImeTarget::AppOwned { owner, field_id } => (*owner, field_id),
    };
    let Ok(activated) = owners.get_mut(owner) else {
        return ActivationSelection::PlaceCaret;
    };
    if let Some(mut activated) = activated {
        if activated.0.contains(field_id) {
            return ActivationSelection::PlaceCaret;
        }
        activated.0.push(field_id.clone());
    } else {
        commands
            .entity(owner)
            .insert(ActivatedImeFields(vec![field_id.clone()]));
    }
    if request.initial_text == initial_text.as_ref() {
        ActivationSelection::SelectAll
    } else {
        ActivationSelection::PlaceCaret
    }
}

/// Prevents editing an app-owned field while a cable owns its live readout.
#[derive(Component, Clone, Copy, Debug, Default)]
#[component(on_add = cancel_edit_on_lock)]
pub struct ImeFieldAuthoringLock;

fn cancel_edit_on_lock(mut world: DeferredWorld, context: HookContext) {
    let session = world.get_resource::<ActiveImeSession>().and_then(|active| {
        let target = active.active_target()?;
        let ImeFieldAuthoringTarget::Panel(panel) = field_authoring_target(target) else {
            return None;
        };
        (panel == context.entity)
            .then_some(active.active_session_id()?)
            .map(|session_id| super::ImeRequestCancel {
                session_id,
                cause: ImeCancelCause::Request,
            })
    });
    if let Some(cancel) = session {
        world.commands().trigger(cancel);
    }
}

enum ImeFieldAuthoringTarget {
    Panel(Entity),
    OtherEditor,
}

const fn field_authoring_target(target: &ImeTarget) -> ImeFieldAuthoringTarget {
    match target {
        ImeTarget::WorldPanelField { panel, .. } | ImeTarget::ScreenPanelField { panel, .. } => {
            ImeFieldAuthoringTarget::Panel(*panel)
        },
        ImeTarget::AppOwned { .. } => ImeFieldAuthoringTarget::OtherEditor,
    }
}

pub(super) fn cancel_locked_session(
    started: On<super::ImeStarted>,
    locks: Query<(), With<ImeFieldAuthoringLock>>,
    mut commands: Commands,
) {
    if let ImeFieldAuthoringTarget::Panel(panel) = field_authoring_target(&started.target)
        && locks.contains(panel)
    {
        commands.trigger(super::ImeRequestCancel {
            session_id: started.session_id,
            cause:      ImeCancelCause::Request,
        });
    }
}

pub(super) fn open_from_semantic_activation(
    intent: On<SemanticWidgetIntent>,
    widgets: Query<(&PanelWidget, &WidgetKind, &WidgetOf)>,
    panels: Query<(
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        Option<&RenderLayers>,
        Option<&ImeAuthoredFieldText>,
        Has<ImeFieldAuthoringLock>,
    )>,
    cameras: Query<(
        Entity,
        &Camera,
        &GlobalTransform,
        Option<&RenderTarget>,
        Option<&RenderLayers>,
    )>,
    windows: Query<(), With<Window>>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    focus_authority: Res<WidgetFocusAuthority>,
    mut pending_anchor: ResMut<PendingImePanelAnchor>,
    mut commands: Commands,
) {
    let SemanticWidgetIntent::Activate { entity, window } = *intent.event() else {
        return;
    };
    let Ok((widget, kind, widget_of)) = widgets.get(entity) else {
        return;
    };
    if *kind != WidgetKind::EditableField {
        return;
    }
    let panel_entity = widget_of.panel();
    let Ok((panel, computed, panel_layers, authored_text, locked)) = panels.get(panel_entity)
    else {
        return;
    };
    if locked {
        return;
    }
    let Some(record) = computed
        .field_records()
        .iter()
        .find(|record| !record.duplicate_id && record.field_id == *widget.id())
    else {
        return;
    };
    let camera = if panel.coordinate_space().is_screen() {
        None
    } else {
        let target_layers = panel_layers
            .cloned()
            .unwrap_or_else(|| RenderLayers::layer(0));
        let preferred = focus_authority.interaction_camera(window, entity);
        widgets::select_window_presentation_camera(
            preferred,
            window,
            &target_layers,
            &cameras,
            &windows,
            &primary_window,
        )
    };
    if !panel.coordinate_space().is_screen() && camera.is_none() {
        return;
    }

    pending_anchor.store(
        panel_entity,
        record.field_id.clone(),
        camera,
        window,
        record.bounds,
        record.presentation().clone(),
    );
    commands.trigger(ImeOpenSession {
        window,
        target: panel_field_target(panel, panel_entity, record.field_id.clone()),
        initial_text: initial_editor_text(record, authored_text),
        field_spec: record.field_spec.clone(),
        anchor: None,
    });
}

pub(super) fn initial_editor_text(
    record: &crate::PanelFieldRecord,
    authored_text: Option<&ImeAuthoredFieldText>,
) -> String {
    match authored_text {
        Some(authored) if authored.field_id() == &record.field_id => authored.text().to_owned(),
        _ => record.field_spec.initial_text(&record.display_text),
    }
}

pub(super) const fn panel_field_target(
    panel: &DiegeticPanel,
    panel_entity: Entity,
    field_id: crate::PanelElementId,
) -> ImeTarget {
    if panel.coordinate_space().is_screen() {
        ImeTarget::ScreenPanelField {
            panel: panel_entity,
            field_id,
        }
    } else {
        ImeTarget::WorldPanelField {
            panel: panel_entity,
            field_id,
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "test fixtures should fail at missing panel facts"
)]
mod tests {
    use bevy::camera::ComputedCameraValues;
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::input::InputPlugin;
    use bevy::picking::backend::HitData;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerId;
    use bevy::prelude::*;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowPlugin;
    use bevy::window::WindowRef;

    use super::*;
    use crate::Anchor;
    use crate::DiegeticTextMeasurer;
    use crate::El;
    use crate::HeadlessDiegeticUiPlugin;
    use crate::ImeAppOwnedFieldSpec;
    use crate::ImeBufferSnapshot;
    use crate::ImeCancelCause;
    use crate::ImeCursorState;
    use crate::ImeEditableFieldSpec;
    use crate::ImeInputBlocker;
    use crate::ImeInsertText;
    use crate::ImeRequestCancel;
    use crate::ImeTextChanged;
    use crate::PanelPicking;
    use crate::Sizing;
    use crate::Text;
    use crate::TextStyle;

    #[derive(Clone, Resource, Default)]
    struct FieldActivations {
        count:         usize,
        initial_texts: Vec<String>,
    }

    #[derive(Resource, Default)]
    struct LatestEdit(Option<ImeBufferSnapshot>);

    fn snapshot(app: &App) -> &ImeBufferSnapshot {
        app.world()
            .resource::<LatestEdit>()
            .0
            .as_ref()
            .expect("activation emitted a snapshot")
    }

    fn record_activation(
        activation: On<ImeOpenSession>,
        mut activations: ResMut<FieldActivations>,
    ) {
        activations.count += 1;
        activations
            .initial_texts
            .push(activation.initial_text.clone());
    }

    fn editable_panel(single_click: bool, field_spec: ImeAppOwnedFieldSpec) -> DiegeticPanel {
        let builder = DiegeticPanel::world()
            .size(0.5, 0.2)
            .world_height(0.2)
            .anchor(Anchor::Center)
            .picking(PanelPicking::INTERACTIVE);
        let builder = if single_click {
            builder.single_click_field_activation()
        } else {
            builder
        };
        builder
            .layout(|builder| {
                builder.with(
                    El::row()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .editable_field(
                            "name",
                            ImeEditableFieldSpec::AppOwned(field_spec),
                            TextStyle::new(0.03),
                        ),
                    |builder| {
                        builder.text(Text::new("Name", TextStyle::new(0.03)));
                    },
                );
            })
            .build()
            .expect("editable panel")
    }

    fn activation_fixture(
        single_click: bool,
        field_spec: ImeAppOwnedFieldSpec,
    ) -> (App, Entity, Entity, Entity, Vec3) {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            InputPlugin,
            WindowPlugin {
                primary_window: None,
                ..default()
            },
        ))
        .insert_resource(DiegeticTextMeasurer::default())
        .add_plugins(HeadlessDiegeticUiPlugin)
        .init_resource::<FieldActivations>()
        .init_resource::<LatestEdit>()
        .add_observer(
            |changed: On<ImeTextChanged>, mut latest: ResMut<LatestEdit>| {
                latest.0 = Some(changed.snapshot.clone());
            },
        )
        .add_observer(record_activation);
        let panel = app
            .world_mut()
            .spawn((
                editable_panel(single_click, field_spec),
                Transform::from_xyz(0.0, 1.0, 0.0),
            ))
            .id();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let camera = app
            .world_mut()
            .spawn((
                Camera {
                    computed: ComputedCameraValues {
                        clip_from_view: bevy::math::proj::perspective_infinite_reverse(
                            std::f32::consts::FRAC_PI_4,
                            1280.0 / 720.0,
                            0.1,
                        ),
                        target_info: Some(RenderTargetInfo {
                            physical_size: UVec2::new(1280, 720),
                            scale_factor:  1.0,
                        }),
                        ..default()
                    },
                    ..default()
                },
                Transform::from_xyz(0.0, 1.0, 1.0),
            ))
            .id();
        app.update();
        let field = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel")
            .field_records()
            .first()
            .expect("editable field");
        let point = Vec2::new(
            field.bounds.width.mul_add(0.5, field.bounds.x),
            field.bounds.height.mul_add(0.5, field.bounds.y),
        );
        let panel_data = app.world().get::<DiegeticPanel>(panel).expect("panel");
        let (ax, ay) = panel_data.anchor_offsets();
        let scale = panel_data.points_to_world();
        let hit = Vec3::new(
            point.x.mul_add(scale, -ax),
            (-point.y).mul_add(scale, ay) + 1.0,
            0.0,
        );
        (app, panel, window, camera, hit)
    }

    fn click_field(
        app: &mut App,
        panel: Entity,
        window: Entity,
        camera: Entity,
        hit: Vec3,
        count: u8,
    ) {
        let target = WindowRef::Entity(window)
            .normalize(None)
            .expect("window target");
        app.world_mut().trigger(PointerClick {
            entity: panel,
            pointer: Pointer::new(
                PointerId::Mouse,
                Location {
                    target:   NormalizedRenderTarget::Window(target),
                    position: Vec2::ZERO,
                },
            ),
            button: PointerButton::Primary,
            hit: HitData::new(camera, 0.0, Some(hit), None),
            duration: std::time::Duration::ZERO,
            count,
        });
        app.world_mut().flush();
    }

    fn activation(single_click: bool, field_spec: ImeAppOwnedFieldSpec) -> FieldActivations {
        let (mut app, panel, window, camera, hit) = activation_fixture(single_click, field_spec);
        click_field(&mut app, panel, window, camera, hit, 1);
        app.world().resource::<FieldActivations>().clone()
    }

    fn cancel_edit(app: &mut App) {
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .expect("field owns input");
        app.world_mut().trigger(ImeRequestCancel {
            session_id,
            cause: ImeCancelCause::Escape,
        });
        app.world_mut().flush();
    }

    #[test]
    fn initial_text_is_selected_once_even_when_an_edit_types_it_back() {
        let spec = ImeAppOwnedFieldSpec::new("name").select_all_on_first_activation("Name");
        let (mut app, panel, window, camera, hit) = activation_fixture(true, spec);
        click_field(&mut app, panel, window, camera, hit, 1);
        assert!(
            matches!(&snapshot(&app).cursor, ImeCursorState::Selection(selection)
            if selection.anchor.as_usize() == 0 && selection.focus.as_usize() == 4)
        );
        let session_id = app
            .world()
            .resource::<ImeInputBlocker>()
            .session_id()
            .expect("field owns input");
        app.world_mut().trigger(ImeInsertText {
            session_id,
            text: "Name".into(),
        });
        app.world_mut().flush();
        cancel_edit(&mut app);
        app.update();

        click_field(&mut app, panel, window, camera, hit, 1);
        assert!(
            matches!(snapshot(&app).cursor, ImeCursorState::Insertion(boundary)
            if boundary.as_usize() == 4)
        );
        let record = app
            .world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("computed panel")
            .field_records()
            .first()
            .expect("name field");
        let measurer = app.world().resource::<DiegeticTextMeasurer>();
        let first_letter =
            (measurer.measure_fn)("N", &record.presentation().text_style.as_measure()).width;
        let panel_data = app.world().get::<DiegeticPanel>(panel).expect("panel");
        let start = Vec3::new(
            first_letter
                .mul_add(0.25, record.bounds.x)
                .mul_add(panel_data.points_to_world(), -panel_data.anchor_offsets().0),
            hit.y,
            hit.z,
        );
        click_field(&mut app, panel, window, camera, start, 1);
        assert!(
            matches!(snapshot(&app).cursor, ImeCursorState::Insertion(boundary)
            if boundary.as_usize() == 0),
            "a click in the first letter places the caret before it: {:?}",
            snapshot(&app)
        );
        click_field(&mut app, panel, window, camera, hit, 2);
        assert!(
            matches!(&snapshot(&app).cursor, ImeCursorState::Selection(selection)
            if selection.anchor.as_usize() == 0 && selection.focus.as_usize() == 4)
        );
    }

    #[test]
    fn first_activation_is_consumed_even_without_an_edit() {
        let spec = ImeAppOwnedFieldSpec::new("name").select_all_on_first_activation("Name");
        let (mut app, panel, window, camera, hit) = activation_fixture(true, spec);
        click_field(&mut app, panel, window, camera, hit, 1);
        cancel_edit(&mut app);
        app.update();
        click_field(&mut app, panel, window, camera, hit, 1);
        assert!(matches!(
            snapshot(&app).cursor,
            ImeCursorState::Insertion(_)
        ));
    }

    #[test]
    fn changed_initial_text_uses_the_clicked_caret() {
        let spec = ImeAppOwnedFieldSpec::new("name")
            .with_initial_text("Changed")
            .select_all_on_first_activation("Name");
        let (mut app, panel, window, camera, hit) = activation_fixture(true, spec);
        click_field(&mut app, panel, window, camera, hit, 1);
        assert!(matches!(
            snapshot(&app).cursor,
            ImeCursorState::Insertion(_)
        ));
    }

    #[test]
    fn a_single_click_opens_an_opted_in_world_panel_field() {
        assert_eq!(activation(true, ImeAppOwnedFieldSpec::new("name")).count, 1);
    }

    #[test]
    fn a_default_world_panel_keeps_double_click_field_activation() {
        assert_eq!(
            activation(false, ImeAppOwnedFieldSpec::new("name")).count,
            0
        );
    }

    #[test]
    fn an_app_owned_field_opens_on_its_initial_edit_text() {
        let activations = activation(
            true,
            ImeAppOwnedFieldSpec::new("name").with_initial_text("Stored name"),
        );

        assert_eq!(activations.initial_texts, ["Stored name"]);
    }
}
