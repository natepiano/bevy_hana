//! Proof that a presenter is putting out what it claims, taken at the output rather than inferred
//! from main-world state.
//!
//! Every main-world probe read green through the black-panel defect while the pixels were black: a
//! component said a texture was bound, a resource said a session was established, a status view
//! said the role was live, and the operator saw black. Nothing in this module trusts any of that.
//! A presenter is confirmed only when a confirmation source — something reading the output itself,
//! or the closest signal the hardware provides — says so, and the confirmation goes stale on its
//! own if the source stops speaking.
//!
//! The shape is three parts. [`OutputProof`] is the always-on, BRP-queryable answer. The
//! [`OutputConfirmation`] trait is the contract a confirmation source implements.
//! [`OutputConfirmationSource`] is the component that binds one source to one presenter and is the
//! only way an entity comes to wear an [`OutputProof`] at all.

use core::marker::PhantomData;
use std::any::TypeId;
use std::any::type_name;
use std::time::Duration;

use bevy::app::App;
use bevy::app::Plugin;
use bevy::app::PostUpdate;
use bevy::ecs::component::Component;
use bevy::ecs::event::EntityEvent;
use bevy::ecs::lifecycle::HookContext;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::ecs::schedule::SystemSet;
use bevy::ecs::world::DeferredWorld;
use bevy::prelude::Commands;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectComponent;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::time::Time;
use bevy::time::Virtual;

/// Whether a presenter is confirmed to be putting out what it claims.
///
/// **Why this exists at all.** Presentation is proven at the output, never by main-world state.
/// Through the black-panel defect every probe that read the main world reported success while the
/// pixels were black, because each of them was reading a decision the code had made rather than a
/// result the hardware had produced. This component holds only what a confirmation source observed.
///
/// **Why [`OutputProof::NoSignalAvailable`] is a value and not an absence.** A presenter whose
/// hardware offers nothing to read is a real, common case — and if it were spelled by leaving the
/// component off, it would be indistinguishable from a presenter that was never wired up. Absence
/// is silent; a variant is not. For the same reason this is never stored as `Option<OutputProof>`:
/// a presenter without a proof component is a defect, and the conformance suite catches it.
///
/// **Why it is always on.** It costs at most one small write per presenter per cadence — a
/// presenter confirmed on every frame refreshes this component only once its newest confirmation is
/// a whole cadence past the one recorded here — and it is the value the operator UI shows when a
/// device stops producing output, the moment when a diagnostic that was switched off is worth
/// nothing.
///
/// `at` and `since` are readings of `Time<Virtual>::elapsed`, which is what the app clock reports
/// outside `FixedMain`, so a BRP reader can compare them directly against `Time`.
#[derive(Component, Reflect, Clone, Debug, PartialEq, Eq)]
#[reflect(Component)]
pub enum OutputProof {
    /// A confirmation source observed output at this app time.
    Confirmed {
        /// App time of the most recent *recorded* confirmation, which is not the newest one.
        ///
        /// The cadence-rate refresh advances this only once the source's newest confirmation is a
        /// whole cadence past the value stored here, so `at` lags the newest confirmation by up to
        /// one cadence; demotion then waits a further cadence for the source to go stale. A
        /// `Confirmed` proof can therefore read just under two cadences old and still be live.
        ///
        /// Never judge staleness from `at`. The variant is the judgement: `Confirmed` means the
        /// presenter is proving its output now, and a presenter that stopped is
        /// [`OutputProof::Unconfirmed`], whose `since` is the last moment it actually spoke.
        at: Duration,
    },
    /// No confirmation has arrived, or the last one aged past the presenter's cadence.
    Unconfirmed {
        /// App time the presenter was last known confirmed, or the time its source was attached
        /// when none ever arrived.
        since: Duration,
    },
    /// This presenter's hardware offers no signal that could confirm its output.
    ///
    /// Written once when the source is attached and never revisited, so a reader can tell "nothing
    /// can prove this" apart from "nothing has proven this yet".
    NoSignalAvailable,
}

/// Whether a confirmation source is able to confirm anything at all.
///
/// The distinction is a property of the source type, not of a moment, so it is an associated const
/// rather than a runtime value: the aging system skips a whole source type without touching an
/// entity.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub enum ConfirmationSignal {
    /// The source can observe output and will call
    /// [`OutputConfirmationSource::confirm`] when it does.
    Provided,
    /// The source can never observe output; its presenters stay
    /// [`OutputProof::NoSignalAvailable`].
    Absent,
}

/// The contract a confirmation source implements to feed an [`OutputProof`].
///
/// A source is whatever is closest to the pixels for a given presenter — a presented-frame
/// callback, a hardware readback, a link-status line. The trait deliberately says nothing about
/// *how* the output is observed, only that the source declares how often it expects to observe it
/// and keeps whatever bookkeeping that costs.
pub trait OutputConfirmation: Send + Sync + 'static {
    /// Whether this source can confirm output at all.
    const SIGNAL: ConfirmationSignal;

    /// The presenter's own expected interval between confirmations.
    ///
    /// A confirmation older than this marks the presenter unconfirmed. It belongs to the source
    /// rather than to the kernel because only the source can state its hardware's normal rhythm
    /// — a 60 Hz display and a once-a-second link probe are both healthy.
    fn cadence(&self) -> Duration;

    /// The source's own bookkeeping when a confirmation arrives.
    ///
    /// A source that needs none leaves the body empty; the proof itself is written by
    /// [`OutputConfirmationSource::confirm`], never here.
    ///
    /// **Why the clock is `Time<Virtual>` and not `Time`.** Bevy swaps the generic `Time` to
    /// `Time<Fixed>` for the duration of `FixedMain`, so a confirmation taken from a fixed-step
    /// system and a proof aged in `PostUpdate` would be stamped from two different clocks and
    /// compared as though they were one. Naming `Time<Virtual>` by type removes the choice:
    /// every `at` and `since` in this module is the same clock's reading wherever it was taken.
    /// Outside `FixedMain`, `Time<Virtual>::elapsed` equals the app clock's elapsed, so a BRP
    /// reader comparing an [`OutputProof`] against `Time` sees the same numbers.
    fn confirm(&mut self, time: &Time<Virtual>);
}

/// The confirmation source for a presenter whose hardware can never prove its output.
///
/// It never confirms, and the aging system never demotes it, so its presenters hold
/// [`OutputProof::NoSignalAvailable`] for their whole lifetime. It exists so that such a presenter
/// still declares a source and still wears a proof: the absence of evidence is stated, not implied.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoConfirmationSignal;

impl OutputConfirmation for NoConfirmationSignal {
    const SIGNAL: ConfirmationSignal = ConfirmationSignal::Absent;

    /// Reports a cadence that never lapses; the aging system skips this source before reading it.
    fn cadence(&self) -> Duration { Duration::MAX }

    fn confirm(&mut self, _: &Time<Virtual>) {}
}

/// Whether a confirmation has ever arrived for a presenter, and when.
///
/// Two facts — "has anything ever confirmed this" and "when" — never share one bare [`Duration`]:
/// a source that has never spoken and a source last heard from at t=0 are different situations,
/// and the type states which one a number means so no reader has to trace callers to find out.
/// [`Self::NeverConfirmed`] carries nothing, because the moment a source was attached is already
/// recorded where a reader needs it — in the presenter's [`OutputProof::Unconfirmed`] `since`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LatestConfirmation {
    /// No confirmation has arrived since the source was attached.
    NeverConfirmed,
    /// A confirmation arrived at this app time.
    Confirmed {
        /// App time of the most recent confirmation.
        at: Duration,
    },
}

/// Which confirmation source type wrote this entity's [`OutputProof`].
///
/// The proof alone cannot say who owns it, and that ambiguity has two costs: re-inserting the same
/// source type — a replace — looks identical to attaching a second, contradictory source, and an
/// [`OutputProof`] someone inserted by hand looks identical to one this module wrote. Recording
/// the owning type turns both into distinguishable, separately reported cases. It is written and
/// removed with the proof, and never leaves this crate.
#[derive(Component)]
struct ProofOwningSource {
    source_type: TypeId,
    source_name: &'static str,
}

/// Marks that [`OutputProofPlugin`] installed aging for one confirmation source type.
///
/// Without it, a source inserted into an app whose owner forgot the plugin freezes at
/// [`OutputProof::Unconfirmed`] forever with nothing reporting why — the silent green reading this
/// whole module exists to prevent. The `on_insert` hook reads this resource so the omission fails
/// loudly at the insert instead of quietly at every frame after it.
#[derive(Resource)]
struct OutputProofAgingInstalled<C: OutputConfirmation>(PhantomData<fn() -> C>);

/// The confirmation source a presenter declares, and the only way to wear an [`OutputProof`].
///
/// Inserting this component writes the presenter's first proof; removing it takes the proof away.
/// Routing every proof through one component is what makes "a presenter without a proof is a
/// defect" checkable — there is no second path by which a proof could appear or vanish.
///
/// **Invariant: at most one confirmation source type per entity.** A presenter has exactly one
/// source for its lifetime. Re-inserting the *same* source type is a replace and is allowed: the
/// presenter starts over unproven, exactly as it would on a first attach, because the replacing
/// source instance has observed nothing yet and crediting it with the previous instance's
/// confirmation would be inferring output from main-world state. That also makes the two replace
/// routes agree — remove-then-insert and insert-over both leave the presenter unproven — so a
/// reader cannot tell which route was taken. Inserting a *different* source type panics, naming
/// both types. Inserting a source onto an entity that already carries an [`OutputProof`] no source
/// wrote also panics, because a proof with no owner means something outside this module has been
/// writing proofs.
///
/// Those checks reach every separate insert, because the hook's queued proof and owner marker are
/// flushed before the next insert's hook reads the entity. They do not reach two sources inserted
/// in one bundle: neither hook has flushed when the other runs, so both see a bare entity and both
/// pass. A bundle carrying two sources is outside this contract and is not detected.
#[derive(Component)]
#[component(on_insert = Self::on_insert, on_remove = Self::on_remove)]
pub struct OutputConfirmationSource<C: OutputConfirmation> {
    source: C,
    latest: LatestConfirmation,
}

impl<C: OutputConfirmation> OutputConfirmationSource<C> {
    /// Declare `source` as this presenter's confirmation source.
    ///
    /// The attachment time is not a parameter: the `on_insert` hook reads the app clock itself, so
    /// no caller can attach a source with a time that disagrees with the proof it produces.
    #[must_use]
    pub const fn new(source: C) -> Self {
        Self {
            source,
            latest: LatestConfirmation::NeverConfirmed,
        }
    }

    /// Record that output was observed now, reading the moment from the app clock.
    ///
    /// This is how a source reports a confirmation. A caller confirms through the component and
    /// never writes [`OutputProof`] itself: after insertion, the aging system is the only writer
    /// of the proof, which is what keeps a confirmation and its proof from disagreeing.
    ///
    /// The moment is taken from the passed `Time<Virtual>` rather than accepted as a [`Duration`],
    /// so no caller can stamp a confirmation with a reading from a clock the proof is not compared
    /// against — see [`OutputConfirmation::confirm`] for why that clock in particular. The same
    /// clock is forwarded to the source's own [`OutputConfirmation::confirm`].
    pub fn confirm(&mut self, time: &Time<Virtual>) {
        self.latest = LatestConfirmation::Confirmed { at: time.elapsed() };
        self.source.confirm(time);
    }

    /// Read the source's own state.
    #[must_use]
    pub const fn source(&self) -> &C { &self.source }

    /// Write the presenter's first proof and record which source type owns it.
    ///
    /// A presenter starts unproven, so its first confirmation is a flip an observer can see rather
    /// than a state it was born in.
    ///
    /// # Panics
    ///
    /// Panics when this source type's [`OutputProofPlugin`] was never added, when the entity
    /// already carries an [`OutputProof`] owned by a different source type, and when it carries an
    /// [`OutputProof`] that no confirmation source wrote.
    fn on_insert(mut world: DeferredWorld<'_>, context: HookContext) {
        let entity = context.entity;
        let incoming_name = type_name::<C>();

        // Checked before anything is written: a proof this app will never age is worse than no
        // proof at all, because it reads as a settled answer.
        assert!(
            world
                .get_resource::<OutputProofAgingInstalled<C>>()
                .is_some(),
            "OutputProofPlugin was never added for confirmation source {incoming_name}, so entity \
             {entity}'s OutputProof would never age; add \
             OutputProofPlugin::<{incoming_name}>::new() beside the source"
        );

        let owner = world
            .get::<ProofOwningSource>(entity)
            .map(|owner| (owner.source_type, owner.source_name));
        let carries_proof = world.get::<OutputProof>(entity).is_some();

        assert!(
            !carries_proof || owner.is_some(),
            "entity {entity} carries an OutputProof that no OutputConfirmationSource wrote; the \
             proof is written only by inserting a source, so remove that hand-written OutputProof \
             before declaring {incoming_name}"
        );

        if let Some((owner_type, owner_name)) = owner {
            // A same-type re-insert falls through to the first-attach write below. Bevy fires
            // `on_replace` and `on_insert` for a replace but never `on_remove`, so the old proof
            // survives on the entity; leaving it there would strand a `Confirmed` reading that the
            // replacing source instance — whose `latest` starts at `NeverConfirmed` — can never
            // age or demote. A presenter frozen at `Confirmed` while nothing watches the output is
            // the exact silent-green reading this module exists to prevent.
            assert!(
                owner_type == TypeId::of::<C>(),
                "a presenter has one confirmation source for its lifetime; entity {entity} \
                 already carries an OutputProof owned by OutputConfirmationSource<{owner_name}>, \
                 so remove that source before inserting \
                 OutputConfirmationSource<{incoming_name}>"
            );
        }

        let clock = world.get_resource::<Time<Virtual>>();
        assert!(
            clock.is_some(),
            "the Time<Virtual> clock is missing, so entity {entity}'s OutputProof would be \
             stamped from a clock that does not exist; add TimePlugin (MinimalPlugins and \
             DefaultPlugins both include it) before inserting a confirmation source"
        );
        let now = clock.map_or(Duration::ZERO, Time::elapsed);

        let proof = match C::SIGNAL {
            ConfirmationSignal::Provided => OutputProof::Unconfirmed { since: now },
            ConfirmationSignal::Absent => OutputProof::NoSignalAvailable,
        };
        world.commands().entity(entity).insert((
            proof,
            ProofOwningSource {
                source_type: TypeId::of::<C>(),
                source_name: incoming_name,
            },
        ));
    }

    /// Take the proof and its ownership record away with the source, so a presenter never keeps a
    /// proof nothing can refresh.
    fn on_remove(mut world: DeferredWorld<'_>, context: HookContext) {
        world
            .commands()
            .entity(context.entity)
            .try_remove::<(OutputProof, ProofOwningSource)>();
    }
}

/// A presenter's output became confirmed.
///
/// Emitted on each `Unconfirmed -> Confirmed` flip so a consumer observes the transition instead of
/// comparing [`OutputProof`] against its own copy every frame.
#[derive(Debug, EntityEvent, Reflect)]
pub struct OutputConfirmed {
    /// Presenter whose output became confirmed.
    #[event_target]
    pub presenter: Entity,
    /// App time of the confirmation that flipped it.
    pub at:        Duration,
}

/// A presenter's output stopped being confirmed.
///
/// Emitted on each `Confirmed -> Unconfirmed` flip: the presenter's last confirmation aged past its
/// source's cadence. This is the edge an operator UI reacts to when a device stops producing
/// output.
///
/// A presenter that has never been confirmed never emits this, however long it waits. It holds
/// [`OutputProof::Unconfirmed`] from the moment its source was attached, and nothing was lost.
/// The asymmetry with [`OutputConfirmed`] is deliberate: this event reports a presenter that
/// stopped, not one that never started, and a UI that treated the two alike would raise an alarm
/// for every presenter still waiting on its first frame.
#[derive(Debug, EntityEvent, Reflect)]
pub struct OutputLost {
    /// Presenter whose output stopped being confirmed.
    #[event_target]
    pub presenter: Entity,
    /// App time of the last confirmation this presenter had.
    pub since:     Duration,
}

/// Ordering boundary for the one system that ages [`OutputProof`].
///
/// Public because a confirmation source's own confirming system orders itself
/// `.before(OutputProofSystems::Age)`: a confirmation taken in a frame is then aged in that same
/// frame and never one later, so a proof can never lag its evidence by a frame.
///
/// **This set lives in [`PostUpdate`], and that is what determines who needs the ordering.** A
/// confirming system in `Update` needs none — `Update` already runs before `PostUpdate`. A
/// confirming system in `PostUpdate` needs the `.before` and gets a silent one-frame lag without
/// it. A confirming system in any other schedule gets nothing from the `.before` at all: an
/// ordering against a set with no members in that schedule is a no-op, and no warning is reported
/// for it, so a wrong schedule reads exactly like a correct one.
///
/// This enum is non-exhaustive for the same reason [`crate::RiggingSystems`] is: a later phase can
/// add an ordering boundary without making a downstream match on `OutputProofSystems` incomplete.
#[derive(Clone, Debug, Hash, PartialEq, Eq, SystemSet)]
#[non_exhaustive]
pub enum OutputProofSystems {
    /// Confirmations recorded this frame are folded into each presenter's [`OutputProof`].
    Age,
}

/// Fold this frame's confirmations into each presenter's [`OutputProof`], emitting the flips.
///
/// After insertion this is the only writer of [`OutputProof`]. It is generic over the source type
/// so that reading a source's cadence costs no dynamic dispatch and so that a crate installs aging
/// only for the sources it declares.
///
/// A confirmation older than the source's cadence proves nothing in either direction: it neither
/// confirms a presenter nor re-confirms one that has already been demoted for that same stale
/// reading. Both the promotion and the demotion therefore test freshness, which is what keeps a
/// source that stopped speaking from alternating [`OutputConfirmed`] and [`OutputLost`] every
/// frame.
///
/// **Why a confirmed presenter's `at` is refreshed at cadence rate and not every frame.** A
/// presenter confirmed on every frame — the healthy case, and the common one — advanced `at` on
/// every frame, so the always-on component was rewritten sixty times a second per presenter and
/// woke every change detector watching it, to say the same thing each time. The source's cadence is
/// already the resolution at which a confirmation means anything, so `at` moves at that resolution:
/// one write per cadence, and freshness still read from the source's own latest confirmation on
/// every frame. Staleness is therefore never inferred from the proof's `at`, which now lags the
/// truth by up to one cadence, but always from the source's [`LatestConfirmation`] — which is why
/// demotion asks only whether the latest confirmation is fresh.
///
/// It is private because nothing outside this module has any reason to call it: `OutputProofPlugin`
/// is how aging is installed, and `OutputProofSystems::Age` is how a caller orders against it.
fn age_output_proofs<C: OutputConfirmation>(
    time: Res<Time<Virtual>>,
    mut presenters: Query<(Entity, &mut OutputProof, &OutputConfirmationSource<C>)>,
    mut commands: Commands,
) {
    if matches!(C::SIGNAL, ConfirmationSignal::Absent) {
        return;
    }
    let now = time.elapsed();
    for (presenter, mut proof, source) in &mut presenters {
        let LatestConfirmation::Confirmed { at: latest_at } = source.latest else {
            continue;
        };
        let cadence = source.source.cadence();
        // One reading of freshness per presenter, taken from the source's own latest confirmation
        // and never from the proof's `at`, which lags it by up to one cadence. All three arms
        // consult it: promotion and the cadence refresh demand a fresh confirmation, demotion
        // demands a stale one.
        let fresh = latest_at.saturating_add(cadence) >= now;
        // Each arm states its whole condition, so the arms may be read or reordered in any order.
        // An arm that relied on an earlier arm having failed would change meaning the moment
        // someone moved it, and nothing in the type would say so. In particular the refresh and
        // the demotion are mutually exclusive rather than merely ordered, so a pending refresh can
        // never postpone a loss by a frame.
        match *proof {
            // A stale latest confirmation never promotes. Without this guard a source that went
            // quiet would flip Confirmed on one frame and Lost on the next, forever, because the
            // demotion leaves `latest` untouched for this arm to read again.
            OutputProof::Unconfirmed { .. } if fresh => {
                *proof = OutputProof::Confirmed { at: latest_at };
                commands.trigger(OutputConfirmed {
                    presenter,
                    at: latest_at,
                });
            },
            // The cadence-rate refresh: `at` advances once the presenter's newest confirmation is
            // a whole cadence past the one the proof records, so a presenter confirmed every frame
            // writes this component once per cadence rather than once per frame.
            OutputProof::Confirmed { at } if fresh && latest_at >= at.saturating_add(cadence) => {
                *proof = OutputProof::Confirmed { at: latest_at };
            },
            // Demotion asks only whether the presenter's newest confirmation has gone stale. It
            // cannot compare against the proof's `at`, which the refresh above deliberately leaves
            // behind; `since` is the latest confirmation for the same reason — it is the last
            // moment this presenter actually spoke.
            OutputProof::Confirmed { .. } if !fresh => {
                *proof = OutputProof::Unconfirmed { since: latest_at };
                commands.trigger(OutputLost {
                    presenter,
                    since: latest_at,
                });
            },
            OutputProof::Unconfirmed { .. }
            | OutputProof::Confirmed { .. }
            | OutputProof::NoSignalAvailable => {},
        }
    }
}

/// Installs aging for one confirmation source type.
///
/// The crate that declares a source adds this plugin beside that source. It is deliberately not
/// unique: two crates — or two drivers — declaring the same source type is normal, and every driver
/// with no signal at all declares [`NoConfirmationSignal`]. Adding it twice must therefore neither
/// panic nor register the aging system twice, so `is_unique` is `false` and `build` returns early
/// on the second add.
pub struct OutputProofPlugin<C: OutputConfirmation>(PhantomData<fn() -> C>);

impl<C: OutputConfirmation> OutputProofPlugin<C> {
    /// Install aging for confirmation source `C`.
    #[must_use]
    pub const fn new() -> Self { Self(PhantomData) }
}

impl<C: OutputConfirmation> Default for OutputProofPlugin<C> {
    fn default() -> Self { Self::new() }
}

impl<C: OutputConfirmation> Plugin for OutputProofPlugin<C> {
    fn build(&self, app: &mut App) {
        if app.is_plugin_added::<Self>() {
            return;
        }
        // The receipt `on_insert` reads to tell "this app ages proofs for C" from "someone forgot
        // the plugin". Nothing here installs a clock: `Time<Virtual>` comes from `TimePlugin`, and
        // substituting a frozen stand-in for it would trade a loud failure for a proof that ages
        // by zero every frame.
        app.insert_resource(OutputProofAgingInstalled::<C>(PhantomData));
        app.add_systems(
            PostUpdate,
            age_output_proofs::<C>.in_set(OutputProofSystems::Age),
        );
    }

    fn is_unique(&self) -> bool { false }
}
