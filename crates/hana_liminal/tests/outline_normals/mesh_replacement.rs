// Included inside outline_normals::tests to exercise the production observers.

use bevy::asset::RenderAssetUsages;
use bevy::prelude::ChildOf;

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

#[test]
fn a_mesh_inserted_after_its_outline_gets_normals_without_readding_the_outline() {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.add_observer(generate_normals_on_outline_added);
    world.add_observer(generate_normals_on_mesh_added);
    let entity = world.spawn(Outline::jump_flood(1.0).build()).id();
    let mesh = world
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::new(2.0, 1.0, 0.2));
    assert!(
        world
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .expect("mesh asset")
            .attribute(ATTRIBUTE_OUTLINE_NORMAL)
            .is_none()
    );
    world.entity_mut(entity).insert(Mesh3d(mesh.clone()));
    world.flush();
    assert!(
        world
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .expect("late mesh asset")
            .attribute(ATTRIBUTE_OUTLINE_NORMAL)
            .is_some()
    );
}

#[test]
fn reparenting_an_existing_panel_under_an_outline_prepares_its_mesh_normals() {
    let mut world = World::new();
    world.init_resource::<Assets<Mesh>>();
    world.add_observer(generate_normals_on_outline_added);
    world.add_observer(generate_normals_on_mesh_added);
    world.add_observer(crate::propagation::propagate_outline_on_child_added);
    world.add_observer(crate::propagation::propagate_outline_on_mesh_added);
    let outside = world.spawn_empty().id();
    let selected = world.spawn(Outline::jump_flood(3.0).build()).id();
    let panel = world.spawn(ChildOf(outside)).id();
    let mesh = world.resource_mut::<Assets<Mesh>>().add(Cuboid::default());
    let leaf = world.spawn((Mesh3d(mesh.clone()), ChildOf(panel))).id();
    assert!(world.get::<Outline>(leaf).is_none());
    assert!(
        world
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .expect("unselected panel mesh")
            .attribute(ATTRIBUTE_OUTLINE_NORMAL)
            .is_none()
    );
    world.entity_mut(panel).insert(ChildOf(selected));
    world.flush();
    assert!(
        world.get::<Outline>(leaf).is_some(),
        "moving an existing panel into selection propagates its outline"
    );
    assert!(
        world
            .resource::<Assets<Mesh>>()
            .get(&mesh)
            .expect("selected panel mesh")
            .attribute(ATTRIBUTE_OUTLINE_NORMAL)
            .is_some(),
        "inherited outlines need prepared mesh normals too"
    );
}

#[test]
fn sharing_a_prepared_mesh_does_not_publish_redundant_asset_modifications() {
    use bevy::asset::AssetApp;
    use bevy::asset::AssetEvent;
    use bevy::asset::AssetPlugin;
    use bevy::prelude::App;
    use bevy::prelude::Messages;
    use bevy::prelude::MinimalPlugins;

    let mut app = App::new();
    app.add_plugins((MinimalPlugins, AssetPlugin::default()))
        .init_asset::<Mesh>()
        .add_observer(generate_normals_on_outline_added)
        .add_observer(generate_normals_on_mesh_added);
    let mesh = app
        .world_mut()
        .resource_mut::<Assets<Mesh>>()
        .add(Cuboid::default());
    let first = app
        .world_mut()
        .spawn((Mesh3d(mesh.clone()), Outline::jump_flood(1.0).build()))
        .id();
    app.update();
    let initial: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<AssetEvent<Mesh>>>()
        .drain()
        .collect();
    assert!(
        initial
            .iter()
            .any(|event| matches!(event, AssetEvent::Modified { id } if *id == mesh.id())),
        "preparing normals changes the real asset on its first use"
    );

    app.world_mut()
        .spawn((Mesh3d(mesh.clone()), Outline::jump_flood(2.0).build()));
    app.world_mut()
        .entity_mut(first)
        .insert(Mesh3d(mesh.clone()));
    app.update();
    let repeated: Vec<_> = app
        .world_mut()
        .resource_mut::<Messages<AssetEvent<Mesh>>>()
        .drain()
        .collect();
    assert!(
        !repeated
            .iter()
            .any(|event| matches!(event, AssetEvent::Modified { id } if *id == mesh.id())),
        "shared/reinserted prepared meshes must not trigger GPU reuploads"
    );
}
