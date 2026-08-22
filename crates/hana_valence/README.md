# hana_valence

hana_valence -- shapes expose connection points and bond into animatable
assemblies; named for valence, an atom's capacity to bond.

> **Work in progress.** This crate is in active development and is not subject
> to semver stability guarantees. APIs may change between commits.

## What it does

In chemistry, an atom's valence is its capacity to bond: the number and
arrangement of connection points it offers. This crate gives authored geometry
the same capability. Providers publish anchor points, entities bond through
those anchors, and systems animate those bonds as assemblies form, separate, and
reconfigure.

- **Anchor geometry** -- providers construct `ResolvedAnchorGeometry` with
  `AnchorSite`, complete `AnchorFrame` values, and ordered edges that define
  hinge axes. Construction rejects duplicate sites, absent edge endpoints, and
  unusable edge directions before the component reaches ECS.
- **Entity bonds** -- `AnchoredTo` connects one entity anchor to another entity
  anchor, while `resolve_anchors` writes the resulting `Transform`.
- **Hinge animation** -- `Hinge` stores the two resting endpoints a member
  travels between and drives `AnchorPose`, so an anchored entity folds around
  one of its authored edges. Arrangement materialization creates every hinge at
  its provider's resting angle; a fold recipe replaces the folded endpoint.
- **Fold recipes** -- `Accordion`, `Coil`, and `Wrap` are transient authors of
  fold endpoints. `Commands::apply_fold_recipe` runs one against a selected
  fold-group alternative and replaces every selected endpoint atomically:
  nothing is written unless every selected member receives exactly one finite
  assignment. Downstream crates implement `FoldRecipe` with their own error and
  their own `ProviderCapability`.
- **Arrangements** -- an `Arrangement` is a non-spatial controller. A `Member`
  relationship lives on each member root and points back to that controller;
  Bevy maintains the controller's `Members` reverse collection, whose inherent
  API exposes read-only access. Membership is independent of physical
  `AnchoredTo` relations.
- **Provider plans** -- an `ArrangementProvider` describes logical members and
  produces one pure, typed `ArrangementPlan` against the library's read-only
  `ArrangementMemberEntities` association. This provider API does not read or
  mutate ECS state.

The crate is named `hana_valence`, but the concrete API keeps the **anchor**
noun: `AnchorSite`, `AnchorFrame`, `AnchoredTo`, `AnchorPose`. A site is the
connection name; a frame is its complete local `Position` and `Orientation`.

## Validated anchor geometry

Providers use their own stable ordering for `AnchorSite::Vertex` and
`AnchorSite::EdgeMidpoint`; those indices carry no cross-provider meaning.
`AnchorSite::Center` is the one whole-member site. `Edge` is ordered, so
reversing its endpoints reverses the `Dir3` axis and therefore the fold sign.

Build geometry with `AnchorFrame::try_new` and
`ResolvedAnchorGeometry::try_new`. The component has no public mutable frame
or edge fields and is opaque to dynamic reflection, so invalid geometry cannot
be introduced after construction. `frame(site)` reports a
`GeometryError::MissingAnchorSite` rather than returning an ambiguous option.

`AnchoredTo` records only point-to-point attachment. Its immutable
relationship replacement is the retarget operation and automatically maintains
the reverse `AnchoredHere` relationship. Its authored offset, a temporary
`ResolvedAnchorOffset`, and `AnchorPose::translation` are all semantic
`Displacement` values; `AnchorPose::rotation` is a valid normalized
`Orientation`.

## Provider-authored arrangement plans

An `ArrangementProvider` is the ordinary extension point for a reusable
logical arrangement. Its associated `Member` is an authoring-time identifier,
not the ECS `Member` component. The library enumerates the logical members in
the provider's order, reserves an ECS entity for each, and presents the
read-only logical-member-to-entity association as
`ArrangementMemberEntities<M>`. A provider obtains entities with
`members.entity(&logical_member)` and builds its plan with that exact value.
Looking up an unlisted logical member returns `ArrangementError::UnlistedMember`
with the member's `Debug` representation.

`ArrangementPlan<S>` is a pure construction value, never a component. `S` is
the provider's fold-group selection vocabulary and stays part of the type until
later materialization. A provider with no fold alternatives uses
`Infallible`. `ArrangementPlan::try_new` stores connections in the provider's
logical-member order even if the connection iterator used a different order;
it never derives ordering from `ResolvedAnchorGeometry::frames()`, whose map
iteration order is unspecified.

Each `ArrangementConnection` names its relationship source with
`member_entity`, supplies the physical target and anchor sites with
`anchored_to`, and provides a source-local ordered `member_edge`. Reversing
that edge reverses the axis sign convention. `base_angle` is a finite,
unwrapped `Angle`, so it may span multiple turns. `HingeClearance` supplies
separate signed semantic `Displacement` values from the shared source edge to
the physical pivot for positive and negative relative fold directions. Use
`HingeClearance::CENTERED` when both pivots lie on the shared edge.

Plan construction accepts empty associations, multiple roots, disconnected
forests, and branches that share a target. It rejects unlisted sources or
targets, a repeated source, self-targeting, physical cycles, a same-site member
edge, and non-finite attachment offsets or directional clearances. This is a
pure failure: it performs no world scan, scene invocation, relationship
insertion, diagnostics retention, or rollback. A plan only checks that the
member edge has unequal endpoints. It deliberately leaves endpoint presence,
separation, and usable-axis validation to `ResolvedAnchorGeometry::try_new`,
which owns the provider geometry.

Provider-specific authoring failures use `ArrangementError::provider`, which
keeps the boxed downstream error available through the standard error source
chain.

## Fold groups and provider capabilities

A provider can offer several fold layouts for the same arrangement. `FoldGroup`
is one nonempty, internally unique, ordered set of members folded as a unit;
vector position is the group-local index. `FoldGroups` is the nonempty ordered
collection of groups that makes up one complete alternative. Groups may overlap
and the same member may appear in several groups. Neither type has an empty
state, an unchecked constructor, or a mutable accessor:
`FoldGroup::try_new(first, remaining)`, `FoldGroup::try_from_iter`,
`FoldGroup::combine`, `From<Entity>`, and `TryFrom<Vec<Entity>>` are the
construction routes, and empty or repeated input is a `FoldAuthorError`.

`ArrangementPlan::with_fold_groups(selection, groups)` retains one alternative
per selection value, and `ArrangementPlan::with_capability(selection, value)`
retains purpose-specific provider knowledge for that alternative. A group member
must already be a connection source of the plan, one selection selects one
alternative, and one selection holds at most one value per concrete capability
type.

`Provides<C>` is how a provider builds a capability while it authors its plan:

```rust,ignore
impl Provides<WindingClearance> for MyProvider {
    fn provide(
        &self,
        selection: &Self::FoldGroupSelection,
        groups: &FoldGroups,
        connections: &[ArrangementConnection],
    ) -> Result<WindingClearance, ArrangementError> { /* ... */ }
}
```

Application code never calls `provide`; by the time an application holds an
arrangement, the capability is already retained beside its groups.

`WindingClearance` is the capability a wrapping recipe requires. Its constructor
takes the fold groups it must cover plus one finite `Displacement` per member,
and rejects a repeated member, a member outside those groups, a non-finite
value, and incomplete coverage. Each stored value is the canonical positive
winding direction; the opposite direction is exactly its negation, so a wrap
cannot be authored with asymmetric winding. `clearance_for(member)` returns a
`Displacement` or `FoldAuthorError::MissingWindingClearance`, never an absence
value the caller has to interpret.

Materialization erases the selection type into private retained state.
`RetainedProviderKnowledge::for_arrangement(world, arrangement)` reads it back
with the provider's own selection type: `groups(&selection)` and
`capability::<S, C>(&selection)` distinguish a selection of the wrong Rust type,
an unknown selection value, and a missing required capability as separate named
errors. Both construction commands retain the same data, and the retained table
itself is private, carries no reflection route, and cannot be mutated through
this read.

## Built-in sheet providers

`QuadSheet::new(rows, columns)` and `TriangleSheet::new(rows, columns)` are
ordinary providers built from the pieces above, so application code can reuse a
rectangular sheet instead of authoring one:

```rust,ignore
let sheet = QuadSheet::new(3, 4);
commands.spawn_arrangement(sheet, |cell: &QuadCell| bsn! {
    Transform,
    Mesh3d(quad_mesh.clone()),
    Name(format!("cell {}x{}", cell.row(), cell.column())),
});
```

Each provider enumerates its logical cells in row-major order, authors one
connection forest, and retains a `Rows` and a `Columns` alternative — both built
from the sources of the connections it just authored, so every non-root cell
appears exactly once and each forest root stays outside every group as that
forest's fixed cell. An N-cell line therefore yields groups totalling N-1
members. Group order runs outward from the fixed cell, so `Accordion` and `Coil`
style recipes can read group index as fold depth. One row or one column is a
normal degenerate sheet: a 1xN quad sheet retains one `Rows` group of N-1 cells
and N-1 single-cell `Columns` groups, and both alternatives stay usable. There is
no separate strip type.

`cell_geometry()` returns the documented per-cell `ResolvedAnchorGeometry` for
member scenes: a center frame, one frame per vertex, one per edge midpoint, and
the provider's ordered edges. Quad cells are all identical, so `QuadSheet` takes
no cell argument; `TriangleSheet::cell_geometry(cell)` needs one because a cell
points up or down by `TriangleCell::orientation()`. Both sheets tile planar cells
sharing one local `+Z` normal, so both implement `Provides<WindingClearance>`
with an exact exterior pivot of `LAYER_THICKNESS` per fold depth. A provider
whose geometry cannot supply that exact displacement should not implement the
capability at all.

Hex sheets and box nets stay downstream. They are ordinary implementations of
the same public trait and need no core type.

## Provider-authored fold sequences

A provider can carry the fold sequence for the arrangement it authors, so the
application spawns one value instead of spawning the arrangement and then
authoring stages against entity IDs it has to look up afterwards.

`with_fold_sequence` folds one selected group per stage, in the provider's own
outward group order, with every stage inheriting one `FoldTiming`:

```rust,ignore
use hana_valence::FoldTiming;

let unfold = FoldTiming::new(Duration::from_millis(400), EaseFunction::CubicInOut);
let sheet = QuadSheet::new(3, 4).with_fold_sequence(QuadFoldGroupSelection::Rows, unfold);
let arrangement = commands.spawn_arrangement(sheet, |cell: &QuadCell| { /* ... */ })?;
```

`FoldTiming` also accepts an immutable `EasingCurve` built from semantic
`EasingInput`, `EasingOutput`, and `EasingSlope` values. The sequence owns that
lookup data and samples it without Bevy asset access. A future editable or
asset-backed source will be a separate easing variant with its own availability
behavior.

`with_custom_fold_sequence` hands the selected `FoldGroups` to a closure that
returns the whole sequence, so it can combine, subdivide, reorder, or omit
groups and choose every stage's and member's timing:

```rust,ignore
use hana_valence::FoldSequenceBuilder;
use hana_valence::FoldTiming;

let sheet = QuadSheet::new(3, 4).with_custom_fold_sequence(
    QuadFoldGroupSelection::Rows,
    |groups: &FoldGroups| {
        // Every row moves at once, each cell a beat behind the one inside it.
        Ok(FoldSequenceBuilder::new(FoldTiming::snap())
            .stage(
                FoldStage::from(FoldGroup::combine(groups.iter())?)
                    .override_member_timings_with(|member_index, _| {
                        FoldTiming::new(TRAVEL, EaseFunction::CubicInOut)
                            .with_start_offset(BEAT * member_index as u32)
                    }),
            )
            .build())
    },
);
```

The closure is `Fn`, not `FnOnce`: `generate_plan` takes `&self` and may run
more than once. It runs once per plan generation, synchronously, after the
provider's logical members already have entities and before any scene or ECS
write, so its failure aborts the spawn with no arrangement materialized and the
original `FoldAuthorError` kept as the returned `ArrangementError`'s source.

Both adapters resolve their groups from the plan the wrapped provider just
produced. A provider that retains no alternative under that selection — a
`QuadSheet::new(1, 1)` or `TriangleSheet::new(1, 1)` has no crease, so it
retains none at all — fails with `ArrangementError::UnknownFoldGroupSelection`
and authors nothing, rather than materializing an empty sequence.

The resolved sequence travels with `ArrangementPlan` as its
`PlannedFoldSequence`, which names the `Authored` and `Unauthored` states a
plan can carry, and materialization inserts it on the arrangement controller
only after the whole plan validated.

## Fold boundary events

A retained fold sequence emits one event per boundary it actually crosses, in
the order its ledger records them. Stage and endpoint events are triggered on
the arrangement, member events on the member itself:

| Event | Triggered on | Also carries |
| --- | --- | --- |
| `FoldStageBegin` / `FoldStageEnd` | arrangement | `stage: SequenceStageId`, `direction`, `timing` |
| `FoldMemberBegin` / `FoldMemberEnd` | member | `arrangement`, `stage`, `direction`, `timing`, `member_timing: FoldTiming` |
| `FoldEndpointReached` | arrangement | `endpoint: FoldEndpoint`, `direction`, `timing` |

`timing` is a `FoldEventTiming`: the boundary's exact authored `elapsed`, the
sequence's `total` extent, and its normalized `position`. Position is carried
rather than divided out, because a sequence whose stages are all zero-duration
has `elapsed` and `total` both zero at every boundary and only `position` still
separates its two endpoints.

Events follow raw movement, never eased output. An owned curve changes only
what a member's pose interpolates and emits nothing on its own, because raw
position and raw traversal did not move. A rejected curve or non-finite sample
holds the current pose and likewise emits nothing. A hold, a movement the shared
layer refused, and an unchanged producer position also emit nothing. A large
seek emits every boundary between the two positions, and signed multi-wrap
movement emits every crossing of every repetition.

Direction names travel, not the record: crossing a stage's authored end while
travelling backward emits `FoldStageBegin`, because that stage is the one now
being travelled.

`FoldEndpointReached` fires whenever travel crosses an endpoint, arriving or
departing, so a forward play starting at the base emits
`FoldEndpoint::Base` as it leaves and a backward play emits
`FoldEndpoint::Folded` as it leaves. Read `endpoint` together with `direction`
to tell the two apart: travel that reaches an endpoint moves toward it —
forward to `Folded`, backward to `Base` — and the other two pairings name a
departure. Reading the event alone as "the fold finished" reports a false
positive at the start of every full run.

An observer receives the event after local position already moved, so a
secondary animation can start where the fold actually is instead of at zero:

```rust,ignore
fn flash_panel(
    began: On<FoldMemberBegin>,
    mut commands: Commands,
    playbacks: Query<&FoldSequencePlayback>,
) -> Result {
    let playback = playbacks.get(began.arrangement)?;
    // How far past its own boundary the sequence already stands.
    let travelled = f64::from(playback.position().normalized())
        - f64::from(began.timing.position().normalized());
    let elapsed = Duration::try_from_secs_f64(
        travelled.abs() * began.timing.total().as_seconds_f64(),
    )?;
    commands.entity(began.member).insert(PanelFlash {
        elapsed: elapsed.min(began.member_timing.duration),
    });
    Ok(())
}
```

`examples/staggered_unfold.rs` runs this pattern as an emissive flash on each
panel whose own stage began travelling.

## Arrangement construction

Add `ArrangementPlugin` after Bevy's `AssetPlugin` (included in
`DefaultPlugins`). It owns arrangement command materialization and adds
`ScenePlugin` if it is not already present. It deliberately does not install
anchor resolution, transform propagation, or application-specific arrangement
systems: those remain application-owned.

```rust
use bevy::prelude::*;
use hana_valence::*;

App::new()
    .add_plugins(DefaultPlugins)
    .add_plugins(ArrangementPlugin);
```

`ArrangementCommandsExt::spawn_arrangement` consumes a provider's logical
members in provider order, reserves one controller and one member root per
logical member, builds the crate-owned `ArrangementMemberEntities<M>` exactly
once, and validates the typed `ArrangementPlan<S>` before it invokes a scene
factory. On any synchronous association, provider, or plan failure, it returns
the original `ArrangementError`, preserves a boxed provider source, queues all
reserved IDs for cleanup, and does not invoke a member scene.

```rust
let arrangement = commands.spawn_arrangement(provider, |logical_member| {
    bsn! { MemberVisual::for_member(logical_member) }
})?;
```

Each scene is queued with `queue_apply_scene` on the reserved member root, not
on a second scene root. `()` is the empty-scene choice. A queued scene applies
in the same frame only when its dependencies are ready and deferred commands
run before Bevy's scene-spawn system; otherwise Bevy applies it later. Entities
inside the scene are not members unless they separately receive `Member`.

The provider association and typed plan are transient. Materialization keeps
only `Arrangement`, the `Member` relationships, valid `AnchoredTo` values for
the provider's connections, and private retained connection/selection data for
later fold support. A listed member without a connection is a physical root;
the `Members` iteration order is for enumeration only and never names its
physical parent.

`spawn_arrangement_from_members` creates only the controller. Its binding
closure must return `MemberBinding::Bound(entity)` for every required logical
member. `MemberBinding::Missing` means binding failed, not that an optional
member should be omitted. The command consumes every binding before it inserts
any relationship, rejects duplicate bound entities, and does not preflight
whether a bound entity still exists when deferred commands run. It never creates
replacement member roots or scenes.

To fold a materialized arrangement, name the provider's fold-group selection
and a recipe:

```rust,ignore
commands.apply_fold_recipe(arrangement, QuadFoldGroupSelection::Rows, Accordion::default());
```

The command is deferred, so it may be issued in the same `Commands` batch as
the construction command that returned `arrangement`. It reads the retained
groups, connections, and capability, splits the selection into physically
connected pieces, runs the recipe once per piece, and validates the whole
result before the first hinge write. A selection, capability, coverage,
duplicate, foreign, non-finite, or away-from-base failure warns through Bevy's
command-error handler and leaves every current hinge unchanged. A recipe may
only replace an endpoint while the hinge rests at its base.

For advanced authoring, insert `Member::new(arrangement_controller)` directly
on a member root and insert `AnchoredTo`, `AnchorPose`, and `Hinge` according to
the application's own geometry and animation rules. `Member` is not a physical
attachment: a `Member` replacement retargets only Bevy's controller-side
`Members` collection, while an `AnchoredTo` replacement retargets only physical
attachment. `Members` has read-only inherent `iter`, `len`, and `is_empty`
accessors. Bevy's public `RelationshipTarget` trait still exposes its documented
`collection_mut_risky` and `from_collection_risky` maintenance hooks; using
either can violate the reverse-collection invariant.

`resolve_anchors` is intentionally best effort. It rebuilds dependency order
from the current ECS state every frame, skips only sources whose live geometry,
relationship, transform, or target is unavailable, and leaves authoring in
place for a later frame. Its local scratch storage preserves temporary
allocation capacity only; it is not retained topology reconciliation.

## Examples

The examples in `examples/` are standalone Bevy apps:

- `staggered_unfold` -- five quads with staggered hinge animation.
- `triangles` -- equilateral triangles with direct relationship and hinge
  authoring.
- `box` -- a six-quad cross net that folds into a closed box with direct
  `AnchoredTo` and `Hinge` relations.

Run them with:

```sh
cargo run -p hana_valence --example triangles
cargo run -p hana_valence --example box
cargo run -p hana_valence --example staggered_unfold
```

No example uses the optional `tween` feature. It gates the `bevy_tween`
dependency for the tween adapter, which arrives in a later phase.

## Bevy compatibility

| hana_valence | Bevy |
|--------------|------|
| main         | 0.19 |

## License

MIT OR Apache-2.0
