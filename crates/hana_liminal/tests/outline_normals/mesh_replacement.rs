// Included inside outline_normals::tests to exercise the production observers.

use bevy::asset::RenderAssetUsages;

#[test]
fn replacing_an_outlined_entitys_mesh_gives_the_new_mesh_outline_normals() {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.add_observer(generate_normals_on_outline_added);
    world.add_observer(generate_normals_on_mesh_added);
    let first = world.resource_mut::<Assets<Mesh>>().add(Cuboid::default());
    let replacement = world
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(2.0, 2.0, 2.0));
    let entity = world
        .spawn((Mesh3d(first), Outline::jump_flood(1.0).build()))
        .id();

    // A resized body swaps in a new mesh: an insert over the old one, not an add.
    world.entity_mut(entity).insert(Mesh3d(replacement.clone()));
    world.flush();

    let mesh = world
        .resource::<Assets<Mesh>>()
        .get(&replacement)
        .expect("the replacement mesh");
    assert!(
        mesh.attribute(ATTRIBUTE_OUTLINE_NORMAL).is_some(),
        "an outlined entity's replacement mesh must get outline normals"
    );
}

#[test]
fn u16_indices_keep_the_existing_outline_normals() {
    let mut mesh = indexed_triangles(Indices::U16(vec![2, 0, 1, 5, 3, 4]));

    generate_outline_normals(&mut mesh);

    assert_outline_normals(
        &mesh,
        &[
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 1.0, 0.0],
        ],
    );
}

#[test]
fn u32_indices_keep_the_existing_outline_normals() {
    let mut mesh = indexed_triangles(Indices::U32(vec![1, 0, 2, 4, 3, 5]));

    generate_outline_normals(&mut mesh);

    assert_outline_normals(
        &mesh,
        &[
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 1.0, 0.0],
        ],
    );
}

#[test]
fn unindexed_triangles_keep_the_existing_outline_normals() {
    let mut mesh = two_separate_triangles();

    generate_outline_normals(&mut mesh);

    assert_outline_normals(
        &mesh,
        &[
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, 1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
            [0.0, 0.0, -1.0],
        ],
    );
}

#[test]
fn hundreds_of_thousands_of_indices_get_outline_normals() {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
    );
    mesh.insert_indices(Indices::U32(
        [0, 1, 2].into_iter().cycle().take(300_000).collect(),
    ));

    generate_outline_normals(&mut mesh);

    assert_outline_normals(&mesh, &[[0.0, 0.0, 1.0]; 3]);
}

#[test]
fn incomplete_index_triangle_leaves_outline_normals_absent() {
    let mut mesh = two_separate_triangles();
    mesh.insert_indices(Indices::U16(vec![0, 1]));

    generate_outline_normals(&mut mesh);

    assert!(mesh.attribute(ATTRIBUTE_OUTLINE_NORMAL).is_none());
}

#[test]
#[should_panic(expected = "index out of bounds")]
fn out_of_range_index_still_panics() {
    let mut mesh = two_separate_triangles();
    mesh.insert_indices(Indices::U32(vec![0, 1, 9]));

    generate_outline_normals(&mut mesh);
}

fn two_separate_triangles() -> Mesh {
    let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default());
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [3.0, 0.0, 0.0],
        ],
    );
    mesh
}

fn indexed_triangles(indices: Indices) -> Mesh {
    let mut mesh = two_separate_triangles();
    mesh.insert_attribute(
        Mesh::ATTRIBUTE_POSITION,
        vec![
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            [2.0, 0.0, 0.0],
            [2.0, 1.0, 0.0],
            [3.0, 0.0, 0.0],
            [10.0, 10.0, 10.0],
        ],
    );
    mesh.insert_indices(indices);
    mesh
}

fn assert_outline_normals(mesh: &Mesh, expected: &[[f32; 3]]) {
    let actual = mesh
        .attribute(ATTRIBUTE_OUTLINE_NORMAL)
        .and_then(VertexAttributeValues::as_float3)
        .expect("outline normals");
    let actual_bits: Vec<_> = actual
        .iter()
        .map(|normal| normal.map(f32::to_bits))
        .collect();
    let expected_bits: Vec<_> = expected
        .iter()
        .map(|normal| normal.map(f32::to_bits))
        .collect();
    assert_eq!(actual_bits, expected_bits);
}
