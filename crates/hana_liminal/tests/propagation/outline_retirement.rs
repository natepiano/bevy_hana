// Included inside propagation::tests to exercise the production observer fixture.

#[test]
fn outline_propagation_tolerates_a_target_retired_earlier_in_the_same_flush() {
    let mut world = world_with_observers();
    let root = world.spawn_empty().id();
    let retired = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
    let survivor = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_by_callback = observed.clone();
    world.add_observer(move |added: On<Add, Outline>, meshes: Query<&Mesh3d>| {
        if added.entity == root {
            observed_by_callback.store(meshes.get(retired).is_ok(), std::sync::atomic::Ordering::SeqCst);
        }
    });

    // The observer walks while the child still exists; its deferred insert follows
    // the already queued despawn when EntityWorldMut flushes observer commands.
    world.commands().entity(retired).despawn();
    world.entity_mut(root).insert(outline());
    world.flush();

    assert!(observed.load(std::sync::atomic::Ordering::SeqCst), "the retired mesh must exist when propagation selects its targets");
    assert!(world.get_entity(retired).is_err());
    let propagated = world.get::<Outline>(survivor).expect("surviving siblings still receive the outline");
    assert_eq!(propagated.group_source, Some(root));
    assert!((propagated.width - OUTLINE_WIDTH).abs() < f32::EPSILON);
}
