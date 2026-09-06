//! Behavior coverage for `OutputProof` — the always-on component that says whether a presenter is
//! confirmed to be putting out what it claims.
//!
//! Every assertion here is written against the phase specification, not against the module's
//! implementation: the proof is a value a BRP reader compares against the app clock, so the tests
//! drive a deterministic clock and assert the exact `Duration` each variant carries rather than
//! merely which variant is present. A test that accepted any `Unconfirmed` would pass on a proof
//! whose `since` pointed at the wrong moment, and the operator UI reads that field.
//!
//! The clock these tests drive and read is [`Time<Virtual>`], the same clock the module writes its
//! timestamps from. `Time<()>` is deliberately never read here: Bevy swaps it to `Time<Fixed>`
//! inside `FixedMain`, so a fixture that took its expected timestamps from `Time<()>` would agree
//! with the module by coincidence of schedule rather than by contract.

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use bevy::MinimalPlugins;
use bevy::app::App;
use bevy::app::PostUpdate;
use bevy::ecs::change_detection::DetectChanges;
use bevy::ecs::observer::On;
use bevy::ecs::reflect::AppTypeRegistry;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::schedule::IntoScheduleConfigs;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Ref;
use bevy::prelude::Res;
use bevy::prelude::ResMut;
use bevy::prelude::Resource;
use bevy::reflect::TypePath;
use bevy::time::Time;
use bevy::time::TimeUpdateStrategy;
use bevy::time::Virtual;
use hana_rigging::prelude::ConfirmationSignal;
use hana_rigging::prelude::NoConfirmationSignal;
use hana_rigging::prelude::OutputConfirmation;
use hana_rigging::prelude::OutputConfirmationSource;
use hana_rigging::prelude::OutputConfirmed;
use hana_rigging::prelude::OutputLost;
use hana_rigging::prelude::OutputProof;
use hana_rigging::prelude::OutputProofPlugin;
use hana_rigging::prelude::OutputProofSystems;

/// One frame of the manual clock. Kept well under `Time<Virtual>`'s 250ms `max_delta` so a frame
/// step is never clamped and elapsed time is exactly `frames * FRAME_STEP`.
const FRAME_STEP: Duration = Duration::from_millis(100);

/// The confirmation interval every scripted presenter in this file declares unless it says
/// otherwise. Two and a half frames, so a lapse takes three idle frames to cross and the crossing
/// frame is unambiguous.
const CADENCE: Duration = Duration::from_millis(250);

/// The tighter of the two cadences in the mixed-cadence test: one and a half frames, so it lapses
/// on the second idle frame.
const SHORT_CADENCE: Duration = Duration::from_millis(150);

/// The looser of the two cadences in the mixed-cadence test: four and a half frames, so it is still
/// fresh on the frame the short cadence lapses and only lapses three frames later.
const LONG_CADENCE: Duration = Duration::from_millis(450);

/// A confirmation source the tests script by hand.
///
/// It counts the confirmations forwarded to it so a test can assert that confirming through the
/// component reaches the source's own bookkeeping, which is the whole reason `confirm` is on the
/// trait rather than only on the component.
#[derive(Debug)]
struct ScriptedConfirmation {
    /// Interval this presenter expects between confirmations.
    cadence:       Duration,
    /// How many confirmations the component has forwarded here.
    confirmations: usize,
}

impl ScriptedConfirmation {
    /// Build a source that expects a confirmation every `cadence` and has received none.
    const fn new(cadence: Duration) -> Self {
        Self {
            cadence,
            confirmations: 0,
        }
    }

    /// How many confirmations reached this source.
    const fn confirmations(&self) -> usize { self.confirmations }
}

impl OutputConfirmation for ScriptedConfirmation {
    const SIGNAL: ConfirmationSignal = ConfirmationSignal::Provided;

    fn cadence(&self) -> Duration { self.cadence }

    fn confirm(&mut self, _: &Time<Virtual>) { self.confirmations += 1; }
}

/// A confirmation source that records how often the aging pass asked it for its cadence.
///
/// This is the instrument for the idempotent-plugin test: whether `age_output_proofs::<C>` was
/// registered once or twice is invisible in the proof value itself, because a second pass over an
/// already-settled proof writes nothing. It is visible in how many times one frame reads the
/// cadence, which the aging rule has to read to decide whether a confirmation has gone stale.
#[derive(Debug)]
struct CountingConfirmation {
    /// Interval this presenter expects between confirmations.
    cadence:       Duration,
    /// Shared with the test so it can read the count without touching the world.
    cadence_reads: Arc<AtomicUsize>,
}

impl OutputConfirmation for CountingConfirmation {
    const SIGNAL: ConfirmationSignal = ConfirmationSignal::Provided;

    fn cadence(&self) -> Duration {
        self.cadence_reads.fetch_add(1, Ordering::Relaxed);
        self.cadence
    }

    fn confirm(&mut self, _: &Time<Virtual>) {}
}

/// Whether the scripted confirming systems are currently reporting output.
///
/// Flipping this is how a test makes a presenter go quiet without despawning it or removing its
/// source, which is what a real device that stops producing frames does.
#[derive(Resource, Clone, Copy, Debug)]
enum ConfirmationSchedule {
    /// The scripted sources confirm once per frame.
    Confirming,
    /// The scripted sources report nothing.
    Quiet,
}

/// Every `OutputConfirmed` and `OutputLost` observed, in order.
///
/// The transitions are the product under test, so they are recorded rather than sampled: a test
/// that only read the final proof could not tell one flip from a proof that oscillated all the way
/// there.
#[derive(Resource, Default)]
struct ProofEventLog {
    /// Presenter and confirmation time of each `OutputConfirmed`.
    confirmed: Vec<(Entity, Duration)>,
    /// Presenter and last-confirmed time of each `OutputLost`.
    lost:      Vec<(Entity, Duration)>,
}

/// Record one confirmed transition.
fn record_output_confirmed(confirmed: On<OutputConfirmed>, mut log: ResMut<ProofEventLog>) {
    log.confirmed.push((confirmed.presenter, confirmed.at));
}

/// Record one lost transition.
fn record_output_lost(lost: On<OutputLost>, mut log: ResMut<ProofEventLog>) {
    log.lost.push((lost.presenter, lost.since));
}

/// Confirm every scripted presenter once per frame while the schedule says to.
fn confirm_scripted_sources(
    time: Res<Time<Virtual>>,
    schedule: Res<ConfirmationSchedule>,
    mut sources: Query<&mut OutputConfirmationSource<ScriptedConfirmation>>,
) {
    if matches!(*schedule, ConfirmationSchedule::Quiet) {
        return;
    }
    for mut source in &mut sources {
        source.confirm(&time);
    }
}

/// Confirm every counting presenter once per frame while the schedule says to.
fn confirm_counting_sources(
    time: Res<Time<Virtual>>,
    schedule: Res<ConfirmationSchedule>,
    mut sources: Query<&mut OutputConfirmationSource<CountingConfirmation>>,
) {
    if matches!(*schedule, ConfirmationSchedule::Quiet) {
        return;
    }
    for mut source in &mut sources {
        source.confirm(&time);
    }
}

/// App time of every frame in which a presenter's `OutputProof` was actually written.
///
/// The write rate is the product under test, and change detection is the only place it shows: a
/// refresh that rewrote `Confirmed { at }` with the value the component already held would be
/// invisible in the proof and visible here. It is also what every reactive reader of the component
/// keys off — a `Changed<OutputProof>` system, a BRP watch — so counting writes is counting the
/// wake-ups a presenter costs its consumers, not an implementation detail.
#[derive(Resource, Default)]
struct ProofWriteLog {
    /// Presenter and app time of each frame the proof was written in.
    writes: Vec<(Entity, Duration)>,
}

/// Record every proof written this frame, after the aging pass that writes them.
fn record_proof_writes(
    time: Res<Time<Virtual>>,
    mut log: ResMut<ProofWriteLog>,
    proofs: Query<(Entity, Ref<'_, OutputProof>)>,
) {
    let now = time.elapsed();
    for (presenter, proof) in &proofs {
        if proof.is_changed() {
            log.writes.push((presenter, now));
        }
    }
}

/// An app with a deterministic clock, the aging system for scripted sources, and observers on both
/// transition events.
fn scripted_proof_app() -> App {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(OutputProofPlugin::<ScriptedConfirmation>::default())
        .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP))
        .insert_resource(ConfirmationSchedule::Confirming)
        .init_resource::<ProofEventLog>()
        .add_observer(record_output_confirmed)
        .add_observer(record_output_lost)
        .add_systems(
            PostUpdate,
            confirm_scripted_sources.before(OutputProofSystems::Age),
        );
    app
}

/// Spawn a presenter wearing a scripted source with `cadence`, settling the hook's deferred write.
fn spawn_presenter_with_cadence(app: &mut App, cadence: Duration) -> Entity {
    let presenter = app
        .world_mut()
        .spawn(OutputConfirmationSource::new(ScriptedConfirmation::new(
            cadence,
        )))
        .id();
    app.world_mut().flush();
    presenter
}

/// Spawn a presenter wearing a scripted source at the file's default cadence.
fn spawn_scripted_presenter(app: &mut App) -> Entity { spawn_presenter_with_cadence(app, CADENCE) }

/// App time as of the most recent frame, read from the clock the module stamps its proofs with.
fn elapsed(app: &App) -> Duration { app.world().resource::<Time<Virtual>>().elapsed() }

/// The presenter's current proof, or `None` when it wears no proof at all.
fn proof(app: &App, presenter: Entity) -> Option<OutputProof> {
    app.world().get::<OutputProof>(presenter).cloned()
}

/// How many confirmations the presenter's own source has recorded.
fn source_confirmations(app: &App, presenter: Entity) -> Option<usize> {
    app.world()
        .get::<OutputConfirmationSource<ScriptedConfirmation>>(presenter)
        .map(|source| source.source().confirmations())
}

/// Every confirmed transition observed so far.
fn confirmed_events(app: &App) -> Vec<(Entity, Duration)> {
    app.world().resource::<ProofEventLog>().confirmed.clone()
}

/// Every lost transition observed so far.
fn lost_events(app: &App) -> Vec<(Entity, Duration)> {
    app.world().resource::<ProofEventLog>().lost.clone()
}

/// The scripted app with a counter reading every proof the aging pass wrote.
///
/// A separate builder rather than a flag on `scripted_proof_app`: the counter is only meaningful
/// for the write-rate case, and adding it to every fixture would have each unrelated test paying
/// for a log it never reads.
fn write_counting_proof_app() -> App {
    let mut app = scripted_proof_app();
    app.init_resource::<ProofWriteLog>().add_systems(
        PostUpdate,
        record_proof_writes.after(OutputProofSystems::Age),
    );
    app
}

/// Every proof write observed so far.
fn proof_writes(app: &App) -> Vec<(Entity, Duration)> {
    app.world().resource::<ProofWriteLog>().writes.clone()
}

/// Forget the writes observed so far, so a measurement window can start on a known frame.
fn clear_proof_writes(app: &mut App) {
    app.world_mut()
        .resource_mut::<ProofWriteLog>()
        .writes
        .clear();
}

/// Stop the scripted sources confirming.
fn go_quiet(app: &mut App) { app.world_mut().insert_resource(ConfirmationSchedule::Quiet); }

/// Stop confirming and run frames until the cadence lapses, returning the last confirmation time.
///
/// The caller gets the exact `Duration` the proof is required to carry into `Unconfirmed { since
/// }`, so the lapse assertions never hardcode a frame count.
fn confirm_then_go_quiet(
    app: &mut App,
    frames_of_confirmation: usize,
    idle_frames: usize,
) -> Duration {
    for _ in 0..frames_of_confirmation {
        app.update();
    }
    let last_confirmed_at = elapsed(app);
    go_quiet(app);
    for _ in 0..idle_frames {
        app.update();
    }
    last_confirmed_at
}

/// How many times one frame reads the cadence of an idle-but-still-confirmed presenter, with the
/// plugin added `plugin_additions` times.
///
/// The measurement window is deliberately after the first confirmation and before the cadence
/// lapses: that is the state in which the aging rule must evaluate freshness, so the read is
/// guaranteed to happen and a duplicate registration would double it.
fn cadence_reads_over_idle_frames(plugin_additions: usize, idle_frames: usize) -> usize {
    let cadence_reads = Arc::new(AtomicUsize::new(0));
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP))
        .insert_resource(ConfirmationSchedule::Confirming)
        .add_systems(
            PostUpdate,
            confirm_counting_sources.before(OutputProofSystems::Age),
        );
    for _ in 0..plugin_additions {
        app.add_plugins(OutputProofPlugin::<CountingConfirmation>::default());
    }
    app.world_mut()
        .spawn(OutputConfirmationSource::new(CountingConfirmation {
            cadence:       CADENCE,
            cadence_reads: Arc::clone(&cadence_reads),
        }));
    app.world_mut().flush();

    app.update();
    app.world_mut().insert_resource(ConfirmationSchedule::Quiet);
    cadence_reads.store(0, Ordering::Relaxed);
    for _ in 0..idle_frames {
        app.update();
    }
    cadence_reads.load(Ordering::Relaxed)
}

#[test]
fn a_freshly_inserted_source_reads_unconfirmed_before_its_first_confirmation() {
    let mut app = scripted_proof_app();
    app.update();
    app.update();
    let inserted_at = elapsed(&app);

    let presenter = spawn_scripted_presenter(&mut app);

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed { since: inserted_at }),
        "a presenter starts unproven, carrying the moment its source was inserted, so its first \
         confirmation is a flip and not a no-op"
    );
    assert!(
        confirmed_events(&app).is_empty() && lost_events(&app).is_empty(),
        "insertion alone is not a transition and must publish no event"
    );
}

#[test]
fn a_confirming_source_holds_a_confirmed_proof_and_flips_only_once() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);

    for _ in 0..6 {
        app.update();
    }

    // `CADENCE` is two and a half frames, so the refresh lands on the fourth frame and the fifth
    // and sixth confirmations are inside the cadence the fourth one bought.
    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: FRAME_STEP * 3 }),
        "a source confirming every frame stays proven, its `at` advancing at cadence rate rather \
         than once per frame: a presenter that is healthy every frame must not wake every reader \
         of its proof every frame to say so"
    );
    assert!(
        elapsed(&app) > FRAME_STEP * 3,
        "the run must actually continue past the last refresh, or the assertion above cannot tell \
         a cadence-rate stamp from a per-frame one"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        1,
        "only the first confirmation is a transition; refreshing `at` publishes nothing, or every \
         consumer would be woken once per frame"
    );
    assert!(
        lost_events(&app).is_empty(),
        "a source that never stops confirming is never lost"
    );
    assert_eq!(
        source_confirmations(&app, presenter),
        Some(6),
        "confirming through the component forwards to the source's own bookkeeping every time"
    );
}

#[test]
fn a_source_that_stops_confirming_is_lost_once_and_stays_lost() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);

    // Three confirming frames, then enough quiet frames to carry `now` past `last + CADENCE`.
    let last_confirmed_at = confirm_then_go_quiet(&mut app, 3, 3);

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: last_confirmed_at,
        }),
        "a lapsed presenter reverts to unconfirmed, carrying the last moment it was actually \
         confirmed rather than the moment the lapse was noticed"
    );
    assert_eq!(
        lost_events(&app),
        vec![(presenter, last_confirmed_at)],
        "the lapse is published exactly once, naming the presenter and when its output was last \
         proven"
    );

    let confirmed_at_lapse = confirmed_events(&app).len();

    for _ in 0..6 {
        app.update();
    }

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: last_confirmed_at,
        }),
        "a presenter that is still silent stays unconfirmed; nothing in the world has re-proven it"
    );
    assert_eq!(
        lost_events(&app).len(),
        1,
        "staying lost is not a transition and must not publish a second loss"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        confirmed_at_lapse,
        "a stale confirmation must never re-prove a lost presenter: without a fresh confirmation \
         the proof would oscillate and publish an event every frame"
    );
}

#[test]
fn a_presenter_confirmed_every_frame_writes_its_proof_once_per_cadence() {
    /// Frames of uninterrupted confirmation the write rate is measured over.
    const MEASURED_FRAMES: u32 = 12;

    let mut app = write_counting_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);

    // The promotion frame is left out of the measurement window on purpose: it is a real
    // transition and must write, and Bevy reports every component changed on a detecting system's
    // very first run, so a window that included it would be measuring the harness.
    app.update();
    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: Duration::ZERO }),
        "the presenter is proven on its first confirmation before the rate is measured; the \
         manual clock does not advance on the first update, so that confirmation is stamped at \
         the origin"
    );
    clear_proof_writes(&mut app);

    for _ in 0..MEASURED_FRAMES {
        app.update();
    }

    // `CADENCE` is two and a half frames, so `at` advances on the first frame whose confirmation
    // is a whole cadence past the one the proof records — every third frame, counting from the
    // confirmation at the origin.
    let expected_writes = vec![
        (presenter, FRAME_STEP * 3),
        (presenter, FRAME_STEP * 6),
        (presenter, FRAME_STEP * 9),
        (presenter, FRAME_STEP * 12),
    ];
    assert_eq!(
        proof_writes(&app),
        expected_writes,
        "a presenter confirmed every frame writes its proof once per cadence, on the frames its \
         newest confirmation is a whole cadence past the recorded one. The proof is always on and \
         every presenter wears one, so a per-frame write is a per-frame wake-up for every reader \
         of the component across every presenter in the app"
    );
    assert!(
        proof_writes(&app).len() < usize::try_from(MEASURED_FRAMES).unwrap_or(usize::MAX),
        "the rate must actually be below one write per frame, or this test would pass on the \
         per-frame write it exists to rule out"
    );
    let writes = proof_writes(&app);
    for ((_, earlier), (_, later)) in writes.iter().zip(writes.iter().skip(1)) {
        assert!(
            later.saturating_sub(*earlier) >= CADENCE,
            "successive writes are at least one cadence apart; {earlier:?} then {later:?} is not"
        );
    }

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed {
            at: FRAME_STEP * 12,
        }),
        "each write carries the confirmation that triggered it, so on a refresh frame `at` is the \
         newest confirmation exactly. Between refreshes it falls up to one cadence behind, and \
         because demotion then waits a further cadence a `Confirmed` proof can read just under two \
         cadences old and still be live: a reader judges staleness from `OutputLost` and from \
         `Unconfirmed`'s own `since`, never from `at`"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        1,
        "refreshing `at` is not a transition, whatever rate it runs at"
    );
    assert!(
        lost_events(&app).is_empty(),
        "a source confirming every frame is never lost, and the slower write rate must not make \
         it look like one"
    );
    assert_eq!(
        source_confirmations(&app, presenter),
        Some(usize::try_from(MEASURED_FRAMES).unwrap_or(usize::MAX) + 1),
        "every frame really did confirm; the rate under test is the proof's write rate, not the \
         source's confirmation rate"
    );
}

#[test]
fn a_presenter_confirmed_for_many_frames_and_then_starved_is_lost_once_a_cadence_later() {
    /// Enough confirming frames that the last refresh is deliberately not the last confirmation.
    const CONFIRMING_FRAMES: u32 = 11;

    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);

    for _ in 0..CONFIRMING_FRAMES {
        app.update();
    }
    let last_confirmed_at = elapsed(&app);

    // The proof's own stamp is a frame behind the last confirmation here, and that gap is the
    // hazard this case exists for: a demotion that asked whether the confirmation had stopped
    // advancing past the recorded one would never fire, and this presenter would read healthy
    // for the rest of the run with nothing watching its output.
    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: FRAME_STEP * 9 }),
        "the cadence-rate refresh must actually have left the proof's stamp behind the last \
         confirmation, or the starvation below is not the case under test"
    );
    assert_ne!(
        FRAME_STEP * 9,
        last_confirmed_at,
        "the recorded moment and the last confirmation must differ, or this test proves nothing \
         a per-frame write would not also prove"
    );

    go_quiet(&mut app);

    // Two idle frames, both still inside `last_confirmed_at + CADENCE`.
    app.update();
    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: FRAME_STEP * 9 }),
        "freshness is measured from the presenter's last confirmation, not from the older moment \
         the proof happens to record; measuring from the record would report a healthy presenter \
         lost up to a whole cadence early"
    );
    assert!(
        lost_events(&app).is_empty(),
        "nothing is lost while the last confirmation is still inside the cadence"
    );

    // One more idle frame carries `now` past `last_confirmed_at + CADENCE`.
    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: last_confirmed_at,
        }),
        "the starved presenter is demoted, carrying the moment it last actually confirmed — not \
         the older moment its proof recorded, which would tell an operator the output stopped up \
         to a cadence before it did"
    );
    assert_eq!(
        lost_events(&app),
        vec![(presenter, last_confirmed_at)],
        "the loss is published exactly once, naming the presenter and its last real confirmation"
    );

    // Ten further idle frames: four cadences of silence with nothing left to change.
    for _ in 0..10 {
        app.update();
    }

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: last_confirmed_at,
        }),
        "a presenter that is still silent stays lost at the same moment"
    );
    assert_eq!(
        lost_events(&app).len(),
        1,
        "staying lost is not a transition; a stale confirmation must never re-prove the presenter \
         and let it be lost a second time"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        1,
        "only the original promotion was ever published; nothing re-confirmed this presenter"
    );
}

#[test]
fn a_never_confirmed_presenter_stays_unconfirmed_since_attachment_and_is_never_lost() {
    let mut app = scripted_proof_app();
    go_quiet(&mut app);
    app.update();
    app.update();
    let attached_at = elapsed(&app);

    let presenter = spawn_scripted_presenter(&mut app);

    // Twenty frames is two seconds of app time against a 250ms cadence: eight lapses' worth of
    // silence, so a rule that aged an absent confirmation would have fired long before this.
    for _ in 0..20 {
        app.update();
    }

    assert!(
        elapsed(&app) > attached_at.saturating_add(CADENCE),
        "the fixture must actually run well past the cadence, or this test proves nothing"
    );
    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed { since: attached_at }),
        "a presenter whose source has never confirmed keeps the moment of attachment as its \
         `since`; aging must not advance that stamp to the current frame, or the operator could \
         not tell how long the presenter has been waiting on its first frame"
    );
    assert!(
        lost_events(&app).is_empty(),
        "a presenter that was never confirmed has lost nothing; publishing OutputLost here would \
         raise an alarm for every presenter still waiting to start"
    );
    assert!(
        confirmed_events(&app).is_empty(),
        "nothing confirmed this presenter, so nothing may report it confirmed"
    );
}

#[test]
fn two_presenters_going_quiet_together_lapse_in_cadence_order() {
    let mut app = scripted_proof_app();
    let brisk = spawn_presenter_with_cadence(&mut app, SHORT_CADENCE);
    let patient = spawn_presenter_with_cadence(&mut app, LONG_CADENCE);

    // One confirming frame proves both, then both go silent on the same frame: the only thing that
    // can separate them afterwards is the cadence each one declared.
    app.update();
    let confirmed_at = elapsed(&app);
    go_quiet(&mut app);

    assert_eq!(
        confirmed_events(&app),
        vec![(brisk, confirmed_at), (patient, confirmed_at)],
        "both presenters are proven at the same moment, so the lapse below is decided by cadence \
         alone"
    );

    // Two idle frames carry `now` past `confirmed_at + SHORT_CADENCE` and not past
    // `confirmed_at + LONG_CADENCE`.
    app.update();
    app.update();

    assert_eq!(
        proof(&app, brisk),
        Some(OutputProof::Unconfirmed {
            since: confirmed_at,
        }),
        "the presenter that declared the tighter cadence is unconfirmed first: cadence belongs to \
         the source because only the source knows its hardware's normal rhythm"
    );
    assert_eq!(
        proof(&app, patient),
        Some(OutputProof::Confirmed { at: confirmed_at }),
        "the presenter that declared the looser cadence is still proven on that same frame; a \
         single kernel-wide timeout would have demoted both together and reported a healthy device \
         as failed"
    );
    assert_eq!(
        lost_events(&app),
        vec![(brisk, confirmed_at)],
        "exactly one lapse is published on that frame, naming the presenter whose cadence actually \
         elapsed"
    );

    // Three more idle frames carry `now` past `confirmed_at + LONG_CADENCE` as well.
    for _ in 0..3 {
        app.update();
    }

    assert_eq!(
        proof(&app, patient),
        Some(OutputProof::Unconfirmed {
            since: confirmed_at,
        }),
        "the looser cadence still lapses once its own interval elapses; a longer cadence delays \
         the verdict and never suppresses it"
    );
    assert_eq!(
        lost_events(&app),
        vec![(brisk, confirmed_at), (patient, confirmed_at)],
        "both lapses are published, in the order their cadences elapsed, each exactly once"
    );
}

#[test]
fn a_source_that_resumes_is_confirmed_again_exactly_once() {
    /// Confirming frames run after the resume.
    ///
    /// Deliberately not a whole number of refresh periods. `at` moves only on a refresh, so a loop
    /// length that happened to end on one would let an assertion reading the current frame's clock
    /// pass while proving nothing about the rule.
    const FRAMES_AFTER_RESUME: u32 = 7;

    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);

    let last_confirmed_at = confirm_then_go_quiet(&mut app, 3, 3);
    let confirmed_before_resume = confirmed_events(&app).len();
    let lost_before_resume = lost_events(&app).len();

    app.world_mut()
        .insert_resource(ConfirmationSchedule::Confirming);
    app.update();
    let resumed_at = elapsed(&app);

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: resumed_at }),
        "the first confirmation after a lapse re-proves the presenter at the new moment, not at \
         the stale one"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        confirmed_before_resume + 1,
        "resuming publishes exactly one confirmed transition"
    );
    assert_eq!(
        confirmed_events(&app).last().copied(),
        Some((presenter, resumed_at)),
        "the published confirmation names the presenter and the moment output was proven again"
    );
    assert_ne!(
        resumed_at, last_confirmed_at,
        "the fixture must actually let time pass across the lapse, or this test proves nothing"
    );

    // The refresh period in frames, derived from the fixture's own constants rather than written
    // out, so changing `CADENCE` or `FRAME_STEP` moves the expected moment with them instead of
    // silently invalidating it.
    let frames_per_refresh =
        u32::try_from(CADENCE.as_millis().div_ceil(FRAME_STEP.as_millis())).unwrap_or(u32::MAX);
    assert_ne!(
        FRAMES_AFTER_RESUME % frames_per_refresh,
        0,
        "the loop must not end on a refresh frame, or an assertion that read the current frame's \
         clock would pass here and this case would go back to proving nothing"
    );

    for _ in 0..FRAMES_AFTER_RESUME {
        app.update();
    }

    // `at` advances only once the newest confirmation is a whole cadence past the recorded one, so
    // a presenter confirming every frame reads the last refresh point, not the current frame.
    let last_refresh_at =
        resumed_at + FRAME_STEP * (FRAMES_AFTER_RESUME / frames_per_refresh * frames_per_refresh);

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed {
            at: last_refresh_at,
        }),
        "a resumed presenter that keeps confirming inside its cadence stays proven, and its `at` \
         is the moment of the last cadence refresh; the resume does not put the proof back on a \
         per-frame write"
    );
    assert!(
        last_refresh_at < elapsed(&app),
        "the fixture must end on a frame no refresh wrote, or the assertion above would also hold \
         for a proof that tracked every confirmation"
    );
    assert!(
        elapsed(&app).saturating_sub(last_refresh_at) < CADENCE,
        "the refresh still keeps `at` inside one cadence of the newest confirmation while the \
         presenter keeps confirming; a wider gap would mean the refresh stopped running and the \
         next lapse would be reported from a moment the presenter had long passed"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        confirmed_before_resume + 1,
        "the resume is one transition, not one per frame; a consumer woken again here would be \
         woken forever"
    );
    assert_eq!(
        lost_events(&app).len(),
        lost_before_resume,
        "a presenter confirming inside its cadence is never lost again"
    );
}

#[test]
fn a_presenter_with_no_confirmation_signal_stays_no_signal_available() {
    let mut app = App::new();
    app.add_plugins(MinimalPlugins)
        .add_plugins(OutputProofPlugin::<NoConfirmationSignal>::default())
        .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP))
        .init_resource::<ProofEventLog>()
        .add_observer(record_output_confirmed)
        .add_observer(record_output_lost);
    let presenter = app
        .world_mut()
        .spawn(OutputConfirmationSource::new(NoConfirmationSignal))
        .id();
    app.world_mut().flush();
    app.world_mut().flush();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::NoSignalAvailable),
        "a source that can never confirm wears its inability as a value from the moment it is \
         inserted, so a missing proof is never mistaken for an unavailable signal"
    );

    for _ in 0..40 {
        app.update();
    }

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::NoSignalAvailable),
        "aging never demotes a presenter that has no signal to lose, however long it runs"
    );
    assert!(
        confirmed_events(&app).is_empty() && lost_events(&app).is_empty(),
        "a presenter with no confirmation signal has no transitions to publish"
    );
}

#[test]
fn the_plugin_added_twice_ages_once_per_frame() {
    let single = cadence_reads_over_idle_frames(1, 2);
    let doubled = cadence_reads_over_idle_frames(2, 2);

    assert!(
        single > 0,
        "the measurement window must actually exercise the aging rule, or comparing counts proves \
         nothing"
    );
    assert_eq!(
        doubled, single,
        "adding the plugin twice must neither panic nor register the aging system twice: two \
         crates declaring the same source type is normal, and a second pass over the same \
         presenters is duplicated work and, on the frame a proof settles, a second chance to \
         rewrite it"
    );
}

#[test]
#[should_panic(expected = "one confirmation source for its lifetime")]
fn a_second_confirmation_source_of_another_type_on_one_presenter_is_rejected() {
    let mut app = scripted_proof_app();
    // The rejected type needs its own plugin installed, so the panic under test is the one about a
    // second source and not the one about a missing plugin.
    app.add_plugins(OutputProofPlugin::<NoConfirmationSignal>::default());
    let presenter = spawn_scripted_presenter(&mut app);

    // The rejection reads the proof the first source's hook wrote, so the write has to have landed.
    assert!(
        proof(&app, presenter).is_some(),
        "the first source must have produced a proof before the second insertion is attempted"
    );

    app.world_mut()
        .entity_mut(presenter)
        .insert(OutputConfirmationSource::new(NoConfirmationSignal));
}

#[test]
fn a_same_type_re_insert_is_tolerated_and_restarts_the_proof() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);
    app.update();
    let first_confirmed_at = elapsed(&app);
    app.update();

    // The second frame's confirmation is inside the cadence the first one bought, so the proof
    // still carries the first one's stamp. That is the value the replace below has to clear.
    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed {
            at: first_confirmed_at,
        }),
        "the presenter is proven before its source is replaced in place"
    );

    // One quiet frame inside the cadence: the proof still stands, and the clock has moved on, so
    // the stamp the re-insert writes below is distinguishable from the one it replaces.
    go_quiet(&mut app);
    app.update();
    let reattached_at = elapsed(&app);

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed {
            at: first_confirmed_at,
        }),
        "the quiet frame is inside the cadence, so the presenter is still proven when the          replacement arrives"
    );
    assert!(
        reattached_at > first_confirmed_at,
        "the fixture must let the clock move between the confirmation and the re-insert, or the          restart below is indistinguishable from the proof being left alone"
    );

    // Inserting a second source of the SAME type is a replace, not a contradictory second source.
    app.world_mut()
        .entity_mut(presenter)
        .insert(OutputConfirmationSource::new(ScriptedConfirmation::new(
            CADENCE,
        )));
    app.world_mut().flush();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: reattached_at,
        }),
        "a same-type re-insert is tolerated and restarts the presenter unproven. Bevy fires          on_replace and on_insert for a replace but never on_remove, so the old proof survives on          the entity while the replacing source has confirmed nothing; carrying the old verdict          forward would strand a Confirmed reading that the new source can never refresh or          demote — a presenter reported healthy with nothing watching its output"
    );
    assert_eq!(
        source_confirmations(&app, presenter),
        Some(0),
        "the re-insert really did put a fresh source on the presenter, or the assertion above          would be reading the original component"
    );
    assert!(
        lost_events(&app).is_empty(),
        "a deliberate replacement is not a lapse; nothing aged out"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        1,
        "restarting the proof publishes no confirmation of its own; only the original flip has          been reported so far"
    );

    app.world_mut()
        .insert_resource(ConfirmationSchedule::Confirming);
    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: elapsed(&app) }),
        "the replacement source proves the presenter again on its own first confirmation, and          from that moment the proof is one this source can refresh and demote"
    );
    assert_eq!(
        confirmed_events(&app),
        vec![(presenter, first_confirmed_at), (presenter, elapsed(&app))],
        "the restart is a real transition and is published exactly once: the original source's          flip, then the replacement's"
    );
}

#[test]
#[should_panic(expected = "no OutputConfirmationSource wrote")]
fn a_source_landing_on_a_hand_written_proof_is_rejected() {
    let mut app = scripted_proof_app();
    let impostor = app
        .world_mut()
        .spawn(OutputProof::Confirmed {
            at: Duration::from_secs(9),
        })
        .id();

    // Nothing in this module wrote that proof, so nothing in this module can refresh or retract
    // it; adopting it would let a hand-written value masquerade as an observation of the output.
    app.world_mut()
        .entity_mut(impostor)
        .insert(OutputConfirmationSource::new(ScriptedConfirmation::new(
            CADENCE,
        )));
}

#[test]
#[should_panic(expected = "OutputProofPlugin")]
fn a_source_whose_plugin_was_never_added_is_rejected_at_insertion() {
    let mut app = App::new();
    // TimePlugin is present, so the only thing missing is the aging registration itself.
    app.add_plugins(MinimalPlugins)
        .insert_resource(TimeUpdateStrategy::ManualDuration(FRAME_STEP));

    app.world_mut()
        .spawn(OutputConfirmationSource::new(ScriptedConfirmation::new(
            CADENCE,
        )));
}

#[test]
#[should_panic(expected = "add TimePlugin")]
fn a_source_inserted_into_an_app_with_no_clock_is_rejected_at_insertion() {
    let mut app = App::new();
    // The plugin is added and MinimalPlugins is not, so the aging registration is in place and the
    // only thing missing is the clock itself: an app with no `TimePlugin` carries no
    // `Time<Virtual>`, and every `at` and `since` this module writes is a reading of that clock.
    // Stamping a proof from a clock that does not exist would hand a reader a timestamp of zero
    // that looks exactly like a real one taken on the first frame.
    app.add_plugins(OutputProofPlugin::<ScriptedConfirmation>::default());

    app.world_mut()
        .spawn(OutputConfirmationSource::new(ScriptedConfirmation::new(
            CADENCE,
        )));
}

#[test]
fn removing_the_source_removes_the_proof() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);
    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: elapsed(&app) }),
        "the presenter is proven before its source is taken away"
    );

    app.world_mut()
        .entity_mut(presenter)
        .remove::<OutputConfirmationSource<ScriptedConfirmation>>();
    app.world_mut().flush();

    assert_eq!(
        proof(&app, presenter),
        None,
        "a proof outliving its source would be a claim nothing can refresh or retract"
    );

    app.update();

    assert_eq!(
        proof(&app, presenter),
        None,
        "aging must not resurrect a proof for a presenter that no longer declares a source"
    );
}

#[test]
fn replacing_a_source_by_removal_and_insertion_restarts_the_proof_unconfirmed() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);
    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: elapsed(&app) }),
        "the presenter is proven before the documented replace path is walked"
    );

    app.world_mut()
        .entity_mut(presenter)
        .remove::<OutputConfirmationSource<ScriptedConfirmation>>();
    app.world_mut().flush();
    let reattached_at = elapsed(&app);
    app.world_mut()
        .entity_mut(presenter)
        .insert(OutputConfirmationSource::new(ScriptedConfirmation::new(
            CADENCE,
        )));
    app.world_mut().flush();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Unconfirmed {
            since: reattached_at,
        }),
        "the documented replace path — remove the source, then insert the new one — restarts the \
         presenter unproven: the removal took the old proof, and the new source has confirmed \
         nothing yet, so carrying the old verdict forward would credit the replacement with output \
         it never observed"
    );
    assert_eq!(
        lost_events(&app),
        Vec::new(),
        "a deliberate replacement is not a lapse; the proof went away with its source rather than \
         aging out"
    );

    app.update();

    assert_eq!(
        proof(&app, presenter),
        Some(OutputProof::Confirmed { at: elapsed(&app) }),
        "the replacement source proves the presenter again on its own first confirmation"
    );
    assert_eq!(
        confirmed_events(&app).len(),
        2,
        "the restart is a real transition and is published: the first source's flip, then the \
         replacement's"
    );
}

#[test]
fn despawning_a_presenter_takes_its_source_and_proof_without_disturbing_the_app() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);
    let survivor = spawn_scripted_presenter(&mut app);
    app.update();

    app.world_mut().entity_mut(presenter).despawn();
    app.world_mut().flush();

    assert!(
        !app.world().entities().contains(presenter),
        "the despawn must actually take the presenter, or the frames below prove nothing"
    );

    for _ in 0..6 {
        app.update();
    }

    assert_eq!(
        proof(&app, survivor),
        Some(OutputProof::Confirmed { at: elapsed(&app) }),
        "teardown of one presenter leaves every other presenter proven; a source's removal path \
         runs on every despawn and must not take the app down with it"
    );
}

#[test]
fn the_proof_is_reachable_as_a_reflected_component_through_the_type_registry() {
    let mut app = scripted_proof_app();
    let presenter = spawn_scripted_presenter(&mut app);
    app.update();

    let type_path = <OutputProof as TypePath>::type_path();
    let reflect_component = app
        .world()
        .resource::<AppTypeRegistry>()
        .read()
        .get_with_type_path(type_path)
        .and_then(|registration| registration.data::<ReflectComponent>().cloned());

    assert!(
        reflect_component.is_some(),
        "`{type_path}` must resolve through `AppTypeRegistry` carrying `ReflectComponent` data; \
         the proof exists to be read over BRP"
    );
    assert!(
        reflect_component
            .is_some_and(|component| component.reflect(app.world().entity(presenter)).is_some()),
        "`{type_path}` is registered but does not reflect off a live presenter, so a BRP reader \
         would see an empty result where a proof exists"
    );
}
