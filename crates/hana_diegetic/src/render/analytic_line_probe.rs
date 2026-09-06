//! Standalone analytic-path line probe.
//!
//! Bypasses panel-line resolution and layout entirely: an authored
//! [`AnalyticLine`] component is converted directly into one path-style
//! analytic batch (packed rectangle outline -> atlas buffers -> one instance +
//! one run -> the shared `PathExtendedMaterial`). This is the minimum path that proves
//! the path renderer draws a crisp stroked line, and a reference to compare
//! the panel-line ruler route against.

use bevy::asset::RenderAssetUsages;
use bevy::camera::visibility::NoFrustumCulling;
use bevy::light::NotShadowCaster;
use bevy::mesh::Indices;
use bevy::prelude::AlphaMode;
use bevy::prelude::App;
use bevy::prelude::Assets;
use bevy::prelude::Color;
use bevy::prelude::Commands;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Handle;
use bevy::prelude::IntoScheduleConfigs;
use bevy::prelude::Mat4;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::MeshMaterial3d;
use bevy::prelude::Plugin;
use bevy::prelude::PostUpdate;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::StandardMaterial;
use bevy::prelude::Transform;
use bevy::prelude::Vec2;
use bevy::prelude::default;
use bevy::render::render_resource::PrimitiveTopology;
use bevy::render::storage::ShaderBuffer;

use super::AntiAlias;
use super::Bounds;
use super::PackedPathRecord;
use super::PathContour;
use super::PathExtendedMaterial;
use super::PathMaterialBuffers;
use super::PathOutline;
use super::PathQuadRecord;
use super::PathRenderRecord;
use super::QuadraticSegment;
use super::RenderMode;
use super::analytic_material_slot_candidate;
use super::analytic_paths;
use super::material_table;
use super::material_table::BatchMaterialTableRegistry;
use super::material_table::FrameMaterialTableBuild;
use super::material_table::MaterialSlotAppend;
use super::material_table::MaterialSlotCandidate;
use super::material_table::MaterialSlotId;
use super::material_table::MaterialSlotInput;
use super::material_table::MaterialTableAppendReady;
use super::material_table::MaterialTableUpdatedToCurrent;
use crate::layout::Lighting;
use crate::layout::Sidedness;

/// Design-space units assigned to the stroke (thin) axis. The thin axis is
/// fixed at this resolution so the anti-aliased edge stays sharp no matter how
/// long or thin the line is, the same way a path stem always packs into ~1000
/// font units regardless of point size.
const STROKE_DESIGN_UNITS: f32 = 128.0;
/// Anti-aliasing fringe added around the outline, in design units. Converted to
/// world units through the same uniform scale used for the outline.
const PADDING_DESIGN_UNITS: f32 = 16.0;
const MIN_EXTENT: f32 = 1.0e-6;

/// Authored standalone analytic line.
///
/// Spawn alongside a [`Transform`] that places the line's plane in the world.
/// `start`/`end` are 2D points in that plane (meters), and `width` is the
/// stroke thickness (meters).
#[derive(Component, Clone, Debug)]
pub struct AnalyticLine {
    /// Start point in the placement plane (meters).
    pub start: Vec2,
    /// End point in the placement plane (meters).
    pub end:   Vec2,
    /// Stroke thickness (meters).
    pub width: f32,
    /// Fill color.
    pub color: Color,
}

impl AnalyticLine {
    /// Creates a 1cm-wide white line between two in-plane points.
    #[must_use]
    pub const fn new(start: Vec2, end: Vec2) -> Self {
        Self {
            start,
            end,
            width: 0.01,
            color: Color::WHITE,
        }
    }

    /// Sets the stroke thickness in meters.
    #[must_use]
    pub const fn width(mut self, width: f32) -> Self {
        self.width = width;
        self
    }

    /// Sets the fill color.
    #[must_use]
    pub const fn color(mut self, color: Color) -> Self {
        self.color = color;
        self
    }
}

/// Marker on the analytic-path batch entity built for an [`AnalyticLine`].
#[derive(Component)]
pub struct AnalyticLineProbe;

/// Registers the standalone analytic-line build system.
pub struct AnalyticLineProbePlugin;

impl Plugin for AnalyticLineProbePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PostUpdate,
            build_analytic_lines
                .after(MaterialTableAppendReady)
                .before(MaterialTableUpdatedToCurrent),
        );
    }
}

/// Rebuilds the analytic batch for every authored line each frame. The probe is
/// diagnostic-only, and a frame-local material slot has to be rewritten even
/// when geometry and placement did not change.
fn build_analytic_lines(
    lines: Query<(Entity, &AnalyticLine, Option<&Transform>)>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<PathExtendedMaterial>>,
    mut storage_buffers: ResMut<Assets<ShaderBuffer>>,
    mut material_table: ResMut<FrameMaterialTableBuild>,
    mut registry: ResMut<BatchMaterialTableRegistry>,
    mut commands: Commands,
) {
    for (entity, line, transform) in &lines {
        let place = transform.copied().unwrap_or_default().to_matrix();
        let base = line_base_material();
        let input = AnalyticLineMaterialSlotInput {
            entity,
            base_material: &base,
            color: line.color,
        };
        let MaterialSlotAppend::Appended(appended) =
            material_table::append_material_slot(material_table.builder_mut(), &input)
        else {
            registry.unregister_path(entity);
            commands
                .entity(entity)
                .remove::<(Mesh3d, MeshMaterial3d<PathExtendedMaterial>)>();
            continue;
        };
        let Some(built) = build_line(
            line,
            place,
            appended.slot,
            base,
            &mut meshes,
            &mut materials,
            &mut storage_buffers,
        ) else {
            registry.unregister_path(entity);
            commands
                .entity(entity)
                .remove::<(Mesh3d, MeshMaterial3d<PathExtendedMaterial>)>();
            continue;
        };
        let material = built.material.clone();
        commands.entity(entity).insert((
            AnalyticLineProbe,
            Mesh3d(built.mesh),
            MeshMaterial3d(material.clone()),
            NoFrustumCulling,
            NotShadowCaster,
        ));
        registry.register_path(entity, material);
    }
}

struct AnalyticLineMaterialSlotInput<'a> {
    entity:        Entity,
    base_material: &'a StandardMaterial,
    color:         Color,
}

impl MaterialSlotInput for AnalyticLineMaterialSlotInput<'_> {
    type Key = Entity;

    fn key(&self) -> Self::Key { self.entity }

    fn material_slot_candidate(&self) -> MaterialSlotCandidate {
        analytic_material_slot_candidate(
            self.base_material,
            self.color,
            AlphaMode::Blend,
            Lighting::Unlit,
            Sidedness::BothSides,
        )
    }
}

struct BuiltLine {
    mesh:     Handle<Mesh>,
    material: Handle<PathExtendedMaterial>,
}

fn build_line(
    line: &AnalyticLine,
    place: Mat4,
    material_slot: MaterialSlotId,
    base_material: StandardMaterial,
    meshes: &mut Assets<Mesh>,
    materials: &mut Assets<PathExtendedMaterial>,
    storage_buffers: &mut Assets<ShaderBuffer>,
) -> Option<BuiltLine> {
    let delta = line.end - line.start;
    let length = delta.length();
    if length <= MIN_EXTENT || line.width <= MIN_EXTENT {
        return None;
    }
    let direction = delta / length;
    let midpoint = (line.start + line.end) * 0.5;

    // Design rectangle: the thin (stroke) axis is fixed at STROKE_DESIGN_UNITS,
    // the long axis scales with the aspect ratio. The design->world map is then
    // a single uniform scale, so padding and UVs stay consistent on both axes.
    let design = Vec2::new(
        length / line.width * STROKE_DESIGN_UNITS,
        STROKE_DESIGN_UNITS,
    );
    // One band per axis: a rectangle has four curves, and at line scale the
    // default 96 bands starve the distance scan (see
    // PANEL_LINE_BAND_TARGET_DESIGN_UNITS).
    let packed = super::build_packed_path(rectangle_outline(design), 1);
    let band_count = u32::try_from(packed.bands().len() / 2).ok()?;
    let path_record = PackedPathRecord::new(
        packed.bounds(),
        0,
        band_count,
        band_count,
        band_count,
        design.x.min(design.y),
    );

    let curves = storage_buffers.add(ShaderBuffer::from(packed.curves().to_vec()));
    let bands = storage_buffers.add(ShaderBuffer::from(packed.bands().to_vec()));
    let path_records = storage_buffers.add(ShaderBuffer::from(vec![path_record]));

    // Local frame: origin at the segment midpoint, +X along the segment, +Y
    // across the stroke. The run transform rotates and places that frame into
    // the world; the inert quad supplies only the pipeline UV defs.
    let design_to_world = line.width / design.y;
    let padding = PADDING_DESIGN_UNITS * design_to_world;
    let half = Vec2::new(length, line.width) * 0.5;
    let instance = PathQuadRecord {
        rect_min:          -half - Vec2::splat(padding),
        rect_size:         half * 2.0 + Vec2::splat(padding * 2.0),
        uv_min:            Vec2::new(
            -PADDING_DESIGN_UNITS / design.x,
            -PADDING_DESIGN_UNITS / design.y,
        ),
        uv_size:           Vec2::new(
            1.0 + 2.0 * PADDING_DESIGN_UNITS / design.x,
            1.0 + 2.0 * PADDING_DESIGN_UNITS / design.y,
        ),
        box_uv_min:        Vec2::ZERO,
        box_uv_size:       Vec2::ONE,
        packed_path_index: 0,
        render_index:      0,
        box_uv_flip_x:     0,
    };

    let run_transform = place
        * Mat4::from_translation(midpoint.extend(0.0))
        * Mat4::from_rotation_z(direction.to_angle());
    let run = PathRenderRecord {
        transform:          run_transform,
        material:           material_slot.into(),
        render_mode:        u32::from(RenderMode::Text),
        clip_depth_nudge:   0.0,
        oit_depth_offset:   0.0,
        // Matches the probe material's supersample + aa_band settings.
        aa_flags:           AntiAlias::Both.aa_flags(),
        text_coverage_bias: 0.0,
        text_em_height:     0.0,
    };

    let instances = storage_buffers.add(ShaderBuffer::from(vec![instance]));
    let run_records = storage_buffers.add(ShaderBuffer::from(vec![run]));
    let mesh = meshes.add(inert_quad_mesh());
    let material = materials.add(PathExtendedMaterial {
        base:      base_material,
        extension: analytic_paths::vertex_pull(
            RenderMode::Text,
            0.0,
            AntiAlias::Both,
            PathMaterialBuffers {
                curves,
                bands,
                path_records,
                instances,
                run_records,
            },
        ),
    });
    Some(BuiltLine { mesh, material })
}

fn rectangle_outline(design: Vec2) -> PathOutline {
    let corners = [
        Vec2::ZERO,
        Vec2::new(design.x, 0.0),
        design,
        Vec2::new(0.0, design.y),
    ];
    PathOutline {
        bounds:   Bounds {
            min: Vec2::ZERO,
            max: design,
        },
        contours: vec![PathContour {
            min_feature:   design.x.min(design.y),
            fade_exponent: 0.0,
            segments:      corners
                .iter()
                .copied()
                .zip(corners.iter().copied().cycle().skip(1))
                .take(corners.len())
                .map(|(start, end)| QuadraticSegment {
                    start,
                    control: (start + end) * 0.5,
                    end,
                })
                .collect(),
        }],
    }
}

fn inert_quad_mesh() -> Mesh {
    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::default(),
    );
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, vec![[0.0_f32; 3]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, vec![[0.0_f32; 2]; 4]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_1, vec![[0.0_f32; 2]; 4]);
    mesh.insert_indices(Indices::U32(vec![0, 3, 2, 0, 2, 1]));
    mesh
}

fn line_base_material() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::WHITE,
        unlit: true,
        double_sided: true,
        cull_mode: None,
        alpha_mode: AlphaMode::Blend,
        ..default()
    }
}

#[cfg(test)]
mod tests {
    use hana_kana::ToF32;
    use hana_kana::ToUsize;

    use super::*;
    use crate::render::CurveRecord;
    use crate::render::PackedPath;

    const EPS: f32 = 1.0e-5;

    fn winding_for_t(curve: &CurveRecord, point: Vec2, t: f32) -> i32 {
        let dy = 2.0 * curve.curve_end.y.mul_add(t, curve.start_delta.w);
        if dy.abs() < EPS {
            return 0;
        }
        // Half-open in y, not t: upward crossings count on t ∈ [0, 1),
        // downward on t ∈ (0, 1] (the shader's grazing-row parity rule).
        let upward = dy > 0.0;
        if upward && !(0.0..1.0).contains(&t) {
            return 0;
        }
        if !upward && (t <= 0.0 || t > 1.0) {
            return 0;
        }
        let linear_x = (2.0 * curve.start_delta.z).mul_add(t, curve.start_delta.x);
        let curve_x = (curve.curve_end.x * t).mul_add(t, linear_x);
        if curve_x <= point.x {
            return 0;
        }
        if upward { 1 } else { -1 }
    }

    fn curve_winding(curve: &CurveRecord, point: Vec2) -> i32 {
        let a = curve.curve_end.y;
        let b = 2.0 * curve.start_delta.w;
        let c = curve.start_delta.y - point.y;
        if a.abs() < EPS {
            if b.abs() < EPS {
                return 0;
            }
            return winding_for_t(curve, point, -c / b);
        }
        let disc = (4.0 * a).mul_add(-c, b * b);
        if disc < 0.0 {
            return 0;
        }
        let root = disc.sqrt();
        winding_for_t(curve, point, (-b - root) / (2.0 * a))
            + winding_for_t(curve, point, (-b + root) / (2.0 * a))
    }

    fn ground_truth_inside(design: Vec2, point: Vec2) -> bool {
        rectangle_outline(design).contours[0]
            .segments
            .iter()
            .map(|segment| curve_winding(&CurveRecord::from(segment), point))
            .sum::<i32>()
            != 0
    }

    /// Mirrors the shader: pick the along-Y band by y, sum winding over only
    /// that band's curves.
    fn banded_inside(packed: &PackedPath, bounds: Bounds, point: Vec2) -> bool {
        let band_count = packed.bands().len() / 2;
        if band_count == 0 {
            return false;
        }
        let size = bounds.max - bounds.min;
        let normalized = ((point.y - bounds.min.y) / size.y.max(EPS)).clamp(0.0, 0.999_999);
        let index = band_index(normalized, band_count);
        let band = packed.bands()[index];
        (0..band.count)
            .map(|offset| curve_winding(&packed.curves()[(band.start + offset).to_usize()], point))
            .sum::<i32>()
            != 0
    }

    fn band_index(normalized: f32, band_count: usize) -> usize {
        let scaled = normalized * band_count.to_f32();
        for index in 0..band_count {
            if scaled < (index + 1).to_f32() {
                return index;
            }
        }
        band_count - 1
    }

    /// Builds the design rect for a line of the given world length/width the same
    /// way `build_line` does.
    fn design_for(length: f32, width: f32) -> Vec2 {
        Vec2::new(length / width * STROKE_DESIGN_UNITS, STROKE_DESIGN_UNITS)
    }

    fn report_fill_holes(length: f32, width: f32) {
        let design = design_for(length, width);
        let packed = super::super::build_packed_path(
            rectangle_outline(design),
            crate::render::DEFAULT_BAND_COUNT,
        );
        let bounds = packed.bounds();
        let cols = 60usize;
        let rows = 24usize;
        let mut holes = 0usize;
        let mut total_inside = 0usize;
        for row in 0..rows {
            let y_factor = (row.to_f32() + 0.5) / rows.to_f32();
            let y = y_factor.mul_add(bounds.max.y - bounds.min.y, bounds.min.y);
            for col in 0..cols {
                let x_factor = (col.to_f32() + 0.5) / cols.to_f32();
                let x = x_factor.mul_add(bounds.max.x - bounds.min.x, bounds.min.x);
                let point = Vec2::new(x, y);
                let gt = ground_truth_inside(design, point);
                let banded = banded_inside(&packed, bounds, point);
                if gt {
                    total_inside += 1;
                }
                if gt != banded {
                    holes += 1;
                }
            }
        }
        println!(
            "length={length} width={width} design={design:?} aspect={:.1} bands={} curves={} \
             inside={total_inside} holes={holes}",
            design.x / design.y,
            packed.bands().len(),
            packed.curves().len(),
        );
        for &index in &[
            0usize,
            (packed.bands().len() / 4),
            (packed.bands().len() / 2),
        ] {
            let band = packed.bands()[index];
            println!(
                "  band[{index}] start={} count={} y=[{:.1},{:.1}]",
                band.start, band.count, band.range_min, band.range_max
            );
        }
        assert_eq!(
            holes, 0,
            "banded fill diverges from ground truth in {holes} cells"
        );
    }

    #[test]
    fn stem_aspect_fills_without_holes() { report_fill_holes(0.26, 0.02); }

    #[test]
    fn ruler_tick_aspect_fills_without_holes() { report_fill_holes(0.05, 0.004); }
}
