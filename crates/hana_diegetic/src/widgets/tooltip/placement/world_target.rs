use std::sync::Arc;

#[cfg(test)]
use bevy::camera::RenderTarget;
#[cfg(test)]
use bevy::camera::visibility::RenderLayers;
use bevy::ecs::system::SystemParam;
#[cfg(test)]
use bevy::prelude::App;
use bevy::prelude::Camera;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::Rect;
#[cfg(test)]
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
#[cfg(test)]
use bevy::prelude::Window;
use bevy::prelude::With;
use bevy::transform::helper::TransformHelper;
#[cfg(test)]
use bevy::window::PrimaryWindow;
#[cfg(test)]
use bevy::window::WindowRef;
#[cfg(test)]
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSite;
use hana_valence::ResolvedAnchorGeometry;

#[cfg(test)]
use super::placement_state::TooltipPlacementState;
use super::placement_state::WorldAnchorFrame;
use super::placement_state::WorldTargetGeometry;
use super::screen;
use super::viewport;
use super::viewport::HostBorder;
use super::viewport::OrientedScreenRect;
use super::viewport::TooltipSlideBounds;
use crate::layout::Anchor;
use crate::layout::BoundingBox;
use crate::layout::Unit;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
#[cfg(test)]
use crate::panel::PanelAnchorOffset;
#[cfg(test)]
use crate::panel::PanelEntity;
use crate::panel::PanelPlane;
use crate::panel::PanelScreenBounds;
#[cfg(test)]
use crate::panel::PanelSpace;
#[cfg(test)]
use crate::panel::WidgetEntity;
#[cfg(test)]
use crate::panel::World;
use crate::widgets::PanelWidget;
use crate::widgets::TooltipBoundary;
use crate::widgets::WidgetAnchorRect;
use crate::widgets::WidgetOf;
use crate::widgets::constants::QUAD_ANCHOR_COUNT;
use crate::widgets::tooltip::MaterializedTooltip;
#[cfg(test)]
use crate::widgets::tooltip::PrepareTooltip;
#[cfg(test)]
use crate::widgets::tooltip::Tooltip;
#[cfg(test)]
use crate::widgets::tooltip::TooltipPresentationCamera;
#[cfg(test)]
use crate::widgets::tooltip::TooltipReadiness;

/// Card-local occupied rectangles that a jack's help must clear.
/// Rectangles use the card's XY plane, with Y increasing toward its top.
#[derive(Clone, Component)]
pub struct TooltipCardOccupiedRegions {
    card:       Entity,
    card_rect:  Rect,
    occupied:   Arc<[Rect]>,
    placement:  CardTooltipPlacement,
    target_row: TargetControlRow<Rect>,
}

/// Whether card help may leave its host or must stay within it.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(super) enum CardTooltipPlacement {
    #[default]
    InsideOrOutside,
    InsideOnly,
}

impl TooltipCardOccupiedRegions {
    /// The card boundary and its readout, headings, jacks, labels, and controls.
    #[must_use]
    pub fn new(card: Entity, card_rect: Rect, occupied: Vec<Rect>) -> Self {
        Self {
            card,
            card_rect,
            occupied: occupied.into(),
            placement: CardTooltipPlacement::InsideOrOutside,
            target_row: TargetControlRow::TargetOnly,
        }
    }

    /// Keeps the help clear of the target's complete card-local control row.
    #[must_use]
    pub const fn with_target_row(mut self, row: Rect) -> Self {
        self.target_row = TargetControlRow::CompleteRow(row);
        self
    }
}

/// The authored control row, or the target alone for hosts without one.
#[derive(Clone, Copy)]
pub(super) enum TargetControlRow<Bounds> {
    TargetOnly,
    CompleteRow(Bounds),
}

impl TooltipCardOccupiedRegions {
    /// Keeps the complete help box on the card, even if it must overlap controls.
    #[must_use]
    pub const fn inside_only(mut self) -> Self {
        self.placement = CardTooltipPlacement::InsideOnly;
        self
    }
}

/// Where a world tooltip's card comes from: the occupied regions an app
/// authors on the target, or the layout element marked as the target's
/// tooltip boundary.
#[derive(SystemParam)]
pub(in crate::widgets::tooltip) struct WorldTooltipCardSources<'w, 's> {
    card_regions:       Query<'w, 's, &'static TooltipCardOccupiedRegions>,
    transforms:         Query<'w, 's, &'static GlobalTransform>,
    tooltip_boundaries: Query<'w, 's, &'static TooltipBoundary>,
    panel_layouts:      Query<'w, 's, &'static ComputedDiegeticPanel>,
}

/// Where a world tooltip may go.
#[derive(Clone)]
pub(super) enum WorldTooltipPlacementArea {
    /// The placement area, keeping clear of the border of the panel holding a
    /// target inside a marked element.
    Viewport(TooltipSlideBounds),
    /// A card whose occupied regions the help must clear.
    Card {
        bounds:         PanelScreenBounds,
        outside_bounds: PanelScreenBounds,
        occupied:       Vec<PanelScreenBounds>,
        target_row:     TargetControlRow<PanelScreenBounds>,
        gap_pixels:     f32,
        placement:      CardTooltipPlacement,
    },
}

impl WorldTooltipPlacementArea {
    /// Whether `painted` keeps clear of the border this area holds; card help
    /// keeps its card's own rule.
    pub(super) fn clears_border(&self, painted: PanelScreenBounds) -> bool {
        match self {
            Self::Viewport(bounds) => bounds.clears_host_border(painted),
            Self::Card { .. } => true,
        }
    }
}

#[derive(Clone)]
pub(super) struct WorldTooltipTarget {
    pub(super) bounds:                  PanelScreenBounds,
    anchors:                            [WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    pub(super) layout_unit:             Unit,
    pub(super) world_per_layout_unit:   Vec2,
    pub(super) world_per_layout_unit_z: f32,
    pub(super) placement_area:          WorldTooltipPlacementArea,
}

impl WorldTooltipTarget {
    pub(super) const fn anchor(&self, anchor: Anchor) -> WorldAnchorFrame {
        self.anchors[anchor_index(anchor)]
    }

    pub(super) const fn geometry(&self) -> WorldTargetGeometry {
        WorldTargetGeometry {
            anchors:                 self.anchors,
            layout_unit:             self.layout_unit,
            world_per_layout_unit:   self.world_per_layout_unit,
            world_per_layout_unit_z: self.world_per_layout_unit_z,
        }
    }
}

pub(super) fn world_control_sample_points(
    target: Entity,
    transforms: &TransformHelper<'_, '_>,
    panels: &Query<&DiegeticPanel>,
    anchor_geometry: &Query<&ResolvedAnchorGeometry>,
) -> Option<[Vec3; 5]> {
    let transform = transforms.compute_global_transform(target).ok()?;
    let anchors = if let Ok(panel) = panels.get(target) {
        world_panel_anchor_frames(PanelPlane::from_panel(panel, &transform).ok()?)
    } else {
        let geometry = anchor_geometry.get(target).ok()?;
        world_geometry_anchor_frames(geometry, &transform)?
    };
    Some(
        [
            Anchor::TopLeft,
            Anchor::TopRight,
            Anchor::BottomRight,
            Anchor::BottomLeft,
            Anchor::Center,
        ]
        .map(|anchor| anchors[anchor_index(anchor)].position),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum WorldTargetChange {
    Initial,
    Stable,
    Changed,
}

pub(super) struct TooltipCameraView<'a> {
    pub(super) camera:           &'a Camera,
    pub(super) global_transform: &'a GlobalTransform,
    pub(super) window:           Entity,
    pub(super) viewport:         Rect,
}

pub(super) fn panel_local_anchor(panel: &DiegeticPanel, anchor: Anchor) -> Vec3 {
    let size = Vec2::new(panel.world_width(), panel.world_height());
    let panel_anchor = Vec2::from(panel.anchor().offset(size.x, size.y));
    let anchor = Vec2::from(anchor.offset(size.x, size.y));
    Vec3::new(anchor.x - panel_anchor.x, panel_anchor.y - anchor.y, 0.0)
}

pub(super) fn project_world_point(point: Vec3, camera: &TooltipCameraView<'_>) -> Option<Vec2> {
    camera
        .camera
        .world_to_viewport(camera.global_transform, point)
        .ok()
}

pub(super) fn projected_points_bounds<const N: usize>(
    points: [Vec3; N],
    camera: &TooltipCameraView<'_>,
) -> Option<PanelScreenBounds> {
    let mut minimum = Vec2::splat(f32::INFINITY);
    let mut maximum = Vec2::splat(f32::NEG_INFINITY);
    for point in points {
        let projected = camera
            .camera
            .world_to_viewport(camera.global_transform, point)
            .ok()?;
        minimum = minimum.min(projected);
        maximum = maximum.max(projected);
    }
    PanelScreenBounds::new(minimum, maximum - minimum).ok()
}

pub(super) fn world_tooltip_target(
    materialized: &MaterializedTooltip,
    gap_meters: f32,
    front_clearance_meters: f32,
    camera: &TooltipCameraView<'_>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    anchor_geometry: &Query<(&ResolvedAnchorGeometry, &GlobalTransform)>,
    card_sources: &WorldTooltipCardSources<'_, '_>,
) -> Option<WorldTooltipTarget> {
    let placement_area = match card_sources.card_regions.get(materialized.target) {
        Ok(regions) => {
            let transform = card_sources.transforms.get(regions.card).ok()?;
            let card_normal = transform.affine().transform_vector3(Vec3::Z).normalize();
            let project = |rect: Rect, depth: f32| {
                projected_points_bounds(
                    [
                        Vec3::new(rect.min.x, rect.min.y, 0.0),
                        Vec3::new(rect.max.x, rect.min.y, 0.0),
                        Vec3::new(rect.max.x, rect.max.y, 0.0),
                        Vec3::new(rect.min.x, rect.max.y, 0.0),
                    ]
                    .map(|point| transform.transform_point(point) + card_normal * depth),
                    camera,
                )
            };
            let bounds = project(regions.card_rect, 0.0)?;
            let outside_bounds = project(regions.card_rect, front_clearance_meters)?;
            let occupied = regions
                .occupied
                .iter()
                .copied()
                .map(|rect| project(rect, 0.0))
                .collect::<Option<Vec<_>>>()?;
            let target_row = match regions.target_row {
                TargetControlRow::TargetOnly => TargetControlRow::TargetOnly,
                TargetControlRow::CompleteRow(row) => {
                    TargetControlRow::CompleteRow(project(row, 0.0)?)
                },
            };
            let card_scale = transform.affine().transform_vector3(Vec3::X).length();
            let gap_pixels =
                gap_meters / card_scale * outside_bounds.size().x / regions.card_rect.width();
            WorldTooltipPlacementArea::Card {
                bounds,
                outside_bounds,
                occupied,
                target_row,
                gap_pixels,
                placement: regions.placement,
            }
        },
        Err(_) => WorldTooltipPlacementArea::Viewport(world_slide_bounds(
            materialized.target,
            camera,
            panels,
            widgets,
            card_sources,
        )),
    };
    if let Ok((panel, global_transform)) = panels.get(materialized.target) {
        let plane = crate::panel::PanelPlane::from_panel(panel, global_transform).ok()?;
        let anchors = world_panel_anchor_frames(plane);
        let world_per_layout_unit = Vec2::new(
            plane.size().x / panel.width(),
            plane.size().y / panel.height(),
        );
        return Some(WorldTooltipTarget {
            bounds: projected_world_anchors(&anchors, camera)?,
            anchors,
            layout_unit: panel.layout_unit(),
            world_per_layout_unit,
            world_per_layout_unit_z: world_per_layout_unit.x,
            placement_area,
        });
    }
    let (geometry, global_transform) = anchor_geometry.get(materialized.target).ok()?;
    let anchors = world_geometry_anchor_frames(geometry, global_transform)?;
    let world_per_layout_unit = widgets
        .get(materialized.target)
        .ok()
        .and_then(|widget_of| panels.get(widget_of.panel()).ok())
        .and_then(|(panel, global_transform)| {
            let plane = crate::panel::PanelPlane::from_panel(panel, global_transform).ok()?;
            Some(Vec2::new(
                plane.size().x / panel.width(),
                plane.size().y / panel.height(),
            ))
        })
        .unwrap_or_else(|| Vec2::splat(materialized.layout_unit.meters_per_unit()));
    viewport::valid_screen_scale(world_per_layout_unit).then_some(WorldTooltipTarget {
        bounds: projected_world_anchors(&anchors, camera)?,
        anchors,
        layout_unit: materialized.layout_unit,
        world_per_layout_unit,
        world_per_layout_unit_z: world_per_layout_unit.x,
        placement_area,
    })
}

/// What bounds a widget target's tooltip: inside a marked element, the element
/// and its panel's border, each projected through the panel as the panel's
/// corners are, the tooltip keeping clear of the panel's border inside or
/// outside; else the placement area alone.
fn world_slide_bounds(
    tooltip_target: Entity,
    camera: &TooltipCameraView<'_>,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    card_sources: &WorldTooltipCardSources<'_, '_>,
) -> TooltipSlideBounds {
    let Ok(TooltipBoundary::Element(element)) = card_sources.tooltip_boundaries.get(tooltip_target)
    else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let Ok(widget_of) = widgets.get(tooltip_target) else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let Ok((panel, panel_global)) = panels.get(widget_of.panel()) else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let Some(result) = card_sources
        .panel_layouts
        .get(widget_of.panel())
        .ok()
        .and_then(|computed| computed.layout().solved())
    else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    let orient = |bounds: BoundingBox| {
        let rect = WidgetAnchorRect::new(panel, bounds);
        let centre = rect.panel_offset();
        let half = rect.size() * 0.5;
        let corner = |x: f32, y: f32| {
            project_world_point(
                panel_global.transform_point(centre + Vec3::new(x, y, 0.0)),
                camera,
            )
        };
        Some(OrientedScreenRect::from_corners(
            corner(-half.x, half.y)?,
            corner(half.x, half.y)?,
            corner(-half.x, -half.y)?,
        ))
    };
    // The panel is the host: its tooltip slides inside the marked element's
    // inner edge, then inside the panel's, or leaves past the panel's border,
    // never across it.
    let Some(host) = result.computed.first().and_then(|root| {
        Some(HostBorder::new(
            orient(root.bounds)?,
            orient(screen::border_inner_edge(result, root.bounds))?,
        ))
    }) else {
        return TooltipSlideBounds::PlacementAreaOnly;
    };
    orient(screen::border_inner_edge(result, *element))
        .and_then(|inner| {
            let bounds = inner.upright_bounds();
            PanelScreenBounds::new(bounds.min, bounds.size()).ok()
        })
        .map_or(TooltipSlideBounds::Host(host), |element| {
            TooltipSlideBounds::marked_element(element, host)
        })
}

pub(super) fn world_tooltip_depth_scale(
    materialized: &MaterializedTooltip,
    panels: &Query<(&DiegeticPanel, &GlobalTransform)>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
) -> f32 {
    let target_panel = panels.get(materialized.target).ok().or_else(|| {
        widgets
            .get(materialized.target)
            .ok()
            .and_then(|widget_of| panels.get(widget_of.panel()).ok())
    });
    target_panel
        .and_then(|(panel, transform)| {
            let plane = PanelPlane::from_panel(panel, transform).ok()?;
            Some(plane.size().x / panel.width())
        })
        .unwrap_or_else(|| materialized.layout_unit.meters_per_unit())
}

fn world_panel_anchor_frames(plane: PanelPlane) -> [WorldAnchorFrame; QUAD_ANCHOR_COUNT] {
    all_anchors().map(|anchor| WorldAnchorFrame {
        position: plane.point(anchor),
        right:    plane.right(),
        up:       plane.up(),
        normal:   plane.normal(),
    })
}

fn world_geometry_anchor_frames(
    geometry: &ResolvedAnchorGeometry,
    global_transform: &GlobalTransform,
) -> Option<[WorldAnchorFrame; QUAD_ANCHOR_COUNT]> {
    let rotation = global_transform.rotation();
    all_anchors()
        .map(|anchor| {
            let frame = geometry.frame(AnchorSite::from(anchor)).ok()?;
            let orientation = rotation * frame.orientation().into_inner();
            Some(WorldAnchorFrame {
                position: global_transform.transform_point(frame.position().into_inner()),
                right:    orientation * Vec3::X,
                up:       orientation * Vec3::Y,
                normal:   orientation * Vec3::Z,
            })
        })
        .into_iter()
        .collect::<Option<Vec<_>>>()?
        .try_into()
        .ok()
}

fn projected_world_anchors(
    anchors: &[WorldAnchorFrame; QUAD_ANCHOR_COUNT],
    camera: &TooltipCameraView<'_>,
) -> Option<PanelScreenBounds> {
    projected_points_bounds(anchors.map(|anchor| anchor.position), camera)
}

const fn all_anchors() -> [Anchor; QUAD_ANCHOR_COUNT] {
    [
        Anchor::TopLeft,
        Anchor::TopCenter,
        Anchor::TopRight,
        Anchor::CenterLeft,
        Anchor::Center,
        Anchor::CenterRight,
        Anchor::BottomLeft,
        Anchor::BottomCenter,
        Anchor::BottomRight,
    ]
}

const fn anchor_index(anchor: Anchor) -> usize {
    match anchor {
        Anchor::TopLeft => 0,
        Anchor::TopCenter => 1,
        Anchor::TopRight => 2,
        Anchor::CenterLeft => 3,
        Anchor::Center => 4,
        Anchor::CenterRight => 5,
        Anchor::BottomLeft => 6,
        Anchor::BottomCenter => 7,
        Anchor::BottomRight => 8,
    }
}

#[cfg(test)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::MinimalPlugins;
    use bevy::transform::TransformPlugin;
    use hana_valence::AnchoredTo;
    use hana_valence::ResolvedAnchorOffset;

    use super::*;
    use crate::DiegeticPanel;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::PanelElementId;
    use crate::PanelWidgetReader;
    use crate::Pt;
    use crate::layout::El;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::TooltipCommandsExt;

    const GEOMETRY_EPSILON: f32 = 1e-4;

    const MATERIALIZATION_UPDATES: usize = 5;

    const TEST_CAMERA_DISTANCE: f32 = 5.0;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    fn materialization_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app
    }

    fn update_materialization(app: &mut App) {
        for _ in 0..MATERIALIZATION_UPDATES {
            app.update();
        }
    }

    fn prepare_tooltip(app: &mut App, controller: Entity) {
        app.world_mut()
            .entity_mut(controller)
            .insert(PrepareTooltip);
    }

    fn seeded_perspective_camera() -> Camera { seeded_perspective_camera_for(TEST_VIEWPORT) }

    fn seeded_perspective_camera_for(viewport: Vec2) -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: viewport.as_uvec2(),
            scale_factor:  1.0,
        });
        let mut projection = Projection::Perspective(PerspectiveProjection::default());
        projection.update(viewport.x, viewport.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn spawn_presentation_camera(
        app: &mut App,
        window: Entity,
        render_layers: RenderLayers,
    ) -> Entity {
        let transform = Transform::from_xyz(0.0, 0.0, TEST_CAMERA_DISTANCE);
        app.world_mut()
            .spawn((
                seeded_perspective_camera(),
                RenderTarget::Window(WindowRef::Entity(window)),
                render_layers,
                transform,
                GlobalTransform::from(transform),
            ))
            .id()
    }

    fn fixed_size_tooltip(size: Vec2) -> Tooltip {
        let mut tooltip = Tooltip::new(El::new());
        tooltip.with(El::new().size(size.x, size.y), |_| {});
        tooltip
    }

    fn tooltip_target_tree() -> crate::LayoutTree {
        let mut builder = LayoutBuilder::new(100.0, 50.0);
        builder.with(El::new().size(50.0, 20.0).button("action"), |_| {});
        builder.build()
    }

    fn widget(app: &mut App, panel: Entity) -> Entity {
        let widget = app
            .world_mut()
            .run_system_once(move |reader: PanelWidgetReader| {
                reader.entity(panel, &PanelElementId::named("action"))
            })
            .ok()
            .flatten()
            .unwrap_or(Entity::PLACEHOLDER);
        assert_ne!(widget, Entity::PLACEHOLDER);
        widget
    }

    #[test]
    fn world_panel_and_widget_targets_inherit_layout_unit_and_apply_widget_offset() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: TEST_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(0));
        let target_panel = DiegeticPanel::world()
            .size(Pt(100.0), Pt(50.0))
            .with_tree(tooltip_target_tree())
            .build();
        assert!(target_panel.is_ok());
        let target_panel = target_panel.map_or(Entity::PLACEHOLDER, |panel| {
            app.world_mut().spawn(panel).id()
        });
        assert_ne!(target_panel, Entity::PLACEHOLDER);
        app.update();
        let target_widget = widget(&mut app, target_panel);

        let panel_controller = app.world_mut().commands().spawn_tooltip(
            PanelEntity::<World>::from_validated(target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(30.0, 10.0)),
        );
        let widget_offset = 12.0;
        let widget_controller = app.world_mut().commands().spawn_tooltip(
            WidgetEntity::<World>::from_validated(target_widget, target_panel, PanelSpace::World),
            fixed_size_tooltip(Vec2::new(30.0, 10.0))
                .offset(PanelAnchorOffset::new(0.0, widget_offset)),
        );
        app.update();
        for controller in [panel_controller, widget_controller] {
            prepare_tooltip(&mut app, controller);
            app.world_mut()
                .entity_mut(controller)
                .insert(TooltipPresentationCamera::new(camera));
        }
        update_materialization(&mut app);

        for (controller, target) in [
            (panel_controller, target_panel),
            (widget_controller, target_widget),
        ] {
            assert_eq!(
                app.world()
                    .get::<MaterializedTooltip>(controller)
                    .map(MaterializedTooltip::layout_unit),
                Some(Unit::Points)
            );
            assert_eq!(
                app.world()
                    .get::<AnchoredTo>(controller)
                    .map(AnchoredTo::target),
                Some(target)
            );
            let diagnostics = app
                .world()
                .resource::<AnchorResolveDiagnostics>()
                .current()
                .copied()
                .collect::<Vec<_>>();
            assert_eq!(
                app.world().get::<TooltipReadiness>(controller),
                Some(&TooltipReadiness::Ready),
                "controller {controller:?}: placement={:?}, diagnostics={diagnostics:?}",
                app.world().get::<TooltipPlacementState>(controller),
            );
            let local = app.world().get::<Transform>(controller);
            let global = app.world().get::<GlobalTransform>(controller);
            assert!(local.is_some());
            assert!(global.is_some());
            if let (Some(local), Some(global)) = (local, global) {
                assert!(
                    global
                        .translation()
                        .abs_diff_eq(local.translation, GEOMETRY_EPSILON)
                );
            }
        }

        let target_panel_data = app.world().get::<DiegeticPanel>(target_panel);
        assert!(target_panel_data.is_some());
        let Some(target_panel_data) = target_panel_data else {
            return;
        };
        let expected_y = -widget_offset
            * target_panel_data.layout_unit().to_points()
            * target_panel_data.points_to_world();
        assert!(
            app.world()
                .get::<ResolvedAnchorOffset>(widget_controller)
                .is_some_and(|offset| {
                    let offset = offset.offset().into_inner();
                    offset
                        .truncate()
                        .abs_diff_eq(Vec2::new(0.0, expected_y), GEOMETRY_EPSILON)
                        && offset.z > 0.0
                })
        );
    }
}
