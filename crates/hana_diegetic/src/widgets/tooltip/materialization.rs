use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::transform::helper::TransformHelper;
use bevy::window::PrimaryWindow;
use bevy::window::WindowRef;
use hana_valence::AnchorSite;
use hana_valence::ResolvedAnchorGeometry;

use super::controller::TooltipFor;
use super::declaration::AuthoredTooltipTargetSpace;
use super::declaration::PrepareTooltip;
use super::declaration::Tooltip;
use super::declaration::TooltipExclusivity;
use super::declaration::TooltipFontUnit;
use super::declaration::TooltipInkPadding;
use super::declaration::TooltipPhase;
use super::placement;
use super::placement::TooltipPlacementResult;
use super::placement::TooltipPlacementState;
use super::target::MeshAnchorTarget;
use crate::cascade::CascadeEntityCommandsExt;
use crate::constants::DIAGNOSTIC_LOG_TARGET;
use crate::layout;
use crate::layout::Border;
use crate::layout::ChildLayoutState;
use crate::layout::CornerRadius;
use crate::layout::Dimension;
use crate::layout::El;
use crate::layout::LayoutOnly;
use crate::layout::LayoutTree;
use crate::layout::Lighting;
use crate::layout::Padding;
use crate::layout::Px;
use crate::layout::Sidedness;
use crate::layout::Sizing;
use crate::layout::TextInkMeasurement;
use crate::layout::TextMeasure;
use crate::layout::TextStyle;
use crate::layout::Unit;
use crate::panel;
use crate::panel::CoordinateSpace;
use crate::panel::DiegeticPanel;
use crate::panel::PanelAnchorOffset;
use crate::panel::PanelAttachment;
use crate::panel::PanelAttachmentAuthored;
use crate::panel::PanelPlane;
use crate::panel::PanelSpace;
use crate::render;
use crate::render::TooltipDepthPriority;
use crate::screen_space::ScreenAnchorTarget;
use crate::text::DiegeticTextMeasurer;
use crate::text::FontId;
use crate::text::FontRegistry;
use crate::widgets::ControlPalette;
use crate::widgets::PanelPicking;
use crate::widgets::PanelWidget;
use crate::widgets::WidgetOf;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum TooltipPresentationSpace {
    World,
    Screen {
        window:       Entity,
        camera_order: isize,
    },
}

impl TooltipPresentationSpace {
    pub(in crate::widgets::tooltip) const fn space(self) -> PanelSpace {
        match self {
            Self::World => PanelSpace::World,
            Self::Screen { .. } => PanelSpace::Screen,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum PreMaterializationComponent<T> {
    Absent,
    Present(T),
}

impl<T: Copy> PreMaterializationComponent<T> {
    const fn from_query_item(item: Option<&T>) -> Self {
        match item {
            Some(component) => Self::Present(*component),
            None => Self::Absent,
        }
    }
}

#[derive(Clone, Component)]
pub(crate) struct MaterializedTooltip {
    pub(super) target:              Entity,
    pub(super) presentation:        TooltipPresentationSpace,
    pub(super) layout_unit:         Unit,
    pub(super) render_layers:       RenderLayers,
    pub(super) authored_attachment: PanelAttachment,
    pub(super) previous_transform:  PreMaterializationComponent<Transform>,
    pub(super) previous_global:     PreMaterializationComponent<GlobalTransform>,
    pub(super) previous_visibility: PreMaterializationComponent<Visibility>,
}

impl MaterializedTooltip {
    pub(crate) const fn target(&self) -> Entity { self.target }

    pub(crate) const fn space(&self) -> PanelSpace { self.presentation.space() }

    pub(crate) const fn layout_unit(&self) -> Unit { self.layout_unit }
}

#[derive(Clone, Copy, Component)]
pub(super) struct TooltipPresentationCamera {
    pub(super) camera: Entity,
}

/// How a tooltip's inward border meets its projected outer edge.
#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(crate) enum TooltipPaintInsideBorder {
    ScreenPixelGrid,
    ViewAlignedWorldPixelGrid,
    AntialiasedWorldPlane,
}

impl TooltipPresentationCamera {
    #[cfg(test)]
    pub(super) const fn new(camera: Entity) -> Self { Self { camera } }
}

#[derive(Clone, Copy, Component, Debug, Eq, PartialEq)]
pub(super) enum TooltipReadiness {
    Pending,
    Ready,
}

#[derive(Clone, Copy, Component)]
pub(super) struct TooltipWidthConstraint(pub(super) f32);

#[derive(Resource)]
pub(super) struct TooltipSurfaceMaterial(pub(super) Handle<StandardMaterial>);

impl FromWorld for TooltipSurfaceMaterial {
    fn from_world(world: &mut World) -> Self {
        world.init_resource::<Assets<StandardMaterial>>();
        let handle = world
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                alpha_mode: AlphaMode::Opaque,
                ..render::default_panel_material()
            });
        Self(handle)
    }
}

#[derive(Clone, Copy, Component)]
pub(super) enum TooltipWidthConstraintRequest {
    Apply(f32),
    Restore,
}

#[derive(Clone)]
struct TooltipMaterializationContext {
    target:        Entity,
    presentation:  TooltipPresentationSpace,
    layout_unit:   Unit,
    render_layers: RenderLayers,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum TooltipFrameRasterGrid {
    Unprojected,
    ViewAligned {
        border_physical_per_design_pixel: Vec2,
        body_physical_per_design_pixel:   Vec2,
    },
}

#[derive(Clone, Copy)]
enum TooltipProjectionCameraPose {
    LastPropagated,
    Current(GlobalTransform),
}

struct WorldTooltipAnchorPlane {
    origin: Vec3,
    right:  Vec3,
    up:     Vec3,
    normal: Vec3,
}

/// Built raster grid and the latest observation, retained across hides.
/// Two equal observations let a moving view settle before replacing fitted layout.
#[derive(Clone, Copy, Component)]
pub(super) struct WorldTooltipFrameProjection {
    grid:          TooltipFrameRasterGrid,
    observed_grid: TooltipFrameRasterGrid,
    world_scale:   Vec3,
    calibration:   WorldTooltipRasterCalibration,
}

#[derive(Clone, Copy)]
enum WorldTooltipRasterCalibration {
    Current,
    AwaitingSecondObservation,
    MovingGrid,
}

/// A help revealed during motion has fitted its final raster grid but still
/// needs the existing chosen attachment snapped to that grid.
#[derive(Component)]
pub(super) struct WorldTooltipRasterFitAwaitingPlacement;

impl WorldTooltipFrameProjection {
    /// A newly changed grid gets one frame to settle. Continuous motion must
    /// not withhold already fitted help while calibration waits for the view.
    pub(super) const fn allows_reveal(&self) -> bool {
        !matches!(
            self.calibration,
            WorldTooltipRasterCalibration::AwaitingSecondObservation
        )
    }

    fn needs_rebuild(&mut self, grid: TooltipFrameRasterGrid) -> bool {
        if grid != self.observed_grid {
            self.observed_grid = grid;
            self.calibration = match self.calibration {
                WorldTooltipRasterCalibration::Current => {
                    WorldTooltipRasterCalibration::AwaitingSecondObservation
                },
                _ => WorldTooltipRasterCalibration::MovingGrid,
            };
            return false;
        }
        if grid == self.grid {
            if !matches!(self.calibration, WorldTooltipRasterCalibration::Current) {
                self.calibration = WorldTooltipRasterCalibration::Current;
            }
            return false;
        }
        true
    }
}

pub(super) fn materialize_requested_tooltips(
    help_controllers: Query<(Entity, &Tooltip, &TooltipPhase)>,
    controllers: Query<
        (
            Entity,
            &Tooltip,
            &TooltipFor,
            Option<&AuthoredTooltipTargetSpace>,
            Has<DiegeticPanel>,
            Option<&Transform>,
            Option<&GlobalTransform>,
            Option<&Visibility>,
            Option<&TooltipPresentationCamera>,
        ),
        With<PrepareTooltip>,
    >,
    panels: Query<&DiegeticPanel>,
    widgets: Query<&WidgetOf, With<PanelWidget>>,
    panel_globals: Query<&GlobalTransform, With<DiegeticPanel>>,
    projection_entities: Query<(&GlobalTransform, Option<&Camera>)>,
    anchor_geometry: Query<&ResolvedAnchorGeometry>,
    mesh_targets: Query<(), With<MeshAnchorTarget>>,
    screen_targets: Query<&ScreenAnchorTarget>,
    target_layers: Query<&RenderLayers>,
    windows: Query<&Window>,
    primary_window: Query<Entity, With<PrimaryWindow>>,
    defaults: Res<crate::cascade::PanelDefaults>,
    surface_material: Res<TooltipSurfaceMaterial>,
    text: Option<(Res<FontRegistry>, Res<DiegeticTextMeasurer>)>,
    mut commands: Commands,
) {
    let text_measure = text
        .as_ref()
        .map_or(TooltipTextMeasure::TextNotReady, |(fonts, measurer)| {
            TooltipTextMeasure::Ready { fonts, measurer }
        });
    let sole_help = sole_help_controller(&help_controllers);
    for (
        controller,
        tooltip,
        tooltip_for,
        authored_space,
        has_panel,
        previous_transform,
        previous_global,
        previous_visibility,
        presentation_camera,
    ) in &controllers
    {
        if sole_help.is_some_and(|sole_help| sole_help != controller) {
            continue;
        }
        if has_panel {
            commands.entity(controller).remove::<PrepareTooltip>();
            continue;
        }
        let Some(context) = tooltip_materialization_context(
            tooltip_for.target(),
            authored_space,
            &panels,
            &widgets,
            &anchor_geometry,
            &mesh_targets,
            &screen_targets,
            &target_layers,
            &windows,
            &primary_window,
            &defaults,
        ) else {
            continue;
        };
        let Some(mut panel) = build_tooltip_panel(
            tooltip,
            tooltip.blueprint(),
            &context,
            &windows,
            &surface_material.0,
            defaults.panel_font_unit,
            text_measure,
            TooltipFrameRasterGrid::Unprojected,
        ) else {
            continue;
        };
        let width_constraint = initial_tooltip_width_constraint(tooltip, &context, &windows);
        let world_scale = if tooltip.has_source_label_sized_text() {
            world_tooltip_scale(context.target, &panel, &panels, &widgets, &panel_globals)
        } else {
            Vec3::ONE
        };
        let (sized_panel, frame_grid) = size_world_tooltip_frame_to_projection(
            tooltip,
            panel,
            world_scale,
            &context,
            presentation_camera,
            &projection_entities,
            &anchor_geometry,
            &panels,
            &windows,
            &surface_material.0,
            defaults.panel_font_unit,
            text_measure,
        );
        panel = sized_panel;
        let authored_attachment = tooltip.attachment();
        finish_tooltip_materialization(
            &mut commands,
            controller,
            panel,
            &context,
            authored_attachment,
            previous_transform,
            previous_global,
            previous_visibility,
            world_scale,
            width_constraint,
            frame_grid,
        );
    }
}

fn finish_tooltip_materialization(
    commands: &mut Commands,
    controller: Entity,
    panel: DiegeticPanel,
    context: &TooltipMaterializationContext,
    authored_attachment: PanelAttachment,
    previous_transform: Option<&Transform>,
    previous_global: Option<&GlobalTransform>,
    previous_visibility: Option<&Visibility>,
    world_scale: Vec3,
    width_constraint: Option<TooltipWidthConstraint>,
    frame_grid: TooltipFrameRasterGrid,
) {
    let materialized = MaterializedTooltip {
        target: context.target,
        presentation: context.presentation,
        layout_unit: context.layout_unit,
        render_layers: context.render_layers.clone(),
        authored_attachment,
        previous_transform: PreMaterializationComponent::from_query_item(previous_transform),
        previous_global: PreMaterializationComponent::from_query_item(previous_global),
        previous_visibility: PreMaterializationComponent::from_query_item(previous_visibility),
    };
    commands
        .entity(controller)
        .remove::<(PrepareTooltip, PanelPicking)>()
        .insert((
            panel,
            Visibility::Hidden,
            materialized,
            TooltipReadiness::Pending,
            TooltipPlacementState::pending(authored_attachment),
            match context.presentation {
                TooltipPresentationSpace::Screen { .. } => {
                    TooltipPaintInsideBorder::ScreenPixelGrid
                },
                TooltipPresentationSpace::World
                    if matches!(frame_grid, TooltipFrameRasterGrid::ViewAligned { .. }) =>
                {
                    TooltipPaintInsideBorder::ViewAlignedWorldPixelGrid
                },
                TooltipPresentationSpace::World => TooltipPaintInsideBorder::AntialiasedWorldPlane,
            },
        ));
    if matches!(context.presentation, TooltipPresentationSpace::World) {
        commands
            .entity(controller)
            .insert(WorldTooltipFrameProjection {
                grid: frame_grid,
                observed_grid: frame_grid,
                world_scale,
                calibration: WorldTooltipRasterCalibration::Current,
            });
        let pose = previous_transform
            .copied()
            .unwrap_or_default()
            .with_scale(world_scale);
        insert_world_tooltip_pose(commands, controller, pose);
    }
    if let Some(width_constraint) = width_constraint {
        commands.entity(controller).insert(width_constraint);
    }
    panel::write_owned_render_layers(
        commands,
        controller,
        controller,
        Some(context.render_layers.clone()),
    );
}

fn size_world_tooltip_frame_to_projection(
    tooltip: &Tooltip,
    panel: DiegeticPanel,
    world_scale: Vec3,
    context: &TooltipMaterializationContext,
    presentation_camera: Option<&TooltipPresentationCamera>,
    projection_entities: &Query<(&GlobalTransform, Option<&Camera>)>,
    anchor_geometry: &Query<&ResolvedAnchorGeometry>,
    target_panels: &Query<&DiegeticPanel>,
    windows: &Query<&Window>,
    surface_material: &Handle<StandardMaterial>,
    panel_font_unit: Unit,
    text_measure: TooltipTextMeasure<'_>,
) -> (DiegeticPanel, TooltipFrameRasterGrid) {
    let frame_grid = projected_world_tooltip_frame_grid(
        tooltip,
        &panel,
        world_scale,
        tooltip_front_depth_meters(tooltip, &panel, world_scale),
        context.target,
        presentation_camera,
        projection_entities,
        anchor_geometry,
        target_panels,
        TooltipProjectionCameraPose::LastPropagated,
    );
    if let TooltipFrameRasterGrid::ViewAligned { .. } = frame_grid {
        let sized = build_tooltip_panel(
            tooltip,
            tooltip.blueprint(),
            context,
            windows,
            surface_material,
            panel_font_unit,
            text_measure,
            frame_grid,
        )
        .unwrap_or(panel);
        return (sized, frame_grid);
    }
    (panel, frame_grid)
}

fn projected_world_tooltip_frame_grid(
    tooltip: &Tooltip,
    panel: &DiegeticPanel,
    world_scale: Vec3,
    front_depth_meters: f32,
    target: Entity,
    presentation_camera: Option<&TooltipPresentationCamera>,
    projection_entities: &Query<(&GlobalTransform, Option<&Camera>)>,
    anchor_geometry: &Query<&ResolvedAnchorGeometry>,
    target_panels: &Query<&DiegeticPanel>,
    camera_pose: TooltipProjectionCameraPose,
) -> TooltipFrameRasterGrid {
    let Some(presentation_camera) = presentation_camera else {
        return TooltipFrameRasterGrid::Unprojected;
    };
    let (Ok((target_global, _)), Ok((camera_global, Some(camera)))) = (
        projection_entities.get(target),
        projection_entities.get(presentation_camera.camera),
    ) else {
        return TooltipFrameRasterGrid::Unprojected;
    };
    let camera_global = match camera_pose {
        TooltipProjectionCameraPose::LastPropagated => *camera_global,
        TooltipProjectionCameraPose::Current(current) => current,
    };
    let anchor = world_tooltip_anchor_plane(
        tooltip,
        target,
        target_global,
        anchor_geometry,
        target_panels,
    );
    let WorldTooltipAnchorPlane {
        origin,
        right,
        up,
        normal,
    } = anchor;
    if !normal.is_finite() || normal.dot(camera_global.forward().as_vec3()).abs() < 0.9999 {
        return TooltipFrameRasterGrid::Unprojected;
    }
    let (Some(physical), Some(logical)) = (
        camera.physical_viewport_size(),
        camera.logical_viewport_size(),
    ) else {
        return TooltipFrameRasterGrid::Unprojected;
    };
    let physical_per_logical = physical.as_vec2() / logical;
    let frame_scale = TooltipFrameScale::for_presentation(
        TooltipPresentationSpace::World,
        tooltip.drawn_label_size,
    );
    let frame_world = frame_scale
        .dimension(1.0)
        .to_meters(panel.layout_unit().meters_per_unit());
    let projected_scale = |origin| {
        let center = camera.world_to_viewport(&camera_global, origin).ok()?;
        let x = camera
            .world_to_viewport(&camera_global, origin + right * frame_world * world_scale.x)
            .ok()?;
        let y = camera
            .world_to_viewport(&camera_global, origin + up * frame_world * world_scale.y)
            .ok()?;
        let x = (x - center) * physical_per_logical;
        let y = (y - center) * physical_per_logical;
        (x.y.abs() <= 0.01 && y.x.abs() <= 0.01).then_some(Vec2::new(x.x.abs(), y.y.abs()))
    };
    let Some(border_scale) = projected_scale(origin) else {
        return TooltipFrameRasterGrid::Unprojected;
    };
    if !border_scale.is_finite() || border_scale.min_element() <= 0.0 {
        return TooltipFrameRasterGrid::Unprojected;
    }
    // The body sits in front of the target. Its perspective scale must be
    // measured there, while the border retains the target-plane authored width.
    let Some(body_scale) = projected_scale(origin + normal * front_depth_meters) else {
        return TooltipFrameRasterGrid::Unprojected;
    };
    if !body_scale.is_finite() || body_scale.min_element() <= 0.0 {
        return TooltipFrameRasterGrid::Unprojected;
    }
    TooltipFrameRasterGrid::ViewAligned {
        border_physical_per_design_pixel: border_scale,
        body_physical_per_design_pixel:   body_scale,
    }
}

fn world_tooltip_anchor_plane(
    tooltip: &Tooltip,
    target: Entity,
    global: &GlobalTransform,
    anchor_geometry: &Query<&ResolvedAnchorGeometry>,
    target_panels: &Query<&DiegeticPanel>,
) -> WorldTooltipAnchorPlane {
    let anchor = tooltip.attachment().target_anchor();
    if let Ok(geometry) = anchor_geometry.get(target)
        && let Ok(frame) = geometry.frame(AnchorSite::from(anchor))
    {
        let orientation = global.rotation() * frame.orientation().into_inner();
        return WorldTooltipAnchorPlane {
            origin: global.transform_point(frame.position().into_inner()),
            right:  orientation * Vec3::X,
            up:     orientation * Vec3::Y,
            normal: orientation * Vec3::Z,
        };
    }
    let affine = global.affine();
    let right = affine.transform_vector3(Vec3::X).normalize();
    let up = affine.transform_vector3(Vec3::Y).normalize();
    let origin = target_panels
        .get(target)
        .ok()
        .and_then(|panel| PanelPlane::from_panel(panel, global).ok())
        .map_or_else(|| global.translation(), |plane| plane.point(anchor));
    WorldTooltipAnchorPlane {
        origin,
        right,
        up,
        normal: right.cross(up).normalize(),
    }
}

fn tooltip_front_depth_meters(tooltip: &Tooltip, panel: &DiegeticPanel, world_scale: Vec3) -> f32 {
    let authored_z = tooltip
        .attachment()
        .offset()
        .to_layout_units(panel.layout_unit())
        .z;
    let world_per_layout_unit =
        panel.points_to_world() * world_scale.z * panel.layout_unit().to_points();
    (authored_z * world_per_layout_unit).max(placement::WORLD_TOOLTIP_FRONT_CLEARANCE_METERS)
}

/// Poses an unlit world tooltip in its depth band, retaining its plain back.
fn insert_world_tooltip_pose(commands: &mut Commands, controller: Entity, pose: Transform) {
    commands
        .entity(controller)
        .insert((TooltipDepthPriority, pose, GlobalTransform::from(pose)))
        .override_sidedness(Sidedness::BothSides)
        .override_lighting(Lighting::Unlit);
}

fn sole_help_controller(controllers: &Query<(Entity, &Tooltip, &TooltipPhase)>) -> Option<Entity> {
    controllers.iter().find_map(|(entity, tooltip, phase)| {
        (tooltip.exclusivity == TooltipExclusivity::SoleHelp
            && matches!(
                phase,
                TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible
            ))
        .then_some(entity)
    })
}

/// Match the source panel's drawn point size, including its authored world
/// width and ancestor scale. Fit sizing changes the tooltip's dimensions but
/// keeps its points-to-world factor stable.
fn world_tooltip_scale(
    target: Entity,
    tooltip: &DiegeticPanel,
    panels: &Query<&DiegeticPanel>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    globals: &Query<&GlobalTransform, With<DiegeticPanel>>,
) -> Vec3 {
    let owner = panels
        .get(target)
        .map(|_| target)
        .ok()
        .or_else(|| widgets.get(target).ok().map(WidgetOf::panel));
    let Some((source, global)) =
        owner.and_then(|owner| Some((panels.get(owner).ok()?, globals.get(owner).ok()?)))
    else {
        return Vec3::ONE;
    };
    let source_points_to_world = source.points_to_world();
    let tooltip_points_to_world = tooltip.points_to_world();
    let affine = global.affine();
    let scale = Vec3::new(
        source_points_to_world * affine.transform_vector3(Vec3::X).length()
            / tooltip_points_to_world,
        source_points_to_world * affine.transform_vector3(Vec3::Y).length()
            / tooltip_points_to_world,
        source_points_to_world * affine.transform_vector3(Vec3::X).length()
            / tooltip_points_to_world,
    );
    if scale.is_finite() && scale.min_element() > 0.0 {
        scale
    } else {
        Vec3::ONE
    }
}

pub(super) fn resolve_tooltip_body_look<L>(root: El<L, LayoutOnly>) -> El<L, LayoutOnly>
where
    L: ChildLayoutState,
{
    let palette = ControlPalette::system_dark();
    let frame = tooltip_frame(Px);
    root.with_tooltip_body_defaults(
        frame.0,
        Border::all(frame.1, palette.tooltip_border),
        palette.field_fill,
        frame.2,
    )
}

const TOOLTIP_DEFAULT_LABEL_PIXELS: f32 = 11.0 * 4.0 / 3.0;
const TOOLTIP_PADDING_PIXELS: f32 = 4.0;
const TOOLTIP_BORDER_PIXELS: f32 = 1.0;
const TOOLTIP_INK_GAP_CAP_RATIO: f32 = 0.4;
const TOOLTIP_INK_GAP_EDGE_PIXELS: f32 = 0.8;
const TOOLTIP_RADIUS_PIXELS: f32 = 6.0;

fn tooltip_frame<D: Copy + Into<Dimension>>(
    dimension: impl Fn(f32) -> D,
) -> (Padding, D, CornerRadius) {
    (
        Padding::all(dimension(TOOLTIP_PADDING_PIXELS)),
        dimension(TOOLTIP_BORDER_PIXELS),
        CornerRadius::all(dimension(TOOLTIP_RADIUS_PIXELS)),
    )
}

/// How a tooltip draws its frame, designed in screen pixels, in its
/// presentation: the unit its padding, border and radius take, and how much
/// of that unit one design pixel spans.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TooltipFrameScale {
    unit:            Unit,
    units_per_pixel: f32,
}

impl TooltipFrameScale {
    /// A world tooltip sized in millimetres scales its frame with its label.
    /// Screen tooltips, and world tooltips in points or with no unit, keep
    /// pixels.
    const fn for_presentation(
        presentation: TooltipPresentationSpace,
        drawn_label_size: Dimension,
    ) -> Self {
        match (presentation, drawn_label_size.unit) {
            (TooltipPresentationSpace::World, Some(Unit::Millimeters)) => Self {
                unit:            Unit::Millimeters,
                units_per_pixel: drawn_label_size.value / TOOLTIP_DEFAULT_LABEL_PIXELS,
            },
            _ => Self {
                unit:            Unit::Pixels,
                units_per_pixel: 1.0,
            },
        }
    }

    const fn dimension(self, pixels: f32) -> Dimension {
        Dimension {
            value: pixels * self.units_per_pixel,
            unit:  Some(self.unit),
        }
    }

    /// Design pixels one layout point spans.
    const fn pixels_per_point(self) -> f32 { 1.0 / (self.unit.to_points() * self.units_per_pixel) }
}

impl TooltipInkPadding {
    fn design_pixels(self, default: f32, font_unit: Unit, frame: TooltipFrameScale) -> f32 {
        match self {
            Self::WidgetFrame => default,
            Self::Uniform(gap) => {
                gap.value * gap.unit.unwrap_or(font_unit).to_points() * frame.pixels_per_point()
            },
        }
    }

    /// The standard frame preserves the solid raster edge of horizontal glyph bearings.
    fn horizontal_raster_allowance(
        self,
        presentation: TooltipPresentationSpace,
        raster_grid: TooltipFrameRasterGrid,
    ) -> f32 {
        match (self, presentation, raster_grid) {
            (Self::Uniform(_), _, _)
            | (_, TooltipPresentationSpace::Screen { .. }, TooltipFrameRasterGrid::Unprojected) => {
                0.0
            },
            (_, TooltipPresentationSpace::World, TooltipFrameRasterGrid::Unprojected) => {
                TOOLTIP_BORDER_PIXELS * TOOLTIP_INK_GAP_CAP_RATIO
            },
            (
                _,
                _,
                TooltipFrameRasterGrid::ViewAligned {
                    body_physical_per_design_pixel,
                    ..
                },
            ) => TOOLTIP_BORDER_PIXELS / body_physical_per_design_pixel.x,
        }
    }
}

/// Whether tooltip words can be measured, so their frame is padded from the
/// ink they draw. An app without the text plugin's font registry and
/// measurer cannot measure, and pads with the balanced line-box frame.
#[derive(Clone, Copy)]
enum TooltipTextMeasure<'a> {
    Ready {
        fonts:    &'a FontRegistry,
        measurer: &'a DiegeticTextMeasurer,
    },
    TextNotReady,
}

pub(super) fn resolve_tooltip_look(
    tree: &mut LayoutTree,
    unstyled_text: &[usize],
    label_size: Dimension,
) {
    let palette = ControlPalette::system_dark();
    let text_size = Dimension {
        value: label_size.value * 0.85,
        unit:  label_size.unit,
    };
    tree.apply_tooltip_look(
        unstyled_text,
        TextStyle::new(text_size).with_color(palette.knob),
    );
}

fn initial_tooltip_width_constraint(
    _tooltip: &Tooltip,
    context: &TooltipMaterializationContext,
    windows: &Query<&Window>,
) -> Option<TooltipWidthConstraint> {
    let TooltipPresentationSpace::Screen { window, .. } = context.presentation else {
        return None;
    };
    let max_width = placement::usable_viewport_axis(windows.get(window).ok()?.width())?;
    Some(TooltipWidthConstraint(max_width))
}

fn tooltip_materialization_context(
    target: Entity,
    authored_space: Option<&AuthoredTooltipTargetSpace>,
    panels: &Query<&DiegeticPanel>,
    widgets: &Query<&WidgetOf, With<PanelWidget>>,
    anchor_geometry: &Query<&ResolvedAnchorGeometry>,
    mesh_targets: &Query<(), With<MeshAnchorTarget>>,
    screen_targets: &Query<&ScreenAnchorTarget>,
    target_layers: &Query<&RenderLayers>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
    defaults: &crate::cascade::PanelDefaults,
) -> Option<TooltipMaterializationContext> {
    if let Ok(panel) = panels.get(target) {
        return panel_tooltip_context(
            target,
            panel,
            target_layers.get(target).ok(),
            windows,
            primary_window,
        );
    }
    if let Ok(widget_of) = widgets.get(target) {
        let panel = panels.get(widget_of.panel()).ok()?;
        return panel_tooltip_context(
            target,
            panel,
            target_layers.get(widget_of.panel()).ok(),
            windows,
            primary_window,
        );
    }
    let space = authored_space?.0;
    match space {
        PanelSpace::World => {
            if anchor_geometry.get(target).is_err() && mesh_targets.get(target).is_err() {
                return None;
            }
            Some(TooltipMaterializationContext {
                target,
                presentation: TooltipPresentationSpace::World,
                layout_unit: defaults.layout_unit,
                render_layers: target_layers
                    .get(target)
                    .cloned()
                    .unwrap_or_else(|_| RenderLayers::layer(0)),
            })
        },
        PanelSpace::Screen => {
            let screen_target = screen_targets.get(target).ok()?;
            windows.get(screen_target.window()).ok()?;
            Some(TooltipMaterializationContext {
                target,
                presentation: TooltipPresentationSpace::Screen {
                    window:       screen_target.window(),
                    camera_order: screen_target.camera_order(),
                },
                layout_unit: screen_target.layout_unit(),
                render_layers: screen_target.render_layers().clone(),
            })
        },
    }
}

fn panel_tooltip_context(
    target: Entity,
    panel: &DiegeticPanel,
    target_layers: Option<&RenderLayers>,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<TooltipMaterializationContext> {
    match panel.coordinate_space() {
        CoordinateSpace::World { .. } => Some(TooltipMaterializationContext {
            target,
            presentation: TooltipPresentationSpace::World,
            layout_unit: panel.layout_unit(),
            render_layers: target_layers
                .cloned()
                .unwrap_or_else(|| RenderLayers::layer(0)),
        }),
        CoordinateSpace::Screen {
            camera_order,
            render_layers,
            window,
            ..
        } => {
            let window = live_window(*window, windows, primary_window)?;
            Some(TooltipMaterializationContext {
                target,
                presentation: TooltipPresentationSpace::Screen {
                    window,
                    camera_order: *camera_order,
                },
                layout_unit: panel.layout_unit(),
                render_layers: render_layers.clone(),
            })
        },
    }
}

pub(super) fn live_window(
    window_ref: WindowRef,
    windows: &Query<&Window>,
    primary_window: &Query<Entity, With<PrimaryWindow>>,
) -> Option<Entity> {
    let entity = match window_ref {
        WindowRef::Primary => primary_window.single().ok()?,
        WindowRef::Entity(entity) => entity,
    };
    windows.get(entity).ok()?;
    Some(entity)
}

fn build_tooltip_panel(
    tooltip: &Tooltip,
    blueprint: &LayoutTree,
    context: &TooltipMaterializationContext,
    windows: &Query<&Window>,
    surface_material: &Handle<StandardMaterial>,
    panel_font_unit: Unit,
    text_measure: TooltipTextMeasure,
    raster_grid: TooltipFrameRasterGrid,
) -> Option<DiegeticPanel> {
    let mut tree = blueprint.clone();
    let (root_width, root_height) = tree.root_sizing();
    let world_font_unit = world_tooltip_font_unit(
        tooltip.font_unit,
        (root_width, root_height),
        context.layout_unit,
    );
    // A screen panel authors no font unit, so it draws in the panel default.
    let font_unit = match context.presentation {
        TooltipPresentationSpace::World => world_font_unit,
        TooltipPresentationSpace::Screen { .. } => panel_font_unit,
    };
    let frame_scale =
        TooltipFrameScale::for_presentation(context.presentation, tooltip.drawn_label_size);
    let (_, border_width, radius) = tooltip_frame(|pixels| frame_scale.dimension(pixels));
    let border = match raster_grid {
        TooltipFrameRasterGrid::Unprojected => {
            Border::all(border_width, ControlPalette::system_dark().tooltip_border)
        },
        TooltipFrameRasterGrid::ViewAligned {
            border_physical_per_design_pixel,
            ..
        } => Border {
            left:   frame_scale
                .dimension(TOOLTIP_BORDER_PIXELS / border_physical_per_design_pixel.x),
            right:  frame_scale
                .dimension(TOOLTIP_BORDER_PIXELS / border_physical_per_design_pixel.x),
            top:    frame_scale
                .dimension(TOOLTIP_BORDER_PIXELS / border_physical_per_design_pixel.y),
            bottom: frame_scale
                .dimension(TOOLTIP_BORDER_PIXELS / border_physical_per_design_pixel.y),
            color:  ControlPalette::system_dark().tooltip_border,
        },
    };
    let padding = measured_tooltip_padding(
        &tree,
        text_measure,
        font_unit,
        context.presentation,
        frame_scale,
        raster_grid,
        tooltip.ink_padding,
    );
    tree.apply_tooltip_frame(padding, border, radius);
    let (body_width, body_height) = rounded_world_root_sizing(
        &mut tree,
        (root_width, root_height),
        context.presentation,
        context.layout_unit,
        frame_scale,
        raster_grid,
    );
    let panel_axis = |root, fallback| match root {
        Sizing::Fixed(_) => root,
        _ => fallback,
    };
    // Fixed world tooltips author their text beside their dimensions in the
    // layout unit. Fit tooltips retain their established point-sized text.
    match context.presentation {
        TooltipPresentationSpace::World => DiegeticPanel::world()
            .material(surface_material.clone())
            .size(
                panel_axis(body_width, fit_sizing(context.layout_unit, f32::MAX)),
                panel_axis(body_height, fit_sizing(context.layout_unit, f32::MAX)),
            )
            .font_unit(world_font_unit)
            .anchor(tooltip.source_anchor)
            .picking(PanelPicking::PASS_THROUGH)
            .with_tree(tree)
            .build()
            .map_err(|error| {
                error!(target: DIAGNOSTIC_LOG_TARGET, "failed to materialize world tooltip panel: {error}");
            })
            .ok(),
        TooltipPresentationSpace::Screen {
            window,
            camera_order,
        } => {
            let max_width = initial_tooltip_width_constraint(tooltip, context, windows)
                .map_or(f32::MAX, |constraint| constraint.0);
            DiegeticPanel::screen()
                .size(
                    panel_axis(root_width, fit_sizing(Unit::Pixels, max_width)),
                    panel_axis(root_height, fit_sizing(Unit::Pixels, f32::MAX)),
                )
                .anchor(tooltip.source_anchor)
                .window_entity(window)
                .camera_order(camera_order)
                .render_layers(context.render_layers.clone())
                .picking(PanelPicking::PASS_THROUGH)
                .with_tree(tree)
                .build()
                .map_err(|error| {
                    error!(target: DIAGNOSTIC_LOG_TARGET, "failed to materialize screen tooltip panel: {error}");
                })
                .ok()
        },
    }
}

const fn world_tooltip_font_unit(
    requested: TooltipFontUnit,
    root_sizing: (Sizing, Sizing),
    layout_unit: Unit,
) -> Unit {
    match requested {
        TooltipFontUnit::Explicit(unit) => unit,
        TooltipFontUnit::FixedLayoutOrFitPoints
            if matches!(root_sizing, (Sizing::Fixed(_), Sizing::Fixed(_))) =>
        {
            layout_unit
        },
        TooltipFontUnit::FixedLayoutOrFitPoints => Unit::Points,
    }
}

fn rounded_world_root_sizing(
    tree: &mut LayoutTree,
    (root_width, root_height): (Sizing, Sizing),
    presentation: TooltipPresentationSpace,
    panel_unit: Unit,
    frame_scale: TooltipFrameScale,
    raster_grid: TooltipFrameRasterGrid,
) -> (Sizing, Sizing) {
    let (body_width, body_height) = match (presentation, raster_grid) {
        (
            TooltipPresentationSpace::World,
            TooltipFrameRasterGrid::ViewAligned {
                body_physical_per_design_pixel,
                ..
            },
        ) => (
            round_fixed_tooltip_axis_up(
                root_width,
                body_physical_per_design_pixel.x,
                frame_scale,
                panel_unit,
            ),
            round_fixed_tooltip_axis_up(
                root_height,
                body_physical_per_design_pixel.y,
                frame_scale,
                panel_unit,
            ),
        ),
        _ => (root_width, root_height),
    };
    if let Sizing::Fixed(width) = body_width
        && body_width != root_width
    {
        layout::set_root_fit_width(tree, width, width);
    }
    if let Sizing::Fixed(height) = body_height
        && body_height != root_height
    {
        layout::set_root_fit_height(tree, height, height);
    }
    (body_width, body_height)
}

fn round_fixed_tooltip_axis_up(
    sizing: Sizing,
    physical_per_design_pixel: f32,
    frame_scale: TooltipFrameScale,
    panel_unit: Unit,
) -> Sizing {
    let Sizing::Fixed(authored) = sizing else {
        return sizing;
    };
    let points_per_authored_unit = authored
        .unit
        .map_or_else(|| panel_unit.to_points(), Unit::to_points);
    let physical_per_authored_unit =
        points_per_authored_unit * frame_scale.pixels_per_point() * physical_per_design_pixel;
    if !physical_per_authored_unit.is_finite() || physical_per_authored_unit <= 0.0 {
        return sizing;
    }
    let rounded = (authored.value * physical_per_authored_unit).ceil() / physical_per_authored_unit;
    Sizing::Fixed(Dimension {
        value: rounded.max(authored.value),
        unit:  authored.unit,
    })
}

/// Frame a short help run from its visible caps and ink columns, rather than
/// from the font's line box and advance width. Words that cannot be measured,
/// or draw no ink, get the balanced line-box padding.
fn measured_tooltip_padding(
    tree: &LayoutTree,
    text_measure: TooltipTextMeasure,
    font_unit: Unit,
    presentation: TooltipPresentationSpace,
    frame_scale: TooltipFrameScale,
    raster_grid: TooltipFrameRasterGrid,
    ink_padding: TooltipInkPadding,
) -> Padding {
    let balanced = Padding::xy(
        frame_scale.dimension(ink_padding.design_pixels(
            TOOLTIP_PADDING_PIXELS,
            font_unit,
            frame_scale,
        )),
        frame_scale.dimension(ink_padding.design_pixels(
            crate::constants::TOOLTIP_WORLD_VERTICAL_PADDING_PIXELS,
            font_unit,
            frame_scale,
        )),
    );
    let TooltipTextMeasure::Ready { fonts, measurer } = text_measure else {
        return balanced;
    };
    let ink_run = (0..tree.len()).find_map(|index| {
        let (Some(text), Some(style)) = (tree.element_text(index), tree.element_style(index))
        else {
            return None;
        };
        // The panel scales authored text to points before both layout and
        // shaping. Measuring the tiny world-unit size directly makes parley
        // quantize the run baseline to zero.
        let scale = style.unit().map_or(font_unit.to_points(), Unit::to_points);
        let measure = style.as_measure().scaled(scale);
        let TextInkMeasurement::Visible(ink) = measure_drawn_ink(fonts, text, &measure) else {
            return None;
        };
        let line = (measurer.measure_fn)(text, &measure);
        Some((ink, line, measure))
    });
    let Some((ink, line, measure)) = ink_run else {
        return balanced;
    };
    let pixels_per_point = frame_scale.pixels_per_point();
    let left_slack = ink.left.max(0.0) * pixels_per_point;
    let right_slack = (line.width - ink.right).max(0.0) * pixels_per_point;
    let cap = match measure_drawn_ink(fonts, "H", &measure) {
        TextInkMeasurement::Visible(cap) => cap,
        TextInkMeasurement::NoVisibleInk => ink,
    };
    let drawn_cap_height = (cap.bottom - cap.top) * pixels_per_point;
    // Solid raster bounds omit the antialiased edge of both the glyph and
    // box. The fixed fraction restores that lost edge at every cap size.
    let ink_gap = ink_padding.design_pixels(
        drawn_cap_height.mul_add(TOOLTIP_INK_GAP_CAP_RATIO, TOOLTIP_INK_GAP_EDGE_PIXELS),
        font_unit,
        frame_scale,
    );
    // Line-box slack is part of the visible gap, so padding may be negative:
    // it trims that slack without clipping the measured ink. This matters at
    // the 9 px help floor, where an untrimmed bearing is most of a cap high.
    let horizontal_gap = ink_gap;
    let top_slack = ink.top.max(0.0) * pixels_per_point;
    let bottom_slack = (line.height - ink.bottom).max(0.0) * pixels_per_point;
    let vertical_gap = ink_gap;
    let horizontal = horizontal_gap.mul_add(2.0, -(left_slack + right_slack));
    let vertical = vertical_gap.mul_add(2.0, -(top_slack + bottom_slack));
    let border_design_width = match raster_grid {
        TooltipFrameRasterGrid::Unprojected => Vec2::splat(TOOLTIP_BORDER_PIXELS),
        TooltipFrameRasterGrid::ViewAligned {
            border_physical_per_design_pixel,
            ..
        } => Vec2::splat(TOOLTIP_BORDER_PIXELS) / border_physical_per_design_pixel,
    };
    let body = Vec2::new(line.width, line.height) * pixels_per_point
        + Vec2::new(horizontal, vertical)
        + 2.0 * border_design_width;
    // A screen tooltip's body spans whole pixels, so with its top-left corner
    // placed on the grid its far frame edges land there too.
    let mut whole_pixel_growth = match (presentation, raster_grid) {
        (TooltipPresentationSpace::Screen { .. }, _) => 0.5 * (body.round() - body),
        (
            TooltipPresentationSpace::World,
            TooltipFrameRasterGrid::ViewAligned {
                body_physical_per_design_pixel,
                ..
            },
        ) => {
            0.5 * ((body * body_physical_per_design_pixel).round() / body_physical_per_design_pixel
                - body)
        },
        (TooltipPresentationSpace::World, TooltipFrameRasterGrid::Unprojected) => Vec2::ZERO,
    };
    if matches!(tree.root_sizing().0, Sizing::Fixed(_)) {
        whole_pixel_growth.x = 0.0;
    }
    if matches!(tree.root_sizing().1, Sizing::Fixed(_)) {
        whole_pixel_growth.y = 0.0;
    }
    let horizontal_raster_allowance =
        ink_padding.horizontal_raster_allowance(presentation, raster_grid);
    Padding::new(
        frame_scale.dimension(
            horizontal_gap - left_slack + whole_pixel_growth.x + horizontal_raster_allowance,
        ),
        frame_scale.dimension(
            horizontal_gap - right_slack + whole_pixel_growth.x + horizontal_raster_allowance,
        ),
        frame_scale.dimension(vertical_gap - top_slack + whole_pixel_growth.y),
        frame_scale.dimension(vertical_gap - bottom_slack + whole_pixel_growth.y),
    )
}

/// Measures the ink `text` draws as the renderer draws it: in the default
/// family while the style's font is still loading.
fn measure_drawn_ink(
    fonts: &FontRegistry,
    text: &str,
    measure: &TextMeasure,
) -> TextInkMeasurement {
    fonts
        .measure_ink(text, measure)
        .or_else(|_| {
            let default_family = TextMeasure {
                font_id: FontId::MONOSPACE.0,
                ..*measure
            };
            fonts.measure_ink(text, &default_family)
        })
        .unwrap_or(TextInkMeasurement::NoVisibleInk)
}

const fn fit_sizing(unit: Unit, max: f32) -> Sizing {
    Sizing::Fit {
        min: Dimension {
            value: 0.0,
            unit:  Some(unit),
        },
        max: Dimension {
            value: max,
            unit:  Some(unit),
        },
    }
}

pub(super) const fn zero_origin_viewport(size: Vec2) -> Rect {
    Rect::from_corners(Vec2::ZERO, size)
}

/// Keep fitted layout while its projection moves. Refresh a waiting or visible
/// frame once two consecutive observations agree on a different raster grid.
pub(super) fn invalidate_stale_world_tooltip_frames(
    mut controllers: Query<(
        Entity,
        &Tooltip,
        &TooltipPhase,
        &DiegeticPanel,
        &MaterializedTooltip,
        &TooltipPresentationCamera,
        &mut WorldTooltipFrameProjection,
        Option<&TooltipWidthConstraint>,
    )>,
    projection_entities: Query<(&GlobalTransform, Option<&Camera>)>,
    anchor_geometry: Query<&ResolvedAnchorGeometry>,
    target_panels: Query<&DiegeticPanel>,
    transforms: TransformHelper,
    windows: Query<&Window>,
    defaults: Res<crate::cascade::PanelDefaults>,
    surface_material: Res<TooltipSurfaceMaterial>,
    text: Option<(Res<FontRegistry>, Res<DiegeticTextMeasurer>)>,
    mut commands: Commands,
) {
    let text_measure = text
        .as_ref()
        .map_or(TooltipTextMeasure::TextNotReady, |(fonts, measurer)| {
            TooltipTextMeasure::Ready { fonts, measurer }
        });
    for (entity, tooltip, phase, panel, materialized, camera, mut last, width_constraint) in
        &mut controllers
    {
        if materialized.space() != PanelSpace::World
            || !matches!(
                phase,
                TooltipPhase::WaitingToShow(_) | TooltipPhase::Visible
            )
        {
            continue;
        }
        // Once a settled tooltip is shown its placement and frame stay frozen.
        // A reveal during motion still owes the calibration begun while waiting.
        if phase.is_visible() && matches!(last.calibration, WorldTooltipRasterCalibration::Current)
        {
            continue;
        }
        let Ok(current_camera_pose) = transforms.compute_global_transform(camera.camera) else {
            continue;
        };
        let grid = projected_world_tooltip_frame_grid(
            tooltip,
            panel,
            last.world_scale,
            tooltip_front_depth_meters(tooltip, panel, last.world_scale),
            materialized.target,
            Some(camera),
            &projection_entities,
            &anchor_geometry,
            &target_panels,
            TooltipProjectionCameraPose::Current(current_camera_pose),
        );
        if !last.needs_rebuild(grid) {
            continue;
        }
        let context = TooltipMaterializationContext {
            target:        materialized.target,
            presentation:  materialized.presentation,
            layout_unit:   materialized.layout_unit,
            render_layers: materialized.render_layers.clone(),
        };
        let Some(mut rebuilt) = build_tooltip_panel(
            tooltip,
            tooltip.blueprint(),
            &context,
            &windows,
            &surface_material.0,
            defaults.panel_font_unit,
            text_measure,
            grid,
        ) else {
            continue;
        };
        if let Some(constraint) = width_constraint {
            panel::constrain_fit_width(&mut rebuilt, constraint.0);
        }
        commands.run_system_cached_with(panel::apply_precompose_helper_panel, (entity, rebuilt));
        if phase.is_visible() {
            commands
                .entity(entity)
                .insert(WorldTooltipRasterFitAwaitingPlacement);
        }
        commands.entity(entity).insert((
            WorldTooltipFrameProjection {
                grid,
                observed_grid: grid,
                world_scale: last.world_scale,
                calibration: WorldTooltipRasterCalibration::Current,
            },
            match grid {
                TooltipFrameRasterGrid::ViewAligned { .. } => {
                    TooltipPaintInsideBorder::ViewAlignedWorldPixelGrid
                },
                TooltipFrameRasterGrid::Unprojected => {
                    TooltipPaintInsideBorder::AntialiasedWorldPlane
                },
            },
        ));
    }
}

pub(super) fn invalidate_stale_materialized_tooltips(
    mut controllers: Query<
        (
            Entity,
            &TooltipFor,
            &MaterializedTooltip,
            &mut TooltipReadiness,
            &mut TooltipPlacementState,
            &mut Visibility,
        ),
        Changed<TooltipFor>,
    >,
    mut commands: Commands,
) {
    for (controller, tooltip_for, materialized, mut readiness, mut placement, mut visibility) in
        &mut controllers
    {
        if tooltip_for.target() == materialized.target {
            continue;
        }
        *readiness = TooltipReadiness::Pending;
        placement::set_tooltip_placement_result(
            &mut placement,
            TooltipPlacementResult::Unavailable,
        );
        *visibility = Visibility::Hidden;
        commands.entity(controller).remove::<(
            PanelAttachmentAuthored,
            PanelAnchorOffset,
            TooltipWidthConstraintRequest,
        )>();
    }
}

pub(super) fn apply_tooltip_width_constraints(
    mut controllers: Query<
        (
            Entity,
            &mut DiegeticPanel,
            &TooltipWidthConstraintRequest,
            Option<&TooltipWidthConstraint>,
            &mut TooltipReadiness,
        ),
        With<MaterializedTooltip>,
    >,
    mut commands: Commands,
) {
    for (entity, mut panel, request, current, mut readiness) in &mut controllers {
        match *request {
            TooltipWidthConstraintRequest::Apply(max_width)
                if current.is_some_and(|current| (current.0 - max_width).abs() <= f32::EPSILON) =>
            {
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>();
            },
            TooltipWidthConstraintRequest::Apply(max_width) => {
                panel::constrain_fit_width(&mut panel, max_width);
                *readiness = TooltipReadiness::Pending;
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>()
                    .insert(TooltipWidthConstraint(max_width));
            },
            TooltipWidthConstraintRequest::Restore if current.is_none() => {
                commands
                    .entity(entity)
                    .remove::<TooltipWidthConstraintRequest>();
            },
            TooltipWidthConstraintRequest::Restore => {
                panel::constrain_fit_width(&mut panel, f32::MAX);
                *readiness = TooltipReadiness::Pending;
                commands
                    .entity(entity)
                    .remove::<(TooltipWidthConstraintRequest, TooltipWidthConstraint)>();
            },
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTarget;
    use bevy::camera::RenderTargetInfo;
    use bevy::camera::primitives::Aabb;
    use bevy::camera::visibility::RenderLayers;
    use bevy::color::Alpha;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::mesh::Mesh3d;
    use bevy::prelude::App;
    use bevy::prelude::Camera;
    use bevy::prelude::Entity;
    use bevy::prelude::GlobalTransform;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::Transform;
    use bevy::prelude::Vec2;
    use bevy::prelude::Vec3A;
    use bevy::prelude::Visibility;
    use bevy::prelude::Window;
    use bevy::transform::TransformPlugin;
    use bevy::window::PrimaryWindow;
    use bevy::window::WindowRef;
    use hana_valence::AnchoredHere;
    use hana_valence::AnchoredTo;
    use hana_valence::ResolvedAnchorGeometry;

    use super::*;
    use crate::HeadlessLayoutPlugin;
    use crate::LayoutBuilder;
    use crate::Mm;
    use crate::PanelDefaults;
    use crate::PanelPicking;
    use crate::Px;
    use crate::layout::El;
    use crate::layout::LayoutTreeChange;
    use crate::layout::Pt;
    use crate::layout::RectangleSource;
    use crate::layout::RenderCommandKind;
    use crate::layout::Unit;
    use crate::panel::ComputedDiegeticPanel;
    use crate::panel::DiegeticPanel;
    use crate::panel::PanelScreenBounds;
    use crate::panel::PanelSpace;
    use crate::panel::World;
    use crate::screen_space::ScreenSpacePlugin;
    use crate::text::DiegeticTextMeasurer;
    use crate::widgets::WidgetsPlugin;
    use crate::widgets::tooltip::*;

    const MATERIALIZATION_UPDATES: usize = 5;

    const SMALL_SCREEN_VIEWPORT: Vec2 = Vec2::new(240.0, 180.0);

    const TEST_CAMERA_DISTANCE: f32 = 5.0;

    const TEST_VIEWPORT: Vec2 = Vec2::new(800.0, 600.0);

    fn built_tooltip_panel(
        app: &mut App,
        tooltip: Tooltip,
        presentation: TooltipPresentationSpace,
        window: Entity,
    ) -> DiegeticPanel {
        let blueprint = (**tooltip.blueprint()).clone();
        let context = TooltipMaterializationContext {
            target: window,
            presentation,
            layout_unit: if matches!(presentation, TooltipPresentationSpace::World) {
                Unit::Millimeters
            } else {
                Unit::Pixels
            },
            render_layers: RenderLayers::default(),
        };
        app.world_mut()
            .run_system_once(move |windows: Query<&Window>| {
                build_tooltip_panel(
                    &tooltip,
                    &blueprint,
                    &context,
                    &windows,
                    &Handle::default(),
                    PanelDefaults::default().panel_font_unit,
                    TooltipTextMeasure::TextNotReady,
                    TooltipFrameRasterGrid::Unprojected,
                )
            })
            .expect("tooltip build system")
            .expect("materialized tooltip panel")
    }

    #[test]
    fn world_tooltip_lines_use_a_readable_share_of_their_fixed_meter_panel() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let mut tooltip = Tooltip::new(El::column().size(0.22, 0.06));
        tooltip.text(("No camera chosen", TextStyle::new(0.015)));
        tooltip.text(("Choose a camera", TextStyle::new(0.015)));
        let blueprint = (**tooltip.blueprint()).clone();
        let context = TooltipMaterializationContext {
            target:        window,
            presentation:  TooltipPresentationSpace::World,
            layout_unit:   Unit::Meters,
            render_layers: RenderLayers::default(),
        };
        let panel = app
            .world_mut()
            .run_system_once(move |windows: Query<&Window>| {
                build_tooltip_panel(
                    &tooltip,
                    &blueprint,
                    &context,
                    &windows,
                    &Handle::default(),
                    PanelDefaults::default().panel_font_unit,
                    TooltipTextMeasure::TextNotReady,
                    TooltipFrameRasterGrid::Unprojected,
                )
            })
            .expect("tooltip build system")
            .expect("world tooltip panel");
        let controller = app.world_mut().spawn(panel).id();
        app.update();
        let computed = app
            .world()
            .get::<ComputedDiegeticPanel>(controller)
            .expect("tooltip layout");
        assert!((computed.content_width() - 0.22).abs() < 0.0001);
        assert!((computed.content_height() - 0.06).abs() < 0.0001);
        let panel = app
            .world()
            .get::<DiegeticPanel>(controller)
            .expect("materialized world tooltip");
        let line_heights = computed
            .layout()
            .solved()
            .expect("solved tooltip")
            .commands
            .iter()
            .filter_map(|command| match &command.kind {
                RenderCommandKind::Text { .. } => {
                    Some(command.bounds.height * panel.points_to_world())
                },
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(line_heights.len(), 2);
        for height in line_heights {
            assert!(height >= 0.0055, "tooltip line is only {height} m high");
            assert!(height >= 0.06 * 0.1, "tooltip line is only {height} m high");
            assert!(height <= 0.06, "tooltip line exceeds its panel: {height} m");
        }
    }

    #[test]
    fn world_tooltip_frame_matches_screen_label_proportions() {
        let mut screen = Tooltip::new(El::column()).with_label_size(Pt(11.0));
        screen.text("Screen tooltip");
        let mut world = Tooltip::new(El::column()).with_label_size(Mm(38.0));
        world.text("World tooltip");
        let mut screen_metric = Tooltip::new(El::column()).with_label_size(Mm(38.0));
        screen_metric.text("Metric screen tooltip");
        let mut authored = Tooltip::new(
            El::column()
                .padding(Padding::all(Mm(2.0)))
                .corner_radius(CornerRadius::all(Mm(3.0))),
        )
        .with_label_size(Mm(38.0));
        authored.text("Authored world tooltip");

        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins(HeadlessLayoutPlugin);
        let window = app.world_mut().spawn(Window::default()).id();
        let screen = built_tooltip_panel(
            &mut app,
            screen,
            TooltipPresentationSpace::Screen {
                window,
                camera_order: 0,
            },
            window,
        );
        let world = built_tooltip_panel(&mut app, world, TooltipPresentationSpace::World, window);
        let screen_metric = built_tooltip_panel(
            &mut app,
            screen_metric,
            TooltipPresentationSpace::Screen {
                window,
                camera_order: 0,
            },
            window,
        );
        let authored =
            built_tooltip_panel(&mut app, authored, TooltipPresentationSpace::World, window);
        let screen_radius = screen.tree().element_corner_radius(0).top_left.value;
        let world_radius = world.tree().element_corner_radius(0).top_left.value;
        let screen_metric_radius = screen_metric.tree().element_corner_radius(0).top_left.value;
        let authored_radius = authored.tree().element_corner_radius(0).top_left.value;
        let screen_panel = app.world_mut().spawn(screen).id();
        let world_panel = app.world_mut().spawn(world).id();
        let authored_panel = app.world_mut().spawn(authored).id();
        app.update();
        let solved_frame = |entity: Entity| {
            let panel = app
                .world()
                .get::<ComputedDiegeticPanel>(entity)
                .expect("tooltip layout");
            let solved = panel.layout().solved().expect("solved tooltip");
            let root = solved.computed[0].bounds;
            let text = solved.computed[1].bounds;
            let text_size = solved
                .commands
                .iter()
                .find_map(|command| {
                    if let RenderCommandKind::Text { config, .. } = &command.kind {
                        Some(config.size())
                    } else {
                        None
                    }
                })
                .expect("tooltip text command");
            let border_width = solved
                .commands
                .iter()
                .find_map(|command| {
                    if let RenderCommandKind::Border { border } = &command.kind {
                        Some(border.left.value)
                    } else {
                        None
                    }
                })
                .expect("tooltip border command");
            (text.x - root.x, border_width, text_size)
        };
        let (screen_padding, screen_border, screen_text_size) = solved_frame(screen_panel);
        let (world_padding, world_border, world_text_size) = solved_frame(world_panel);
        let (authored_padding, authored_border, authored_text_size) = solved_frame(authored_panel);
        let screen_label_pixels = 11.0 * 4.0 / 3.0;
        assert!(
            (screen_padding / screen_text_size - world_padding / world_text_size).abs() < 0.01,
            "screen {screen_padding}/{screen_text_size}, world {world_padding}/{world_text_size}"
        );
        assert!((screen_radius / screen_label_pixels - world_radius / 38.0).abs() < 0.01);
        assert!((screen_border / screen_text_size - world_border / world_text_size).abs() < 0.01);
        assert!(
            (((authored_padding - authored_border) / authored_text_size) - (2.0 / (38.0 * 0.85)))
                .abs()
                < 0.01
        );
        assert!((authored_radius - 3.0).abs() < 0.001);
        assert!((screen_metric_radius - 6.0).abs() < 0.001);
    }

    #[test]
    fn world_tooltip_frame_scales_the_screen_defaults() {
        let (screen_padding, screen_border, screen_radius) = tooltip_frame(Px);
        for world_label in [Mm(22.0), Mm(38.0)] {
            let scale = world_label.0 / TOOLTIP_DEFAULT_LABEL_PIXELS;
            let (world_padding, world_border, world_radius) =
                tooltip_frame(|pixels| Mm(pixels * scale));
            assert!(
                screen_padding
                    .top
                    .value
                    .mul_add(-scale, world_padding.top.value)
                    .abs()
                    < 0.001
            );
            assert!(screen_border.0.mul_add(-scale, world_border.0).abs() < 0.001);
            assert!(
                screen_radius
                    .top_left
                    .value
                    .mul_add(-scale, world_radius.top_left.value)
                    .abs()
                    < 0.001
            );
        }
    }

    #[test]
    fn unmeasurable_text_pads_balanced_in_the_tooltips_unit() {
        let mut tooltip = Tooltip::new(El::column());
        tooltip.text("Attach");
        let screen = TooltipPresentationSpace::Screen {
            window:       Entity::PLACEHOLDER,
            camera_order: 0,
        };
        let millimetres_per_pixel = 38.0 / TOOLTIP_DEFAULT_LABEL_PIXELS;
        let cases: [(TooltipPresentationSpace, Dimension, Padding); 3] = [
            (screen, Pt(11.0).into(), Padding::xy(Px(4.0), Px(2.0))),
            (
                TooltipPresentationSpace::World,
                Pt(11.0).into(),
                Padding::xy(Px(4.0), Px(2.0)),
            ),
            (
                TooltipPresentationSpace::World,
                Mm(38.0).into(),
                Padding::xy(
                    Mm(4.0 * millimetres_per_pixel),
                    Mm(2.0 * millimetres_per_pixel),
                ),
            ),
        ];
        for (presentation, label_size, balanced) in cases {
            let frame_scale = TooltipFrameScale::for_presentation(presentation, label_size);
            assert_eq!(
                measured_tooltip_padding(
                    tooltip.blueprint(),
                    TooltipTextMeasure::TextNotReady,
                    Unit::Points,
                    presentation,
                    frame_scale,
                    TooltipFrameRasterGrid::Unprojected,
                    TooltipInkPadding::WidgetFrame,
                ),
                balanced,
                "{presentation:?} with a {label_size:?} label"
            );
        }
    }

    #[test]
    fn presentation_space_maps_world_and_screen_to_panel_space() {
        let window = Entity::from_bits(1);
        assert_eq!(TooltipPresentationSpace::World.space(), PanelSpace::World);
        assert_eq!(
            TooltipPresentationSpace::Screen {
                window,
                camera_order: 7
            }
            .space(),
            PanelSpace::Screen
        );
    }

    #[test]
    fn pre_materialization_component_records_query_presence() {
        let transform = Transform::from_xyz(4.0, -2.0, 1.5);
        assert_eq!(
            PreMaterializationComponent::<Transform>::from_query_item(None),
            PreMaterializationComponent::Absent
        );
        assert_eq!(
            PreMaterializationComponent::from_query_item(Some(&transform)),
            PreMaterializationComponent::Present(transform)
        );
    }

    #[test]
    fn panel_context_builder_preserves_world_and_screen_presentation() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let world_panel = DiegeticPanel::world()
            .size(Mm(20.0), Mm(10.0))
            .with_tree(LayoutBuilder::new(20.0, 10.0).build())
            .build();
        let screen_panel = DiegeticPanel::screen()
            .size(Px(20.0), Px(10.0))
            .window_entity(window)
            .camera_order(37)
            .with_tree(LayoutBuilder::new(20.0, 10.0).build())
            .build();
        assert!(world_panel.is_ok());
        assert!(screen_panel.is_ok());
        let (Ok(world_panel), Ok(screen_panel)) = (world_panel, screen_panel) else {
            return;
        };
        let world_target = app.world_mut().spawn(world_panel).id();
        let screen_target = app.world_mut().spawn(screen_panel).id();
        let contexts = app.world_mut().run_system_once(
            move |panels: Query<&DiegeticPanel>,
                  windows: Query<&Window>,
                  primary_window: Query<Entity, With<PrimaryWindow>>| {
                [world_target, screen_target].map(|target| {
                    panels.get(target).ok().and_then(|panel| {
                        panel_tooltip_context(target, panel, None, &windows, &primary_window)
                    })
                })
            },
        );
        assert!(matches!(contexts, Ok([Some(_), Some(_)])));
        let Ok([Some(world), Some(screen)]) = contexts else {
            return;
        };
        assert_eq!(world.presentation, TooltipPresentationSpace::World);
        assert_eq!(
            screen.presentation,
            TooltipPresentationSpace::Screen {
                window,
                camera_order: 37
            }
        );
    }

    #[test]
    fn target_context_builder_preserves_world_and_screen_presentation() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        let world_target = mesh_target(&mut app, Vec3A::ONE);
        let screen_target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::splat(20.0), Vec2::splat(10.0));
        assert!(bounds.is_ok());
        let Ok(bounds) = bounds else {
            return;
        };
        app.world_mut().commands().screen_anchor_target(
            screen_target,
            ScreenAnchorTarget::new(bounds, window, 37, RenderLayers::layer(3), Unit::Pixels),
        );
        app.update();
        let contexts = app.world_mut().run_system_once(
            move |panels: Query<&DiegeticPanel>,
                  widgets: Query<&WidgetOf, With<PanelWidget>>,
                  anchor_geometry: Query<&ResolvedAnchorGeometry>,
                  mesh_targets: Query<(), With<MeshAnchorTarget>>,
                  screen_targets: Query<&ScreenAnchorTarget>,
                  target_layers: Query<&RenderLayers>,
                  windows: Query<&Window>,
                  primary_window: Query<Entity, With<PrimaryWindow>>,
                  defaults: Res<crate::cascade::PanelDefaults>| {
                [
                    (world_target, PanelSpace::World),
                    (screen_target, PanelSpace::Screen),
                ]
                .map(|(target, space)| {
                    let authored_space = AuthoredTooltipTargetSpace(space);
                    tooltip_materialization_context(
                        target,
                        Some(&authored_space),
                        &panels,
                        &widgets,
                        &anchor_geometry,
                        &mesh_targets,
                        &screen_targets,
                        &target_layers,
                        &windows,
                        &primary_window,
                        &defaults,
                    )
                })
            },
        );
        assert!(matches!(contexts, Ok([Some(_), Some(_)])));
        let Ok([Some(world), Some(screen)]) = contexts else {
            return;
        };
        assert_eq!(world.presentation, TooltipPresentationSpace::World);
        assert_eq!(
            screen.presentation,
            TooltipPresentationSpace::Screen {
                window,
                camera_order: 37
            }
        );
    }

    fn materialization_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, TransformPlugin))
            .insert_resource(DiegeticTextMeasurer::default())
            .add_plugins((HeadlessLayoutPlugin, WidgetsPlugin, ScreenSpacePlugin));
        app
    }

    #[test]
    fn materialized_tooltip_has_an_opaque_shadow_casting_surface() {
        let mut app = materialization_app();
        app.add_plugins((
            crate::cascade::cascade_plugin::<crate::cascade::SdfMaterial>(),
            crate::cascade::cascade_plugin::<Sidedness>(),
        ));
        let target = mesh_target(&mut app, Vec3A::ONE);
        let tooltip = Tooltip::new(El::column().background(Color::srgba(0.1, 0.2, 0.3, 0.4)));
        let controller = app.world_mut().commands().spawn_tooltip(
            TooltipTargetEntity::<World>::from_validated(target),
            tooltip,
        );
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let computed = app.world().get::<ComputedDiegeticPanel>(controller);
        assert!(computed.is_some());
        let fill_alpha = computed
            .and_then(|computed| computed.layout().solved())
            .and_then(|solved| {
                solved.commands.iter().find_map(|command| {
                    if let RenderCommandKind::Rectangle {
                        color,
                        source: RectangleSource::Background,
                    } = command.kind
                    {
                        Some(color.alpha())
                    } else {
                        None
                    }
                })
            });
        assert_eq!(fill_alpha, Some(1.0));

        let material_handle = crate::cascade::resolved_sdf_material(app.world(), controller);
        assert_eq!(
            material_handle,
            app.world().resource::<TooltipSurfaceMaterial>().0,
        );
        let material = app
            .world()
            .resource::<Assets<StandardMaterial>>()
            .get(&material_handle);
        assert!(material.is_some());
        assert_eq!(
            material.map(|material| material.alpha_mode),
            Some(AlphaMode::Opaque)
        );
        assert_eq!(
            crate::cascade::resolved_shadow_casting(app.world(), controller),
            crate::ShadowCasting::On,
        );
        assert_eq!(
            crate::cascade::resolved_sidedness(app.world(), controller),
            Sidedness::BothSides,
        );
    }

    fn update_materialization(app: &mut App) {
        for _ in 0..MATERIALIZATION_UPDATES {
            app.update();
        }
    }

    fn mesh_target(app: &mut App, half_extents: Vec3A) -> Entity {
        let target = app
            .world_mut()
            .spawn((
                Mesh3d::default(),
                Aabb {
                    center: Vec3A::ZERO,
                    half_extents,
                },
                Transform::default(),
                GlobalTransform::default(),
            ))
            .id();
        app.world_mut()
            .commands()
            .mesh_anchor_target(target, MeshFace::PositiveZ);
        target
    }

    fn standalone_world_tooltip(app: &mut App, target: Entity, tooltip: Tooltip) -> Entity {
        let handle = TooltipTargetEntity::<World>::from_validated(target);
        app.world_mut().commands().spawn_tooltip(handle, tooltip)
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

    fn assert_unmaterialized_tooltip(app: &App, target: Entity, controller: Entity) {
        assert!(app.world().get::<Tooltip>(controller).is_some());
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(app.world().get::<MaterializedTooltip>(controller).is_none());
        assert!(app.world().get::<AnchoredTo>(controller).is_none());
        assert!(app.world().get::<AnchoredHere>(target).is_none());
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_none());
    }

    fn assert_materialized_world_tooltip(
        app: &App,
        target: Entity,
        controller: Entity,
    ) -> Option<u64> {
        let materialized = app.world().get::<MaterializedTooltip>(controller);
        assert!(materialized.is_some());
        if let Some(materialized) = materialized {
            assert_eq!(materialized.target(), target);
            assert_eq!(materialized.space(), PanelSpace::World);
            assert_eq!(materialized.layout_unit(), Unit::Millimeters);
        }
        let panel = app.world().get::<DiegeticPanel>(controller);
        assert!(panel.is_some());
        if let Some(panel) = panel {
            assert_eq!(panel.layout_unit(), Unit::Millimeters);
        }
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
        assert_eq!(
            app.world().get::<PanelPicking>(controller),
            Some(&PanelPicking::PASS_THROUGH)
        );
        assert!(
            app.world()
                .get::<TooltipPaintInsideBorder>(controller)
                .is_some()
        );
        assert_eq!(
            app.world()
                .get::<AnchoredTo>(controller)
                .map(AnchoredTo::target),
            Some(target)
        );
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(target)
                .map(AnchoredHere::len),
            Some(1)
        );
        assert!(app.world().get::<ResolvedAnchorGeometry>(target).is_some());
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );
        panel.map(|panel| u64::from(panel.tree_revision()))
    }

    #[test]
    fn plain_text_look_survives_width_reflow_while_authored_style_stays_whole() {
        let authored = TextStyle::new(Px(23.0)).with_color(Color::srgb(0.9, 0.1, 0.2));
        let mut tooltip = Tooltip::new(El::column()).with_label_size(Pt(11.0));
        tooltip.text("Plain tooltip text");
        tooltip.text(("Authored", authored.clone()));
        let expected =
            TextStyle::new(Pt(11.0 * 0.85)).with_color(ControlPalette::system_dark().knob);
        let panel = DiegeticPanel::world()
            .size(
                fit_sizing(Unit::Millimeters, f32::MAX),
                fit_sizing(Unit::Millimeters, f32::MAX),
            )
            .with_tree(tooltip.blueprint().as_ref().clone())
            .build();
        assert!(panel.is_ok(), "tooltip panel should build");
        let Ok(mut panel) = panel else { return };

        for width in [f32::MAX, 40.0, f32::MAX] {
            panel::constrain_fit_width(&mut panel, width);
            assert_eq!(panel.tree().element_style(1), Some(&expected));
            assert_eq!(panel.tree().element_style(2), Some(&authored));
        }
    }

    /// A width reflow refits the root and keeps the frame the tooltip was built with, so
    /// measured padding outlives every Apply and Restore.
    #[test]
    fn width_reflow_keeps_the_built_tooltip_frame() {
        let mut tooltip = Tooltip::new(El::column());
        tooltip.text("Plug a jack");
        let mut framed = tooltip.blueprint().as_ref().clone();
        framed.apply_tooltip_frame(
            Padding::new(Px(5.5), Px(5.5), Px(3.25), Px(1.75)),
            Border::all(Px(1.0), ControlPalette::system_dark().tooltip_border),
            CornerRadius::all(Px(6.0)),
        );
        let panel = DiegeticPanel::world()
            .size(
                fit_sizing(Unit::Millimeters, f32::MAX),
                fit_sizing(Unit::Millimeters, f32::MAX),
            )
            .with_tree(framed)
            .build();
        assert!(panel.is_ok(), "tooltip panel should build");
        let Ok(mut panel) = panel else { return };

        for width in [40.0, f32::MAX] {
            let mut refitted = panel.tree().clone();
            crate::layout::set_root_fit_width(&mut refitted, Mm(0.0).into(), Mm(width).into());
            panel::constrain_fit_width(&mut panel, width);
            assert_eq!(
                panel.tree().classify_change(&refitted),
                LayoutTreeChange::Identical,
                "a reflow to {width} mm changed more than the root width"
            );
        }
        assert_ne!(
            panel.tree().classify_change(tooltip.blueprint()),
            LayoutTreeChange::Identical,
            "the reflowed tooltip lost its built frame"
        );
    }

    #[test]
    fn preparation_materializes_once_and_waits_for_a_compatible_camera() {
        let mut app = materialization_app();
        app.world_mut().resource_mut::<PanelDefaults>().layout_unit = Unit::Millimeters;
        let target = mesh_target(&mut app, Vec3A::ONE);
        let controller =
            standalone_world_tooltip(&mut app, target, fixed_size_tooltip(Vec2::new(30.0, 10.0)));
        app.update();

        assert_unmaterialized_tooltip(&app, target, controller);

        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        let tree_revision = assert_materialized_world_tooltip(&app, target, controller);

        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);
        assert_eq!(
            app.world()
                .get::<DiegeticPanel>(controller)
                .map(|panel| u64::from(panel.tree_revision())),
            tree_revision
        );
        assert_eq!(
            app.world()
                .get::<AnchoredHere>(target)
                .map(AnchoredHere::len),
            Some(1)
        );

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
        let camera = spawn_presentation_camera(&mut app, window, RenderLayers::layer(1));
        app.world_mut()
            .entity_mut(controller)
            .insert(TooltipPresentationCamera::new(camera));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.world_mut()
            .entity_mut(camera)
            .insert(RenderLayers::layer(0));
        update_materialization(&mut app);
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
        assert_eq!(
            app.world().get::<Visibility>(controller),
            Some(&Visibility::Hidden)
        );
    }

    #[test]
    fn a_general_screen_target_with_a_missing_window_stays_lightweight() {
        let mut app = materialization_app();
        let target = app.world_mut().spawn_empty().id();
        let missing_window = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::splat(20.0), Vec2::splat(10.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                missing_window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, Tooltip::new(El::new()));
        app.update();
        prepare_tooltip(&mut app, controller);
        update_materialization(&mut app);

        assert!(app.world().get::<PrepareTooltip>(controller).is_some());
        assert!(app.world().get::<DiegeticPanel>(controller).is_none());
        assert!(app.world().get::<MaterializedTooltip>(controller).is_none());
        assert!(
            app.world()
                .get::<crate::panel::PanelAttachmentAuthored>(controller)
                .is_none()
        );
    }

    #[test]
    fn fixed_tooltip_waits_for_the_first_completed_layout() {
        let mut app = materialization_app();
        let window = app
            .world_mut()
            .spawn((
                Window {
                    resolution: SMALL_SCREEN_VIEWPORT.as_uvec2().into(),
                    ..Default::default()
                },
                PrimaryWindow,
            ))
            .id();
        let target = app.world_mut().spawn_empty().id();
        let bounds = PanelScreenBounds::new(Vec2::new(100.0, 80.0), Vec2::new(40.0, 20.0));
        assert!(bounds.is_ok());
        let Some(bounds) = bounds.ok() else {
            return;
        };
        let handle = app.world_mut().commands().screen_anchor_target(
            target,
            crate::screen_space::ScreenAnchorTarget::new(
                bounds,
                window,
                5,
                RenderLayers::layer(3),
                Unit::Pixels,
            ),
        );
        let controller = app
            .world_mut()
            .commands()
            .spawn_tooltip(handle, fixed_size_tooltip(Vec2::new(60.0, 30.0)));
        app.update();
        prepare_tooltip(&mut app, controller);

        app.update();

        assert!(app.world().get::<MaterializedTooltip>(controller).is_some());
        assert!(
            app.world()
                .get::<ComputedDiegeticPanel>(controller)
                .is_some_and(|computed| !computed.is_solved())
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Pending)
        );

        app.update();

        assert!(
            app.world()
                .get::<ComputedDiegeticPanel>(controller)
                .is_some_and(ComputedDiegeticPanel::is_solved)
        );
        assert_eq!(
            app.world().get::<TooltipReadiness>(controller),
            Some(&TooltipReadiness::Ready)
        );
    }
}
