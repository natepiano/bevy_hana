# hana_rubric

`hana_rubric` provides the JSONC keymap foundation for Bevy applications. It owns command IDs,
load diagnostics, keymap layering, and reload support.

It also owns the multi-dimensional state model that state-specific bindings are authored against.
This document is the whole public contract for that model: one setup mechanism, the JSONC
predicate rules, the observability surfaces, and the borrowed palette query. There is exactly one
way to register state; no resource-backed, optional-state, or Rubric-derived alternative exists.

## The ownership boundary

The application owns state. Rubric observes it.

- The application derives Bevy `States` enums, initializes them, and writes their `NextState`.
  Converting ECS facts (components, selections, tool modes, load progress) into one total typed
  state is application work.
- Rubric registers each state type by name, reads `State<C>` every frame, validates authored
  predicates against the registered vocabulary, materializes an effective keymap for the current
  snapshot, and routes commands.

Rubric never initializes a registered `State<C>`, never writes its `NextState<C>`, never infers
application state from arbitrary ECS facts, and never synthesizes a cross-product state enum.

## Setup

Three steps, in this order:

1. Initialize each application-owned total Bevy state.
2. Register each state as one named dimension through repeated
   `KeymapPlugin::with_state_dimension`.
3. Install one keymap document through `with_defaults`.

```rust
use bevy::prelude::*;
use hana_rubric::KeymapPlugin;
use strum::AsRefStr;
use strum::EnumIter;
use strum::EnumMessage;

#[derive(
    AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
)]
#[strum(serialize_all = "snake_case")]
enum ApplicationState {
    #[default]
    #[strum(message = "While the application is at its main menu")]
    MainMenu,
    #[strum(message = "While the application is running")]
    Running,
}

#[derive(
    AsRefStr, Clone, Copy, Debug, Default, EnumIter, EnumMessage, Eq, Hash, PartialEq, States,
)]
#[strum(serialize_all = "snake_case")]
enum InteractionState {
    #[default]
    #[strum(message = "While no editing interaction is active")]
    Resting,
    #[strum(message = "While dimension lock is the active editing interaction")]
    DimensionLock,
}

app.init_state::<ApplicationState>()
    .init_state::<InteractionState>()
    .add_plugins(
        KeymapPlugin::new()
            .with_app_name("my_app")
            .with_defaults(include_str!("keymap.default.jsonc"))
            .with_state_dimension::<ApplicationState>("application")
            .with_state_dimension::<InteractionState>("interaction"),
    );
```

`with_state_dimension` is repeatable. Adding no dimension at all is a complete configuration: the
application then routes the document's global base only.

A runnable version of exactly this setup is Fairy Dust's canonical example:

```sh
cargo run -p fairy_dust --example keymap_contexts
```

### Registration requirements

`with_state_dimension::<C>(name)` requires:

- a nonempty `name`, unique across every registration in the app;
- a state type `C` registered exactly once — the same type under two names is rejected;
- nonempty, unique value names, taken from `AsRef<str>` over `C::iter()`;
- a nonempty `#[strum(message = "…")]` on every variant, used as the reference and schema
  description for that value.

The `KeymapStateDimension` trait is a blanket implementation over
`AsRef<str> + Copy + EnumMessage + Eq + IntoEnumIterator + Send + Sync + 'static`. Deriving
`strum::AsRefStr`, `strum::EnumIter`, and `strum::EnumMessage` on a Bevy `States` enum satisfies it.

A violated requirement is a retained assembly diagnostic with
`DiagnosticOrigin::ContextRegistration`. Duplicate diagnostics name both registrations — the
dimension name and both fully-qualified state type names — so the conflict is locatable without
reading the plugin chain. Registration is all-or-none per batch: if any registration in one
`add_plugins` call fails, that batch installs no observers, and state collection is permanently
disabled for the app. The active context then stays `AwaitingStateDimensions` and nothing routes.

## Authoring state-specific bindings

One document governs every layer. A block's optional `context` member is a **conjunction** over
named dimensions.

```jsonc
{
  "bindings": [
    // No `context`: global base, effective in every snapshot.
    {
      "bindings": {
        "g": "example::show_global_route",
        "t": "example::show_tombstoned_route"
      }
    },

    // One dimension.
    {
      "context": { "application": "running" },
      "bindings": { "a": "example::show_running_route" }
    },

    // Conjunction: both terms must hold in the active snapshot.
    {
      "context": {
        "application": "running",
        "interaction": "dimension_lock"
      },
      "bindings": {
        "c": "example::show_combined_route",
        "t": null
      }
    }
  ]
}
```

### Validation rules

- Omitting `context` makes the block global.
- `"context": {}` is rejected: a context object must name at least one state dimension.
- `"context": null` and any non-object `context` are rejected as authoring syntax errors. A
  malformed `context` never silently becomes a global block.
- A context value must be a string.
- An unknown dimension name is rejected, and the diagnostic lists the registered dimension names as
  suggestions. When no dimension is registered at all, the diagnostic says the application accepts
  global blocks only.
- An unknown value for a registered dimension is rejected, and the diagnostic lists that
  dimension's registered values.
- Repeating one dimension inside a single `context` object is rejected.

A rejected predicate rejects the whole document, not just its own block. Acceptance returns
diagnostics, no accepted document is retained, and nothing is materialized: at startup that is
`EffectiveKeymapStatus::RejectedInitialDocument`, and on reload the previous generation stays live
while only diagnostics change. One misspelled dimension name therefore costs the entire keymap,
including the global base, until it is corrected.

A rejected **binding** is different. An unknown command id or a reserved keystroke is skipped, and
the rest of the document still applies.

### Precedence and tombstones

Materialization applies the retained global base first, then every matching contextual layer in
authored document order. Later matching blocks override earlier matching blocks at the same
sequence, so the last matching authored value wins.

`null` is a tombstone. A contextual tombstone removes the sequence for that snapshot, including a
sequence the global base supplied; only true absence inherits the global binding. Tombstones remove
complete sequences, not prefix subtrees.

Rubric does **not** precompute a cross-product of dimension values. The accepted document keeps a
global base plus its ordered conditional layers, and one effective keymap is materialized for the
snapshot that is actually active.

## Runtime behavior

### Scheduling

`KeymapSystems` in `PreUpdate` is chained:

```rust
KeymapSystems::ObserveStateDimensions   // read every State<C>
KeymapSystems::UpdateActiveKeymapContext // collect one complete snapshot
KeymapSystems::Route                     // dispatch through the compiled matcher
```

Reload collection, reload commit, and effective-keymap commit run between
`UpdateActiveKeymapContext` and `Route`, so dispatch and palette tables always describe the
snapshot routing is about to use.

Changing state is an application message round trip, not a same-frame write: a command triggers an
application event, an observer sets `NextState<C>`, Bevy applies the transition, and Rubric observes
the applied total state on its next scheduled pass. Callers must not assume a state change routes in
the same frame it was requested. Rubric makes no promise about how a UI displays that latency.

At most one `ActiveKeymapContextTransition` message is published per collection pass, after every
dimension observer has run. Partial snapshots never transition.

### Observing the active state

```rust
pub enum ActiveKeymapContextState {
    GlobalRouting,
    AwaitingStateDimensions,
    StateDimensionsUnavailable { missing: Vec<ContextDimensionName> },
    Resolved(ContextSnapshot),
}
```

- `GlobalRouting` — no dimension is registered, so the global base is the complete application
  state.
- `AwaitingStateDimensions` — at least one registered dimension has not reported a value or its
  absence yet. This is the startup state of every app that registers a dimension.
- `StateDimensionsUnavailable` — one or more `State<C>` resources are absent. `missing` is the
  complete list, sorted by dimension name. Entering this state logs one warning naming the missing
  dimensions.
- `Resolved` — every registered dimension has a value. `ContextSnapshot` iterates
  dimension/value pairs in ascending dimension-name order, not registration order.

**There is no global fallback.** Awaiting, unavailable, and unmaterializable states route nothing at
all — they do not degrade to the document's global base. An application that forgets to initialize a
registered `State<C>` gets a silent-free failure: a warning, an observable state, and no input.

### Effective keymap publication

```rust
pub enum EffectiveKeymapStatus {
    AwaitingAcceptedDocument,
    RejectedInitialDocument,
    AwaitingStateDimensions,
    StateDimensionsUnavailable { missing: Vec<ContextDimensionName> },
    UnmaterializableStateDimensions,
    Loaded(EffectiveKeymapPublication),
}

pub struct EffectiveKeymapPublication {
    pub generation:     KeymapGeneration,
    pub snapshot:       EffectiveKeymapSnapshot,
    pub matched_layers: Vec<MatchedPredicateLayer>,
}
```

`EffectiveKeymapPublication` is the persisted record of what was materialized: the dispatch and
palette generation, the exact `Global` or `Resolved` snapshot identity it came from, and every
matching contextual layer in applied document order with its canonical predicate terms. It is the
provenance answer to "why is this key bound right now".

`UnmaterializableStateDimensions` means a snapshot could not be materialized against the registered
typed vocabulary — for example a reflected `ActiveKeymapContext` written over BRP that names a value
no registered dimension declares, or a forged `GlobalRouting` state in an app that registered
dimensions. It is a refusal, never a fallback.

Publication is atomic and generation-stamped. One materialization produces both the compiled
dispatch matcher and the palette binding table, stamped with the same `KeymapGeneration` and
snapshot. Materialization happens only when an accepted document arrives or the active snapshot
changes; a steady frame re-materializes nothing, republishes nothing, and allocates nothing.

A rejected reload preserves the current generation: `CompiledKeymap` and `KeymapBindings` keep the
last accepted tables, and only diagnostics change.

### Authored bindings and recovery associations

```rust
pub enum AuthoredKeymapBindings<'keymap> {
    Unavailable(KeymapBindingUnavailability),
    Loaded(&'keymap LoadedKeymapBindings),
}
```

`KeymapBindings::authored()` is the authored-availability view. It is deliberately separate from the
protected recovery associations held by the same resource: a recovery chord must stay visible while
authored bindings are unavailable, so the two coexist rather than one masking the other.
`KeymapBindingUnavailability` names the startup reason — `AwaitingInitialLoad`, `Unconfigured`,
`MissingDefault`, or `InvalidDefault`.

A representative binding for a command is the fewest-strokes sequence, and among equal-length
sequences the structurally first one. Structural order is host-independent, so the displayed chord
does not change with platform.

### Transition cleanup

A generation change, an active-snapshot change, entry into any no-matcher state — awaiting,
unavailable, unmaterializable — and keyboard-ownership handover all run the same routing reset,
which, in order:

1. cancels pending and deferred sequences without firing them;
2. releases physical held sources while preserving semantic-event ownership of the same
   `CustomInput`;
3. records the routing state;
4. inhibits already-pressed keys until they are cleanly released.

Nothing routes until that clean release and recovery. Re-entering the same inactive state does not
reset routing a second time.

## Querying the palette

```rust
pub fn query_command_palette<'registry, 'keymap, 'context, 'status>(
    command_registry: &'registry CommandRegistry,
    active_context: &'context ActiveKeymapContext,
    effective_keymap_status: &'status EffectiveKeymapStatus,
    keymap_bindings: &'keymap KeymapBindings,
    query: &str,
) -> CommandPaletteQueryResult<'registry, 'keymap, 'context, 'status>;
```

The query is borrowed, renderer-independent, trait-free, root-exported, and absent from the prelude.
The result retains immutable borrows of all four inputs — including `EffectiveKeymapStatus`, which no
row necessarily reads — so a result cannot outlive or coexist with a mutable update to any input it
described. Missing-dimension names in a row are borrowed straight from the active context rather than
copied.

Rows are ordered by authored title, then command id. Held commands are not palette-invocable and do
not produce rows; a query that matches only held commands selects `NotPaletteInvocable`.

```rust
pub enum PaletteBinding<'keymap, 'context> {
    ApplicationRecovery(&'keymap Keystroke),
    BoundTo(&'keymap KeystrokeSequence),
    Unbound,
    KeymapUnavailable(KeymapBindingUnavailability),
    AwaitingStateDimensions,
    StateDimensionsUnavailable(&'context [ContextDimensionName]),
    UnmaterializableStateDimensions,
}
```

The variants are exhaustive and mutually exclusive, in reader-visible precedence order:
`ApplicationRecovery` wins over every authored-keymap or state outcome, keymap unavailability comes
next, then state outcomes, then the effective table's `BoundTo`/`Unbound`.

`UnmaterializableStateDimensions` is also what a user sees when the published status, the published
binding table, and the active snapshot disagree. That coherence check is private; the public surface
is this one variant.

### Protected recovery

`with_protected_command_binding(command_id, keystroke)` associates an application-owned recovery
chord with a semantic command. The association is published beside the authored bindings and stays
visible as `PaletteBinding::ApplicationRecovery` through invalid defaults, awaiting state,
unavailable state, and unmaterializable snapshots — every case where authored routing cannot answer.

The chord never enters Rubric's matcher. The application detects and invokes it directly, which is
what makes it recovery: it bypasses both things it must repair, a broken keymap and a broken registry.

Association validation is all-or-none. Assembly rejects a duplicate command, a duplicate keystroke,
an unregistered command, and a command that is not `Capability::Unremappable`; if any association in
the set fails, none are installed and each failure is retained as a
`DiagnosticOrigin::CommandRegistration` diagnostic. That origin is a developer configuration fault,
not a user-fixable one — a consumer renders it as a text-only row with no repair action.

### Rendering

Rubric returns typed results and no prose. UI consumers own wording, layout, IME and focus, repair
actions, and invocation; they retain committed query text and refresh on active-snapshot, binding,
or diagnostic changes.

Fairy Dust's `palette_binding_presentation` and `PaletteBindingPresentation` are example-local
rendering of `PaletteBinding`, consumed only by Fairy Dust's own palette panel and example status
surface. `fairy_dust` is an example crate; nothing in it is a reusable contract. Hana renders
`PaletteBinding` with its own code and does not depend on `fairy_dust`.
