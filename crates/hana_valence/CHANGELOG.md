# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `FoldStageBegin`, `FoldStageEnd`, `FoldMemberBegin`, `FoldMemberEnd`, and
  `FoldEndpointReached` are the entity events a retained fold sequence emits for
  every boundary it crosses, once each, in ledger order. Stage and endpoint
  events are triggered on the arrangement and member events on the member, and
  each carries the crossed stage's `SequenceStageId`, the direction of travel,
  and a `FoldEventTiming` holding the boundary's exact authored `elapsed`, the
  sequence's `total` extent, and its normalized `position`. A member event also
  carries the `FoldTiming` resolved for that member in that stage, which is what
  an observer needs to start a secondary animation from the travel the fold
  already passed. Events are derived from raw traversal, so a rejected or
  non-finite easing evaluation emits no additional boundary, and a hold, a
  refused movement, or an unchanged producer position crosses nothing.

- `FoldSequencePlayback` is the retained runtime component for one authored
  `FoldSequence`. `FoldPlugin` inserts it, plus the sequence's `SequenceStages`,
  whenever the authored component changes, and rebuilds nothing per frame. It
  exposes read-only position, boundary, and per-member fraction state, and its
  local position is mutated only through `FoldCommands`.
- `FoldCommands` is the system param that issues the shared `SequenceCommand`
  vocabulary against a retained fold sequence, so a producer selected as its
  `SequenceDriver` is the only writer of local position while it holds the
  sequence. A command issued against an entity carrying no retained playback
  answers `SequenceCommandResponse::NoRetainedSequence`, which is distinct from
  the `Rejected(SequenceOwner)` a genuine owner answers. Every type an
  application needs to drive a retained sequence or read its rejection is
  re-exported from the crate root: `SequenceCommand`,
  `SequenceCommandOutcome`, `SequenceCommandRejected`,
  `SequenceCommandResponse`, `SequenceDirection`, `SequenceDriver`,
  `SequenceDriverTakeover`, `SequenceEasing`, `SequenceEvaluation`,
  `SequenceMovement`, `SequenceOwner`, `SequencePosition`, `SequenceScope`,
  `SequenceSourceState`, `SequenceStageId`, `SequenceStages`,
  `RangeCrossing`, `RangeCrossings`, and `RangeEdge`. Authoring an
  `EasingCurve` by value needs its knot vocabulary, so `EasingCurveBuilder`,
  `EasingCurveError`, `EasingInput`, `EasingInterpolation`, `EasingKnot`,
  `EasingOutput`, `EasingSlope`, and `EasingSlopes` are re-exported alongside
  it.
- `EasedFoldFraction` is the named eased fold fraction `fold_fraction` returns
  and `Hinge::angle_at` consumes, replacing a bare `f64`.
- `HingePoseReported` marks a hinge whose pose conversion already warned, so the
  warning is bounded per entity instead of retained in a system-local set.

- `ArrangementProvider::with_fold_sequence` and
  `ArrangementProvider::with_custom_fold_sequence` return a provider that also
  authors one fold sequence from the groups its own plan retains. The standard
  adapter folds one selected group per stage under one shared `FoldTiming`; the
  custom adapter hands the selected `FoldGroups` to an `Fn` closure that returns
  the whole `FoldSequence` and may combine, subdivide, reorder, or
  omit groups. The closure runs once per plan generation, before any scene or
  ECS write, and its `FoldAuthorError` is kept as the returned
  `ArrangementError`'s source. A selection the wrapped provider never retained —
  a creaseless `QuadSheet::new(1, 1)`, `TriangleSheet::new(1, 1)`, or any zero
  dimension — fails with `ArrangementError::UnknownFoldGroupSelection` and
  authors nothing.
- `PlannedFoldSequence` is the `ArrangementPlan` field naming whether a provider
  authored a fold sequence, readable through `ArrangementPlan::fold_sequence`.
  Materialization inserts an `Authored` sequence on the arrangement controller
  after the plan validated; `Unauthored` materializes exactly as before.
- `FoldSequence` is now a `Component` with opaque reflection, because
  it is the value materialization retains on the controller.
- The crate root holds the pure fold authoring value layer: `FoldTarget`,
  `FoldTiming`, `FoldStage`, `FoldSequence`, `FoldSequenceBuilder`,
  `FoldSegment`, `FoldMemberTrack`, `FoldMemberSample`, `FoldLedger`,
  `fold_fraction`, and `evaluate_fold_angle` with `EasedFoldFraction` and
  `FoldEvaluationError`. None of it is a system or a plugin registration, and
  only `FoldSequence` is a `Component`, so an authored sequence resolves its
  stage extents, per-member segment tracks, and ordered boundary ledger without
  a `World`.
- `FoldAuthorError::NonFiniteFoldTarget`, `FoldAuthorError::FoldTargetOutOfRange`,
  and `FoldAuthorError::UnknownStageMember` reject an invalid authored fold
  destination and a stage member outside its own group.
- `FoldRecipe` authors every selected fold endpoint from one transient value.
  `FoldAssignment` carries the member, its folded `Angle`, and its pivot
  `Displacement`. `ProviderCapability` marks a value a provider files with a
  selection, `FoldRecipeCapability` retrieves it, and `NoCapability` is the
  recipe declaration for needing none.
- `Accordion`, `Coil`, and `Wrap` are the built-in recipes. `Accordion`
  alternates direction with outer group parity and reports
  `ConflictingAccordionDirections` when one member is reached by two groups of
  opposite direction. `Coil` applies one signed offset everywhere. `Wrap`
  scales `WindingClearance` by winding position so each layer clears the one
  beneath it. All three report `FoldAuthorError::AngleOverflow` for an
  unrepresentable endpoint.
- `ArrangementCommandsExt::apply_fold_recipe` applies one recipe to one
  selected fold-group alternative. It resolves knowledge, splits the selection
  into connected pieces, runs the recipe once per piece, and validates the
  concatenated result before any write, so every pre-write failure leaves the
  current hinges in place. It may be queued in the same `Commands` batch as
  either construction command.
- `FoldGroup::connections` yields a group's `ArrangementConnection` values in
  member order.
- `Hinge::try_new` and `HingeError` reject a degenerate edge and a non-finite
  pivot offset before the component reaches ECS.

- `Arrangement`, `Member`, and `Members` now express controller membership as
  Bevy's relationship pair. `Member` is immutable and stored on each member
  root; it points to the non-spatial controller, while `Members` is the
  controller-side Bevy-maintained reverse collection with read-only accessors.
- `ArrangementPlugin` and `ArrangementCommandsExt` materialize a complete
  provider plan through either reserved BSN-capable member roots or bindings to
  existing entities. Scene factories run only after association and plan
  validation; synchronous failures preserve their exact error/source and queue
  reserved-entity cleanup. `MemberBinding::Missing` reports required binding
  failure explicitly.
- `ArrangementProvider`, `ArrangementMemberEntities`, and typed
  `ArrangementPlan` provide a pure provider-authoring path for complete
  arrangement connection forests. Plans preserve logical member order, retain
  the provider's fold-group selection type through construction, and perform no
  ECS mutation before later materialization.
- `ArrangementConnection` now records one relationship source, attachment,
  source-local ordered member edge, finite multi-turn `Angle` endpoint, and
  directional `HingeClearance`. `HingeClearance` exposes independent signed
  source-local pivot displacements for positive and negative folding.
- `FoldGroup` and `FoldGroups` express fold alternatives as nonempty, ordered,
  internally unique values with no empty state, unchecked constructor, or
  mutable accessor. Groups may overlap, and one member may appear in several
  groups.
- `ArrangementPlan::with_fold_groups` and `ArrangementPlan::with_capability`
  retain one complete group alternative and its purpose-specific provider
  knowledge per typed selection. `Provides<C>` is the provider-side extension
  point that builds a capability during `generate_plan`.
- `WindingClearance` is a required wrapping capability: it validates exact
  coverage of its fold groups at construction and answers `clearance_for` with a
  canonical positive `Displacement` or a named error.
- `RetainedProviderKnowledge` reads back everything a provider retained for a
  materialized arrangement — its fold groups and its capabilities — with the
  provider's own selection type. The retained table stays private and
  unreflected.
- `FoldAuthorError` now implements `std::error::Error` through `thiserror` and
  carries fold-group and winding-clearance variants alongside its existing stage
  variants. Its public path is unchanged.
- `QuadSheet` and `TriangleSheet` are built-in reusable providers over
  deterministic row-major logical cells (`QuadCell`, `TriangleCell`, and
  `TriangleCellOrientation`). Each authors one connection forest without a
  previous-member assumption, exposes documented per-cell geometry through
  `cell_geometry`, and retains `Rows` and `Columns` alternatives
  (`QuadFoldGroupSelection`, `TriangleFoldGroupSelection`) built from the sources
  of the connections it just authored, so every non-root cell appears exactly
  once and each forest root stays outside every group. One row or one column is a
  normal degenerate sheet, so an N-cell line yields groups totalling N-1 members;
  there is no separate strip type. Both sheets implement
  `Provides<WindingClearance>` because their planar cells share one local `+Z`
  normal. Hex sheets and box nets stay downstream as ordinary implementations of
  the same trait.
- `ArrangementError` reports logical-member lookup and association failures,
  foreign sources and targets, duplicate sources, self-targets, cycles,
  equal-site edges, and non-finite connection displacements. Provider-specific
  errors retain their boxed downstream source.

### Changed

- **Breaking:** `hinge_to_pose` derives each `AnchorPose` in `PostUpdate` from
  the live `Hinge` and the `EasedFoldFraction` its sequence cached in `Update`,
  and writes only when the pose actually changes. Replacing a `Hinge` before it
  runs changes the pose in the same frame at an unchanged sequence position, and
  an idle fold performs no write at all.
- **Breaking:** `FoldSystems::Advance` runs in `Update` inside
  `SequencePlaybackSystems::EvaluateSequences`, not in `PostUpdate`. A system
  that ordered itself after fold advancement in `PostUpdate` is already after it
  and drops the edge.

- **Breaking:** `FoldEndpoint::Unfolded` is now `FoldEndpoint::Base`, the name
  the endpoint already carries in `Hinge::base_angle` and `FoldTarget::BASE`.
- `FoldEndpoint` now lives beside `FoldBoundary`, which reaches it as a
  fold-sequence boundary. The flat `hana_valence::FoldEndpoint` export is
  unchanged.
- `FoldSegment::raw_progress` returns `FoldSegmentProgress` and
  `FoldMemberSample::Moving` carries one, so a position within a segment cannot
  be non-finite or outside `0..=1`. Easing reads
  `FoldSegmentProgress::normalized`.
- **Breaking:** `Hinge` is now `{ edge, base_angle, folded_angle, pivot_offset }`
  with private fields, opaque reflection, and `#[require(AnchorPose)]`. Build it
  with `Hinge::try_new` and read it through its const accessors. It is no longer
  animated directly: `hinge_to_pose` derives the current angle from the member's
  position in its `FoldSequence`, or from `base_angle` when the member is in no
  sequence, and it writes only entities that carry a `Hinge`. Migrate a
  `Hinge { edge, angle }` to `Hinge::try_new(edge, angle, angle,
  Displacement::default())` and author folded endpoints with a fold recipe.
  `ArrangementPlugin` is now the sole registrar of `hinge_to_pose`.
- **Breaking:** `ArrangementPlan::with_capability` and `Provides<C>` now bound
  `C` by `ProviderCapability`. Implement the marker on each capability type.

- **Breaking:** removed `FoldAngles`, `HingePivot`, `actuate_fold_hinges`,
  `FoldAngleDiagnostic`, `FoldAngleDiagnostics`, `FoldAngleInvalidReason`,
  `FoldSystems::Actuate`, and `Hinge::rotation`. Fold endpoints are now hinge
  state authored by a `FoldRecipe`, not a separate mutable actuation component.
- **Breaking:** removed the `HingeAngleLens` and `AnchorPoseLens` `bevy_tween`
  interpolators. A hinge is driven by its fold sequence, and `hinge_to_pose`
  overwrites `AnchorPose` on every hinged entity.
- **Breaking:** replaced `AnchorId` with `AnchorSite` and renamed
  `EdgeMid` to `EdgeMidpoint`. Vertex and midpoint indices now explicitly use
  provider-defined ordering, while `Center` remains the sole whole-member site.
- **Breaking:** replaced public-field `AnchorPoint` and
  `ResolvedAnchorGeometry` construction with validated `AnchorFrame::try_new`
  and `ResolvedAnchorGeometry::try_new`. Geometry now rejects non-finite
  positions, duplicate sites, missing edge endpoints, and degenerate edges;
  consumers use `frame`, `frames`, and `edges` accessors.
- **Breaking:** migrated attachment offsets and anchor poses to
  `Displacement` and `Orientation`. Dynamic reflection cannot mutate validated
  geometry or pose composites into invalid structural states.
- **Breaking:** promoted `Coil` to a first-class arrangement component and
  removed `FoldPattern`; `Accordion` now always alternates adjacent hinge
  directions, and `member_placement` now accepts a `Coil` input alongside
  `Accordion` and `Strip`.
- **Breaking:** removed `ArrangementMembers`, `MemberIndex`, `TilingRule`,
  `QuadTiling`, `ArrangementPlacement`, and `MemberPlacement`, along with their
  index, placement, and arrangement-hinge systems. Use `Arrangement` plus
  `Member::new(controller)` for logical membership, and author `AnchoredTo`,
  `AnchorPose`, and `Hinge` directly or through an `ArrangementProvider`.

### Removed

- **Breaking:** the fold runtime is one retained sequence evaluated by one
  evaluator, so `FoldCommand`, `FoldCommandEvent`, `FoldDirection`,
  `FoldMotion`, `FoldMember`, `FoldMembers`, `FoldSequenceState`,
  `FoldFromArrangement`, `FoldDiagnostic`, `FoldDiagnostics`,
  `FoldInvalidReason`, `FoldSnapshotDiagnostic`, `FoldSnapshotDiagnostics`, and
  `FoldSnapshotInvalidReason` are gone. Author a sequence with
  `FoldSequenceBuilder`, spawn the resulting `FoldSequence`, and drive it with
  `FoldCommands`; membership is the authored stage group, not a component, and
  numeric stage components no longer exist.

- **Breaking:** `FoldAuthorError::EmptyStage` is gone. A fold group is nonempty
  by construction: `FoldGroup::try_from_iter` rejects an empty member sequence
  with `FoldAuthorError::EmptyFoldGroup`, so a stage with no member cannot be
  authored and no stage is silently skipped.
