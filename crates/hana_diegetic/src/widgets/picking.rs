use bevy::camera::visibility::RenderLayers;
use bevy::picking::Pickable;
use bevy::picking::backend::HitData;
use bevy::picking::backend::PointerHits;
use bevy::picking::backend::ray::RayMap;
use bevy::picking::mesh_picking::MeshPickingCamera;
use bevy::picking::mesh_picking::MeshPickingSettings;
use bevy::picking::mesh_picking::ray_cast::MeshRayCast;
use bevy::picking::mesh_picking::ray_cast::MeshRayCastSettings;
use bevy::picking::mesh_picking::ray_cast::RayMeshHit;
use bevy::prelude::Camera;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Has;
use bevy::prelude::MessageWriter;
use bevy::prelude::Query;
use bevy::prelude::Ray3d;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::RelationshipTarget;
use bevy::prelude::Res;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;

use super::PanelWidget;
use super::PanelWidgets;
use super::WidgetOf;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::PanelOwned;
use crate::panel::PanelPlane;
use crate::render;
use crate::render::PanelInteractionMesh;

/// Pointer behavior for one side of a diegetic panel.
///
/// Panel and widget hit regions are rectangles. Transparent pixels inside a
/// reported rectangle still receive pointer hits, and content that renders
/// beyond the panel rectangle does not. Use [`Self::WidgetsOnly`] when a
/// transparent panel background should pass pointer input through while its
/// widget rectangles remain interactive. Non-rectangular hit regions are not
/// supported.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Reflect)]
pub enum FacePicking {
    /// The panel background and widget rectangles receive pointer input.
    #[default]
    Interactive,
    /// The panel background receives pointer input, but widgets do not.
    PanelOnly,
    /// Widget rectangles receive pointer input, but the panel background does
    /// not block lower hits.
    WidgetsOnly,
    /// Neither the panel background nor widgets receive or block pointer input.
    PassThrough,
}

impl FacePicking {
    const fn includes_panel(self) -> bool { matches!(self, Self::Interactive | Self::PanelOnly) }

    const fn includes_widgets(self) -> bool {
        matches!(self, Self::Interactive | Self::WidgetsOnly)
    }

    /// Faces whose blocking of lower diegetic panels needs no widget matching:
    /// the panel background is itself a hit surface, so any lower panel is
    /// always blocked. [`Self::WidgetsOnly`] is excluded — it blocks only when a
    /// widget is hit, which is unknown until widgets are matched.
    const fn always_blocks_lower(self) -> bool {
        matches!(self, Self::Interactive | Self::PanelOnly)
    }
}

#[derive(Clone, Copy)]
enum PanelFace {
    Front,
    Back,
}

/// Configures pointer behavior independently for a panel's front and back.
///
/// An entity without this component behaves as [`Self::INTERACTIVE`]. The
/// picking backend tests the panel and each widget as rectangles: transparent
/// pixels inside those rectangles still receive pointer hits, while rendered
/// content beyond the panel edge does not. Use [`FacePicking::WidgetsOnly`] for
/// a transparent panel whose widget rectangles should remain interactive.
/// Non-rectangular hit regions are not supported.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq, Reflect)]
#[reflect(Component)]
pub struct PanelPicking {
    /// Pointer behavior for rays that meet the panel's front face.
    pub front: FacePicking,
    /// Pointer behavior for rays that meet the panel's back face.
    pub back:  FacePicking,
}

impl PanelPicking {
    /// Both faces report the panel and its widgets.
    pub const INTERACTIVE: Self = Self {
        front: FacePicking::Interactive,
        back:  FacePicking::Interactive,
    };

    /// Both faces pass pointer input through without reporting hits.
    pub const PASS_THROUGH: Self = Self {
        front: FacePicking::PassThrough,
        back:  FacePicking::PassThrough,
    };

    const fn behavior(self, panel_face: PanelFace) -> FacePicking {
        match panel_face {
            PanelFace::Front => self.front,
            PanelFace::Back => self.back,
        }
    }
}

impl Default for PanelPicking {
    fn default() -> Self { Self::INTERACTIVE }
}

#[derive(Clone, Copy)]
enum PickableMarkers {
    Optional,
    Required,
}

struct FaceHit {
    camera:   Entity,
    panel:    Entity,
    point:    Vec3,
    normal:   Vec3,
    distance: f32,
    behavior: FacePicking,
}

enum LowerHits {
    Block,
    Continue,
}

pub(super) fn update_hits(
    backend_settings: Res<MeshPickingSettings>,
    ray_map: Res<RayMap>,
    picking_cameras: Query<(&Camera, Has<MeshPickingCamera>, Option<&RenderLayers>)>,
    interaction_meshes: Query<&PanelOwned, With<PanelInteractionMesh>>,
    panels: Query<(
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &GlobalTransform,
        Option<&PanelWidgets>,
        Option<&PanelPicking>,
    )>,
    widgets: Query<(&PanelWidget, &WidgetOf, Option<&Pickable>)>,
    pickables: Query<&Pickable>,
    layers: Query<&RenderLayers>,
    mut ray_cast: MeshRayCast,
    mut pointer_hits: MessageWriter<PointerHits>,
) {
    let pickable_markers = if backend_settings.require_markers {
        PickableMarkers::Required
    } else {
        PickableMarkers::Optional
    };
    for (&ray_id, &ray) in ray_map.iter() {
        let Ok((camera, camera_can_pick, camera_layers)) = picking_cameras.get(ray_id.camera)
        else {
            continue;
        };
        if backend_settings.require_markers && !camera_can_pick {
            continue;
        }

        let camera_layers = camera_layers.cloned().unwrap_or_default();
        let filter = |mesh_entity| {
            let Ok(ownership) = interaction_meshes.get(mesh_entity) else {
                return false;
            };
            let panel_entity = ownership.owner();
            let marker_requirement =
                !backend_settings.require_markers || pickables.get(panel_entity).is_ok();
            let mesh_layers = layers.get(mesh_entity).cloned().unwrap_or_default();
            let render_layers_match = camera_layers.intersects(&mesh_layers);
            let face_receives_hits = panels
                .get(panel_entity)
                .ok()
                .and_then(|(panel, _, panel_transform, _, panel_picking)| {
                    face_behavior(panel, panel_transform, panel_picking, ray)
                })
                .is_some_and(|behavior| behavior != FacePicking::PassThrough);
            marker_requirement && render_layers_match && face_receives_hits
        };
        let early_exit = |mesh_entity| {
            interaction_meshes
                .get(mesh_entity)
                .ok()
                .and_then(|ownership| {
                    let (panel, _, panel_transform, _, panel_picking) =
                        panels.get(ownership.owner()).ok()?;
                    face_behavior(panel, panel_transform, panel_picking, ray)
                })
                .is_some_and(FacePicking::always_blocks_lower)
        };
        let settings = MeshRayCastSettings {
            visibility:      backend_settings.ray_cast_visibility,
            filter:          &filter,
            early_exit_test: &early_exit,
        };
        let panel_hits = ray_cast.cast_ray(ray, &settings);
        let mut picks = Vec::new();

        for (mesh_entity, hit) in panel_hits {
            let Ok(ownership) = interaction_meshes.get(*mesh_entity) else {
                continue;
            };
            let Some((face_hit, matching_widgets)) = resolve_face_hit(
                ray_id.camera,
                ray,
                hit,
                ownership.owner(),
                pickable_markers,
                &panels,
                &widgets,
            ) else {
                continue;
            };
            if matches!(
                append_face_picks(&mut picks, face_hit, matching_widgets, &pickables),
                LowerHits::Block
            ) {
                break;
            }
        }

        if !picks.is_empty() {
            pointer_hits.write(PointerHits::new(
                ray_id.pointer,
                picks,
                camera_order(camera.order),
            ));
        }
    }
}

/// Resolves one interaction-mesh ray hit into its per-face pick inputs: the
/// owning panel's [`FaceHit`] and, when the face includes widgets, the
/// rank-sorted widgets under the hit point.
fn resolve_face_hit(
    camera: Entity,
    ray: Ray3d,
    hit: &RayMeshHit,
    panel_entity: Entity,
    pickable_markers: PickableMarkers,
    panels: &Query<(
        &DiegeticPanel,
        &ComputedDiegeticPanel,
        &GlobalTransform,
        Option<&PanelWidgets>,
        Option<&PanelPicking>,
    )>,
    widgets: &Query<(&PanelWidget, &WidgetOf, Option<&Pickable>)>,
) -> Option<(FaceHit, Option<Vec<(usize, Entity)>>)> {
    let (panel, computed, panel_transform, panel_widgets, panel_picking) =
        panels.get(panel_entity).ok()?;
    let face_behavior = face_behavior(panel, panel_transform, panel_picking, ray)?;
    let panel_local = render::project_flat_panel_hit(hit.point, panel, panel_transform)?;
    let matching_widgets = face_behavior.includes_widgets().then(|| {
        matching_widgets(
            panel_entity,
            panel_local,
            computed,
            panel_widgets,
            widgets,
            pickable_markers,
        )
    });
    Some((
        FaceHit {
            camera,
            panel: panel_entity,
            point: hit.point,
            normal: hit.normal,
            distance: hit.distance,
            behavior: face_behavior,
        },
        matching_widgets,
    ))
}

fn append_face_picks(
    picks: &mut Vec<(Entity, HitData)>,
    face_hit: FaceHit,
    matching_widgets: Option<Vec<(usize, Entity)>>,
    pickables: &Query<&Pickable>,
) -> LowerHits {
    let widgets_block_lower = matching_widgets.as_ref().is_some_and(|matching_widgets| {
        matching_widgets.iter().any(|(_, widget_entity)| {
            pickables
                .get(*widget_entity)
                .ok()
                .is_none_or(|pickable| pickable.should_block_lower)
        })
    });

    let pick_count = picks.len();
    let mut widget_depth = face_hit.distance;
    if let Some(matching_widgets) = matching_widgets {
        for (_, widget_entity) in matching_widgets {
            widget_depth = widget_depth.next_down();
            picks.push((
                widget_entity,
                HitData::new(
                    face_hit.camera,
                    widget_depth,
                    Some(face_hit.point),
                    Some(face_hit.normal),
                ),
            ));
        }
    }
    if face_hit.behavior.includes_panel() {
        picks.push((
            face_hit.panel,
            HitData::new(
                face_hit.camera,
                face_hit.distance,
                Some(face_hit.point),
                Some(face_hit.normal),
            ),
        ));
    }

    let blocks_lower = match face_hit.behavior {
        FacePicking::WidgetsOnly => widgets_block_lower,
        behavior => behavior.always_blocks_lower(),
    };
    if picks.len() != pick_count && blocks_lower {
        LowerHits::Block
    } else {
        LowerHits::Continue
    }
}

fn face_behavior(
    panel: &DiegeticPanel,
    panel_transform: &GlobalTransform,
    panel_picking: Option<&PanelPicking>,
    ray: Ray3d,
) -> Option<FacePicking> {
    let normal = PanelPlane::from_panel(panel, panel_transform)
        .ok()?
        .normal();
    let panel_face = if ray.direction.as_vec3().dot(normal) < 0.0 {
        PanelFace::Front
    } else {
        PanelFace::Back
    };
    Some(
        panel_picking
            .copied()
            .unwrap_or_default()
            .behavior(panel_face),
    )
}

fn matching_widgets(
    panel_entity: Entity,
    panel_local: Vec2,
    computed: &ComputedDiegeticPanel,
    panel_widgets: Option<&PanelWidgets>,
    widgets: &Query<(&PanelWidget, &WidgetOf, Option<&Pickable>)>,
    markers: PickableMarkers,
) -> Vec<(usize, Entity)> {
    let mut matching = panel_widgets.map_or_else(Vec::new, |panel_widgets| {
        panel_widgets
            .iter()
            .filter_map(|widget_entity| {
                let (widget, widget_of, pickable) = widgets.get(widget_entity).ok()?;
                if widget_of.panel() != panel_entity
                    || pickable.is_some_and(|pickable| !pickable.is_hoverable)
                    || (matches!(markers, PickableMarkers::Required) && pickable.is_none())
                {
                    return None;
                }
                let record = computed
                    .widget_records()
                    .iter()
                    .find(|record| record.id() == widget.id())?;
                record
                    .clipped_rect()
                    .filter(|rect| rect.contains(panel_local))
                    .map(|_| (record.interaction_rank(), widget_entity))
            })
            .collect::<Vec<_>>()
    });
    matching.sort_by_key(|(rank, _)| *rank);
    matching
}

#[expect(
    clippy::cast_precision_loss,
    reason = "matches Bevy's mesh backend conversion of the complete isize camera-order domain"
)]
const fn camera_order(order: isize) -> f32 { order as f32 }

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::asset::AssetPlugin;
    use bevy::camera::NormalizedRenderTarget;
    use bevy::camera::visibility::RenderLayers;
    use bevy::camera::visibility::VisibilityPlugin;
    use bevy::ecs::message::MessageCursor;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::MeshPlugin;
    use bevy::picking::InteractionPlugin;
    use bevy::picking::PickingPlugin;
    use bevy::picking::PickingSettings;
    use bevy::picking::backend::PointerHits;
    use bevy::picking::backend::ray::RayId;
    use bevy::picking::backend::ray::RayMap;
    use bevy::picking::events::Out;
    use bevy::picking::events::Over;
    use bevy::picking::events::Pointer;
    use bevy::picking::hover::PickingInteraction;
    use bevy::picking::mesh_picking::MeshPickingPlugin;
    use bevy::picking::mesh_picking::MeshPickingSettings;
    use bevy::picking::mesh_picking::ray_cast::RayCastBackfaces;
    use bevy::picking::mesh_picking::ray_cast::RayCastVisibility;
    use bevy::picking::pointer::Location;
    use bevy::picking::pointer::PointerAction;
    use bevy::picking::pointer::PointerButton;
    use bevy::picking::pointer::PointerId;
    use bevy::picking::pointer::PointerInput;
    use bevy::picking::pointer::PointerLocation;
    use bevy::picking::pointer::update_pointer_map;
    use bevy::prelude::App;
    use bevy::prelude::AssetApp;
    use bevy::prelude::Assets;
    use bevy::prelude::Camera;
    use bevy::prelude::ChildOf;
    use bevy::prelude::DetectChangesMut;
    use bevy::prelude::Dir3;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MeshMaterial3d;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::Pickable;
    use bevy::prelude::Ray3d;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::Shader;
    use bevy::prelude::StandardMaterial;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::prelude::default;
    use bevy::transform::TransformPlugin;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;

    use super::FacePicking;
    use super::PanelPicking;
    use super::camera_order;
    use crate::Anchor;
    use crate::ButtonClicked;
    use crate::ButtonPressed;
    use crate::ButtonReleased;
    use crate::DiegeticPanel;
    use crate::El;
    use crate::FitMax;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Px;
    use crate::Sizing;
    use crate::cascade;
    use crate::cascade::SdfMaterial;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::PanelOwned;
    use crate::render;
    use crate::render::PanelGeometryPlugin;
    use crate::render::PanelInteractionMesh;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::ComputedWidgetRecord;
    use crate::widgets::WidgetsPlugin;

    #[cfg(target_pointer_width = "64")]
    const LARGE_CAMERA_ORDER: isize = 1_isize << 40;
    #[cfg(target_pointer_width = "64")]
    const LARGE_CAMERA_ORDER_F32: f32 = 1_099_511_627_776.0;
    const LOWER_PANEL_DEPTH_OFFSET: Vec3 = Vec3::new(0.0, 0.0, -0.5);
    const SCREEN_PANEL_MAX_HEIGHT: Px = Px(120.0);
    const SCREEN_PANEL_MAX_WIDTH: Px = Px(250.0);
    const SCREEN_PANEL_SETTLE_UPDATES: usize = 4;
    const SCREEN_SPACER_WIDTH: f32 = 200.0;
    const SCREEN_TARGET_HEIGHT: f32 = 40.0;
    const SCREEN_TARGET_ID: &str = "screen-target";
    const SCREEN_TARGET_WIDTH: f32 = 40.0;
    const SCREEN_TEST_WINDOW_HEIGHT: u32 = 700;
    const SCREEN_TEST_WINDOW_WIDTH: u32 = 1000;

    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Resource)]
    struct ButtonLifecycle {
        pressed:  usize,
        released: usize,
        clicked:  usize,
    }

    fn record_button_pressed(_: On<ButtonPressed>, mut lifecycle: ResMut<ButtonLifecycle>) {
        lifecycle.pressed += 1;
    }

    fn record_button_released(_: On<ButtonReleased>, mut lifecycle: ResMut<ButtonLifecycle>) {
        lifecycle.released += 1;
    }

    fn record_button_clicked(_: On<ButtonClicked>, mut lifecycle: ResMut<ButtonLifecycle>) {
        lifecycle.clicked += 1;
    }

    #[test]
    fn camera_order_preserves_common_signed_values() {
        assert_close(camera_order(-2), -2.0);
        assert_close(camera_order(3), 3.0);
    }

    #[cfg(target_pointer_width = "64")]
    #[test]
    fn camera_order_preserves_values_outside_i32_range() {
        assert_close(camera_order(LARGE_CAMERA_ORDER), LARGE_CAMERA_ORDER_F32);
        assert_close(camera_order(-LARGE_CAMERA_ORDER), -LARGE_CAMERA_ORDER_F32);
    }

    struct PickingScene {
        app:             App,
        panel:           Entity,
        widget:          Entity,
        interaction:     Entity,
        back:            Entity,
        front_first:     Entity,
        front_last:      Entity,
        camera:          Entity,
        pointer:         PointerId,
        local_hit:       Vec3,
        background_hit:  Vec3,
        clipped_hit:     Vec3,
        overlap_hit:     Vec3,
        partial_inside:  Vec3,
        partial_outside: Vec3,
    }

    fn picking_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            AssetPlugin::default(),
            MeshPlugin,
            TransformPlugin,
            VisibilityPlugin,
        ));
        app.insert_resource(PickingSettings {
            is_input_enabled: false,
            is_window_picking_enabled: false,
            ..default()
        });
        app.init_asset::<Shader>().add_plugins((
            PickingPlugin,
            InteractionPlugin,
            MeshPickingPlugin,
        ));
        app.world_mut()
            .resource_mut::<MeshPickingSettings>()
            .ray_cast_visibility = RayCastVisibility::Visible;
        render::seed_default_material_cascades(&mut app);
        app.add_plugins(cascade::cascade_plugin::<SdfMaterial>())
            .insert_resource(DiegeticTextMeasurer::default())
            .init_resource::<ButtonLifecycle>()
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, PanelGeometryPlugin))
            .add_observer(record_button_pressed)
            .add_observer(record_button_released)
            .add_observer(record_button_clicked);
        app
    }

    fn screen_picking_app() -> App {
        let mut app = picking_app();
        app.add_plugins(ScreenSpacePlugin);
        app.world_mut().spawn((
            Window {
                resolution: (SCREEN_TEST_WINDOW_WIDTH, SCREEN_TEST_WINDOW_HEIGHT).into(),
                ..default()
            },
            PrimaryWindow,
        ));
        app
    }

    fn spawn_picking_panel(app: &mut App) -> Entity {
        let mut layout = LayoutBuilder::new(100.0, 50.0);
        layout.with(El::row().size(20.0, 10.0).clip(), |layout| {
            layout.with(El::new().size(30.0, 10.0).button("partial"), |_| {});
            layout.with(El::new().size(10.0, 10.0).button("clipped"), |_| {});
        });
        layout.with(El::overlay().size(20.0, 20.0), |layout| {
            layout.with(
                El::new().size(20.0, 20.0).button("back").z_index(-1),
                |_| {},
            );
            layout.with(
                El::new().size(20.0, 20.0).button("front-first").z_index(2),
                |_| {},
            );
            layout.with(
                El::new().size(20.0, 20.0).button("front-last").z_index(2),
                |_| {},
            );
        });
        layout.with(El::row().size(100.0, 10.0), |layout| {
            layout.with(El::new().size(30.0, 10.0), |_| {});
            layout.with(El::new().size(20.0, 10.0).button("target"), |_| {});
        });
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .with_tree(layout.build())
            .build()
            .expect("panel builds");
        app.world_mut().spawn(panel).id()
    }

    fn spawn_dynamic_screen_picking_panel(app: &mut App) -> Entity {
        let mut layout = LayoutBuilder::with_root(El::row().width(Sizing::FIT).height(Sizing::FIT));
        layout.with(
            El::new().size(SCREEN_SPACER_WIDTH, SCREEN_TARGET_HEIGHT),
            |_| {},
        );
        layout.with(
            El::new()
                .size(SCREEN_TARGET_WIDTH, SCREEN_TARGET_HEIGHT)
                .button(SCREEN_TARGET_ID),
            |_| {},
        );
        let panel = DiegeticPanel::screen()
            .size(
                FitMax(SCREEN_PANEL_MAX_WIDTH.into()),
                FitMax(SCREEN_PANEL_MAX_HEIGHT.into()),
            )
            .anchor(Anchor::TopRight)
            .with_tree(layout.build())
            .build()
            .expect("dynamic screen panel builds");
        app.world_mut().spawn(panel).id()
    }

    fn spawn_picking_input(app: &mut App) -> (Entity, PointerId) {
        let camera = app
            .world_mut()
            .spawn(Camera {
                order: 7,
                ..default()
            })
            .id();
        let pointer = PointerId::Touch(42);
        let window = app.world_mut().spawn(Window::default()).id();
        let window = WindowRef::Entity(window)
            .normalize(None)
            .expect("entity window references always normalize");
        let pointer_location = PointerLocation::new(Location {
            target:   NormalizedRenderTarget::Window(window),
            position: Vec2::ZERO,
        });
        app.world_mut().spawn((pointer, pointer_location.clone()));
        app.world_mut().spawn((PointerId::Mouse, pointer_location));
        app.world_mut()
            .run_system_once(update_pointer_map)
            .expect("pointer map updates");
        (camera, pointer)
    }

    fn build_picking_scene() -> PickingScene {
        let mut app = picking_app();
        let panel = spawn_picking_panel(&mut app);
        app.update();
        app.update();

        let widget = resolve_widget(&mut app, panel, "target");
        let back = resolve_widget(&mut app, panel, "back");
        let front_first = resolve_widget(&mut app, panel, "front-first");
        let front_last = resolve_widget(&mut app, panel, "front-last");
        let interaction = interaction_mesh(&mut app, panel);
        let (camera, pointer) = spawn_picking_input(&mut app);
        let local_hit = record_center_world(&app, panel, "target");
        let overlap_hit = record_center_world(&app, panel, "front-last");
        let clipped_hit = record_center_world(&app, panel, "clipped");
        let partial_record = widget_record(&app, panel, "partial");
        let partial_clip = partial_record
            .clipped_rect()
            .expect("partially clipped widget should retain a visible rect");
        let partial_inside = panel_point_to_world(&app, panel, partial_clip.center());
        let outside_point = Vec2::new(
            partial_record.rect().x + partial_record.rect().width - 0.5,
            partial_record
                .rect()
                .height
                .mul_add(0.5, partial_record.rect().y),
        );
        assert!(!partial_clip.contains(outside_point));
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel should remain live");
        let layout_to_points = panel_component.layout_unit().to_points();
        let background_hit = panel_point_to_world(
            &app,
            panel,
            (
                panel_component.width().mul_add(layout_to_points, -1.0),
                panel_component.height().mul_add(layout_to_points, -1.0),
            ),
        );
        let partial_outside = panel_point_to_world(&app, panel, (outside_point.x, outside_point.y));

        PickingScene {
            app,
            panel,
            widget,
            interaction,
            back,
            front_first,
            front_last,
            camera,
            pointer,
            local_hit,
            background_hit,
            clipped_hit,
            overlap_hit,
            partial_inside,
            partial_outside,
        }
    }

    fn resolve_widget(app: &mut App, panel: Entity, id: &str) -> Entity {
        let id = PanelElementId::named(id);
        app.world_mut()
            .run_system_once(move |reader: PanelWidgetReader| reader.entity(panel, &id))
            .expect("reader system should run")
            .expect("widget should be reified")
    }

    fn interaction_mesh(app: &mut App, panel: Entity) -> Entity {
        let world = app.world_mut();
        let mut query = world.query::<(Entity, &PanelInteractionMesh, &PanelOwned)>();
        let entities = query
            .iter(world)
            .filter(|(_, _, ownership)| ownership.owner() == panel)
            .map(|(entity, _, _)| entity)
            .collect::<Vec<_>>();
        assert_eq!(
            entities.len(),
            1,
            "production should create one interaction mesh for the panel"
        );
        entities[0]
    }

    fn widget_record<'a>(app: &'a App, panel: Entity, id: &str) -> &'a ComputedWidgetRecord {
        app.world()
            .get::<ComputedDiegeticPanel>(panel)
            .expect("panel should have computed output")
            .widget_records()
            .iter()
            .find(|record| record.id() == &PanelElementId::named(id))
            .expect("widget should have a production computed record")
    }

    fn record_center_world(app: &App, panel: Entity, id: &str) -> Vec3 {
        let record = widget_record(app, panel, id);
        panel_point_to_world(app, panel, record.rect().center())
    }

    fn panel_point_to_world(app: &App, panel: Entity, point: (f32, f32)) -> Vec3 {
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("panel remains live");
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        let scale = panel_component.points_to_world();
        let local = Vec3::new(
            point.0.mul_add(scale, -anchor_x),
            point.1.mul_add(-scale, anchor_y),
            0.0,
        );
        app.world()
            .get::<GlobalTransform>(panel)
            .expect("panel should have a global transform")
            .transform_point(local)
    }

    #[track_caller]
    fn assert_close(actual: f32, expected: f32) {
        assert!((actual - expected).abs() < f32::EPSILON);
    }

    #[track_caller]
    fn assert_hit_group(group: &PointerHits, scene: &PickingScene) {
        assert_eq!(group.pointer, scene.pointer);
        assert_close(group.order, 7.0);
        assert!(
            group
                .picks
                .iter()
                .all(|(_, hit)| hit.camera == scene.camera)
        );
        assert!(
            group
                .picks
                .iter()
                .any(|(entity, _)| *entity == scene.widget)
        );
        assert!(group.picks.iter().any(|(entity, _)| *entity == scene.panel));
        assert!(
            !group
                .picks
                .iter()
                .any(|(entity, _)| *entity == scene.interaction)
        );
        let widget_depth = group
            .picks
            .iter()
            .find(|(entity, _)| *entity == scene.widget)
            .map(|(_, hit)| hit.depth)
            .expect("widget hit has depth");
        let panel_depth = group
            .picks
            .iter()
            .find(|(entity, _)| *entity == scene.panel)
            .map(|(_, hit)| hit.depth)
            .expect("panel hit has depth");
        assert!(widget_depth < panel_depth);
    }

    fn settle_panel_source_change(scene: &mut PickingScene) {
        scene.app.world_mut().resource_mut::<RayMap>().map.clear();
        scene.app.update();
    }

    fn cast_scene_ray(
        scene: &mut PickingScene,
        cursor: &mut MessageCursor<PointerHits>,
        origin: Vec3,
        direction: Dir3,
    ) -> Vec<PointerHits> {
        cast_ray(
            &mut scene.app,
            cursor,
            scene.camera,
            scene.pointer,
            origin,
            direction,
        )
    }

    fn cast_ray(
        app: &mut App,
        cursor: &mut MessageCursor<PointerHits>,
        camera: Entity,
        pointer: PointerId,
        origin: Vec3,
        direction: Dir3,
    ) -> Vec<PointerHits> {
        app.world_mut()
            .resource_mut::<RayMap>()
            .map
            .insert(RayId::new(camera, pointer), Ray3d::new(origin, direction));
        app.update();
        cursor
            .read(app.world().resource::<Messages<PointerHits>>())
            .cloned()
            .collect()
    }

    /// Spawns a second diegetic panel directly behind the scene's panel along
    /// the shared pointer ray, so both panels compete inside Hana's own backend.
    fn stack_lower_panel(scene: &mut PickingScene) -> Entity {
        let lower = spawn_picking_panel(&mut scene.app);
        scene
            .app
            .world_mut()
            .entity_mut(lower)
            .insert(Transform::from_translation(LOWER_PANEL_DEPTH_OFFSET));
        scene.app.update();
        scene.app.update();
        lower
    }

    /// Casts one pointer ray and lets Bevy resolve hover, so assertions read the
    /// resolved [`PickingInteraction`] state rather than raw backend hits.
    fn resolve_hover(scene: &mut PickingScene, origin: Vec3) {
        let mut ray_map = scene.app.world_mut().resource_mut::<RayMap>();
        ray_map.map.clear();
        ray_map.map.insert(
            RayId::new(scene.camera, scene.pointer),
            Ray3d::new(origin, Dir3::NEG_Z),
        );
        scene.app.update();
    }

    fn hover_state(scene: &PickingScene, entity: Entity) -> Option<PickingInteraction> {
        scene.app.world().get::<PickingInteraction>(entity).copied()
    }

    fn pointer_location(app: &mut App, pointer: PointerId) -> Location {
        let world = app.world_mut();
        let mut query = world.query::<(&PointerId, &PointerLocation)>();
        query
            .iter(world)
            .find(|(pointer_id, _)| **pointer_id == pointer)
            .and_then(|(_, pointer_location)| pointer_location.location().cloned())
            .expect("test pointer should have a location")
    }

    fn dispatch_button_click(scene: &mut PickingScene) {
        let ray = Ray3d::new(scene.local_hit + Vec3::Z, Dir3::NEG_Z);
        scene
            .app
            .world_mut()
            .resource_mut::<RayMap>()
            .map
            .insert(RayId::new(scene.camera, scene.pointer), ray);
        scene.app.update();

        let location = pointer_location(&mut scene.app, scene.pointer);
        scene.app.world_mut().write_message(PointerInput::new(
            scene.pointer,
            location.clone(),
            PointerAction::Press(PointerButton::Primary),
        ));
        scene.app.update();
        scene.app.world_mut().write_message(PointerInput::new(
            scene.pointer,
            location,
            PointerAction::Release(PointerButton::Primary),
        ));
        scene.app.update();
    }

    #[test]
    fn panel_picking_defaults_and_symmetric_constants_are_stable() {
        assert_eq!(PanelPicking::default(), PanelPicking::INTERACTIVE);
        assert_eq!(PanelPicking::INTERACTIVE.front, FacePicking::Interactive);
        assert_eq!(PanelPicking::INTERACTIVE.back, FacePicking::Interactive);
        assert_eq!(PanelPicking::PASS_THROUGH.front, FacePicking::PassThrough);
        assert_eq!(PanelPicking::PASS_THROUGH.back, FacePicking::PassThrough);
    }

    #[test]
    fn world_and_screen_builders_install_authored_panel_picking() {
        let authored = PanelPicking {
            front: FacePicking::WidgetsOnly,
            back:  FacePicking::PanelOnly,
        };
        let mut app = screen_picking_app();
        let world_panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .picking(authored)
            .build()
            .expect("world panel builds");
        let screen_panel = DiegeticPanel::screen()
            .size(Px(100.0), Px(50.0))
            .picking(authored)
            .build()
            .expect("screen panel builds");
        let world_entity = app.world_mut().spawn(world_panel).id();
        let screen_entity = app.world_mut().spawn(screen_panel).id();
        app.update();

        assert_eq!(
            app.world().get::<PanelPicking>(world_entity),
            Some(&authored),
        );
        assert_eq!(
            app.world().get::<PanelPicking>(screen_entity),
            Some(&authored),
        );
    }

    #[test]
    fn explicit_same_bundle_panel_picking_overrides_builder_seed() {
        let mut app = picking_app();
        let seed = PanelPicking {
            front: FacePicking::WidgetsOnly,
            back:  FacePicking::WidgetsOnly,
        };
        let build_seeded_panel = || {
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .picking(seed)
                .build()
                .expect("panel builds")
        };
        let pass_through_entity = app
            .world_mut()
            .spawn((build_seeded_panel(), PanelPicking::PASS_THROUGH))
            .id();
        let interactive_entity = app
            .world_mut()
            .spawn((build_seeded_panel(), PanelPicking::INTERACTIVE))
            .id();
        app.update();

        // An explicit component in the same spawn bundle is authoritative over
        // the builder seed — including `INTERACTIVE`, which equals the default.
        assert_eq!(
            app.world().get::<PanelPicking>(pass_through_entity),
            Some(&PanelPicking::PASS_THROUGH),
        );
        assert_eq!(
            app.world().get::<PanelPicking>(interactive_entity),
            Some(&PanelPicking::INTERACTIVE),
        );
    }

    #[test]
    fn replacing_panel_preserves_live_picking_of_any_value() {
        let mut app = picking_app();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .build()
            .expect("panel builds");
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::default()),
        );

        // A live component is never rewritten, even while it holds the
        // default: a replacement's non-default seed does not apply.
        let seeded = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .picking(PanelPicking::PASS_THROUGH)
            .build()
            .expect("panel builds");
        app.world_mut().entity_mut(entity).insert(seeded);
        app.update();
        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::INTERACTIVE),
        );

        // A runtime-authored value survives a later replacement seed too.
        app.world_mut()
            .entity_mut(entity)
            .insert(PanelPicking::PASS_THROUGH);
        let rebuilt = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .picking(PanelPicking {
                front: FacePicking::WidgetsOnly,
                back:  FacePicking::WidgetsOnly,
            })
            .build()
            .expect("panel builds");
        app.world_mut().entity_mut(entity).insert(rebuilt);
        app.update();
        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::PASS_THROUGH),
        );
    }

    #[test]
    fn panel_replacement_seed_fills_absent_picking() {
        let mut app = picking_app();
        let panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .build()
            .expect("panel builds");
        let entity = app.world_mut().spawn(panel).id();
        app.update();
        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::default()),
        );

        // With no live component at replacement time, the seed fills it.
        app.world_mut().entity_mut(entity).remove::<PanelPicking>();
        let rebuilt = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .picking(PanelPicking::PASS_THROUGH)
            .build()
            .expect("panel builds");
        app.world_mut().entity_mut(entity).insert(rebuilt);
        app.update();
        assert_eq!(
            app.world().get::<PanelPicking>(entity),
            Some(&PanelPicking::PASS_THROUGH),
        );
    }

    #[test]
    fn dynamic_fitmax_screen_panel_uses_resolved_interaction_geometry_for_widget_rays() {
        let mut app = screen_picking_app();
        let panel = spawn_dynamic_screen_picking_panel(&mut app);
        for _ in 0..SCREEN_PANEL_SETTLE_UPDATES {
            app.update();
        }

        let widget = resolve_widget(&mut app, panel, SCREEN_TARGET_ID);
        let interaction = interaction_mesh(&mut app, panel);
        let panel_component = app
            .world()
            .get::<DiegeticPanel>(panel)
            .expect("dynamic screen panel remains live");
        let panel_size = Vec2::new(panel_component.width(), panel_component.height());
        let (anchor_x, anchor_y) = panel_component.anchor_offsets();
        let expected_center = Vec2::new(
            panel_size.x.mul_add(0.5, -anchor_x),
            panel_size.y.mul_add(-0.5, anchor_y),
        );
        let interaction_component = app
            .world()
            .get::<PanelInteractionMesh>(interaction)
            .expect("production interaction mesh should remain live");
        assert!(
            *interaction_component == PanelInteractionMesh::new(panel_size, expected_center),
            "interaction metadata should use resolved screen dimensions",
        );
        let interaction_transform = app
            .world()
            .get::<Transform>(interaction)
            .expect("production interaction mesh should have a local transform");
        assert_eq!(
            [
                interaction_transform.translation.x.to_bits(),
                interaction_transform.translation.y.to_bits(),
            ],
            [expected_center.x.to_bits(), expected_center.y.to_bits()],
        );

        let target_hit = record_center_world(&app, panel, SCREEN_TARGET_ID);
        let target_rect = widget_record(&app, panel, SCREEN_TARGET_ID).rect();
        let outside_point = Vec2::new(
            target_rect.x * 0.5,
            target_rect.height.mul_add(0.5, target_rect.y),
        );
        assert!(!target_rect.contains(outside_point));
        let outside_hit = panel_point_to_world(&app, panel, (outside_point.x, outside_point.y));
        let (camera, pointer) = spawn_picking_input(&mut app);
        let interaction_layers = app
            .world()
            .get::<RenderLayers>(interaction)
            .cloned()
            .expect("screen interaction mesh should inherit render layers");
        app.world_mut()
            .entity_mut(camera)
            .insert(interaction_layers);
        let mut cursor = MessageCursor::<PointerHits>::default();

        let target_hits = cast_ray(
            &mut app,
            &mut cursor,
            camera,
            pointer,
            target_hit + Vec3::Z,
            Dir3::NEG_Z,
        );
        assert_eq!(target_hits.len(), 1);
        assert!(
            target_hits[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == widget),
        );
        assert!(
            target_hits[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == panel),
        );
        assert!(
            !target_hits[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == interaction),
        );

        let outside_hits = cast_ray(
            &mut app,
            &mut cursor,
            camera,
            pointer,
            outside_hit + Vec3::Z,
            Dir3::NEG_Z,
        );
        assert_eq!(outside_hits.len(), 1);
        assert_eq!(outside_hits[0].picks.len(), 1);
        assert_eq!(outside_hits[0].picks[0].0, panel);
    }

    #[test]
    fn front_and_back_rays_report_off_origin_widget_before_panel_root() {
        let mut scene = build_picking_scene();
        let mut cursor = MessageCursor::<PointerHits>::default();

        for (origin, direction) in [
            (scene.local_hit + Vec3::Z, Dir3::NEG_Z),
            (scene.local_hit - Vec3::Z, Dir3::Z),
        ] {
            let hits = cast_scene_ray(&mut scene, &mut cursor, origin, direction);
            assert_eq!(hits.len(), 1);
            assert_hit_group(&hits[0], &scene);
        }
    }

    #[test]
    fn absent_panel_picking_behaves_as_interactive_on_both_faces() {
        let mut scene = build_picking_scene();
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .remove::<PanelPicking>();
        let mut cursor = MessageCursor::<PointerHits>::default();

        for (origin, direction) in [
            (scene.local_hit + Vec3::Z, Dir3::NEG_Z),
            (scene.local_hit - Vec3::Z, Dir3::Z),
        ] {
            let hits = cast_scene_ray(&mut scene, &mut cursor, origin, direction);
            assert_eq!(hits.len(), 1);
            assert_hit_group(&hits[0], &scene);
        }
    }

    #[test]
    fn panel_only_back_face_reports_panel_without_widgets() {
        let mut scene = build_picking_scene();
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(PanelPicking {
                front: FacePicking::Interactive,
                back:  FacePicking::PanelOnly,
            });
        let mut cursor = MessageCursor::<PointerHits>::default();
        let origin = scene.local_hit - Vec3::Z;
        let hits = cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::Z);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].picks.len(), 1);
        assert_eq!(hits[0].picks[0].0, scene.panel);
    }

    #[test]
    fn widgets_only_empty_front_face_resolves_hover_to_lower_panel() {
        let mut scene = build_picking_scene();
        let lower = stack_lower_panel(&mut scene);
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(PanelPicking {
                front: FacePicking::WidgetsOnly,
                back:  FacePicking::PassThrough,
            });

        let origin = scene.background_hit + Vec3::Z;
        resolve_hover(&mut scene, origin);

        // The front's empty background reports nothing and does not block, so
        // hover resolves onto the lower panel's background behind it.
        assert_eq!(
            hover_state(&scene, lower),
            Some(PickingInteraction::Hovered)
        );
        assert_ne!(
            hover_state(&scene, scene.panel),
            Some(PickingInteraction::Hovered),
        );
    }

    #[test]
    fn pass_through_front_face_resolves_hover_to_lower_panel() {
        let mut scene = build_picking_scene();
        let lower = stack_lower_panel(&mut scene);
        let lower_widget = resolve_widget(&mut scene.app, lower, "target");
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(PanelPicking::PASS_THROUGH);

        let origin = scene.local_hit + Vec3::Z;
        resolve_hover(&mut scene, origin);

        // The pass-through front emits no hits at all; hover resolves onto the
        // lower panel's widget.
        assert_eq!(
            hover_state(&scene, lower_widget),
            Some(PickingInteraction::Hovered),
        );
        assert_ne!(
            hover_state(&scene, scene.widget),
            Some(PickingInteraction::Hovered),
        );
        assert_ne!(
            hover_state(&scene, scene.panel),
            Some(PickingInteraction::Hovered),
        );
    }

    #[test]
    fn widgets_only_front_widget_blocks_lower_panel() {
        let mut scene = build_picking_scene();
        let lower = stack_lower_panel(&mut scene);
        let lower_widget = resolve_widget(&mut scene.app, lower, "target");
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(PanelPicking {
                front: FacePicking::WidgetsOnly,
                back:  FacePicking::PassThrough,
            });

        let origin = scene.local_hit + Vec3::Z;
        resolve_hover(&mut scene, origin);

        // A matching nearer WidgetsOnly widget still blocks: the lower panel
        // receives no hover.
        assert_eq!(
            hover_state(&scene, scene.widget),
            Some(PickingInteraction::Hovered),
        );
        assert_ne!(
            hover_state(&scene, lower),
            Some(PickingInteraction::Hovered)
        );
        assert_ne!(
            hover_state(&scene, lower_widget),
            Some(PickingInteraction::Hovered),
        );
    }

    #[test]
    fn blocking_front_faces_withhold_hover_from_lower_panel() {
        enum FrontSurface {
            Widget,
            Panel,
        }

        for (front, front_surface) in [
            (FacePicking::Interactive, FrontSurface::Widget),
            (FacePicking::PanelOnly, FrontSurface::Panel),
        ] {
            let mut scene = build_picking_scene();
            let lower = stack_lower_panel(&mut scene);
            let lower_widget = resolve_widget(&mut scene.app, lower, "target");
            scene
                .app
                .world_mut()
                .entity_mut(scene.panel)
                .insert(PanelPicking {
                    front,
                    back: FacePicking::PassThrough,
                });

            let origin = scene.local_hit + Vec3::Z;
            resolve_hover(&mut scene, origin);

            let hovered_front = match front_surface {
                FrontSurface::Widget => scene.widget,
                FrontSurface::Panel => scene.panel,
            };
            assert_eq!(
                hover_state(&scene, hovered_front),
                Some(PickingInteraction::Hovered),
                "{front:?} should hover its own surface",
            );
            assert_ne!(
                hover_state(&scene, lower),
                Some(PickingInteraction::Hovered),
                "{front:?} must block the lower panel root",
            );
            assert_ne!(
                hover_state(&scene, lower_widget),
                Some(PickingInteraction::Hovered),
                "{front:?} must block the lower panel widget",
            );
        }
    }

    #[test]
    fn face_picking_controls_real_button_pointer_dispatch() {
        for (face_picking, expected) in [
            (
                FacePicking::Interactive,
                ButtonLifecycle {
                    pressed:  1,
                    released: 1,
                    clicked:  1,
                },
            ),
            (
                FacePicking::WidgetsOnly,
                ButtonLifecycle {
                    pressed:  1,
                    released: 1,
                    clicked:  1,
                },
            ),
            (FacePicking::PanelOnly, ButtonLifecycle::default()),
            (FacePicking::PassThrough, ButtonLifecycle::default()),
        ] {
            let mut scene = build_picking_scene();
            scene
                .app
                .world_mut()
                .entity_mut(scene.panel)
                .insert(PanelPicking {
                    front: face_picking,
                    back:  FacePicking::PassThrough,
                });

            dispatch_button_click(&mut scene);

            assert_eq!(*scene.app.world().resource::<ButtonLifecycle>(), expected);
        }
    }

    #[test]
    fn production_interaction_surface_has_two_sided_picking_components() {
        let scene = build_picking_scene();
        let world = scene.app.world();

        assert!(world.get::<RayCastBackfaces>(scene.interaction).is_some());
        assert_eq!(
            world
                .get::<PanelOwned>(scene.interaction)
                .map(|ownership| ownership.owner()),
            Some(scene.panel),
        );
        let pickable = world
            .get::<Pickable>(scene.interaction)
            .expect("interaction mesh should carry Pickable::IGNORE");
        assert!(!pickable.is_hoverable);
        assert!(!pickable.should_block_lower);
        let material_handle = world
            .get::<MeshMaterial3d<StandardMaterial>>(scene.interaction)
            .expect("interaction mesh should carry a standard material");
        let material = world
            .resource::<Assets<StandardMaterial>>()
            .get(&material_handle.0)
            .expect("interaction material should remain in assets");
        assert!(material.double_sided);
        assert_eq!(material.cull_mode, None);
    }

    #[test]
    fn reparented_interaction_surface_keeps_its_recorded_panel_identity() {
        let mut scene = build_picking_scene();
        let other_panel = spawn_picking_panel(&mut scene.app);
        scene.app.update();
        scene.app.update();
        let other_interaction = interaction_mesh(&mut scene.app, other_panel);
        scene
            .app
            .world_mut()
            .entity_mut(other_interaction)
            .despawn();
        scene
            .app
            .world_mut()
            .entity_mut(scene.interaction)
            .insert(ChildOf(other_panel));

        scene
            .app
            .world_mut()
            .get_mut::<ComputedDiegeticPanel>(scene.panel)
            .expect("recorded owner should retain computed output")
            .set_changed();
        scene.app.update();

        assert_eq!(
            interaction_mesh(&mut scene.app, scene.panel),
            scene.interaction
        );
        scene
            .app
            .world_mut()
            .entity_mut(other_panel)
            .insert(RenderLayers::layer(6));
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(RenderLayers::layer(4));
        scene.app.update();
        assert_eq!(
            scene.app.world().get::<RenderLayers>(scene.interaction),
            Some(&RenderLayers::layer(4)),
        );

        scene
            .app
            .world_mut()
            .entity_mut(scene.camera)
            .insert(RenderLayers::layer(4));
        let mut cursor = MessageCursor::<PointerHits>::default();
        let origin = scene.local_hit + Vec3::Z;
        let hits = cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z);
        assert_eq!(hits.len(), 1);
        assert_hit_group(&hits[0], &scene);
        assert!(
            !hits[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == other_panel),
        );

        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .remove::<DiegeticPanel>();
        scene.app.update();
        assert!(scene.app.world().get_entity(scene.interaction).is_err());
        assert!(scene.app.world().get_entity(other_panel).is_ok());
    }

    #[test]
    fn mouse_and_touch_hits_drive_widget_hover_and_out_presentation() {
        let mut scene = build_picking_scene();
        let ray = Ray3d::new(scene.local_hit + Vec3::Z, Dir3::NEG_Z);
        scene
            .app
            .world_mut()
            .resource_mut::<RayMap>()
            .map
            .insert(RayId::new(scene.camera, scene.pointer), ray);
        scene
            .app
            .world_mut()
            .resource_mut::<RayMap>()
            .map
            .insert(RayId::new(scene.camera, PointerId::Mouse), ray);
        let mut over_cursor = MessageCursor::<Pointer<Over>>::default();
        scene.app.update();

        assert_eq!(
            scene.app.world().get::<PickingInteraction>(scene.widget),
            Some(&PickingInteraction::Hovered)
        );
        let over_pointers = over_cursor
            .read(scene.app.world().resource::<Messages<Pointer<Over>>>())
            .filter(|event| event.entity == scene.widget)
            .map(|event| event.pointer_id)
            .collect::<Vec<_>>();
        assert!(over_pointers.contains(&PointerId::Mouse));
        assert!(over_pointers.contains(&scene.pointer));

        scene.app.world_mut().resource_mut::<RayMap>().map.clear();
        let mut out_cursor = MessageCursor::<Pointer<Out>>::default();
        scene.app.update();

        assert_eq!(
            scene.app.world().get::<PickingInteraction>(scene.widget),
            Some(&PickingInteraction::None)
        );
        let out_pointers = out_cursor
            .read(scene.app.world().resource::<Messages<Pointer<Out>>>())
            .filter(|event| event.entity == scene.widget)
            .map(|event| event.pointer_id)
            .collect::<Vec<_>>();
        assert!(out_pointers.contains(&PointerId::Mouse));
        assert!(out_pointers.contains(&scene.pointer));
    }

    #[test]
    fn panel_background_reports_only_the_panel_root() {
        let mut scene = build_picking_scene();
        let mut cursor = MessageCursor::<PointerHits>::default();
        let origin = scene.background_hit + Vec3::Z;
        let hits = cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z);

        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].picks.len(), 1);
        assert_eq!(hits[0].picks[0].0, scene.panel);
    }

    #[test]
    fn production_records_gate_partial_and_full_ancestor_clipping() {
        let mut scene = build_picking_scene();
        let partial = resolve_widget(&mut scene.app, scene.panel, "partial");
        let clipped = resolve_widget(&mut scene.app, scene.panel, "clipped");
        let mut cursor = MessageCursor::<PointerHits>::default();
        let partial_inside = scene.partial_inside;
        let partial_outside = scene.partial_outside;
        let clipped_hit = scene.clipped_hit;

        let inside = cast_scene_ray(
            &mut scene,
            &mut cursor,
            partial_inside + Vec3::Z,
            Dir3::NEG_Z,
        );
        assert_eq!(inside.len(), 1);
        assert!(inside[0].picks.iter().any(|(entity, _)| *entity == partial));

        let outside = cast_scene_ray(
            &mut scene,
            &mut cursor,
            partial_outside + Vec3::Z,
            Dir3::NEG_Z,
        );
        assert_eq!(outside.len(), 1);
        assert!(
            !outside[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == partial)
        );

        let fully_clipped =
            cast_scene_ray(&mut scene, &mut cursor, clipped_hit + Vec3::Z, Dir3::NEG_Z);
        assert_eq!(fully_clipped.len(), 1);
        assert!(
            !fully_clipped[0]
                .picks
                .iter()
                .any(|(entity, _)| *entity == clipped)
        );
    }

    #[test]
    fn actual_ray_selects_highest_z_then_latest_source_widget() {
        let mut scene = build_picking_scene();
        let mut cursor = MessageCursor::<PointerHits>::default();
        let overlap_hit = scene.overlap_hit;
        let hits = cast_scene_ray(&mut scene, &mut cursor, overlap_hit + Vec3::Z, Dir3::NEG_Z);

        assert_eq!(hits.len(), 1);
        let depth = |entity| {
            hits[0]
                .picks
                .iter()
                .find(|(picked, _)| *picked == entity)
                .map(|(_, hit)| hit.depth)
                .expect("overlapping widget should be in the production hit group")
        };
        assert!(depth(scene.front_last) < depth(scene.front_first));
        assert!(depth(scene.front_first) < depth(scene.back));
        let nearest = hits[0]
            .picks
            .iter()
            .min_by(|left, right| left.1.depth.total_cmp(&right.1.depth))
            .map(|(entity, _)| *entity)
            .expect("hit group should not be empty");
        assert_eq!(nearest, scene.front_last);
    }

    #[test]
    fn visibility_layers_and_panel_picking_filter_panel_hits() {
        let mut scene = build_picking_scene();
        let mut cursor = MessageCursor::<PointerHits>::default();
        let origin = scene.local_hit + Vec3::Z;

        scene
            .app
            .world_mut()
            .entity_mut(scene.camera)
            .insert(RenderLayers::layer(1));
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(RenderLayers::layer(2));
        settle_panel_source_change(&mut scene);
        assert!(cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).is_empty());

        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(RenderLayers::layer(1));
        settle_panel_source_change(&mut scene);
        assert_eq!(
            cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).len(),
            1
        );

        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(Visibility::Hidden);
        settle_panel_source_change(&mut scene);
        assert!(cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).is_empty());

        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(Visibility::Visible);
        settle_panel_source_change(&mut scene);
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(PanelPicking::PASS_THROUGH);
        assert!(cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).is_empty());
    }

    #[test]
    fn removing_panel_layers_resets_the_production_interaction_surface() {
        let mut scene = build_picking_scene();
        let mut cursor = MessageCursor::<PointerHits>::default();
        let origin = scene.local_hit + Vec3::Z;
        scene
            .app
            .world_mut()
            .entity_mut(scene.camera)
            .insert(RenderLayers::layer(3));
        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .insert(RenderLayers::layer(3));
        settle_panel_source_change(&mut scene);
        assert_eq!(
            cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).len(),
            1
        );

        scene
            .app
            .world_mut()
            .entity_mut(scene.panel)
            .remove::<RenderLayers>();
        settle_panel_source_change(&mut scene);
        assert!(cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).is_empty());

        scene
            .app
            .world_mut()
            .entity_mut(scene.camera)
            .remove::<RenderLayers>();
        assert_eq!(
            cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z).len(),
            1
        );
    }

    #[test]
    fn two_cameras_keep_their_camera_and_order_in_separate_groups() {
        let mut scene = build_picking_scene();
        let second_camera = scene
            .app
            .world_mut()
            .spawn(Camera {
                order: 11,
                ..default()
            })
            .id();
        let origin = scene.local_hit + Vec3::Z;
        scene.app.world_mut().resource_mut::<RayMap>().map.insert(
            RayId::new(second_camera, scene.pointer),
            Ray3d::new(origin, Dir3::NEG_Z),
        );
        let mut cursor = MessageCursor::<PointerHits>::default();
        let mut hits = cast_scene_ray(&mut scene, &mut cursor, origin, Dir3::NEG_Z);
        hits.sort_by(|left, right| left.order.total_cmp(&right.order));

        assert_eq!(hits.len(), 2);
        assert_close(hits[0].order, 7.0);
        assert_close(hits[1].order, 11.0);
        assert!(
            hits[0]
                .picks
                .iter()
                .all(|(_, hit)| hit.camera == scene.camera)
        );
        assert!(
            hits[1]
                .picks
                .iter()
                .all(|(_, hit)| hit.camera == second_camera)
        );
    }
}
