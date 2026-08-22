//! Screen-space panel attachment resolution.

use bevy::platform::collections::HashSet;
use bevy::prelude::Entity;
use bevy::prelude::Local;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Window;
use hana_valence::AnchorPose;
use hana_valence::AttachmentResolveCandidate;
use hana_valence::AttachmentResolveDiagnostics;
use hana_valence::AttachmentResolverScratch;

use super::ScreenAttachmentResolveSkip;
use super::candidate;
use super::placement;
use super::placement::ScreenAttachmentPlacer;
use super::rect;
use super::window;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::ResolvedScreenPanelPosition;
use crate::screen_space::CandidateQueries;

pub(crate) type ScreenAttachmentResolveDiagnostics =
    AttachmentResolveDiagnostics<ScreenAttachmentResolveSkip>;

/// Per-system state for the screen-space attachment resolver.
///
/// Each registered [`resolve_screen_space_panel_attachments`] system owns one
/// instance as [`Local`] state. Both `candidates` and
/// [`AttachmentResolverScratch`] retain their allocations. Each invocation
/// clears `candidates` before reading current ECS state, drains it while
/// resolving, and clears all entity topology before returning.
///
/// [`Local`]: bevy::ecs::system::Local
#[derive(Default)]
pub(crate) struct ScreenAttachmentResolverScratch {
    candidates:          Vec<AttachmentResolveCandidate<ScreenAttachmentResolveSkip>>,
    attachment_resolver: AttachmentResolverScratch,
}

pub(crate) fn attachment_is_ready(
    source: Entity,
    position: &ResolvedScreenPanelPosition,
    diagnostics: &ScreenAttachmentResolveDiagnostics,
) -> bool {
    position.anchor_position.is_some() && diagnostics.current().all(|entry| entry.source != source)
}

/// Resolves screen-space panel attachments for this frame.
pub(super) fn resolve_screen_space_panel_attachments(
    windows: Query<(Entity, &Window)>,
    attachments: Query<(Entity, &PanelAttachmentAuthored, &PanelAnchorOffset)>,
    anchor_poses: Query<(Entity, &AnchorPose)>,
    candidate_queries: CandidateQueries,
    mut resolved_positions: Query<&mut ResolvedScreenPanelPosition>,
    mut diagnostics: ResMut<ScreenAttachmentResolveDiagnostics>,
    mut scratch: Local<ScreenAttachmentResolverScratch>,
) {
    let ScreenAttachmentResolverScratch {
        candidates,
        attachment_resolver,
    } = &mut *scratch;
    let window_sizes = window::window_size_lookup(&windows);
    let mut desired_placements = placement::desired_placement_map(&candidate_queries.panels);
    let mut depths = placement::panel_depths(
        &candidate_queries.panels,
        &resolved_positions,
        &candidate_queries.transforms,
    );
    let anchor_poses = placement::panel_anchor_pose_map(&anchor_poses);
    let attachments_by_source = placement::screen_attachment_map(&attachments);
    let widget_proxies = placement::screen_widget_proxy_map(
        &candidate_queries.proxy_placements,
        &candidate_queries.transforms,
    );
    let mut rects = rect::screen_panel_rects(
        &candidate_queries.panels,
        &candidate_queries.screen_targets,
        &resolved_positions,
        &candidate_queries.transforms,
        &candidate_queries.primary,
        &window_sizes,
    );
    candidates.clear();
    candidate::proxy_candidates(&candidate_queries, candidates);
    candidate::classify_candidates(&attachments, &candidate_queries, &window_sizes, candidates);
    let mut placer = ScreenAttachmentPlacer {
        rects:              &mut rects,
        desired_placements: &mut desired_placements,
        depths:             &mut depths,
        anchor_poses:       &anchor_poses,
        attachments:        &attachments_by_source,
        widget_proxies:     &widget_proxies,
        resolved_depths:    HashSet::default(),
    };
    hana_valence::resolve_attachments_with_scratch(
        candidates,
        placement::screen_attachment_resolve_reasons(),
        &mut diagnostics,
        attachment_resolver,
        |action| placer.handle(action),
    );
    placement::write_desired_placements(desired_placements, &mut resolved_positions);
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::f32::consts::FRAC_PI_2;
    use std::sync::Arc;

    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::App;
    use bevy::prelude::Bundle;
    use bevy::prelude::Commands;
    use bevy::prelude::Component;
    use bevy::prelude::DetectChanges;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::IntoScheduleConfigs;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Quat;
    use bevy::prelude::Query;
    use bevy::prelude::Ref;
    use bevy::prelude::RelationshipTarget;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Transform;
    use bevy::prelude::Update;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::With;
    use bevy::window::PrimaryWindow;
    use bevy::window::Window;
    use hana_valence::AnchorPose;
    use hana_valence::Displacement;
    use hana_valence::Orientation;
    use hana_valence::ResolvedAnchorGeometry;

    use super::ScreenAttachmentResolveDiagnostics;
    use super::ScreenAttachmentResolveSkip;
    use crate::DiegeticPanelCommands as _;
    use crate::El;
    use crate::Fit;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::LayoutTree;
    use crate::PanelAnchorOffset;
    use crate::PanelAttachment;
    use crate::PanelDimensionsChanged;
    use crate::PanelElementId;
    use crate::PanelEntityReader;
    use crate::PanelSystems;
    use crate::PanelWidget;
    use crate::PanelWidgetReader;
    use crate::PanelWidgets;
    use crate::Pt;
    use crate::Px;
    use crate::ScreenPosition;
    use crate::layout::Anchor;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::panel::DiegeticPanel;
    use crate::panel::PanelAttachmentAuthored;
    use crate::panel::PanelComponentOwnership;
    use crate::panel::ResolvedScreenPanelPosition;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::screen_space::ScreenSpaceSystems;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ScreenWidgetAnchorProxy;
    use crate::widgets::ScreenWidgetAnchoredHere;
    use crate::widgets::ScreenWidgetAnchoredTo;
    use crate::widgets::WidgetAnchorRect;
    use crate::widgets::WidgetOf;
    use crate::widgets::WidgetSystems;

    fn anchor_pose(rotation: Quat, translation: Vec3) -> AnchorPose {
        AnchorPose {
            rotation:    Orientation::try_from(rotation)
                .expect("test rotations are finite normalized quaternions"),
            translation: Displacement::from(translation),
        }
    }

    #[derive(Bundle)]
    struct TestAttachment {
        authored: PanelAttachmentAuthored,
        offset:   PanelAnchorOffset,
    }

    impl TestAttachment {
        const fn new(target: Entity, source: Anchor, target_anchor: Anchor) -> Self {
            Self {
                authored: PanelAttachmentAuthored::new(target, source, target_anchor),
                offset:   PanelAnchorOffset::ZERO,
            }
        }

        const fn with_offset(mut self, offset: PanelAnchorOffset) -> Self {
            self.offset = offset;
            self
        }
    }
    use crate::widgets::WidgetsPlugin;

    #[derive(Component)]
    struct SourcePanel;

    #[derive(Component)]
    struct DependentPanel;

    #[derive(Resource)]
    struct AttachmentWrite {
        source: Entity,
        target: Entity,
        done:   bool,
    }

    #[derive(Clone, Copy, Default)]
    enum WidgetAttachmentWriteState {
        #[default]
        Pending,
        Applied,
    }

    #[derive(Resource)]
    struct WidgetAttachmentWrite {
        owner:  Entity,
        source: Entity,
        state:  WidgetAttachmentWriteState,
    }

    #[derive(Resource, Default)]
    struct ResolverChangeLog(Vec<Vec<(Entity, Option<Vec2>)>>);

    const ASSERT_CLOSE_EPSILON: f32 = 1e-4;

    fn app_with_window() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.insert_resource(DiegeticTextMeasurer {
            measure_fn: Arc::new(|_text: &str, measure: &TextMeasure| TextDimensions {
                width:       measure.size,
                height:      measure.size,
                line_height: measure.size,
            }),
        });
        app.add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app.world_mut().spawn((
            Window {
                resolution: (800_u32, 600_u32).into(),
                ..Default::default()
            },
            PrimaryWindow,
        ));
        app
    }

    fn fixed_screen_panel(size: Vec2, anchor: Anchor, screen_position: Vec2) -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(size.x), Px(size.y))
            .anchor(anchor)
            .screen_position(screen_position.x, screen_position.y)
            .layout(|_| {})
            .build()
            .expect("screen panel builds")
    }

    fn fixed_screen_panel_in_window(
        window: Entity,
        size: Vec2,
        anchor: Anchor,
        screen_position: Vec2,
    ) -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(size.x), Px(size.y))
            .anchor(anchor)
            .screen_position(screen_position.x, screen_position.y)
            .window_entity(window)
            .layout(|_| {})
            .build()
            .expect("screen panel builds")
    }

    fn fit_screen_panel(anchor: Anchor, screen_position: Vec2) -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Fit, Fit)
            .anchor(anchor)
            .screen_position(screen_position.x, screen_position.y)
            .layout(|builder| {
                builder.text(("fit", TextStyle::new(10.0)));
            })
            .build()
            .expect("fit screen panel builds")
    }

    fn widget_tree(width: f32) -> LayoutTree {
        let mut builder = LayoutBuilder::new(200.0, 100.0);
        builder.with(El::new().size(Px(width), Px(30.0)).button("target"), |_| {});
        builder.build()
    }

    fn screen_widget_panel(width: f32, screen_position: Vec2) -> DiegeticPanel {
        DiegeticPanel::screen()
            .size(Px(200.0), Px(100.0))
            .anchor(Anchor::TopLeft)
            .screen_position(screen_position.x, screen_position.y)
            .with_tree(widget_tree(width))
            .build()
            .expect("screen widget panel builds")
    }

    fn screen_attachment_source(app: &mut App) -> Entity {
        app.world_mut()
            .spawn(fixed_screen_panel(
                Vec2::splat(20.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id()
    }

    fn widget_entity(app: &App, panel: Entity) -> Entity {
        app.world()
            .get::<PanelWidgets>(panel)
            .into_iter()
            .flat_map(PanelWidgets::iter)
            .find(|widget| {
                app.world()
                    .get::<PanelWidget>(*widget)
                    .is_some_and(|panel_widget| {
                        panel_widget.id() == &PanelElementId::named("target")
                    })
            })
            .expect("target widget is reified")
    }

    fn attach_to_screen_widget(
        app: &mut App,
        source: Entity,
        owner: Entity,
        widget: Entity,
        authored: PanelAttachment,
    ) {
        let target_id = PanelElementId::named("target");
        app.world_mut()
            .run_system_once(
                move |panels: PanelEntityReader,
                      widgets: PanelWidgetReader,
                      mut commands: Commands| {
                    let source = panels
                        .screen(source)
                        .expect("source has a screen panel handle");
                    let owner = panels
                        .screen(owner)
                        .expect("owner has a screen panel handle");
                    let target = widgets
                        .typed_entity(owner, &target_id)
                        .expect("target has a screen widget handle");
                    assert_eq!(target.entity(), widget);
                    commands.attach_to_widget(source, target, authored);
                },
            )
            .expect("screen widget attachment system runs");
    }

    fn queue_widget_attachment(
        panel_entities: PanelEntityReader,
        reader: PanelWidgetReader,
        mut attachments: Commands,
        mut write: ResMut<WidgetAttachmentWrite>,
    ) {
        if matches!(write.state, WidgetAttachmentWriteState::Applied) {
            return;
        }
        let id = PanelElementId::named("target");
        let Some(owner) = panel_entities.screen(write.owner) else {
            return;
        };
        let Some(source) = panel_entities.screen(write.source) else {
            return;
        };
        let Some(widget) = reader.typed_entity(owner, &id) else {
            return;
        };
        let authored = PanelAttachment::new(Anchor::TopLeft, Anchor::BottomRight)
            .with_offset(PanelAnchorOffset::new(Px(4.0), Px(7.0)));
        attachments.attach_to_widget(source, widget, authored);
        write.state = WidgetAttachmentWriteState::Applied;
    }

    fn assert_close(actual: f32, expected: f32) {
        assert!(
            (actual - expected).abs() < ASSERT_CLOSE_EPSILON,
            "expected {expected}, got {actual}",
        );
    }

    fn assert_vec2_close(actual: Vec2, expected: Vec2) {
        assert_close(actual.x, expected.x);
        assert_close(actual.y, expected.y);
    }

    fn assert_translation(app: &App, entity: Entity, expected: Vec2) {
        let transform = app
            .world()
            .get::<Transform>(entity)
            .expect("panel has transform");
        assert_close(transform.translation.x, expected.x);
        assert_close(transform.translation.y, expected.y);
    }

    fn assert_rotation_z(app: &App, entity: Entity, expected: f32) {
        let transform = app
            .world()
            .get::<Transform>(entity)
            .expect("panel has transform");
        assert_close(
            super::super::screen_in_plane_angle(transform.rotation),
            expected,
        );
    }

    fn assert_current_diagnostic(
        app: &App,
        source: Entity,
        target: Entity,
        reason: ScreenAttachmentResolveSkip,
    ) {
        let diagnostics = app.world().resource::<ScreenAttachmentResolveDiagnostics>();
        assert!(
            diagnostics.current().any(|entry| entry.source == source
                && entry.target == target
                && entry.reason == reason),
            "missing current diagnostic {reason:?}",
        );
    }

    fn assert_diagnostic_count(
        app: &App,
        source: Entity,
        target: Entity,
        reason: ScreenAttachmentResolveSkip,
        expected_count: u32,
    ) {
        let matching = app
            .world()
            .resource::<ScreenAttachmentResolveDiagnostics>()
            .entries()
            .filter(|entry| {
                entry.source == source && entry.target == target && entry.reason == reason
            })
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1);
        assert_eq!(matching[0].count, expected_count);
    }

    fn resolved_anchor_position(app: &App, entity: Entity) -> Option<Vec2> {
        app.world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position")
            .anchor_position
    }

    fn resolved_depth(app: &App, entity: Entity) -> Option<f32> {
        app.world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position")
            .depth
    }

    fn authored_depth(app: &App, entity: Entity) -> Option<f32> {
        app.world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position")
            .authored_depth
    }

    fn resolved_rotation(app: &App, entity: Entity) -> Option<f32> {
        app.world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position")
            .rotation
    }

    fn authored_rotation(app: &App, entity: Entity) -> Option<f32> {
        app.world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position")
            .authored_rotation
    }

    fn assert_depth(app: &App, entity: Entity, expected: f32) {
        let transform = app
            .world()
            .get::<Transform>(entity)
            .expect("panel has transform");
        assert_close(transform.translation.z, expected);
    }

    fn set_authored_depth(app: &mut App, entity: Entity, depth: f32) {
        app.world_mut()
            .get_mut::<Transform>(entity)
            .expect("panel has transform")
            .translation
            .z = depth;
    }

    fn set_authored_rotation(app: &mut App, entity: Entity, angle: f32) {
        app.world_mut()
            .get_mut::<Transform>(entity)
            .expect("panel has transform")
            .rotation = Quat::from_rotation_z(angle);
    }

    fn panel_anchor_screen_point(app: &App, entity: Entity, anchor: Anchor) -> Vec2 {
        let panel = app
            .world()
            .get::<DiegeticPanel>(entity)
            .expect("panel exists");
        let resolved_position = app
            .world()
            .get::<ResolvedScreenPanelPosition>(entity)
            .expect("panel has resolved position");
        let transform = app
            .world()
            .get::<Transform>(entity)
            .expect("panel has transform");
        let size = Vec2::new(panel.width(), panel.height());
        let panel_offset = anchor_offset(panel.anchor(), size);
        let anchor_offset = anchor_offset(anchor, size);
        resolved_position
            .anchor_position
            .expect("panel has resolved anchor position")
            + super::super::rotate_screen_offset(
                anchor_offset - panel_offset,
                super::super::screen_in_plane_angle(transform.rotation),
            )
    }

    fn anchor_offset(anchor: Anchor, size: Vec2) -> Vec2 {
        let (x, y) = anchor.offset(size.x, size.y);
        Vec2::new(x, y)
    }

    fn record_non_added_resolver_changes(
        panels: Query<(Entity, Ref<ResolvedScreenPanelPosition>)>,
        mut log: ResMut<ResolverChangeLog>,
    ) {
        let mut frame_changes = Vec::new();
        for (entity, resolved) in &panels {
            if resolved.is_changed() && !resolved.is_added() {
                frame_changes.push((entity, resolved.anchor_position));
            }
        }
        log.0.push(frame_changes);
    }

    #[test]
    fn screen_attachment_positions_source_anchor_against_target_anchor() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0))),
            ))
            .id();

        app.update();

        let resolved = app
            .world()
            .get::<ResolvedScreenPanelPosition>(source)
            .expect("source has resolved position");
        assert_eq!(resolved.anchor_position, Some(Vec2::new(100.0, 141.0)));
        assert_translation(&app, source, Vec2::new(-300.0, 159.0));
    }

    #[test]
    fn screen_attachment_applies_panel_anchor_pose_in_screen_plane() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0))),
                anchor_pose(
                    Quat::from_rotation_z(FRAC_PI_2),
                    Vec3::new(10.0, 20.0, 30.0),
                ),
            ))
            .id();

        app.update();

        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(110.0, 121.0))
        );
        assert_translation(&app, source, Vec2::new(-290.0, 179.0));
        assert_depth(&app, source, 30.0);
        assert_close(
            resolved_rotation(&app, source).expect("source has resolved rotation"),
            FRAC_PI_2,
        );
        assert_rotation_z(&app, source, FRAC_PI_2);
    }

    #[test]
    fn attachment_math_handles_different_panel_and_source_anchors() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::Center, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::BottomRight, Anchor::TopRight)
                    .with_offset(PanelAnchorOffset::new(Px(5.0), Px(2.0))),
            ))
            .id();

        app.update();

        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(280.0, 97.0))
        );
        assert_translation(&app, source, Vec2::new(-120.0, 203.0));
    }

    #[test]
    fn screen_pose_rotates_leaf_around_pinned_source_anchor() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::Center, Vec2::ZERO),
                TestAttachment::new(target, Anchor::BottomRight, Anchor::BottomLeft),
                anchor_pose(Quat::IDENTITY, Vec3::ZERO),
            ))
            .id();

        app.update();

        let identity_pin = panel_anchor_screen_point(&app, source, Anchor::BottomRight);
        assert_vec2_close(identity_pin, Vec2::new(100.0, 140.0));

        app.world_mut()
            .entity_mut(source)
            .insert(anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO));
        app.update();

        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(95.0, 165.0))
        );
        assert_vec2_close(
            panel_anchor_screen_point(&app, source, Anchor::BottomRight),
            identity_pin,
        );
        assert_rotation_z(&app, source, FRAC_PI_2);
    }

    #[test]
    fn out_of_plane_screen_pose_rotation_has_no_screen_effect() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0))),
                anchor_pose(Quat::from_rotation_x(FRAC_PI_2), Vec3::ZERO),
            ))
            .id();

        app.update();

        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(100.0, 141.0))
        );
        assert_close(
            resolved_rotation(&app, source).expect("source has resolved rotation"),
            0.0,
        );
        let transform = app
            .world()
            .get::<Transform>(source)
            .expect("panel has transform");
        assert_eq!(transform.rotation, Quat::IDENTITY);
    }

    #[test]
    fn first_frame_fit_target_resolves_before_attachment_positioning() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fit_screen_panel(Anchor::TopLeft, Vec2::new(100.0, 100.0)))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0))),
            ))
            .id();

        app.update();

        let target_panel = app
            .world()
            .get::<DiegeticPanel>(target)
            .expect("target still exists");
        assert!(target_panel.width() > 0.0);
        assert!(target_panel.height() > 0.0);
        assert_translation(
            &app,
            source,
            Vec2::new(-300.0, 300.0 - 100.0 - target_panel.height() - 1.0),
        );
    }

    fn attach_dependent_from_dimension_event(
        event: On<PanelDimensionsChanged>,
        sources: Query<(), With<SourcePanel>>,
        dependents: Query<Entity, With<DependentPanel>>,
        panel_entities: PanelEntityReader,
        mut attachments: Commands,
    ) {
        if sources.get(event.event().entity).is_err() {
            return;
        }
        for dependent in &dependents {
            let Some(source) = panel_entities.screen(dependent) else {
                continue;
            };
            let Some(target) = panel_entities.screen(event.event().entity) else {
                continue;
            };
            let authored = PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft)
                .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0)));
            attachments.attach_to_panel(source, target, authored);
        }
    }

    #[test]
    fn dimension_observer_can_queue_attachment_for_same_update_positioning() {
        let mut app = app_with_window();
        app.add_observer(attach_dependent_from_dimension_event);
        let target = app
            .world_mut()
            .spawn((
                SourcePanel,
                fixed_screen_panel(
                    Vec2::new(200.0, 40.0),
                    Anchor::TopLeft,
                    Vec2::new(100.0, 100.0),
                ),
            ))
            .id();
        let dependent = app
            .world_mut()
            .spawn((
                DependentPanel,
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
            ))
            .id();

        app.update();

        let attachment = app
            .world()
            .get::<PanelAttachmentAuthored>(dependent)
            .expect("observer inserted attachment authoring");
        assert_eq!(attachment.target(), target);
        assert_translation(&app, dependent, Vec2::new(-300.0, 159.0));
    }

    fn queue_attachment_once(
        panel_entities: PanelEntityReader,
        mut attachments: Commands,
        mut write: ResMut<AttachmentWrite>,
    ) {
        if write.done {
            return;
        }
        let Some(source) = panel_entities.screen(write.source) else {
            return;
        };
        let Some(target) = panel_entities.screen(write.target) else {
            return;
        };
        attachments.attach_to_panel(
            source,
            target,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
        );
        write.done = true;
    }

    #[test]
    fn command_writes_before_observer_flush_affect_current_update() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(50.0, 10.0),
                Anchor::TopLeft,
                Vec2::new(0.0, 0.0),
            ))
            .id();
        app.insert_resource(AttachmentWrite {
            source,
            target,
            done: false,
        });
        app.add_systems(
            Update,
            queue_attachment_once.before(ScreenSpaceSystems::FlushObserverCommands),
        );

        app.update();

        assert_translation(&app, source, Vec2::new(-300.0, 160.0));
    }

    #[test]
    fn command_writes_after_resolver_affect_next_update() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(50.0, 10.0),
                Anchor::TopLeft,
                Vec2::new(0.0, 0.0),
            ))
            .id();
        app.insert_resource(AttachmentWrite {
            source,
            target,
            done: false,
        });
        app.add_systems(
            Update,
            queue_attachment_once.after(PanelSystems::ResolvePanelAttachments),
        );

        app.update();

        assert_translation(&app, source, Vec2::new(-400.0, 300.0));

        app.update();

        assert_translation(&app, source, Vec2::new(-300.0, 160.0));
    }

    #[test]
    fn target_size_and_position_changes_reposition_dependent() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomRight),
            ))
            .id();

        app.update();
        assert_translation(&app, source, Vec2::new(-100.0, 160.0));

        app.world_mut()
            .entity_mut(target)
            .insert(fixed_screen_panel(
                Vec2::new(300.0, 50.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ));
        app.update();
        assert_translation(&app, source, Vec2::new(0.0, 150.0));

        {
            let mut target_panel = app
                .world_mut()
                .get_mut::<DiegeticPanel>(target)
                .expect("target still exists");
            assert!(target_panel.set_screen_position(Vec2::new(120.0, 130.0)));
        }
        app.update();
        assert_translation(&app, source, Vec2::new(20.0, 120.0));
    }

    #[test]
    fn registered_resolver_rereads_target_transform_after_recovery() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(100.0, 140.0))
        );

        app.world_mut().entity_mut(target).remove::<Transform>();
        app.update();
        assert_eq!(resolved_anchor_position(&app, source), None);
        assert_current_diagnostic(
            &app,
            source,
            target,
            ScreenAttachmentResolveSkip::TargetTransformMissing,
        );

        app.world_mut()
            .entity_mut(target)
            .insert(Transform::default());
        app.update();

        assert_eq!(
            resolved_anchor_position(&app, source),
            Some(Vec2::new(100.0, 140.0))
        );
        assert!(
            app.world()
                .resource::<ScreenAttachmentResolveDiagnostics>()
                .current()
                .all(|entry| entry.source != source)
        );
    }

    #[test]
    fn screen_position_screen_targets_track_window_resize() {
        let mut app = app_with_window();
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("primary window exists");
        let target = app
            .world_mut()
            .spawn(
                DiegeticPanel::screen()
                    .size(Px(200.0), Px(40.0))
                    .anchor(Anchor::BottomRight)
                    .layout(|_| {})
                    .build()
                    .expect("screen panel builds"),
            )
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::TopLeft),
            ))
            .id();

        app.update();
        assert_translation(&app, source, Vec2::new(200.0, -260.0));

        app.world_mut()
            .get_mut::<Window>(window)
            .expect("window exists")
            .resolution
            .set(1000.0, 700.0);
        app.update();

        assert_translation(&app, source, Vec2::new(300.0, -310.0));
    }

    #[test]
    fn primary_and_entity_window_refs_match_same_window() {
        let mut app = app_with_window();
        let window = app
            .world_mut()
            .query_filtered::<Entity, With<PrimaryWindow>>()
            .single(app.world())
            .expect("primary window exists");
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel_in_window(
                    window,
                    Vec2::new(50.0, 10.0),
                    Anchor::TopLeft,
                    Vec2::new(0.0, 0.0),
                ),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();

        assert_translation(&app, source, Vec2::new(-300.0, 160.0));
    }

    #[test]
    fn point_offsets_resolve_to_screen_pixels() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        assert_translation(&app, source, Vec2::new(-300.0, 160.0));

        app.world_mut().entity_mut(source).insert(
            TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                .with_offset(PanelAnchorOffset::new(Pt(12.0), Px(2.0))),
        );
        app.update();

        let resolved = app
            .world()
            .get::<ResolvedScreenPanelPosition>(source)
            .expect("source has resolved position");
        assert_eq!(resolved.anchor_position, Some(Vec2::new(116.0, 142.0)));
        assert_translation(&app, source, Vec2::new(-284.0, 158.0));
    }

    #[test]
    fn z_offset_writes_translation_depth_relative_to_target() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        set_authored_depth(&mut app, target, 3.0);
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0)).with_z(Px(5.0))),
            ))
            .id();

        app.update();

        assert_translation(&app, source, Vec2::new(-300.0, 159.0));
        assert_depth(&app, source, 8.0);
        assert_eq!(resolved_depth(&app, source), Some(8.0));
    }

    #[test]
    fn cancelling_authored_z_inputs_still_own_depth() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        set_authored_depth(&mut app, target, 3.0);
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::ZERO.with_z(Px(5.0))),
                anchor_pose(Quat::IDENTITY, Vec3::new(0.0, 0.0, -5.0)),
            ))
            .id();
        set_authored_depth(&mut app, source, 7.0);

        app.update();

        assert_depth(&app, source, 3.0);
        assert_eq!(resolved_depth(&app, source), Some(3.0));
        assert_eq!(authored_depth(&app, source), Some(7.0));

        app.world_mut()
            .entity_mut(source)
            .insert(TestAttachment::new(
                target,
                Anchor::TopLeft,
                Anchor::BottomLeft,
            ))
            .remove::<AnchorPose>();
        app.update();

        assert_eq!(resolved_depth(&app, source), None);
        assert_eq!(authored_depth(&app, source), None);
        assert_depth(&app, source, 7.0);
    }

    #[test]
    fn z_offset_chain_accumulates_depth_in_one_update() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::ZERO.with_z(Px(2.0))),
            ))
            .id();
        let leaf = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(25.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(middle, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::ZERO.with_z(Px(3.0))),
            ))
            .id();

        app.update();

        assert_depth(&app, middle, 2.0);
        assert_depth(&app, leaf, 5.0);
    }

    #[test]
    fn removing_attachment_restores_authored_depth() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::ZERO.with_z(Px(5.0))),
            ))
            .id();
        set_authored_depth(&mut app, source, 7.0);

        app.update();
        assert_depth(&app, source, 5.0);
        assert_eq!(resolved_depth(&app, source), Some(5.0));

        app.world_mut()
            .entity_mut(source)
            .remove::<TestAttachment>();
        app.update();

        assert_eq!(resolved_depth(&app, source), None);
        assert_depth(&app, source, 7.0);
    }

    #[test]
    fn removing_panel_anchor_pose_restores_authored_rotation() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
                anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO),
            ))
            .id();
        set_authored_rotation(&mut app, source, 0.0);

        app.update();
        assert_close(
            resolved_rotation(&app, source).expect("source has resolved rotation"),
            FRAC_PI_2,
        );
        assert_eq!(authored_rotation(&app, source), Some(0.0));
        assert_rotation_z(&app, source, FRAC_PI_2);

        app.world_mut().entity_mut(source).remove::<AnchorPose>();
        app.update();

        assert_eq!(resolved_rotation(&app, source), None);
        assert_eq!(authored_rotation(&app, source), None);
        assert_rotation_z(&app, source, 0.0);
    }

    #[test]
    fn zero_z_offset_keeps_authored_depth_untouched() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft)
                    .with_offset(PanelAnchorOffset::new(Px(0.0), Px(1.0))),
            ))
            .id();
        set_authored_depth(&mut app, source, 7.0);

        app.update();

        assert_translation(&app, source, Vec2::new(-300.0, 159.0));
        assert_eq!(resolved_depth(&app, source), None);
        assert_depth(&app, source, 7.0);
    }

    #[test]
    fn removing_attachment_restores_configured_position() {
        let mut app = app_with_window();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(
                    Vec2::new(50.0, 10.0),
                    Anchor::TopLeft,
                    Vec2::new(20.0, 30.0),
                ),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        assert_translation(&app, source, Vec2::new(-300.0, 160.0));

        app.world_mut()
            .entity_mut(source)
            .remove::<TestAttachment>();
        app.update();

        assert_eq!(resolved_anchor_position(&app, source), None);
        assert_translation(&app, source, Vec2::new(-380.0, 270.0));
    }

    #[test]
    fn cross_window_attachment_falls_back_with_diagnostic() {
        let mut app = app_with_window();
        let secondary = app
            .world_mut()
            .spawn(Window {
                resolution: (1200_u32, 400_u32).into(),
                ..Default::default()
            })
            .id();
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel_in_window(
                    secondary,
                    Vec2::new(50.0, 10.0),
                    Anchor::TopLeft,
                    Vec2::new(20.0, 30.0),
                ),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();

        assert_eq!(resolved_anchor_position(&app, source), None);
        assert_translation(&app, source, Vec2::new(-580.0, 170.0));
        assert_current_diagnostic(
            &app,
            source,
            target,
            ScreenAttachmentResolveSkip::CrossWindow,
        );
    }

    #[test]
    fn chain_resolves_and_retargeting_middle_updates_downstream() {
        let mut app = app_with_window();
        let root_a = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let root_b = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(200.0, 200.0),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(root_a, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();
        let leaf = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(25.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(middle, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        assert_translation(&app, middle, Vec2::new(-300.0, 180.0));
        assert_translation(&app, leaf, Vec2::new(-300.0, 170.0));

        app.world_mut()
            .entity_mut(middle)
            .insert(TestAttachment::new(
                root_b,
                Anchor::TopLeft,
                Anchor::BottomLeft,
            ));
        app.update();

        assert_translation(&app, middle, Vec2::new(-200.0, 80.0));
        assert_translation(&app, leaf, Vec2::new(-200.0, 70.0));
    }

    #[test]
    fn target_anchor_offset_uses_rotated_target_frame() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let target = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::TopLeft),
                anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(25.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::TopLeft)
                    .with_offset(PanelAnchorOffset::new(Px(5.0), Px(2.0))),
            ))
            .id();

        app.update();

        let target_pin = panel_anchor_screen_point(&app, target, Anchor::TopLeft);
        let expected_pin =
            target_pin + super::super::rotate_screen_offset(Vec2::new(5.0, 2.0), FRAC_PI_2);
        let source_pin = panel_anchor_screen_point(&app, source, Anchor::TopLeft);
        assert_vec2_close(source_pin, expected_pin);
        assert_vec2_close(
            resolved_anchor_position(&app, source).expect("source has resolved position"),
            expected_pin,
        );
        assert_translation(&app, source, Vec2::new(-298.0, 205.0));
    }

    #[test]
    fn rotated_screen_target_repositions_dependent_in_same_chain_update() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let middle = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::BottomLeft),
                anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO),
            ))
            .id();
        let leaf = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(25.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(middle, Anchor::TopLeft, Anchor::BottomLeft),
                anchor_pose(Quat::from_rotation_z(-FRAC_PI_2), Vec3::ZERO),
            ))
            .id();

        app.update();

        assert_eq!(
            resolved_anchor_position(&app, middle),
            Some(Vec2::new(100.0, 120.0))
        );
        assert_eq!(
            resolved_anchor_position(&app, leaf),
            Some(Vec2::new(110.0, 120.0))
        );
        assert_translation(&app, middle, Vec2::new(-300.0, 180.0));
        assert_translation(&app, leaf, Vec2::new(-290.0, 180.0));
        assert_rotation_z(&app, middle, FRAC_PI_2);
        assert_rotation_z(&app, leaf, -FRAC_PI_2);
    }

    #[test]
    fn resolver_change_log_ignores_spawn_add_and_stable_frames() {
        let mut app = app_with_window();
        app.init_resource::<ResolverChangeLog>();
        app.add_systems(
            Update,
            record_non_added_resolver_changes.after(PanelSystems::ResolvePanelAttachments),
        );
        let target = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let source = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::ZERO),
                TestAttachment::new(target, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();

        app.update();
        app.update();
        app.world_mut()
            .entity_mut(source)
            .remove::<TestAttachment>();
        app.update();
        app.update();

        let log = app.world().resource::<ResolverChangeLog>();
        assert!(
            log.0[0].is_empty(),
            "spawn-frame add is not a resolver change"
        );
        assert!(log.0[1].is_empty(), "stable frame should not change");
        assert_eq!(log.0[2], vec![(source, None)]);
        assert!(log.0[3].is_empty(), "stable fallback should not change");

        let panel = app
            .world()
            .get::<DiegeticPanel>(source)
            .expect("source still exists");
        assert!(matches!(
            panel.coordinate_space(),
            crate::panel::CoordinateSpace::Screen {
                position: ScreenPosition::At(position),
                ..
            } if *position == Vec2::ZERO
        ));
    }

    #[test]
    fn cycle_does_not_block_unrelated_valid_chain() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(200.0, 40.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 100.0),
            ))
            .id();
        let valid = app
            .world_mut()
            .spawn((
                fixed_screen_panel(Vec2::new(50.0, 10.0), Anchor::TopLeft, Vec2::new(0.0, 0.0)),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();
        let cycle_a = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(30.0, 10.0),
                Anchor::TopLeft,
                Vec2::new(20.0, 20.0),
            ))
            .id();
        let cycle_b = app
            .world_mut()
            .spawn((
                fixed_screen_panel(
                    Vec2::new(30.0, 10.0),
                    Anchor::TopLeft,
                    Vec2::new(40.0, 40.0),
                ),
                TestAttachment::new(cycle_a, Anchor::TopLeft, Anchor::BottomLeft),
            ))
            .id();
        app.world_mut()
            .entity_mut(cycle_a)
            .insert(TestAttachment::new(
                cycle_b,
                Anchor::TopLeft,
                Anchor::BottomLeft,
            ));

        app.update();

        assert_translation(&app, valid, Vec2::new(-300.0, 160.0));
        assert_translation(&app, cycle_a, Vec2::new(-380.0, 280.0));
        assert_translation(&app, cycle_b, Vec2::new(-360.0, 260.0));
        assert_current_diagnostic(&app, cycle_a, cycle_b, ScreenAttachmentResolveSkip::Cycle);
        assert_current_diagnostic(&app, cycle_b, cycle_a, ScreenAttachmentResolveSkip::Cycle);
    }

    #[test]
    fn queued_attachment_to_new_widget_resolves_after_both_command_fences() {
        let mut app = app_with_window();
        let owner = app
            .world_mut()
            .spawn(screen_widget_panel(60.0, Vec2::new(120.0, 90.0)))
            .id();
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(40.0, 12.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        app.insert_resource(WidgetAttachmentWrite {
            owner,
            source,
            state: WidgetAttachmentWriteState::Pending,
        });
        app.add_systems(
            Update,
            queue_widget_attachment
                .after(WidgetSystems::ReifyCommandsApplied)
                .before(ScreenSpaceSystems::FlushObserverCommands),
        );

        app.update();

        let widget = widget_entity(&app, owner);
        assert!(resolved_anchor_position(&app, source).is_some());
        assert_eq!(
            app.world()
                .get::<ScreenWidgetAnchoredTo>(source)
                .map(|relation| relation.target()),
            Some(widget),
        );
        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_some());
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_some());

        let first_position = resolved_anchor_position(&app, source);
        app.world_mut()
            .commands()
            .set_tree(owner, widget_tree(110.0))
            .expect("replacement widget tree is valid");
        app.update();

        assert_eq!(widget_entity(&app, owner), widget);
        assert_ne!(resolved_anchor_position(&app, source), first_position);
    }

    #[test]
    fn owner_widget_dependent_chain_uses_current_owner_rect_and_not_widget_global() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 80.0),
            ))
            .id();
        let owner = app
            .world_mut()
            .spawn((
                screen_widget_panel(80.0, Vec2::ZERO),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::BottomLeft),
                anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO),
                Transform::from_scale(Vec3::new(1.5, 0.75, 1.0)),
            ))
            .id();
        app.update();
        let widget = widget_entity(&app, owner);
        app.world_mut()
            .entity_mut(widget)
            .insert(GlobalTransform::from_translation(Vec3::splat(9_000.0)));
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(40.0, 12.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        attach_to_screen_widget(
            &mut app,
            source,
            owner,
            widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomRight)
                .with_offset(PanelAnchorOffset::new(Px(6.0), Px(9.0))),
        );

        app.update();

        let owner_position =
            resolved_anchor_position(&app, owner).expect("attached owner has a resolved position");
        let owner_transform = app
            .world()
            .get::<Transform>(owner)
            .expect("owner has a transform");
        let anchor_rect = *app
            .world()
            .get::<WidgetAnchorRect>(widget)
            .expect("widget has local anchor geometry");
        let angle = super::super::screen_in_plane_angle(owner_transform.rotation);
        let scale = owner_transform.scale.truncate();
        let center = owner_position
            + super::super::rotate_screen_offset(
                Vec2::new(
                    anchor_rect.panel_offset().x * scale.x,
                    -anchor_rect.panel_offset().y * scale.y,
                ),
                angle,
            );
        let size = anchor_rect.size() * scale.abs();
        let (right, bottom) = Anchor::BottomRight.offset(size.x, size.y);
        let (center_x, center_y) = Anchor::Center.offset(size.x, size.y);
        let expected = center
            + super::super::rotate_screen_offset(
                Vec2::new(right - center_x, bottom - center_y),
                angle,
            )
            + super::super::rotate_screen_offset(Vec2::new(6.0, 9.0) * scale, angle);
        assert_vec2_close(
            resolved_anchor_position(&app, source).expect("dependent has a resolved position"),
            expected,
        );
        assert!(expected.length() < 1_000.0);
    }

    #[test]
    fn mirrored_owner_scale_reflects_widget_target_anchor_before_rotation() {
        let mut app = app_with_window();
        let root = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(100.0, 20.0),
                Anchor::TopLeft,
                Vec2::new(100.0, 80.0),
            ))
            .id();
        let owner = app
            .world_mut()
            .spawn((
                screen_widget_panel(80.0, Vec2::ZERO),
                TestAttachment::new(root, Anchor::TopLeft, Anchor::BottomLeft),
                anchor_pose(Quat::from_rotation_z(FRAC_PI_2), Vec3::ZERO),
                Transform::from_scale(Vec3::new(-1.5, 0.75, 1.0)),
            ))
            .id();
        app.update();
        let widget = widget_entity(&app, owner);
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::new(40.0, 12.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        attach_to_screen_widget(
            &mut app,
            source,
            owner,
            widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomRight)
                .with_offset(PanelAnchorOffset::new(Px(6.0), Px(9.0))),
        );

        app.update();

        let owner_position =
            resolved_anchor_position(&app, owner).expect("attached owner has a resolved position");
        let owner_transform = app
            .world()
            .get::<Transform>(owner)
            .expect("owner has a transform");
        let anchor_rect = *app
            .world()
            .get::<WidgetAnchorRect>(widget)
            .expect("widget has local anchor geometry");
        assert_ne!(anchor_rect.panel_offset().truncate(), Vec2::ZERO);
        let angle = super::super::screen_in_plane_angle(owner_transform.rotation);
        let scale = owner_transform.scale.truncate();
        let center = owner_position
            + super::super::rotate_screen_offset(
                Vec2::new(
                    anchor_rect.panel_offset().x * scale.x,
                    -anchor_rect.panel_offset().y * scale.y,
                ),
                angle,
            );
        let size = anchor_rect.size() * scale.abs();
        let (right, bottom) = Anchor::BottomRight.offset(size.x, size.y);
        let (center_x, center_y) = Anchor::Center.offset(size.x, size.y);
        let authored_corner = Vec2::new(right - center_x, bottom - center_y) * scale.signum();
        let expected = center
            + super::super::rotate_screen_offset(authored_corner, angle)
            + super::super::rotate_screen_offset(Vec2::new(6.0, 9.0) * scale, angle);
        assert_vec2_close(
            resolved_anchor_position(&app, source).expect("dependent has a resolved position"),
            expected,
        );
    }

    #[test]
    fn screen_widget_proxy_and_panel_candidates_refresh_with_current_demand() {
        let mut app = app_with_window();
        let first_owner = app
            .world_mut()
            .spawn(screen_widget_panel(60.0, Vec2::new(100.0, 80.0)))
            .id();
        let second_owner = app
            .world_mut()
            .spawn(screen_widget_panel(90.0, Vec2::new(400.0, 80.0)))
            .id();
        app.update();
        let first_widget = widget_entity(&app, first_owner);
        let second_widget = widget_entity(&app, second_owner);
        let first_source = screen_attachment_source(&mut app);
        let second_source = screen_attachment_source(&mut app);
        attach_to_screen_widget(
            &mut app,
            first_source,
            first_owner,
            first_widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
        );
        attach_to_screen_widget(
            &mut app,
            second_source,
            first_owner,
            first_widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomRight),
        );
        app.update();

        assert!(resolved_anchor_position(&app, first_source).is_some());
        assert!(resolved_anchor_position(&app, second_source).is_some());
        assert_eq!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(first_widget)
                .map(RelationshipTarget::len),
            Some(2),
        );
        app.world_mut()
            .run_system_once(
                move |entities: PanelEntityReader, mut attachments: Commands| {
                    let source = entities
                        .screen(first_source)
                        .expect("first source has a screen handle");
                    attachments.detach(source);
                },
            )
            .expect("detach system runs");
        app.update();
        assert_eq!(resolved_anchor_position(&app, first_source), None);
        assert!(resolved_anchor_position(&app, second_source).is_some());
        assert_eq!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(first_widget)
                .map(RelationshipTarget::len),
            Some(1),
        );
        assert!(
            app.world()
                .get::<ResolvedAnchorGeometry>(first_widget)
                .is_some()
        );

        app.world_mut()
            .run_system_once(
                move |entities: PanelEntityReader,
                      widgets: PanelWidgetReader,
                      mut attachments: Commands| {
                    let source = entities
                        .screen(second_source)
                        .expect("second source has a screen handle");
                    let owner = entities
                        .screen(second_owner)
                        .expect("second owner has a screen handle");
                    let target = widgets
                        .typed_entity(owner, &PanelElementId::named("target"))
                        .expect("second target has a screen handle");
                    assert_eq!(target.entity(), second_widget);
                    attachments.retarget_to_widget(source, target);
                },
            )
            .expect("retarget system runs");
        app.update();
        assert_eq!(resolved_anchor_position(&app, first_source), None);
        assert!(resolved_anchor_position(&app, second_source).is_some());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(first_widget)
                .is_none()
        );
        assert!(
            app.world()
                .get::<ResolvedAnchorGeometry>(first_widget)
                .is_none()
        );
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(second_widget)
                .is_some_and(|demand| demand.contains(&second_source))
        );
    }

    #[test]
    fn missing_widget_geometry_diagnostic_deduplicates_by_key() {
        let mut app = app_with_window();
        let owner = app
            .world_mut()
            .spawn(screen_widget_panel(60.0, Vec2::new(100.0, 80.0)))
            .id();
        app.update();
        let widget = widget_entity(&app, owner);
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::splat(20.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        attach_to_screen_widget(
            &mut app,
            source,
            owner,
            widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
        );
        app.update();
        app.world_mut()
            .entity_mut(widget)
            .remove::<ResolvedAnchorGeometry>();

        app.update();
        app.update();

        assert_diagnostic_count(
            &app,
            source,
            widget,
            ScreenAttachmentResolveSkip::TargetGeometryMissing,
            2,
        );
    }

    #[test]
    fn missing_widget_owner_uses_owner_diagnostic() {
        let mut app = app_with_window();
        let owner = app
            .world_mut()
            .spawn(screen_widget_panel(60.0, Vec2::new(100.0, 80.0)))
            .id();
        app.update();
        let widget = widget_entity(&app, owner);
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::splat(20.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        attach_to_screen_widget(
            &mut app,
            source,
            owner,
            widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
        );
        app.update();
        app.world_mut().entity_mut(widget).remove::<WidgetOf>();

        app.update();
        app.update();

        assert_current_diagnostic(
            &app,
            source,
            widget,
            ScreenAttachmentResolveSkip::TargetOwnerMissing,
        );
        assert_diagnostic_count(
            &app,
            source,
            widget,
            ScreenAttachmentResolveSkip::TargetOwnerMissing,
            2,
        );
    }

    #[test]
    fn final_screen_demand_cleans_owned_widget_state_without_public_owner_relation() {
        let mut app = app_with_window();
        let owner = app
            .world_mut()
            .spawn(screen_widget_panel(60.0, Vec2::new(100.0, 80.0)))
            .id();
        app.update();
        let widget = widget_entity(&app, owner);
        let source = app
            .world_mut()
            .spawn(fixed_screen_panel(
                Vec2::splat(20.0),
                Anchor::TopLeft,
                Vec2::ZERO,
            ))
            .id();
        attach_to_screen_widget(
            &mut app,
            source,
            owner,
            widget,
            PanelAttachment::new(Anchor::TopLeft, Anchor::BottomLeft),
        );
        app.update();

        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_some());
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_some());
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ScreenWidgetAnchorProxy>>(widget)
                .is_some()
        );
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ResolvedAnchorGeometry>>(widget)
                .is_some()
        );

        app.world_mut().entity_mut(widget).remove::<WidgetOf>();
        app.update();
        assert_current_diagnostic(
            &app,
            source,
            widget,
            ScreenAttachmentResolveSkip::TargetOwnerMissing,
        );

        app.world_mut()
            .run_system_once(move |panels: PanelEntityReader, mut commands: Commands| {
                let source = panels
                    .screen(source)
                    .expect("source has a screen panel handle");
                commands.detach(source);
            })
            .expect("screen widget detach system runs");
        app.update();

        assert!(app.world().get_entity(owner).is_ok());
        assert!(app.world().get_entity(widget).is_ok());
        assert!(app.world().get_entity(source).is_ok());
        assert!(app.world().get::<WidgetOf>(widget).is_none());
        assert!(
            app.world()
                .get::<ScreenWidgetAnchoredHere>(widget)
                .is_none()
        );
        assert!(app.world().get::<ScreenWidgetAnchorProxy>(widget).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(widget).is_none());
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ScreenWidgetAnchorProxy>>(widget)
                .is_none()
        );
        assert!(
            app.world()
                .get::<PanelComponentOwnership<ResolvedAnchorGeometry>>(widget)
                .is_none()
        );
    }
}
