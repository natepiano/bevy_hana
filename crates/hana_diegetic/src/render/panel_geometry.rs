//! Panel geometry data extraction from layout render commands.
//!
//! Each element with a background or border produces a `ResolvedSdfSurface`
//! retained in `ResolvedSdfSurfaceRegistry`; `fill_batch::FillBatchPlugin`
//! turns those surfaces into visible batched SDF records. Picking still uses
//! one invisible `PanelInteractionMesh` child per panel. That interaction mesh
//! is picking-only and exempt from the `SdfMaterial` cascade.

use std::collections::HashMap;

use bevy::asset::load_internal_asset;
use bevy::camera::NormalizedRenderTarget;
use bevy::camera::RenderTarget;
use bevy::camera::visibility::RenderLayers;
use bevy::light::NotShadowCaster;
use bevy::picking::Pickable;
use bevy::picking::mesh_picking::ray_cast::RayCastBackfaces;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Camera;
use bevy::prelude::Changed;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Or;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::Rectangle;
use bevy::prelude::RemovedComponents;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::prelude::Shader;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::Vec4;
use bevy::prelude::With;
use bevy::prelude::Without;
use bevy::prelude::default;

use super::CommandIndex;
use super::ElementIndex;
use super::PanelChildSystems;
use super::clip;
use super::constants;
use super::constants::SDF_STROKE_SHADER_HANDLE;
use super::draw_order::DrawCommandDepth;
use super::draw_order::DrawOrder;
use super::material_table::MaterialTableAppendReady;
use crate::cascade::Resolved;
use crate::cascade::SdfMaterial;
use crate::layout::BoundingBox;
use crate::layout::RectangleSource;
use crate::layout::RenderCommand;
use crate::layout::RenderCommandKind;
use crate::layout::ShadowCasting;
use crate::panel::ComputedDiegeticPanel;
use crate::panel::DiegeticPanel;
use crate::panel::DroppedSdfSurfaces;
use crate::panel::FrameWork;
use crate::panel::PanelLayout;
use crate::panel::PanelOwned;

/// The invisible full-panel interaction quad, one per panel, tagged with its
/// panel-local size and center so a rebuild can leave it untouched when the
/// panel has not resized. Its material is a local picking-only invisible asset,
/// not a source material resolved from `SdfMaterial`. Both pairs are stored as
/// `f32` bit patterns for exact equality.
#[derive(Component, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PanelInteractionMesh {
    /// Panel-local size (width, height).
    size:   [u32; 2],
    /// Panel-local transform center (x, y).
    center: [u32; 2],
}

impl PanelInteractionMesh {
    pub(crate) const fn new(size: Vec2, center: Vec2) -> Self {
        Self {
            size:   vec2_bits(size),
            center: vec2_bits(center),
        }
    }
}

/// Plugin that adds panel geometry rendering (backgrounds and borders).
pub(crate) struct PanelGeometryPlugin;

impl Plugin for PanelGeometryPlugin {
    fn build(&self, app: &mut App) {
        load_internal_asset!(
            app,
            SDF_STROKE_SHADER_HANDLE,
            "sdf_stroke.wgsl",
            Shader::from_wgsl
        );
        app.init_resource::<ResolvedSdfSurfaceRegistry>();
        app.add_systems(
            PostUpdate,
            (
                build_panel_geometry.before(MaterialTableAppendReady),
                sync_interaction_mesh_render_layers.after(build_panel_geometry),
            )
                .in_set(PanelChildSystems::Build),
        );
    }
}

/// Copies panel-facing `RenderLayers` changes and removals to the private
/// interaction mesh without waiting for a geometry rebuild.
fn sync_interaction_mesh_render_layers(
    changed_panels: Query<
        (Entity, &RenderLayers),
        (
            With<DiegeticPanel>,
            Without<PanelInteractionMesh>,
            Changed<RenderLayers>,
        ),
    >,
    panels_without_layers: Query<(), (With<DiegeticPanel>, Without<RenderLayers>)>,
    mut removed_layers: RemovedComponents<RenderLayers>,
    mut interaction_meshes: Query<
        (&PanelOwned, &mut RenderLayers),
        (With<PanelInteractionMesh>, Without<DiegeticPanel>),
    >,
) {
    for (panel, layers) in &changed_panels {
        sync_panel_interaction_layers(panel, layers, &mut interaction_meshes);
    }
    for panel in removed_layers.read() {
        if panels_without_layers.contains(panel) {
            sync_panel_interaction_layers(panel, &RenderLayers::layer(0), &mut interaction_meshes);
        }
    }
}

fn sync_panel_interaction_layers(
    panel: Entity,
    layers: &RenderLayers,
    interaction_meshes: &mut Query<
        (&PanelOwned, &mut RenderLayers),
        (With<PanelInteractionMesh>, Without<DiegeticPanel>),
    >,
) {
    for (ownership, mut current) in interaction_meshes {
        if ownership.owner() == panel && *current != *layers {
            current.clone_from(layers);
        }
    }
}

/// Gathered fill + border data for a single element.
#[derive(Clone, Copy)]
pub(super) struct ElementSurface {
    /// Layout-tree identity for this surface, or the parent identity for a child divider.
    index:                  ElementIndex,
    /// Adjacent-child pair ordinal for a child divider; `None` for element surfaces.
    divider_ordinal:        Option<usize>,
    /// Bounding box from the render command.
    bounds:                 BoundingBox,
    /// Fill command state, if the element has an authored fill command.
    fill:                   Option<ElementFill>,
    /// Border command state, if the element has an authored border command.
    border:                 Option<ElementBorder>,
    /// Index of the first render command for this element (the reconcile
    /// reuse key).
    command_index:          CommandIndex,
    /// Projected ordering values of the first render command for this element.
    draw_depth:             DrawCommandDepth,
    /// Active clip rect in layout coordinates when this surface was
    /// gathered. `None` means unclipped.
    clip_rect:              Option<BoundingBox>,
    /// Element material override handling for this record's fill role.
    fill_material_override: FillMaterialOverride,
}

/// Controls material-only fill authorship for split SDF records.
#[derive(Clone, Copy)]
enum FillMaterialOverride {
    /// Element material overrides author the fill role.
    Included,
    /// Element material overrides do not author the fill role.
    Suppressed,
}

impl FillMaterialOverride {
    const fn is_included(self) -> bool { matches!(self, Self::Included) }
}

/// Fill-specific command state retained for split SDF records.
#[derive(Clone, Copy)]
struct ElementFill {
    /// Fill color from the rectangle command.
    color:         Color,
    /// Command identity of the rectangle command.
    command_index: CommandIndex,
    /// Draw ordering for the rectangle command.
    draw_depth:    DrawCommandDepth,
    /// Active clip rect for the rectangle command.
    clip_rect:     Option<BoundingBox>,
}

/// Border-specific command state retained for split SDF records.
#[derive(Clone, Copy)]
struct ElementBorder {
    /// Border widths [top, right, bottom, left] in layout points.
    widths:        [f32; 4],
    /// Border color from the border command.
    color:         Color,
    /// Command identity of the border command.
    command_index: CommandIndex,
    /// Draw ordering for the border command.
    draw_depth:    DrawCommandDepth,
    /// Active clip rect for the border command.
    clip_rect:     Option<BoundingBox>,
}

impl ElementSurface {
    /// Returns the element whose visual properties apply to this surface.
    ///
    /// Child dividers retain their parent `ElementIndex` for material-table
    /// identity, but resolve their material and corner radius from the panel.
    const fn visual_source_element(self) -> Option<ElementIndex> {
        match self.divider_ordinal {
            Some(_) => None,
            None => Some(self.index),
        }
    }

    const fn fill_only(self) -> Self {
        let Some(fill) = self.fill else {
            return self;
        };
        Self {
            fill: Some(fill),
            border: None,
            command_index: fill.command_index,
            draw_depth: fill.draw_depth,
            clip_rect: fill.clip_rect,
            fill_material_override: FillMaterialOverride::Included,
            ..self
        }
    }

    const fn border_only(self) -> Self {
        let Some(border) = self.border else {
            return self;
        };
        Self {
            fill: None,
            border: Some(border),
            command_index: border.command_index,
            draw_depth: border.draw_depth,
            clip_rect: border.clip_rect,
            fill_material_override: FillMaterialOverride::Suppressed,
            ..self
        }
    }
}

/// Resolves each panel's SDF surfaces from its render commands.
///
/// Runs when a panel's layout commands change or when its cascade-resolved
/// `SdfMaterial` changes. The system rewrites the
/// `ResolvedSdfSurfaceRegistry` panel bucket from the current
/// `ComputedDiegeticPanel`, and `fill_batch::FillBatchPlugin` handles record
/// reuse, material-table rows, and visible render entities.
fn build_panel_geometry(
    changed_panels: Query<
        (
            Entity,
            &DiegeticPanel,
            &ComputedDiegeticPanel,
            Option<&RenderLayers>,
            Option<&Resolved<SdfMaterial>>,
            Option<&Resolved<ShadowCasting>>,
        ),
        Or<(
            Changed<ComputedDiegeticPanel>,
            Changed<Resolved<SdfMaterial>>,
            Changed<Resolved<ShadowCasting>>,
        )>,
    >,
    old_interaction: Query<(Entity, &PanelOwned, &PanelInteractionMesh)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut standard_materials: ResMut<Assets<StandardMaterial>>,
    sdf_material_default: Res<SdfMaterial>,
    mut resolved_surfaces: ResMut<ResolvedSdfSurfaceRegistry>,
    mut commands: Commands,
) {
    for (panel_entity, panel, computed, panel_layers, panel_sdf_material, panel_shadow_casting) in
        &changed_panels
    {
        let PanelLayout::Solved(result) = computed.layout() else {
            resolved_surfaces.remove_panel(panel_entity, SurfaceEvictionCause::LayoutUnsolved);
            continue;
        };

        let (anchor_x, anchor_y) = panel.anchor_offsets();
        let context = PanelReconcileContext {
            panel_entity,
            panel,
            points_to_world: panel.points_to_world(),
            anchor_x,
            anchor_y,
            shadow_casting: panel_shadow_casting.map_or(ShadowCasting::On, |resolved| resolved.0),
            layer: panel_layers.cloned().unwrap_or(RenderLayers::layer(0)),
            sdf_material: panel_sdf_material.map_or_else(
                || sdf_material_default.0.clone(),
                |resolved| resolved.0.0.clone(),
            ),
        };
        let gathered = gather_surfaces(context.panel, &result.commands, computed.draw_order());
        let desired = desired_surfaces(gathered);
        let mut resolved: Vec<ResolvedSdfSurface<'_>> = Vec::with_capacity(desired.len());
        for surface in &desired {
            push_resolved_sdf_surfaces(surface, &context, &mut resolved);
        }
        resolved.sort_by(|left, right| {
            left.draw_depth
                .draw_order_index()
                .cmp(&right.draw_depth.draw_order_index())
                .then(left.command_index.cmp(&right.command_index))
        });
        resolved_surfaces.upsert_panel(
            panel_entity,
            resolved
                .iter()
                .map(StoredResolvedSdfSurface::from_resolved)
                .collect(),
        );

        reconcile_interaction_mesh(
            &context,
            &old_interaction,
            &mut meshes,
            &mut standard_materials,
            &mut commands,
        );
    }
}

/// Shared per-panel inputs for one reconcile pass, derived once from the panel.
pub(super) struct PanelReconcileContext<'a> {
    /// Panel entity that owns the resolved SDF surfaces.
    panel_entity:    Entity,
    /// Source panel that provides layout scale, materials, and shadow policy.
    panel:           &'a DiegeticPanel,
    /// Conversion factor from layout points to panel-local world units.
    points_to_world: f32,
    /// Panel-local horizontal anchor offset.
    anchor_x:        f32,
    /// Panel-local vertical anchor offset.
    anchor_y:        f32,
    /// Shadow-casting policy copied onto each resolved SDF surface.
    shadow_casting:  ShadowCasting,
    /// Render layers copied onto each resolved SDF surface.
    layer:           RenderLayers,
    /// Panel's cascade-resolved SDF material handle.
    sdf_material:    Handle<StandardMaterial>,
}

/// Reconciles the invisible full-panel interaction quad: it is respawned only
/// when the panel-local size or center changed, and left untouched otherwise.
fn reconcile_interaction_mesh(
    context: &PanelReconcileContext<'_>,
    old_interaction: &Query<(Entity, &PanelOwned, &PanelInteractionMesh)>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
) {
    let panel_size = interaction_mesh_size(context.panel);
    let center = Vec2::new(
        panel_size.x.mul_add(0.5, -context.anchor_x),
        panel_size.y.mul_add(-0.5, context.anchor_y),
    );
    let interaction = PanelInteractionMesh::new(panel_size, center);
    let existing = old_interaction
        .iter()
        .find(|(_, ownership, _)| ownership.owner() == context.panel_entity);
    match existing {
        Some((_, _, current)) if *current == interaction => {},
        Some((entity, ..)) => {
            commands.entity(entity).despawn();
            spawn_interaction_mesh(
                interaction,
                panel_size,
                center,
                context,
                meshes,
                materials,
                commands,
            );
        },
        None => {
            spawn_interaction_mesh(
                interaction,
                panel_size,
                center,
                context,
                meshes,
                materials,
                commands,
            );
        },
    }
}

fn interaction_mesh_size(panel: &DiegeticPanel) -> Vec2 {
    if panel.coordinate_space().is_screen() {
        return Vec2::new(panel.width(), panel.height());
    }
    Vec2::new(panel.world_width(), panel.world_height())
}

// ── Helpers ─────────────────────────────────────────────────────────────────

struct GatheredCommands {
    surfaces: HashMap<ElementIndex, ElementSurface>,
    dividers: Vec<ElementSurface>,
}

/// Gathers fill + border data per element from render commands.
///
/// Computes the active clip rect for each command via
/// [`clip::compute_clip_rects`] and stores it on each surface and divider.
fn gather_surfaces(
    panel: &DiegeticPanel,
    commands: &[RenderCommand],
    draw_order: &DrawOrder,
) -> GatheredCommands {
    let clip_rects = clip::compute_clip_rects(commands);
    let viewport = clip::panel_viewport(panel);
    let mut surfaces: HashMap<ElementIndex, ElementSurface> = HashMap::new();
    let mut dividers: Vec<ElementSurface> = Vec::new();

    for (cmd_index, cmd) in commands.iter().enumerate() {
        let active_clip = clip::effective_clip(cmd.bounds, clip_rects[cmd_index], viewport);
        match &cmd.kind {
            RenderCommandKind::Rectangle { color, source } => {
                let Some(active_clip) = active_clip else {
                    continue;
                };
                let Some(draw_depth) = draw_order.depth_for(cmd_index) else {
                    continue;
                };
                let command_index = CommandIndex::from(cmd_index);
                let element_index = ElementIndex::from(cmd.element_idx);
                if let RectangleSource::ChildDivider { ordinal } = source {
                    dividers.push(ElementSurface {
                        index: element_index,
                        divider_ordinal: Some(*ordinal),
                        bounds: cmd.bounds,
                        fill: Some(ElementFill {
                            color: *color,
                            command_index,
                            draw_depth,
                            clip_rect: Some(active_clip),
                        }),
                        border: None,
                        command_index,
                        draw_depth,
                        clip_rect: Some(active_clip),
                        fill_material_override: FillMaterialOverride::Included,
                    });
                } else {
                    let surface = surfaces
                        .entry(element_index)
                        .or_insert_with(|| ElementSurface {
                            index: element_index,
                            divider_ordinal: None,
                            bounds: cmd.bounds,
                            fill: None,
                            border: None,
                            command_index,
                            draw_depth,
                            clip_rect: Some(active_clip),
                            fill_material_override: FillMaterialOverride::Included,
                        });
                    surface.fill = Some(ElementFill {
                        color: *color,
                        command_index,
                        draw_depth,
                        clip_rect: Some(active_clip),
                    });
                }
            },
            RenderCommandKind::Border { border } => {
                let Some(active_clip) = active_clip else {
                    continue;
                };
                let Some(draw_depth) = draw_order.depth_for(cmd_index) else {
                    continue;
                };
                let command_index = CommandIndex::from(cmd_index);
                let element_index = ElementIndex::from(cmd.element_idx);
                let surface = surfaces
                    .entry(element_index)
                    .or_insert_with(|| ElementSurface {
                        index: element_index,
                        divider_ordinal: None,
                        bounds: cmd.bounds,
                        fill: None,
                        border: None,
                        command_index,
                        draw_depth,
                        clip_rect: Some(active_clip),
                        fill_material_override: FillMaterialOverride::Included,
                    });
                surface.border = Some(ElementBorder {
                    widths: [
                        border.top.value,
                        border.right.value,
                        border.bottom.value,
                        border.left.value,
                    ],
                    color: border.color,
                    command_index,
                    draw_depth,
                    clip_rect: Some(active_clip),
                });
            },
            _ => {},
        }
    }

    GatheredCommands { surfaces, dividers }
}

/// Flattens the gathered surfaces and dividers into one list of surfaces to
/// render, each keyed by its source element and divider ordinal.
fn desired_surfaces(gathered: GatheredCommands) -> Vec<ElementSurface> {
    let mut desired: Vec<ElementSurface> = gathered.surfaces.into_values().collect();
    desired.extend(gathered.dividers);
    desired
}

/// Borrowed source material handle plus the layout color override for one SDF
/// material slot.
pub(super) struct ResolvedSdfMaterial<'a> {
    /// Authorship state: only an [`SdfRoleAuthorship::Authored`] role appends a
    /// material row.
    pub(super) authorship:    SdfRoleAuthorship,
    /// Element or panel material handle used as the `StandardMaterial` source;
    /// `None` connects this slot to the seeded `SdfMaterial` default.
    pub(super) base_material: Option<&'a Handle<StandardMaterial>>,
    /// Layout color applied to `StandardMaterial::base_color` when the SDF
    /// frame material table builds the owned material.
    pub(super) color:         Option<Color>,
}

/// Render-neutral resolved SDF surface consumed by the SDF fill batch builder.
pub(super) struct ResolvedSdfSurface<'a> {
    /// Entity of the `DiegeticPanel` that owns this surface.
    pub(super) panel_entity:    Entity,
    /// Command identity inside the panel's `LayoutResult::commands` stream.
    pub(super) command_index:   CommandIndex,
    /// Layout-tree identity used by the material-table source key; child
    /// dividers retain their parent element identity here.
    pub(super) element_index:   ElementIndex,
    /// Adjacent-child pair ordinal for a child divider; `None` for element surfaces.
    pub(super) divider_ordinal: Option<usize>,
    /// Retained-record role when clipping separates the fill and border.
    pub(super) record_role:     SdfRecordRole,
    /// `DrawCommandDepth` for sorted and OIT ordering.
    pub(super) draw_depth:      DrawCommandDepth,
    /// Fill material input consumed by the SDF material table.
    pub(super) fill_material:   ResolvedSdfMaterial<'a>,
    /// Border material input consumed by the SDF material table.
    pub(super) border_material: ResolvedSdfMaterial<'a>,
    /// Panel-local center of the batched SDF record.
    pub(super) local_center:    Vec2,
    /// Panel-local transform copied into each `ResolvedSdfBatchRecord`.
    pub(super) local_transform: Transform,
    /// Panel-local half size of the SDF form.
    pub(super) sdf_half_size:   Vec2,
    /// Panel-local half size of the quad mesh, including `SDF_AA_PADDING`.
    pub(super) mesh_half_size:  Vec2,
    /// Panel-local per-corner radii [TL, TR, BR, BL].
    pub(super) corner_radii:    [f32; 4],
    /// Panel-local border widths [top, right, bottom, left].
    pub(super) border_widths:   [f32; 4],
    /// Panel-local clip rect [left, bottom, right, top] after padding
    /// expansion.
    pub(super) clip_rect:       Vec4,
    /// Render layers copied to SDF fill batch records.
    pub(super) render_layers:   RenderLayers,
    /// Shadow-casting policy copied to SDF fill batch records.
    pub(super) shadow_casting:  ShadowCasting,
}

/// Retained-record role for one resolved SDF surface.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(super) enum SdfRecordRole {
    /// The surface keeps its fill and border in one retained record.
    Whole,
    /// The clipped surface retains only its fill.
    Fill,
    /// The clipped surface retains only its border.
    Border,
}

impl ResolvedSdfSurface<'_> {
    /// Returns whether `ResolvedSdfSurface::clip_rect` trims the padded SDF mesh.
    #[must_use]
    pub(super) fn clip_rect_limits_mesh(&self) -> bool {
        let clip_left = self.clip_rect.x;
        let clip_bottom = self.clip_rect.y;
        let clip_right = self.clip_rect.z;
        let clip_top = self.clip_rect.w;
        let mesh_left = -self.mesh_half_size.x;
        let mesh_bottom = -self.mesh_half_size.y;
        let mesh_right = self.mesh_half_size.x;
        let mesh_top = self.mesh_half_size.y;

        clip_left > mesh_left
            || clip_bottom > mesh_bottom
            || clip_right < mesh_right
            || clip_top < mesh_top
    }
}

/// Authorship state for a fill or border role in `ResolvedSdfMaterial`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum SdfRoleAuthorship {
    /// The layout command authored this fill or border role.
    Authored,
    /// The layout command did not author this fill or border role.
    Unauthored,
}

impl SdfRoleAuthorship {
    /// Returns whether this role appends a material-table row.
    #[must_use]
    pub(super) const fn is_authored(self) -> bool { matches!(self, Self::Authored) }
}

/// Owned current-frame SDF source-material handle retained for the batch route.
#[derive(Clone)]
struct StoredResolvedSdfMaterial {
    /// Authorship state copied from `ResolvedSdfMaterial::authorship`.
    authorship:    SdfRoleAuthorship,
    /// Cloned material source handle used when appending the frame material table.
    base_material: Option<Handle<StandardMaterial>>,
    /// Layout color applied to `StandardMaterial::base_color`.
    color:         Option<Color>,
}

impl StoredResolvedSdfMaterial {
    fn from_resolved(material: &ResolvedSdfMaterial<'_>) -> Self {
        Self {
            authorship:    material.authorship,
            base_material: material.base_material.cloned(),
            color:         material.color,
        }
    }

    const fn as_resolved(&self) -> ResolvedSdfMaterial<'_> {
        ResolvedSdfMaterial {
            authorship:    self.authorship,
            base_material: self.base_material.as_ref(),
            color:         self.color,
        }
    }
}

/// Owned resolved SDF surface retained while `ComputedDiegeticPanel` is current.
#[derive(Clone)]
pub(crate) struct StoredResolvedSdfSurface {
    /// Entity of the `DiegeticPanel` that owns this surface.
    panel_entity:    Entity,
    /// Command identity inside the panel's `LayoutResult::commands` stream.
    command_index:   CommandIndex,
    /// Index of the source element in the panel's `LayoutTree`.
    element_index:   ElementIndex,
    /// Adjacent-child pair ordinal for a child divider; `None` for element surfaces.
    divider_ordinal: Option<usize>,
    /// Retained-record role when clipping separates the fill and border.
    record_role:     SdfRecordRole,
    /// `DrawCommandDepth` for sorted and OIT ordering.
    draw_depth:      DrawCommandDepth,
    /// Fill material source for the SDF material table.
    fill_material:   StoredResolvedSdfMaterial,
    /// Border material source for the SDF material table.
    border_material: StoredResolvedSdfMaterial,
    /// Panel-local center of the batched SDF record.
    local_center:    Vec2,
    /// Panel-local transform for this surface.
    local_transform: Transform,
    /// Panel-local half size of the SDF form.
    sdf_half_size:   Vec2,
    /// Panel-local half size of the quad mesh, including `SDF_AA_PADDING`.
    mesh_half_size:  Vec2,
    /// Panel-local per-corner radii [TL, TR, BR, BL].
    corner_radii:    [f32; 4],
    /// Panel-local border widths [top, right, bottom, left].
    border_widths:   [f32; 4],
    /// Panel-local clip rect [left, bottom, right, top].
    clip_rect:       Vec4,
}

impl StoredResolvedSdfSurface {
    fn from_resolved(surface: &ResolvedSdfSurface<'_>) -> Self {
        Self {
            panel_entity:    surface.panel_entity,
            command_index:   surface.command_index,
            element_index:   surface.element_index,
            divider_ordinal: surface.divider_ordinal,
            record_role:     surface.record_role,
            draw_depth:      surface.draw_depth,
            fill_material:   StoredResolvedSdfMaterial::from_resolved(&surface.fill_material),
            border_material: StoredResolvedSdfMaterial::from_resolved(&surface.border_material),
            local_center:    surface.local_center,
            local_transform: surface.local_transform,
            sdf_half_size:   surface.sdf_half_size,
            mesh_half_size:  surface.mesh_half_size,
            corner_radii:    surface.corner_radii,
            border_widths:   surface.border_widths,
            clip_rect:       surface.clip_rect,
        }
    }

    /// Returns the owning `DiegeticPanel` entity.
    pub(crate) const fn panel_entity(&self) -> Entity { self.panel_entity }

    /// Returns the element whose widget visual-slot override applies.
    ///
    /// A child divider uses its parent `ElementIndex` for retained-table
    /// identity only, so it has no visual-slot source element.
    #[must_use]
    pub(super) const fn visual_source_element(&self) -> Option<ElementIndex> {
        match self.divider_ordinal {
            Some(_) => None,
            None => Some(self.element_index),
        }
    }

    /// Borrows the stored surface with current panel render state.
    pub(super) const fn as_resolved(
        &self,
        render_layers: RenderLayers,
        shadow_casting: ShadowCasting,
    ) -> ResolvedSdfSurface<'_> {
        ResolvedSdfSurface {
            panel_entity: self.panel_entity,
            command_index: self.command_index,
            element_index: self.element_index,
            divider_ordinal: self.divider_ordinal,
            record_role: self.record_role,
            draw_depth: self.draw_depth,
            fill_material: self.fill_material.as_resolved(),
            border_material: self.border_material.as_resolved(),
            local_center: self.local_center,
            local_transform: self.local_transform,
            sdf_half_size: self.sdf_half_size,
            mesh_half_size: self.mesh_half_size,
            corner_radii: self.corner_radii,
            border_widths: self.border_widths,
            clip_rect: self.clip_rect,
            render_layers,
            shadow_casting,
        }
    }
}

/// Why a panel's retained SDF surfaces left [`ResolvedSdfSurfaceRegistry`].
///
/// [`ResolvedSdfSurfaceRegistry::remove_panel`] requires one, so an eviction
/// cannot discard surfaces without naming the [`DroppedSdfSurfaces`] field that
/// reports them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SurfaceEvictionCause {
    /// [`build_panel_geometry`] reached a panel whose [`ComputedDiegeticPanel`]
    /// holds no [`PanelLayout::Solved`], so it has no geometry to resolve.
    LayoutUnsolved,
    /// `route_sdf_batch_records` found no [`DiegeticPanel`] for the owning
    /// entity, so the panel's surfaces outlived their panel.
    PanelMissing,
    /// The panel is being torn down by `panel::lifecycle`, which evicts its
    /// surfaces before the route can observe them as orphaned.
    ///
    /// The only cause that reports nothing: these surfaces stop rendering
    /// because their panel was removed on purpose, so counting them would
    /// report normal teardown as a defect.
    PanelDespawned,
}

/// Current resolved SDF surfaces keyed by owning panel for the batch route.
#[derive(Default, Resource)]
pub(crate) struct ResolvedSdfSurfaceRegistry {
    surfaces_by_panel: HashMap<Entity, Vec<StoredResolvedSdfSurface>>,
    /// Surfaces evicted since [`Self::take_evictions`] last ran.
    ///
    /// Evictions and the routing pass that reports them run in different
    /// systems, so the count is held here rather than written to
    /// `DiegeticPerfStats` at the eviction site: `DroppedSdfSurfaces` keeps one
    /// writer, and a whole-struct assignment there cannot overwrite an eviction
    /// counted earlier in the frame.
    evictions:         DroppedSdfSurfaces<FrameWork>,
}

impl ResolvedSdfSurfaceRegistry {
    /// Replaces the retained SDF surfaces for one panel.
    ///
    /// An empty replacement is not an eviction: the panel resolved its layout
    /// and authored no surface, so there is nothing to attribute.
    pub(crate) fn upsert_panel(
        &mut self,
        panel_entity: Entity,
        surfaces: Vec<StoredResolvedSdfSurface>,
    ) {
        if surfaces.is_empty() {
            self.surfaces_by_panel.remove(&panel_entity);
        } else {
            self.surfaces_by_panel.insert(panel_entity, surfaces);
        }
    }

    /// Removes retained SDF surfaces for one panel and records how many were
    /// evicted under `cause`.
    pub(crate) fn remove_panel(&mut self, panel_entity: Entity, cause: SurfaceEvictionCause) {
        let Some(evicted) = self.surfaces_by_panel.remove(&panel_entity) else {
            return;
        };
        let counter = match cause {
            SurfaceEvictionCause::LayoutUnsolved => &mut self.evictions.layout_unsolved,
            SurfaceEvictionCause::PanelMissing => &mut self.evictions.panel_missing,
            SurfaceEvictionCause::PanelDespawned => return,
        };
        *counter += evicted.len();
    }

    /// Returns the evictions recorded since this last ran and clears them.
    ///
    /// The caller merges these into the frame's [`DroppedSdfSurfaces`]; taking
    /// them is what keeps an eviction from being counted on two frames.
    pub(crate) fn take_evictions(&mut self) -> DroppedSdfSurfaces<FrameWork> {
        core::mem::take(&mut self.evictions)
    }

    /// Iterates retained SDF surfaces in panel-bucket order.
    pub(crate) fn surfaces(&self) -> impl Iterator<Item = &StoredResolvedSdfSurface> {
        self.surfaces_by_panel.values().flatten()
    }
}

/// Resolves an [`ElementSurface`] into render-neutral SDF data.
///
/// This performs only pre-propagation panel-local math. `fill_batch` appends
/// `StandardMaterial` rows and writes `SdfRenderRecord` values later in the
/// `PostUpdate` schedule.
fn push_resolved_sdf_surfaces<'a>(
    surface: &ElementSurface,
    context: &'a PanelReconcileContext<'a>,
    output: &mut Vec<ResolvedSdfSurface<'a>>,
) {
    let resolved = resolve_sdf_surface(surface, context);
    if surface.fill.is_none() || surface.border.is_none() {
        output.push(resolved);
        return;
    }

    let border_surface = surface.border_only();
    let mut border_resolved = resolve_sdf_surface(&border_surface, context);
    if !should_split_clipped_border(surface, &resolved, &border_resolved) {
        output.push(resolved);
        return;
    }

    let mut fill_resolved = resolve_sdf_surface(&surface.fill_only(), context);
    fill_resolved.record_role = SdfRecordRole::Fill;
    border_resolved.record_role = SdfRecordRole::Border;
    output.push(fill_resolved);
    output.push(border_resolved);
}

fn should_split_clipped_border(
    surface: &ElementSurface,
    resolved: &ResolvedSdfSurface<'_>,
    border_resolved: &ResolvedSdfSurface<'_>,
) -> bool {
    surface.fill.is_some()
        && surface.border.is_some()
        && resolved.fill_material.authorship.is_authored()
        && resolved.border_material.authorship.is_authored()
        && border_resolved.clip_rect_limits_mesh()
}

fn resolve_sdf_surface<'a>(
    surface: &ElementSurface,
    context: &'a PanelReconcileContext<'a>,
) -> ResolvedSdfSurface<'a> {
    let (visual_source_element, base_material, material_override_authors_fill) =
        resolve_surface_material(surface, context);
    let fill_color = surface.fill.map(|fill| fill.color);
    let border_color = surface.border.map(|border| border.color);

    // Fill color from .background() or element .material() — never panel material.
    let effective_color = fill_color.or({
        if material_override_authors_fill {
            None
        } else {
            Some(Color::NONE)
        }
    });

    let local_width = surface.bounds.width * context.points_to_world;
    let local_height = surface.bounds.height * context.points_to_world;
    let corner_radii = visual_source_element.map_or([0.0; 4], |element_index| {
        panel_local_corner_radii(context.panel, element_index, context.points_to_world)
    });
    let border_widths = surface
        .border
        .map_or([0.0; 4], |border| border.widths)
        .map(|width| width * context.points_to_world);

    let half_width = local_width * 0.5;
    let half_height = local_height * 0.5;

    // Mesh is slightly larger than the SDF form so the exterior
    // anti-aliasing ramp has fragments to render on.
    let pad = constants::SDF_AA_PADDING;
    let mesh_half_width = half_width + pad;
    let mesh_half_height = half_height + pad;

    // Convert layout-space clip rect to local quad coords (centered, Y-up).
    //
    // ── DO NOT REMOVE THE `± pad` EXPANSION BELOW ─────────────────────────
    //
    // Every element's mesh extends `SDF_AA_PADDING` beyond its SDF form so
    // the shader's `smoothstep` exterior AA ramp has fragments to render on.
    // The clip rect coming in from `clip::effective_clip` is bound to the
    // panel viewport (or a tighter scissor), which sits *at* the SDF form
    // boundary — i.e. exactly where the AA ramp needs to fade. Without the
    // padding here, the shader's `discard` (in `sdf_panel.wgsl`) throws away
    // every fragment in the AA region at panel/viewport edges, and the
    // visible boundary collapses to the underlying polygon edge — a hard
    // staircase with no MSAA to smooth it.
    //
    // Expanding the clip outward by `pad` lets the AA ramp survive while
    // still clipping anything that's actually past the panel by more than
    // 1mm of world space (the case the clip was added for: retained
    // children that overflow after a panel shrink). The two regions don't
    // conflict — they're separated by exactly `SDF_AA_PADDING`.
    //
    // If you find yourself "cleaning this up" because the `- pad` / `+ pad`
    // look redundant or suspicious: run the `units` example, look at the
    // ruler ticks, and you'll see the staircase return immediately.
    // ──────────────────────────────────────────────────────────────────────
    let clip_rect = surface.clip_rect.map_or_else(
        || {
            Vec4::new(
                -mesh_half_width,
                -mesh_half_height,
                mesh_half_width,
                mesh_half_height,
            )
        },
        |clip_rect| {
            let (cx, cy) = surface.bounds.center();
            let left = (clip_rect.x - cx) * context.points_to_world;
            let right = (clip_rect.x + clip_rect.width - cx) * context.points_to_world;
            // Layout Y-down → local Y-up.
            let top = -(clip_rect.y - cy) * context.points_to_world;
            let bottom = -(clip_rect.y + clip_rect.height - cy) * context.points_to_world;
            Vec4::new(
                left - pad,
                bottom.min(top) - pad,
                right + pad,
                bottom.max(top) + pad,
            )
        },
    );

    let local_center = bounds_to_panel_local_center(
        &surface.bounds,
        context.points_to_world,
        context.anchor_x,
        context.anchor_y,
    );

    ResolvedSdfSurface {
        panel_entity: context.panel_entity,
        command_index: surface.command_index,
        element_index: surface.index,
        divider_ordinal: surface.divider_ordinal,
        record_role: SdfRecordRole::Whole,
        draw_depth: surface.draw_depth,
        fill_material: ResolvedSdfMaterial {
            authorship:    if fill_color.is_some() || material_override_authors_fill {
                SdfRoleAuthorship::Authored
            } else {
                SdfRoleAuthorship::Unauthored
            },
            base_material: Some(base_material),
            color:         effective_color,
        },
        border_material: ResolvedSdfMaterial {
            authorship:    if border_color.is_some() {
                SdfRoleAuthorship::Authored
            } else {
                SdfRoleAuthorship::Unauthored
            },
            base_material: Some(base_material),
            color:         border_color,
        },
        local_center,
        local_transform: Transform::from_xyz(local_center.x, local_center.y, 0.0),
        sdf_half_size: Vec2::new(half_width, half_height),
        mesh_half_size: Vec2::new(mesh_half_width, mesh_half_height),
        corner_radii,
        border_widths,
        clip_rect,
        render_layers: context.layer.clone(),
        shadow_casting: context.shadow_casting,
    }
}

/// Resolves the panel or element material that applies to an `ElementSurface`.
fn resolve_surface_material<'a>(
    surface: &ElementSurface,
    context: &'a PanelReconcileContext<'a>,
) -> (Option<ElementIndex>, &'a Handle<StandardMaterial>, bool) {
    let visual_source_element = surface.visual_source_element();
    let (base_material, element_material_is_override) = match visual_source_element {
        Some(element_index) => {
            let element_material = context.panel.tree().element_material(element_index.get());
            (
                element_material.map_or(&context.sdf_material, core::convert::identity),
                element_material.is_override(),
            )
        },
        None => (&context.sdf_material, false),
    };
    let material_override_authors_fill =
        element_material_is_override && surface.fill_material_override.is_included();
    (
        visual_source_element,
        base_material,
        material_override_authors_fill,
    )
}

fn panel_local_corner_radii(
    panel: &DiegeticPanel,
    element_index: ElementIndex,
    points_to_world: f32,
) -> [f32; 4] {
    panel
        .tree()
        .element_corner_radius(element_index.get())
        .resolved(panel.layout_unit().to_points())
        .to_array()
        .map(|radius| radius * points_to_world)
}

/// Spawns the invisible full-panel interaction quad as a child of the panel.
fn spawn_interaction_mesh(
    interaction: PanelInteractionMesh,
    size: Vec2,
    center: Vec2,
    context: &PanelReconcileContext<'_>,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<StandardMaterial>,
    commands: &mut Commands,
) {
    let material = materials.add(StandardMaterial {
        base_color: Color::NONE,
        alpha_mode: AlphaMode::Blend,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        depth_bias: -constants::LAYER_DEPTH_BIAS,
        ..default()
    });
    commands.entity(context.panel_entity).with_child((
        interaction,
        RayCastBackfaces,
        Pickable::IGNORE,
        NotShadowCaster,
        Mesh3d(meshes.add(Rectangle::new(size.x, size.y))),
        MeshMaterial3d(material),
        Transform::from_xyz(center.x, center.y, 0.0),
        context.layer.clone(),
        PanelOwned::from(context.panel_entity),
    ));
}

/// Projects a world-space hit on the current flat panel surface into layout coordinates.
pub(crate) fn project_flat_panel_hit(
    position: Vec3,
    panel: &DiegeticPanel,
    transform: &GlobalTransform,
) -> Option<Vec2> {
    let points_to_world = panel.points_to_world();
    if !points_to_world.is_finite() || points_to_world <= 0.0 {
        return None;
    }
    let local = transform.affine().inverse().transform_point3(position);
    let (anchor_x, anchor_y) = panel.anchor_offsets();
    let layout = Vec2::new(
        (local.x + anchor_x) / points_to_world,
        (anchor_y - local.y) / points_to_world,
    );
    layout.is_finite().then_some(layout)
}

/// Live captured-camera state that re-derives the captured pointer's world
/// ray.
///
/// Slider capture records the pointer's `NormalizedRenderTarget` when a press
/// is accepted; each later projection re-reads the live `Camera`,
/// `GlobalTransform`, and `RenderTarget` components and rejects the ray when
/// the camera's current normalized target no longer equals that capture.
pub(crate) struct CapturedCameraRay<'a> {
    /// Live `Camera` of the captured camera entity.
    pub(crate) camera:            &'a Camera,
    /// Live global transform of the captured camera entity.
    pub(crate) camera_transform:  &'a GlobalTransform,
    /// Live `RenderTarget` of the captured camera entity, when it still has
    /// one.
    pub(crate) render_target:     Option<&'a RenderTarget>,
    /// Resolved primary-window entity, when one exists.
    pub(crate) primary_window:    Option<Entity>,
    /// Normalized render target captured when the pointer press was accepted.
    pub(crate) captured_target:   &'a NormalizedRenderTarget,
    /// Current pointer position in viewport coordinates.
    pub(crate) viewport_position: Vec2,
}

/// Why [`project_flat_panel_ray_hit`] produced no panel-layout coordinates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum FlatPanelRayError {
    /// The captured camera no longer has a `RenderTarget` component.
    TargetMissing,
    /// The live `RenderTarget` does not normalize against the resolved
    /// primary window.
    TargetUnresolvable,
    /// The live normalized render target no longer equals the captured
    /// target.
    TargetMismatched,
    /// `Camera::viewport_to_world` produced no ray for the pointer position.
    CameraUnavailable,
    /// The panel's global transform is non-finite or not invertible.
    TransformNonInvertible,
    /// The pointer ray never crosses the panel plane.
    RayParallel,
    /// The panel-plane crossing lies behind the camera.
    HitBehindCamera,
    /// [`project_flat_panel_hit`] rejected the panel's layout scale or its
    /// computed coordinates.
    ScaleInvalid,
}

/// Projects the captured pointer's current viewport position through the live
/// captured camera onto `panel`'s layout coordinates.
///
/// The live `RenderTarget` is normalized against the resolved primary window
/// and must still equal [`CapturedCameraRay::captured_target`]. The pointer
/// ray from [`Camera::viewport_to_world`] intersects the panel's two-sided
/// local z = 0 plane, so front and back hits both project, and the world hit
/// converts through [`project_flat_panel_hit`].
pub(crate) fn project_flat_panel_ray_hit(
    captured_camera_ray: &CapturedCameraRay<'_>,
    panel: &DiegeticPanel,
    panel_transform: &GlobalTransform,
) -> Result<Vec2, FlatPanelRayError> {
    let live_target = captured_camera_ray
        .render_target
        .ok_or(FlatPanelRayError::TargetMissing)?;
    let live_normalized = live_target
        .normalize(captured_camera_ray.primary_window)
        .ok_or(FlatPanelRayError::TargetUnresolvable)?;
    if live_normalized != *captured_camera_ray.captured_target {
        return Err(FlatPanelRayError::TargetMismatched);
    }
    let ray = captured_camera_ray
        .camera
        .viewport_to_world(
            captured_camera_ray.camera_transform,
            captured_camera_ray.viewport_position,
        )
        .map_err(|_| FlatPanelRayError::CameraUnavailable)?;

    let world_from_panel = panel_transform.affine();
    let determinant = world_from_panel.matrix3.determinant();
    if !world_from_panel.is_finite() || !determinant.is_finite() || determinant == 0.0 {
        return Err(FlatPanelRayError::TransformNonInvertible);
    }
    let panel_from_world = world_from_panel.inverse();
    let local_origin = panel_from_world.transform_point3(ray.origin);
    let local_direction = panel_from_world.transform_vector3(*ray.direction);
    let distance = -local_origin.z / local_direction.z;
    if !distance.is_finite() {
        return Err(FlatPanelRayError::RayParallel);
    }
    if distance < 0.0 {
        return Err(FlatPanelRayError::HitBehindCamera);
    }
    project_flat_panel_hit(ray.get_point(distance), panel, panel_transform)
        .ok_or(FlatPanelRayError::ScaleInvalid)
}

const fn vec2_bits(value: Vec2) -> [u32; 2] { [value.x.to_bits(), value.y.to_bits()] }

fn bounds_to_panel_local_center(
    bounds: &BoundingBox,
    points_to_world: f32,
    anchor_x: f32,
    anchor_y: f32,
) -> Vec2 {
    let width = bounds.width * points_to_world;
    let height = bounds.height * points_to_world;
    let left = bounds.x.mul_add(points_to_world, -anchor_x);
    let top = -(bounds.y.mul_add(points_to_world, -anchor_y));

    Vec2::new(width.mul_add(0.5, left), height.mul_add(-0.5, top))
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::sync::Arc;

    use bevy::asset::AssetId;
    use bevy::asset::AssetPlugin;
    use bevy::camera::OrthographicProjection;
    use bevy::camera::PerspectiveProjection;
    use bevy::camera::Projection;
    use bevy::camera::RenderTargetInfo;
    use bevy::ecs::system::RunSystemOnce;
    use bevy::prelude::AssetApp;
    use bevy::prelude::Mat3;
    use bevy::prelude::Mat4;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::World;
    use bevy::window::WindowRef;

    use super::*;
    use crate::Mm;
    use crate::cascade;
    use crate::cascade::CascadeEntityCommandsExt;
    use crate::cascade::SdfMaterial;
    use crate::layout::CornerRadius;
    use crate::layout::El;
    use crate::layout::LayoutBuilder;
    use crate::layout::LayoutTree;
    use crate::layout::Sizing;
    use crate::layout::TextDimensions;
    use crate::layout::TextMeasure;
    use crate::layout::TextStyle;
    use crate::layout::Unit;
    use crate::panel::DiegeticPanelCommands;
    use crate::panel::HeadlessLayoutPlugin;
    use crate::render;
    use crate::text::DiegeticTextMeasurer;

    /// Logical viewport size seeded into every captured test camera.
    const RAY_VIEWPORT_SIZE: Vec2 = Vec2::new(800.0, 600.0);

    /// Minimal measurer for geometry tests that include text commands.
    fn zero_measurer() -> DiegeticTextMeasurer {
        DiegeticTextMeasurer {
            measure_fn: Arc::new(|_: &str, measure: &TextMeasure| TextDimensions {
                width:       0.0,
                height:      measure.size,
                line_height: measure.size,
            }),
        }
    }

    fn geometry_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_plugins(AssetPlugin::default());
        app.init_asset::<Mesh>();
        render::seed_default_material_cascades(&mut app);
        app.init_resource::<ResolvedSdfSurfaceRegistry>();
        app.insert_resource(zero_measurer());
        app.add_plugins(HeadlessLayoutPlugin);
        app.add_plugins(cascade::cascade_plugin::<SdfMaterial>());
        app.add_systems(PostUpdate, build_panel_geometry);
        app
    }

    /// A full-size background with a full-size backed child on top: two
    /// overlapping backing surfaces at consecutive command indices, identical
    /// in geometry and color.
    fn stacked_backgrounds_tree() -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE),
            |builder| {
                builder.with(
                    El::new()
                        .width(Sizing::GROW)
                        .height(Sizing::GROW)
                        .background(Color::WHITE),
                    |_| {},
                );
            },
        );
        builder.build()
    }

    /// Text-command state used by `text_toggle_tree`.
    #[derive(Clone, Copy)]
    enum TextContentState {
        /// The tree includes one text command after the background.
        Present,
        /// The tree includes only the background command.
        Removed,
    }

    fn text_toggle_tree(text_content_state: TextContentState) -> LayoutTree {
        let mut builder = LayoutBuilder::new(Mm(100.0), Mm(50.0));
        builder.with(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .background(Color::WHITE),
            |builder| {
                if matches!(text_content_state, TextContentState::Present) {
                    builder.text(("Alpha", TextStyle::new(10.0)));
                }
            },
        );
        builder.build()
    }

    /// Test-only snapshot of the `ResolvedSdfSurface` fields that
    /// `PanelGeometryPlugin` writes into `ResolvedSdfSurfaceRegistry`.
    #[derive(Clone, Copy)]
    struct SurfaceSnapshot {
        /// Command identity copied from `ResolvedSdfSurface::command_index`.
        command_index:     CommandIndex,
        /// Draw ordering copied from `ResolvedSdfSurface::draw_depth`.
        draw_depth:        DrawCommandDepth,
        /// Rounded-rectangle half-size copied from `ResolvedSdfSurface::sdf_half_size`.
        sdf_half_size:     Vec2,
        /// Padded mesh half-size copied from `ResolvedSdfSurface::mesh_half_size`.
        mesh_half_size:    Vec2,
        /// Corner radii copied from `ResolvedSdfSurface::corner_radii`.
        corner_radii:      [f32; 4],
        /// Border widths copied from `ResolvedSdfSurface::border_widths`.
        border_widths:     [f32; 4],
        /// Clip rectangle copied from `ResolvedSdfSurface::clip_rect`.
        clip_rect:         Vec4,
        /// Fill role state copied from `ResolvedSdfMaterial::authorship`.
        fill_authorship:   SdfRoleAuthorship,
        /// Fill color copied from `ResolvedSdfMaterial::color`.
        fill_color:        Option<Color>,
        /// Fill source handle copied from `ResolvedSdfMaterial::base_material`.
        fill_material:     Option<AssetId<StandardMaterial>>,
        /// Border role state copied from `ResolvedSdfMaterial::authorship`.
        border_authorship: SdfRoleAuthorship,
        /// Border color copied from `ResolvedSdfMaterial::color`.
        border_color:      Option<Color>,
        /// Border source handle copied from `ResolvedSdfMaterial::base_material`.
        border_material:   Option<AssetId<StandardMaterial>>,
    }

    fn surface_snapshots(app: &App) -> Vec<SurfaceSnapshot> {
        let mut snapshots: Vec<SurfaceSnapshot> = app
            .world()
            .resource::<ResolvedSdfSurfaceRegistry>()
            .surfaces()
            .map(|surface| {
                let resolved = surface.as_resolved(RenderLayers::layer(0), ShadowCasting::Off);
                SurfaceSnapshot {
                    command_index:     resolved.command_index,
                    draw_depth:        resolved.draw_depth,
                    sdf_half_size:     resolved.sdf_half_size,
                    mesh_half_size:    resolved.mesh_half_size,
                    corner_radii:      resolved.corner_radii,
                    border_widths:     resolved.border_widths,
                    clip_rect:         resolved.clip_rect,
                    fill_authorship:   resolved.fill_material.authorship,
                    fill_color:        resolved.fill_material.color,
                    fill_material:     resolved.fill_material.base_material.map(Handle::id),
                    border_authorship: resolved.border_material.authorship,
                    border_color:      resolved.border_material.color,
                    border_material:   resolved.border_material.base_material.map(Handle::id),
                }
            })
            .collect();
        snapshots.sort_by_key(|snapshot| snapshot.command_index);
        snapshots
    }

    fn single_surface_snapshot(app: &App) -> SurfaceSnapshot {
        let snapshots = surface_snapshots(app);
        assert_eq!(
            snapshots.len(),
            1,
            "expected exactly one resolved SDF surface"
        );
        snapshots[0]
    }

    fn rounded_root_tree() -> LayoutTree {
        let builder = LayoutBuilder::with_root(
            El::new()
                .width(Sizing::GROW)
                .height(Sizing::GROW)
                .corner_radius(CornerRadius::all(Mm(4.0)))
                .background(Color::WHITE),
        );
        builder.build()
    }

    fn background_tree(material: Option<Handle<StandardMaterial>>) -> LayoutTree {
        let mut element = El::new()
            .width(Sizing::GROW)
            .height(Sizing::GROW)
            .background(Color::WHITE);
        if let Some(material) = material {
            element = element.material(material);
        }
        LayoutBuilder::with_root(element).build()
    }

    fn test_material(app: &mut App, metallic: f32) -> Handle<StandardMaterial> {
        app.world_mut()
            .resource_mut::<Assets<StandardMaterial>>()
            .add(StandardMaterial {
                metallic,
                ..Default::default()
            })
    }

    #[test]
    fn sdf_corner_radii_resolve_authored_units_before_world_scaling() {
        let mut app = geometry_app();
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .world_height(0.5)
                .with_tree(rounded_root_tree())
                .build()
                .expect("panel should build"),
        );
        app.update();
        app.update();

        let surface = single_surface_snapshot(&app);
        let points_to_world = 0.5 / (50.0 * Unit::Millimeters.to_points());
        let expected_radius = 4.0 * Unit::Millimeters.to_points() * points_to_world;
        let actual_radius = surface.corner_radii[0];

        assert!((actual_radius - expected_radius).abs() < 0.001);
    }

    #[test]
    fn text_toggle_keeps_resolved_sdf_oit_depth_positive() {
        let mut app = geometry_app();
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(text_toggle_tree(TextContentState::Present))
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.update();
        app.update();

        let surface_before = single_surface_snapshot(&app);
        assert_eq!(
            surface_before.draw_depth.oit_depth_offset().get().to_bits(),
            constants::OIT_DEPTH_STEP.to_bits(),
        );

        assert!(
            app.world_mut()
                .commands()
                .set_tree(panel, text_toggle_tree(TextContentState::Removed))
                .is_ok()
        );
        app.update();
        app.update();

        let surface_after = single_surface_snapshot(&app);
        assert_eq!(surface_after.command_index, surface_before.command_index);
        assert_eq!(
            surface_before.draw_depth.oit_depth_offset().get().to_bits(),
            surface_after.draw_depth.oit_depth_offset().get().to_bits(),
        );
        assert_eq!(
            surface_after.draw_depth.oit_depth_offset().get().to_bits(),
            constants::OIT_DEPTH_STEP.to_bits(),
        );
    }

    #[test]
    fn overlapping_backings_share_screen_depth_and_order_on_record_paths() {
        let mut app = geometry_app();
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .with_tree(stacked_backgrounds_tree())
                .build()
                .expect("panel should build"),
        );
        app.update();
        app.update();

        let surfaces = surface_snapshots(&app);
        assert_eq!(
            surfaces.len(),
            2,
            "two overlapping backing surfaces expected"
        );

        let below = &surfaces[0];
        let above = &surfaces[1];
        assert!(below.command_index < above.command_index);

        // Same-z-index surfaces share hardware screen depth. Per-record clip
        // and OIT depth still put the higher command index in front
        // (reverse-Z, positive = closer).
        assert_eq!(
            below
                .draw_depth
                .z_index_rank()
                .screen_depth_bias()
                .get()
                .to_bits(),
            above
                .draw_depth
                .z_index_rank()
                .screen_depth_bias()
                .get()
                .to_bits()
        );
        assert!(
            below.draw_depth.clip_depth_nudge().get() < above.draw_depth.clip_depth_nudge().get()
        );
        assert!(
            below.draw_depth.oit_depth_offset().get() < above.draw_depth.oit_depth_offset().get()
        );
        assert_eq!(
            below.draw_depth.oit_depth_offset().get().to_bits(),
            constants::OIT_DEPTH_STEP.to_bits()
        );
        assert_eq!(
            above.draw_depth.oit_depth_offset().get().to_bits(),
            (2.0 * constants::OIT_DEPTH_STEP).to_bits()
        );

        assert_eq!(below.sdf_half_size, above.sdf_half_size);
        assert_eq!(below.mesh_half_size, above.mesh_half_size);
        assert_eq!(
            below.corner_radii.map(f32::to_bits),
            above.corner_radii.map(f32::to_bits)
        );
        assert_eq!(
            below.border_widths.map(f32::to_bits),
            above.border_widths.map(f32::to_bits)
        );
        assert_eq!(below.clip_rect, above.clip_rect);
        assert_eq!(below.fill_authorship, above.fill_authorship);
        assert_eq!(below.fill_color, above.fill_color);
        assert_eq!(below.fill_material, above.fill_material);
        assert_eq!(below.border_authorship, above.border_authorship);
        assert_eq!(below.border_color, above.border_color);
        assert_eq!(below.border_material, above.border_material);
    }

    #[test]
    fn sdf_surface_uses_panel_resolved_material() {
        let mut app = geometry_app();
        let panel_material = test_material(&mut app, 0.42);
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .material(panel_material.clone())
                .with_tree(background_tree(None))
                .build()
                .expect("panel should build"),
        );
        app.update();
        app.update();

        let surface = single_surface_snapshot(&app);
        assert_eq!(surface.fill_material, Some(panel_material.id()));
        assert_eq!(surface.border_material, Some(panel_material.id()));
    }

    #[test]
    fn sdf_material_override_refreshes_surface_without_layout_change() {
        let mut app = geometry_app();
        let first_material = test_material(&mut app, 0.42);
        let second_material = test_material(&mut app, 0.84);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .material(first_material.clone())
                    .with_tree(background_tree(None))
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.update();
        app.update();

        let surface_before = single_surface_snapshot(&app);
        assert_eq!(surface_before.fill_material, Some(first_material.id()));
        assert_eq!(surface_before.border_material, Some(first_material.id()));

        app.world_mut()
            .commands()
            .entity(panel)
            .override_sdf_material(second_material.clone());
        app.update();

        let surface_after = single_surface_snapshot(&app);
        assert_eq!(surface_after.command_index, surface_before.command_index);
        assert_eq!(surface_after.fill_material, Some(second_material.id()));
        assert_eq!(surface_after.border_material, Some(second_material.id()));
    }

    #[test]
    fn sdf_element_material_wins_over_panel_material() {
        let mut app = geometry_app();
        let panel_material = test_material(&mut app, 0.42);
        let element_material = test_material(&mut app, 0.84);
        app.world_mut().spawn(
            DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .material(panel_material)
                .with_tree(background_tree(Some(element_material.clone())))
                .build()
                .expect("panel should build"),
        );
        app.update();
        app.update();

        let surface = single_surface_snapshot(&app);
        assert_eq!(surface.fill_material, Some(element_material.id()));
        assert_eq!(surface.border_material, Some(element_material.id()));
    }

    #[test]
    fn same_frame_panel_layer_reinsertion_wins_over_removal_event() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins)
            .add_systems(PostUpdate, sync_interaction_mesh_render_layers);
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .layout(|_| {})
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.world_mut()
            .entity_mut(panel)
            .insert(RenderLayers::layer(1));
        let interaction = app
            .world_mut()
            .spawn((
                PanelInteractionMesh::new(Vec2::ONE, Vec2::ZERO),
                PanelOwned::from(panel),
                RenderLayers::layer(1),
            ))
            .id();
        app.update();

        app.world_mut()
            .entity_mut(panel)
            .remove::<RenderLayers>()
            .insert(RenderLayers::layer(4));
        app.update();

        assert_eq!(
            app.world().get::<RenderLayers>(interaction),
            Some(&RenderLayers::layer(4)),
        );

        app.world_mut().entity_mut(panel).remove::<RenderLayers>();
        app.update();

        assert_eq!(
            app.world().get::<RenderLayers>(interaction),
            Some(&RenderLayers::layer(0)),
        );
    }

    #[test]
    fn unsolved_layout_eviction_counts_every_surface_it_drops() {
        let mut app = geometry_app();
        let panel = app
            .world_mut()
            .spawn(
                DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .with_tree(stacked_backgrounds_tree())
                    .build()
                    .expect("panel should build"),
            )
            .id();
        app.update();
        app.update();
        let retained = surface_snapshots(&app).len();
        assert!(retained > 0, "panel should resolve surfaces to evict");

        // Return the panel to the state it holds before its first layout pass.
        // `build_panel_geometry` evicts a panel it cannot resolve, which empties
        // a panel that is still spawned and still visible — the eviction has to
        // be counted or that emptying leaves no trace.
        app.world_mut()
            .entity_mut(panel)
            .insert(ComputedDiegeticPanel::default());
        app.world_mut()
            .run_system_once(build_panel_geometry)
            .expect("geometry system should run");

        let mut registry = app.world_mut().resource_mut::<ResolvedSdfSurfaceRegistry>();
        assert_eq!(registry.surfaces().count(), 0);
        let evicted = registry.take_evictions();
        assert_eq!(evicted.layout_unsolved.get(), retained);
        assert_eq!(evicted.panel_missing.get(), 0);
        assert_eq!(
            registry.take_evictions().layout_unsolved.get(),
            0,
            "taking evictions clears them, so one eviction is never counted twice"
        );
    }

    /// Camera whose computed viewport and projection are seeded directly,
    /// matching what `camera_system` would compute for `RAY_VIEWPORT_SIZE`.
    fn seeded_camera(mut projection: Projection) -> Camera {
        let mut camera = Camera::default();
        camera.computed.target_info = Some(RenderTargetInfo {
            physical_size: RAY_VIEWPORT_SIZE.as_uvec2(),
            scale_factor:  1.0,
        });
        projection.update(RAY_VIEWPORT_SIZE.x, RAY_VIEWPORT_SIZE.y);
        camera.computed.clip_from_view = projection.get_clip_from_view();
        camera
    }

    fn perspective_camera() -> Camera {
        seeded_camera(Projection::Perspective(PerspectiveProjection::default()))
    }

    fn orthographic_camera() -> Camera {
        seeded_camera(Projection::Orthographic(
            OrthographicProjection::default_3d(),
        ))
    }

    fn window_pair() -> (Entity, Entity) {
        let mut world = World::new();
        (world.spawn_empty().id(), world.spawn_empty().id())
    }

    /// Owned inputs for one captured-camera ray projection.
    struct RayScene {
        camera:            Camera,
        camera_transform:  GlobalTransform,
        render_target:     RenderTarget,
        primary_window:    Option<Entity>,
        captured_target:   NormalizedRenderTarget,
        panel:             DiegeticPanel,
        panel_transform:   GlobalTransform,
        viewport_position: Vec2,
    }

    impl RayScene {
        /// A camera on a secondary window at `(0, 0, 5)` facing a world panel
        /// at the origin, with the pointer at the viewport center.
        fn secondary_window(camera: Camera) -> Self {
            let (window, _) = window_pair();
            let render_target = RenderTarget::Window(WindowRef::Entity(window));
            let captured_target = render_target
                .normalize(None)
                .expect("entity window targets normalize without a primary window");
            Self {
                camera,
                camera_transform: GlobalTransform::from(
                    Transform::from_xyz(0.0, 0.0, 5.0).looking_at(Vec3::ZERO, Vec3::Y),
                ),
                render_target,
                primary_window: None,
                captured_target,
                panel: DiegeticPanel::world()
                    .size(Mm(100.0), Mm(50.0))
                    .world_height(0.5)
                    .build()
                    .expect("panel should build"),
                panel_transform: GlobalTransform::IDENTITY,
                viewport_position: RAY_VIEWPORT_SIZE * 0.5,
            }
        }

        fn project_with_live_target(
            &self,
            render_target: Option<&RenderTarget>,
        ) -> Result<Vec2, FlatPanelRayError> {
            let captured_camera_ray = CapturedCameraRay {
                camera: &self.camera,
                camera_transform: &self.camera_transform,
                render_target,
                primary_window: self.primary_window,
                captured_target: &self.captured_target,
                viewport_position: self.viewport_position,
            };
            project_flat_panel_ray_hit(&captured_camera_ray, &self.panel, &self.panel_transform)
        }

        fn project(&self) -> Result<Vec2, FlatPanelRayError> {
            self.project_with_live_target(Some(&self.render_target))
        }
    }

    #[track_caller]
    fn assert_layout_close(actual: Vec2, expected: Vec2) {
        assert!(
            (actual - expected).length() < 0.05,
            "expected {expected}, got {actual}"
        );
    }

    #[test]
    fn perspective_and_orthographic_center_rays_map_through_project_flat_panel_hit() {
        for camera in [perspective_camera(), orthographic_camera()] {
            let scene = RayScene::secondary_window(camera);
            let projected = scene.project().expect("center ray should hit the panel");
            // The viewport-center ray meets the panel plane at the world
            // origin, so the ray path must agree with the world-hit converter.
            let expected = project_flat_panel_hit(Vec3::ZERO, &scene.panel, &scene.panel_transform)
                .expect("panel scale should convert the plane origin");
            assert_layout_close(projected, expected);
        }
    }

    #[test]
    fn off_center_orthographic_ray_projects_the_offset_world_hit() {
        let mut scene = RayScene::secondary_window(orthographic_camera());
        // `OrthographicProjection::default_3d` window-size scaling maps one
        // viewport pixel to one world unit; viewport y grows downward while
        // world y grows upward.
        scene.viewport_position = RAY_VIEWPORT_SIZE.mul_add(Vec2::splat(0.5), Vec2::new(10.0, 5.0));
        let projected = scene.project().expect("offset ray should hit the panel");
        let expected = project_flat_panel_hit(
            Vec3::new(10.0, -5.0, 0.0),
            &scene.panel,
            &scene.panel_transform,
        )
        .expect("panel scale should convert the offset hit");
        assert_layout_close(projected, expected);
    }

    #[test]
    fn front_and_back_rays_project_the_same_layout_coordinates() {
        let front = RayScene::secondary_window(perspective_camera());
        let mut back = RayScene::secondary_window(perspective_camera());
        back.camera_transform = GlobalTransform::from(
            Transform::from_xyz(0.0, 0.0, -5.0).looking_at(Vec3::ZERO, Vec3::Y),
        );

        let front_hit = front.project().expect("front ray should hit the panel");
        let back_hit = back
            .project()
            .expect("back ray should hit the two-sided panel plane");
        assert_layout_close(front_hit, back_hit);
    }

    #[test]
    fn primary_window_target_normalizes_and_matches_capture() {
        let (window, _) = window_pair();
        let mut scene = RayScene::secondary_window(perspective_camera());
        scene.render_target = RenderTarget::Window(WindowRef::Primary);
        scene.primary_window = Some(window);
        scene.captured_target = scene
            .render_target
            .normalize(scene.primary_window)
            .expect("primary window targets normalize with a primary window");
        assert!(scene.project().is_ok());

        // Normalization equates the two spellings of the same window: a live
        // entity reference still matches the captured primary reference.
        let live_entity_target = RenderTarget::Window(WindowRef::Entity(window));
        assert!(
            scene
                .project_with_live_target(Some(&live_entity_target))
                .is_ok()
        );
    }

    #[test]
    fn missing_unresolvable_and_mismatched_targets_reject_projection() {
        let (live_window, captured_window) = window_pair();
        let mut scene = RayScene::secondary_window(perspective_camera());
        assert_eq!(
            scene.project_with_live_target(None),
            Err(FlatPanelRayError::TargetMissing),
        );

        let primary_target = RenderTarget::Window(WindowRef::Primary);
        assert_eq!(
            scene.project_with_live_target(Some(&primary_target)),
            Err(FlatPanelRayError::TargetUnresolvable),
        );

        scene.captured_target = RenderTarget::Window(WindowRef::Entity(captured_window))
            .normalize(None)
            .expect("entity window targets normalize without a primary window");
        let moved_target = RenderTarget::Window(WindowRef::Entity(live_window));
        assert_eq!(
            scene.project_with_live_target(Some(&moved_target)),
            Err(FlatPanelRayError::TargetMismatched),
        );
    }

    #[test]
    fn camera_without_computed_viewport_rejects_projection() {
        let mut scene = RayScene::secondary_window(perspective_camera());
        scene.camera = Camera::default();
        assert_eq!(scene.project(), Err(FlatPanelRayError::CameraUnavailable));
    }

    #[test]
    fn non_invertible_panel_transform_rejects_projection() {
        let mut scene = RayScene::secondary_window(perspective_camera());
        scene.panel_transform =
            GlobalTransform::from(Transform::from_scale(Vec3::new(1.0, 0.0, 1.0)));
        assert_eq!(
            scene.project(),
            Err(FlatPanelRayError::TransformNonInvertible),
        );
    }

    #[test]
    fn non_finite_panel_transform_rejects_projection() {
        // A non-finite translation keeps the linear part invertible
        // (determinant 1), so only the affine finiteness guard rejects it; a
        // non-finite scale is also caught by the determinant guard.
        for transform in [
            Transform::from_translation(Vec3::new(f32::NAN, 0.0, 0.0)),
            Transform::from_scale(Vec3::new(f32::INFINITY, 1.0, 1.0)),
        ] {
            let mut scene = RayScene::secondary_window(perspective_camera());
            scene.panel_transform = GlobalTransform::from(transform);
            assert_eq!(
                scene.project(),
                Err(FlatPanelRayError::TransformNonInvertible),
            );
        }
    }

    #[test]
    fn ray_parallel_to_the_panel_plane_rejects_projection() {
        let mut scene = RayScene::secondary_window(perspective_camera());
        // An exact 90-degree rotation about Y turns the panel plane parallel
        // to the camera's -Z center ray; a trigonometric rotation would leave
        // a residual normal component and a finite plane crossing.
        scene.panel_transform = GlobalTransform::from(Mat4::from_mat3(Mat3::from_cols(
            Vec3::new(0.0, 0.0, -1.0),
            Vec3::Y,
            Vec3::X,
        )));
        scene.camera_transform = GlobalTransform::from(Transform::from_xyz(3.0, 0.0, 5.0));
        assert_eq!(scene.project(), Err(FlatPanelRayError::RayParallel));
    }

    #[test]
    fn plane_crossing_behind_the_camera_rejects_projection() {
        let mut scene = RayScene::secondary_window(perspective_camera());
        // The camera sits at z = -5 facing -Z, so the panel plane at z = 0
        // lies behind it.
        scene.camera_transform = GlobalTransform::from(Transform::from_xyz(0.0, 0.0, -5.0));
        assert_eq!(scene.project(), Err(FlatPanelRayError::HitBehindCamera));
    }

    #[test]
    fn invalid_panel_scale_rejects_the_world_hit_conversion() {
        let mut scene = RayScene::secondary_window(perspective_camera());
        scene.panel = DiegeticPanel::world()
            .size(Mm(100.0), Mm(50.0))
            .world_height(0.0)
            .build()
            .expect("panel should build");
        assert_eq!(
            project_flat_panel_hit(Vec3::ZERO, &scene.panel, &scene.panel_transform),
            None,
        );
        assert_eq!(scene.project(), Err(FlatPanelRayError::ScaleInvalid));
    }

    #[test]
    fn non_finite_panel_scale_rejects_the_world_hit_conversion() {
        for world_height in [f32::NAN, f32::INFINITY] {
            let mut scene = RayScene::secondary_window(perspective_camera());
            scene.panel = DiegeticPanel::world()
                .size(Mm(100.0), Mm(50.0))
                .world_height(world_height)
                .build()
                .expect("panel should build");
            assert_eq!(
                project_flat_panel_hit(Vec3::ZERO, &scene.panel, &scene.panel_transform),
                None,
            );
            assert_eq!(scene.project(), Err(FlatPanelRayError::ScaleInvalid));
        }
    }
}
