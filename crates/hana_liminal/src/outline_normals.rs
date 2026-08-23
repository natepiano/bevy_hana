use std::collections::HashMap;

use bevy::mesh::Indices;
use bevy::mesh::PrimitiveTopology;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::Add;
use bevy::prelude::AssetId;
use bevy::prelude::Assets;
use bevy::prelude::Mesh;
use bevy::prelude::Mesh3d;
use bevy::prelude::On;
use bevy::prelude::Query;
use bevy::prelude::ResMut;
use bevy::prelude::Vec3;
use bevy::prelude::With;
use hana_kana::ToUsize;

use super::Outline;
use super::constants::ATTRIBUTE_OUTLINE_NORMAL;
use super::constants::DEGENERATE_EDGE_THRESHOLD;
use super::constants::OUTLINE_NORMAL_COSINE_CLAMP_MAX;
use super::constants::OUTLINE_NORMAL_COSINE_CLAMP_MIN;
use super::constants::TRIANGLE_VERTEX_COUNT;

/// Computes angle-weighted smoothed outline normals and stores them as
/// [`ATTRIBUTE_OUTLINE_NORMAL`] on the mesh.
///
/// Vertices at the same position (but with different normals or UVs due to
/// hard edges or UV seams) receive the same outline normal, producing a
/// continuous silhouette.
///
/// Only operates on `TriangleList` meshes with positions. Returns silently
/// for other topologies.
///
/// Inspired by `bevy_mod_outline`'s `generate_outline_normals()`.
pub fn generate_outline_normals(mesh: &mut Mesh) {
    if mesh.primitive_topology() != PrimitiveTopology::TriangleList {
        return;
    }

    let Some(positions) = mesh
        .attribute(Mesh::ATTRIBUTE_POSITION)
        .and_then(VertexAttributeValues::as_float3)
    else {
        return;
    };

    let index_count = mesh.indices().map_or(positions.len(), Indices::len);
    let triangle_vertex_count = TRIANGLE_VERTEX_COUNT.to_usize();
    if !index_count.is_multiple_of(triangle_vertex_count) {
        return;
    }

    // Accumulate angle-weighted face normals per unique position.
    // Key by `f32::to_bits()` for exact position matching (no floating-point tolerance).
    let mut accumulated_normals: HashMap<[u32; 3], Vec3> = HashMap::new();

    let triangle_count = index_count / triangle_vertex_count;
    for triangle in 0..triangle_count {
        let (i0, i1, i2) = triangle_indices(mesh, positions.len(), triangle);

        let p0 = Vec3::from(positions[i0]);
        let p1 = Vec3::from(positions[i1]);
        let p2 = Vec3::from(positions[i2]);

        let e01 = p1 - p0;
        let e02 = p2 - p0;
        let face_normal = e01.cross(e02).normalize_or_zero();

        // Accumulate for each vertex, weighted by the angle at that vertex.
        accumulate_weighted_normal(
            &mut accumulated_normals,
            positions[i0],
            face_normal,
            e01,
            e02,
        );
        accumulate_weighted_normal(
            &mut accumulated_normals,
            positions[i1],
            face_normal,
            p2 - p1,
            p0 - p1,
        );
        accumulate_weighted_normal(
            &mut accumulated_normals,
            positions[i2],
            face_normal,
            p0 - p2,
            p1 - p2,
        );
    }

    // Normalize all accumulated normals.
    for normal in accumulated_normals.values_mut() {
        *normal = normal.normalize_or_zero();
    }

    // Build per-vertex outline normals by looking up each vertex's position.
    let normals: Vec<[f32; 3]> = positions
        .iter()
        .map(|position| {
            let key = position_key(*position);
            accumulated_normals
                .get(&key)
                .copied()
                .unwrap_or(Vec3::Y)
                .into()
        })
        .collect();

    mesh.insert_attribute(ATTRIBUTE_OUTLINE_NORMAL, normals);
}

/// When `Outline` is added to an entity that has `Mesh3d`, generate outline
/// normals on the mesh asset if not already present.
pub(crate) fn generate_normals_on_outline_added(
    added: On<Add, Outline>,
    mesh_query: Query<&Mesh3d>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let entity = added.entity;
    let Ok(mesh3d) = mesh_query.get(entity) else {
        return;
    };
    generate_normals_for_handle(mesh3d.id(), &mut meshes);
}

/// When `Mesh3d` is added to an entity that already has `Outline`, generate
/// outline normals on the mesh asset if not already present.
pub(crate) fn generate_normals_on_mesh_added(
    added: On<Add, Mesh3d>,
    mesh_query: Query<&Mesh3d>,
    outline_query: Query<(), With<Outline>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    let entity = added.entity;
    if !outline_query.contains(entity) {
        return;
    }
    let Ok(mesh3d) = mesh_query.get(entity) else {
        return;
    };
    generate_normals_for_handle(mesh3d.id(), &mut meshes);
}

fn generate_normals_for_handle(id: AssetId<Mesh>, meshes: &mut Assets<Mesh>) {
    let Some(mut mesh) = meshes.get_mut(id) else {
        return;
    };
    if mesh.attribute(ATTRIBUTE_OUTLINE_NORMAL).is_some() {
        return;
    }
    generate_outline_normals(&mut mesh);
}

const fn position_key(position: [f32; 3]) -> [u32; 3] {
    [
        position[0].to_bits(),
        position[1].to_bits(),
        position[2].to_bits(),
    ]
}

fn accumulate_weighted_normal(
    accumulated_normals: &mut HashMap<[u32; 3], Vec3>,
    position: [f32; 3],
    face_normal: Vec3,
    edge_a: Vec3,
    edge_b: Vec3,
) {
    let len_a = edge_a.length();
    let len_b = edge_b.length();
    if len_a < DEGENERATE_EDGE_THRESHOLD || len_b < DEGENERATE_EDGE_THRESHOLD {
        return;
    }
    let cos_angle = (edge_a / len_a).dot(edge_b / len_b).clamp(
        OUTLINE_NORMAL_COSINE_CLAMP_MIN,
        OUTLINE_NORMAL_COSINE_CLAMP_MAX,
    );
    let angle = cos_angle.acos();

    let entry = accumulated_normals
        .entry(position_key(position))
        .or_insert(Vec3::ZERO);
    *entry += face_normal * angle;
}

fn triangle_indices(mesh: &Mesh, vertex_count: usize, triangle: usize) -> (usize, usize, usize) {
    let base = triangle * TRIANGLE_VERTEX_COUNT.to_usize();
    if let Some(indices) = mesh.indices() {
        let mut iter = indices.iter().skip(base);
        let i0 = iter.next().unwrap_or(0);
        let i1 = iter.next().unwrap_or(0);
        let i2 = iter.next().unwrap_or(0);
        (i0, i1, i2)
    } else {
        let i0 = base.min(vertex_count.saturating_sub(1));
        let i1 = (base + 1).min(vertex_count.saturating_sub(1));
        let i2 = (base + 2).min(vertex_count.saturating_sub(1));
        (i0, i1, i2)
    }
}
