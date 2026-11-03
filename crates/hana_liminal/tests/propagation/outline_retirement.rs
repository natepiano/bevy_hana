// Included inside propagation::tests to exercise the production observer fixture.

#[test]
fn removing_a_self_grouped_source_retires_a_copy_that_lost_its_mesh() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    world.entity_mut(source).insert(
        Outline::jump_flood(OUTLINE_WIDTH)
            .with_group(source)
            .build(),
    );
    let mesh = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    assert_eq!(
        world.get::<Outline>(mesh).map(|copy| copy.group_source),
        Some(Some(source))
    );
    world.entity_mut(mesh).remove::<Mesh3d>();

    world.entity_mut(source).remove::<Outline>();

    assert!(world.get::<Outline>(mesh).is_none());
}

#[test]
fn retiring_a_source_preserves_an_outline_replaced_before_cleanup_applies() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let mesh = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(source).insert(outline());
    world.commands().entity(mesh).insert(outline());

    world.entity_mut(source).remove::<Outline>();

    assert_eq!(
        world
            .get::<Outline>(mesh)
            .map(|outline| outline.group_source),
        Some(None)
    );
}

#[test]
fn removing_a_source_outline_retires_copies_after_mesh_loss_or_reparenting() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let retired_mesh = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    let moved_mesh = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    let survivor = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    let independent = world.spawn_empty().id();
    let independent_mesh = world
        .spawn((Mesh3d(Handle::default()), ChildOf(independent)))
        .id();
    world.entity_mut(source).insert(outline());
    world.entity_mut(independent).insert(outline());
    for entity in [retired_mesh, moved_mesh, survivor] {
        assert_eq!(
            world.get::<Outline>(entity).map(|copy| copy.group_source),
            Some(Some(source))
        );
    }
    world.entity_mut(retired_mesh).remove::<Mesh3d>();
    world.entity_mut(moved_mesh).remove::<ChildOf>();

    world.entity_mut(source).remove::<Outline>();

    let remaining = world
        .query::<(Entity, &Outline)>()
        .iter(&world)
        .filter(|(_, copy)| copy.group_source == Some(source))
        .map(|(entity, _)| entity)
        .collect::<Vec<_>>();
    assert!(
        remaining.is_empty(),
        "retired source still owns copies: {remaining:?}"
    );
    assert!(world.get::<Outline>(independent).is_some());
    assert_eq!(
        world
            .get::<Outline>(independent_mesh)
            .map(|copy| copy.group_source),
        Some(Some(independent))
    );
}

#[test]
fn outline_propagation_tolerates_a_target_retired_earlier_in_the_same_flush() {
    let mut world = world_with_observers();
    let root = world.spawn_empty().id();
    let retired = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
    let survivor = world.spawn((Mesh3d(Handle::default()), ChildOf(root))).id();
    let observed = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed_by_callback = observed.clone();
    world.add_observer(move |added: On<Add<Outline>>, meshes: Query<&Mesh3d>| {
        if added.entity == root {
            observed_by_callback.store(
                meshes.get(retired).is_ok(),
                std::sync::atomic::Ordering::SeqCst,
            );
        }
    });

    // The observer walks while the child still exists; its deferred insert follows
    // the already queued despawn when EntityWorldMut flushes observer commands.
    world.commands().entity(retired).despawn();
    world.entity_mut(root).insert(outline());
    world.flush();

    assert!(
        observed.load(std::sync::atomic::Ordering::SeqCst),
        "the retired mesh must exist when propagation selects its targets"
    );
    assert!(world.get_entity(retired).is_err());
    let propagated = world
        .get::<Outline>(survivor)
        .expect("surviving siblings still receive the outline");
    assert_eq!(propagated.group_source, Some(root));
    assert!((propagated.width - OUTLINE_WIDTH).abs() < f32::EPSILON);
}

#[test]
fn retiring_a_source_preserves_independently_authored_outlines_in_its_group() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(source).insert(outline());
    let authored = world
        .spawn(
            Outline::jump_flood(OUTLINE_WIDTH)
                .with_group(source)
                .build(),
        )
        .id();
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_some());
    assert!(world.get::<PropagatedOutlineOwnership>(authored).is_none());

    world.entity_mut(source).remove::<Outline>();

    assert!(world.get::<Outline>(copy).is_none());
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_none());
    assert_eq!(
        world
            .get::<Outline>(authored)
            .map(|outline| outline.group_source),
        Some(Some(source))
    );
}

#[test]
fn retiring_a_source_preserves_its_queued_self_grouped_replacement() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    world.entity_mut(source).insert(
        Outline::jump_flood(OUTLINE_WIDTH)
            .with_group(source)
            .build(),
    );
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.commands().entity(source).insert(
        Outline::jump_flood(OUTLINE_WIDTH * 2.0)
            .with_group(source)
            .build(),
    );

    world.entity_mut(source).remove::<Outline>();

    let replacement = world
        .get::<Outline>(source)
        .expect("the queued replacement survives");
    assert_eq!(replacement.group_source, Some(source));
    assert!(OUTLINE_WIDTH.mul_add(-2.0, replacement.width).abs() < f32::EPSILON);
    assert!(world.get::<Outline>(copy).is_none());
}

#[test]
fn retiring_a_source_preserves_a_copy_replaced_with_an_authored_outline_in_the_same_group() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(source).insert(outline());
    world.commands().entity(copy).insert(
        Outline::jump_flood(OUTLINE_WIDTH * 2.0)
            .with_group(source)
            .build(),
    );

    world.entity_mut(source).remove::<Outline>();

    let replacement = world
        .get::<Outline>(copy)
        .expect("the authored replacement survives");
    assert!(OUTLINE_WIDTH.mul_add(-2.0, replacement.width).abs() < f32::EPSILON);
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_none());
}

#[test]
fn retiring_a_source_retires_copies_behind_a_new_barrier() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(source).insert(outline());
    world.entity_mut(copy).insert(OutlineBarrier);

    world.entity_mut(source).remove::<Outline>();

    assert!(world.get::<Outline>(copy).is_none());
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_none());
}

#[test]
fn propagated_replacements_keep_their_source_ownership() {
    let mut world = world_with_observers();
    let source = world.spawn(outline()).id();
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(copy).remove::<ChildOf>();
    world.entity_mut(copy).insert(ChildOf(source));

    assert!(matches!(world.get::<PropagatedOutlineOwnership>(copy),
        Some(PropagatedOutlineOwnership::InheritedFrom(owner)) if *owner == source));
    world.entity_mut(source).remove::<Outline>();
    assert!(world.get::<Outline>(copy).is_none());
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_none());
}

#[test]
fn syncing_a_source_preserves_authored_descendant_outlines_in_its_group() {
    let mut world = world_with_observers();
    let source = world.spawn_empty().id();
    let copy = world
        .spawn((Mesh3d(Handle::default()), ChildOf(source)))
        .id();
    world.entity_mut(source).insert(outline());
    world.entity_mut(copy).insert(
        Outline::jump_flood(OUTLINE_WIDTH * 2.0)
            .with_group(source)
            .build(),
    );
    world
        .entity_mut(source)
        .get_mut::<Outline>()
        .expect("source has an outline")
        .width = OUTLINE_WIDTH * 3.0;

    world
        .run_system_once(sync_propagated_outlines)
        .expect("sync system runs");

    let authored = world
        .get::<Outline>(copy)
        .expect("the authored descendant survives sync");
    assert!(OUTLINE_WIDTH.mul_add(-2.0, authored.width).abs() < f32::EPSILON);
    assert!(world.get::<PropagatedOutlineOwnership>(copy).is_none());
}
