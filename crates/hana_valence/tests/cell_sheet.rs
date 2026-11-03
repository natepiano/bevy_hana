//! Sparse-cell strip topology, folding, clearance, and playback coverage.

#![allow(
    clippy::expect_used,
    reason = "tests should fail immediately when required fixture state is absent"
)]
#![allow(clippy::panic, reason = "tests should panic on unexpected values")]

use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::app::TaskPoolPlugin;
use bevy::asset::AssetPlugin;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::math::Quat;
use bevy::math::Vec3;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Transform;
use bevy::time::Time;
use bevy::time::Virtual;
use bevy::transform::TransformPlugin;
use bevy::transform::TransformSystems;
use hana_valence::AnchorResolveDiagnostics;
use hana_valence::AnchorSystems;
use hana_valence::AnchoredTo;
use hana_valence::ArrangementCommandsExt;
use hana_valence::ArrangementProvider;
use hana_valence::CellFoldGroupSelection;
use hana_valence::CellSheet;
use hana_valence::CellSheetError;
use hana_valence::FoldAuthorError;
use hana_valence::FoldGroup;
use hana_valence::FoldGroups;
use hana_valence::FoldPlugin;
use hana_valence::FoldSequence;
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldStage;
use hana_valence::FoldTiming;
use hana_valence::Hinge;
use hana_valence::MemberBinding;
use hana_valence::Members;
use hana_valence::QuadCell;
use hana_valence::RangeCrossings;
use hana_valence::SequenceDirection;
use hana_valence::SequenceDriver;
use hana_valence::SequenceMovement;
use hana_valence::SequencePosition;
use hana_valence::StripFold;
use hana_valence::resolve_anchors;

#[path = "support/exact_fold.rs"]
mod exact_fold;

use exact_fold::FoldModel;

const BEAT: Duration = Duration::from_millis(10);
const CELL_SIZE: f32 = 1.0;
const POSE_TOLERANCE: f32 = 1e-3;

#[expect(
    clippy::cast_precision_loss,
    reason = "test cell indices are small and exactly representable"
)]
fn cell_offset(index: usize) -> f32 { index as f32 * CELL_SIZE }

fn branched_cells() -> Vec<QuadCell> {
    [
        (0, 0),
        (0, 1),
        (0, 2),
        (1, 0),
        (1, 2),
        (2, 0),
        (2, 1),
        (2, 2),
    ]
    .map(|(row, column)| QuadCell::new(row, column))
    .into()
}

fn two_piece_cells() -> Vec<QuadCell> {
    [(0, 0), (0, 1), (1, 0), (1, 1), (4, 5), (4, 6), (5, 5)]
        .map(|(row, column)| QuadCell::new(row, column))
        .into()
}

fn sequence(groups: &FoldGroups) -> Result<FoldSequence, FoldAuthorError> {
    let group = FoldGroup::combine(groups.iter())?;
    let count = group.iter().count();
    let stage = FoldStage::from(group).override_member_timings_with(|index, _| {
        FoldTiming::new(BEAT, bevy::curve::easing::EaseFunction::SmootherStep)
            .with_start_offset(BEAT * u32::try_from(count - index - 1).unwrap_or(u32::MAX))
    });
    Ok(FoldSequenceBuilder::new(FoldTiming::new(
        BEAT,
        bevy::curve::easing::EaseFunction::SmootherStep,
    ))
    .stage(stage)
    .build())
}

fn app() -> App {
    let mut app = App::new();
    app.add_plugins((
        TaskPoolPlugin::default(),
        AssetPlugin::default(),
        TransformPlugin,
        FoldPlugin,
    ))
    .insert_resource(Time::<Virtual>::default())
    .insert_resource(AnchorResolveDiagnostics::default())
    .configure_sets(
        PostUpdate,
        (
            AnchorSystems::FillGeometry,
            AnchorSystems::AnimatePose,
            AnchorSystems::Resolve,
        )
            .chain()
            .before(TransformSystems::Propagate),
    )
    .add_systems(PostUpdate, resolve_anchors.in_set(AnchorSystems::Resolve));
    app
}

struct SpawnedSheet {
    app:        App,
    controller: Entity,
    entities:   HashMap<QuadCell, Entity>,
}

fn spawn_sheet(cells: Vec<QuadCell>) -> SpawnedSheet { spawn_sheet_with(cells, sequence) }

fn spawn_sheet_with<F>(cells: Vec<QuadCell>, sequence_for_groups: F) -> SpawnedSheet
where
    F: Fn(&FoldGroups) -> Result<FoldSequence, FoldAuthorError>,
{
    let sheet = CellSheet::try_new(cells, CELL_SIZE).expect("fixture cells should build");
    let geometry = sheet
        .cell_geometry()
        .expect("fixture geometry should build");
    let mut app = app();
    let entities = sheet
        .cells()
        .iter()
        .copied()
        .map(|cell| {
            let rest =
                Transform::from_xyz(cell_offset(cell.column()), -cell_offset(cell.row()), 0.0);
            let entity = app
                .world_mut()
                .spawn((rest, GlobalTransform::from(rest), geometry.clone()))
                .id();
            (cell, entity)
        })
        .collect::<HashMap<_, _>>();
    let controller = {
        let mut commands = app.world_mut().commands();
        let controller = commands
            .spawn_arrangement_from_members(
                sheet
                    .with_custom_fold_sequence(CellFoldGroupSelection::Strips, sequence_for_groups),
                |cell| {
                    entities
                        .get(cell)
                        .copied()
                        .map_or(MemberBinding::Missing, MemberBinding::Bound)
                },
            )
            .expect("fixture arrangement should build");
        commands.apply_fold_recipe(controller, CellFoldGroupSelection::Strips, StripFold);
        controller
    };
    app.world_mut().flush();
    SpawnedSheet {
        app,
        controller,
        entities,
    }
}

fn simultaneous_sequence(groups: &FoldGroups) -> Result<FoldSequence, FoldAuthorError> {
    let group = FoldGroup::combine(groups.iter())?;
    Ok(FoldSequenceBuilder::new(FoldTiming::new(
        BEAT,
        bevy::curve::easing::EaseFunction::SmootherStep,
    ))
    .stage(FoldStage::from(group))
    .build())
}

fn movement(position: f32, direction: SequenceDirection) -> SequenceMovement {
    SequenceMovement::try_new(
        SequencePosition::try_new(position).expect("fixture position should be valid"),
        direction,
        0,
        RangeCrossings::NONE,
    )
    .expect("fixture movement should be valid")
}

fn drive(sheet: &mut SpawnedSheet, driver: Entity, position: f32) {
    sheet
        .app
        .world_mut()
        .entity_mut(driver)
        .insert(movement(position, SequenceDirection::Backward));
    sheet.app.update();
}

fn translation(sheet: &SpawnedSheet, entity: Entity) -> Vec3 {
    sheet
        .app
        .world()
        .get::<GlobalTransform>(entity)
        .expect("a cell should have a global transform")
        .translation()
}

fn root(sheet: &SpawnedSheet, mut entity: Entity) -> Entity {
    while let Some(anchored) = sheet.app.world().get::<AnchoredTo>(entity) {
        entity = anchored.target();
    }
    entity
}

#[test]
fn cell_sheet_covers_each_cell_once_with_one_root_per_piece() {
    let cells = two_piece_cells();
    let expected = cells.iter().copied().collect::<HashSet<_>>();
    let sheet = spawn_sheet(cells);
    let members = sheet
        .app
        .world()
        .get::<Members>(sheet.controller)
        .expect("the controller should retain its members");
    assert_eq!(members.len(), expected.len());
    assert_eq!(
        sheet.entities.keys().copied().collect::<HashSet<_>>(),
        expected
    );

    let roots = sheet
        .entities
        .values()
        .filter(|entity| sheet.app.world().get::<AnchoredTo>(**entity).is_none())
        .copied()
        .collect::<Vec<_>>();
    assert_eq!(
        roots.len(),
        2,
        "the two edge-connected pieces need two roots"
    );
    assert_eq!(
        sheet
            .entities
            .values()
            .map(|entity| root(&sheet, *entity))
            .collect::<HashSet<_>>(),
        roots.into_iter().collect()
    );
}

#[test]
fn cell_sheet_refuses_zero_negative_and_non_finite_cell_sizes() {
    for cell_size in [0.0, -1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert!(matches!(
            CellSheet::try_new([QuadCell::new(0, 0)], cell_size),
            Err(CellSheetError::InvalidCellSize { cell_size: rejected })
                if rejected.to_bits() == cell_size.to_bits()
        ));
    }
}

#[test]
fn cell_sheet_refuses_a_duplicate_logical_cell() {
    let repeated = QuadCell::new(2, 3);
    assert_eq!(
        CellSheet::try_new([repeated, repeated], CELL_SIZE),
        Err(CellSheetError::DuplicateCell { cell: repeated })
    );
}

#[test]
fn cell_sheet_refuses_a_row_without_a_representable_successor() {
    let cell = QuadCell::new(usize::MAX, 0);
    assert_eq!(
        CellSheet::try_new([cell], CELL_SIZE),
        Err(CellSheetError::RowSuccessorOverflow { cell })
    );
}

#[test]
fn cell_sheet_refuses_a_column_without_a_representable_successor() {
    let cell = QuadCell::new(0, usize::MAX);
    assert_eq!(
        CellSheet::try_new([cell], CELL_SIZE),
        Err(CellSheetError::ColumnSuccessorOverflow { cell })
    );
}

#[test]
fn strip_fold_stacks_each_piece_and_round_trips_to_the_sparse_sheet() {
    let mut sheet = spawn_sheet(two_piece_cells());
    sheet.app.update();
    let driver = sheet
        .app
        .world_mut()
        .spawn((
            SequenceDriver::new(sheet.controller),
            movement(1.0, SequenceDirection::Forward),
        ))
        .id();
    sheet.app.update();

    let mut layers = HashMap::<Entity, Vec<f32>>::new();
    for entity in sheet.entities.values().copied() {
        let root = root(&sheet, entity);
        let offset = translation(&sheet, entity) - translation(&sheet, root);
        assert!(offset.truncate().length() < POSE_TOLERANCE);
        let normal = sheet
            .app
            .world()
            .get::<GlobalTransform>(entity)
            .expect("a cell should have a global transform")
            .back();
        assert!(normal.z.abs() > 1.0 - POSE_TOLERANCE);
        layers.entry(root).or_default().push(offset.z);
    }
    for heights in layers.values_mut() {
        heights.sort_by(f32::total_cmp);
        assert!(heights.windows(2).all(|pair| pair[1] - pair[0] > 0.0));
    }

    drive(&mut sheet, driver, 0.0);
    for (cell, entity) in &sheet.entities {
        let expected = Vec3::new(cell_offset(cell.column()), -cell_offset(cell.row()), 0.0);
        assert!((translation(&sheet, *entity) - expected).length() < POSE_TOLERANCE);
    }
}

#[test]
fn strip_sequence_orders_every_child_after_its_parent() {
    let sheet = spawn_sheet(branched_cells());
    let sequence = sheet
        .app
        .world()
        .get::<FoldSequence>(sheet.controller)
        .expect("the controller should carry its sequence");
    let ends = sequence
        .tracks()
        .iter()
        .map(|track| {
            (
                track.member_entity(),
                track
                    .segments()
                    .first()
                    .expect("each track should move once")
                    .normalized_end(),
            )
        })
        .collect::<HashMap<_, _>>();
    assert!(ends.values().any(|end| (*end - 1.0).abs() < f64::EPSILON));
    assert!(ends.values().any(|end| end.abs() < 1.0));
    for (&member, &end) in &ends {
        if let Some(parent) = sheet
            .app
            .world()
            .get::<AnchoredTo>(member)
            .map(AnchoredTo::target)
            && let Some(&parent_end) = ends.get(&parent)
        {
            assert!(
                end <= parent_end,
                "a child must not unfold before its parent"
            );
        }
    }
}

#[test]
fn branches_leaving_one_square_fold_to_opposite_faces() {
    let sheet = spawn_sheet(branched_cells());
    let mut children = HashMap::<Entity, Vec<Entity>>::new();
    for entity in sheet.entities.values().copied() {
        if let Some(parent) = sheet
            .app
            .world()
            .get::<AnchoredTo>(entity)
            .map(AnchoredTo::target)
        {
            children.entry(parent).or_default().push(entity);
        }
    }
    let siblings = children
        .values()
        .find(|siblings| siblings.len() >= 2)
        .expect("the branched fixture should have sibling strips");
    let signs = siblings
        .iter()
        .take(2)
        .map(|entity| {
            sheet
                .app
                .world()
                .get::<Hinge>(*entity)
                .expect("every non-root should have a hinge")
                .folded_angle()
                .radians()
                .is_sign_positive()
        })
        .collect::<Vec<_>>();
    assert_ne!(signs[0], signs[1]);
}

#[test]
fn strip_fold_has_no_continuous_square_crossings() {
    let cells = branched_cells();
    let sheet = spawn_sheet(cells.clone());
    let squares = cells
        .iter()
        .map(|cell| sheet.entities[cell])
        .collect::<Vec<_>>();
    let model = FoldModel::from_spawned(
        sheet.app.world(),
        &squares,
        &[sheet.controller],
        f64::from(CELL_SIZE) * 0.4985,
    );
    model.assert_no_crossings("branched cell sheet");
}

#[test]
fn exact_checker_reports_overlapping_neighbour_hinges() {
    let cells = branched_cells();
    let sheet = spawn_sheet_with(cells.clone(), simultaneous_sequence);
    let squares = cells
        .iter()
        .map(|cell| sheet.entities[cell])
        .collect::<Vec<_>>();
    let model = FoldModel::from_spawned(
        sheet.app.world(),
        &squares,
        &[sheet.controller],
        f64::from(CELL_SIZE) * 0.4985,
    );
    assert!(model.crossing(0.0, 1.0).is_some());
}

#[test]
fn exact_checker_reports_two_moving_branches_meeting_between_endpoints() {
    let cells = [(0, 0), (0, 1), (3, 0), (3, 1)]
        .map(|(row, column)| QuadCell::new(row, column))
        .to_vec();
    let mut sheet = spawn_sheet_with(cells.clone(), simultaneous_sequence);
    *sheet
        .app
        .world_mut()
        .get_mut::<Transform>(sheet.entities[&QuadCell::new(3, 0)])
        .expect("the second piece root has a transform") = Transform::from_xyz(0.0, 0.0, 1.9)
        .with_rotation(Quat::from_rotation_x(core::f32::consts::PI));
    let squares = cells
        .iter()
        .map(|cell| sheet.entities[cell])
        .collect::<Vec<_>>();
    let model = FoldModel::from_spawned(
        sheet.app.world(),
        &squares,
        &[sheet.controller],
        f64::from(CELL_SIZE) * 0.4985,
    );
    assert!(model.crossing(0.0, 1.0).is_some());
}
