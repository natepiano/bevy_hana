use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use bevy::ecs::entity::Entity;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::reflect::ReflectResource;
use bevy::platform::time::Instant;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use thiserror::Error;

use crate::AttachmentPath;
use crate::AttemptRef;
use crate::Claim;
use crate::ConfiguredDeviceConnection;
use crate::ConfiguredDeviceMode;
use crate::DeviceId;
use crate::DeviceKey;
use crate::IdentityDecisionOwed;
use crate::IdentityVerdict;
use crate::KeyAvailability;
use crate::NonEmptyReporterRefs;
use crate::Presence;
use crate::PresentEvidence;
use crate::ReportedId;
use crate::ReportedParent;
use crate::ReporterId;
use crate::RetirementEvidence;
use crate::RiggingRuntimeClock;
use crate::RiggingRuntimeTime;
use crate::SchemeName;
use crate::UnconfirmedBasis;
use crate::registration::ApplyPermit;

/// First identifier `Attempts` issues, chosen so no issued value equals `AttemptRef::default()`.
const FIRST_ISSUED_ATTEMPT: u64 = 1;

/// Marker for the entity that mirrors one reconciled device.
///
/// Queries and the Bevy Remote Protocol reach device state through entities, so the kernel keeps
/// one entity per reconciled device alongside the `Devices` registry. The marker exists so a query
/// can select devices without naming every component the projection inserts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub(crate) struct Device;

/// Marker inserted only while a device is present **and** its claim permits this process to use
/// it.
///
/// The two facts are separate components because they answer separate questions, and a system that
/// has to combine them at every call site eventually combines them wrongly: a camera that is
/// present but open in another application is not usable. This marker is the combined guarantee,
/// so a kernel system queries it directly instead of restating the rule.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub(crate) struct PresentWithUsableClaim;

/// Whether the Collect stage found an availability whose departure grace has ended.
#[derive(Default, Resource)]
pub(crate) enum DepartureGraceDeadlineStatus {
    /// No retained departure-grace deadline has reached the frame time.
    #[default]
    NoneReached,
    /// At least one retained departure-grace deadline requires reconciliation.
    Reached,
}

impl DepartureGraceDeadlineStatus {
    pub(crate) fn refresh(&mut self, devices: &Devices, runtime_now: RiggingRuntimeTime) {
        *self = if devices.departure_grace_due(runtime_now) {
            Self::Reached
        } else {
            Self::NoneReached
        };
    }

    pub(crate) const fn requires_reconciliation(&self) -> bool { matches!(self, Self::Reached) }
}

/// Whether a key has previously published an availability conclusion.
pub(crate) enum PriorKeyAvailability<'a> {
    /// The key has not entered the retained availability table.
    NeverPublished,
    /// The key has one retained conclusion.
    Published(&'a KeyAvailability),
}

/// Ephemeral evidence calculated from all retained reporter completions for one key.
pub(crate) enum KeyAvailabilityEvidence {
    /// Fresh contributor records combine to `Presence::Present`.
    Present(PresentEvidence),
    /// Fresh coverage establishes absence.
    ConfirmedAbsent(RetirementEvidence),
    /// Covering reporters have not supplied their first complete sets.
    AwaitingFirstReport(NonEmptyReporterRefs),
    /// Fresh evidence confirms neither presence nor absence.
    Unconfirmed(UnconfirmedBasis),
    /// Reporter evidence is expired or explicitly unreachable.
    Unreachable {
        /// Runtime time at which reachability became uncertain.
        since:     RiggingRuntimeTime,
        /// Reporters contributing the uncertainty.
        reporters: NonEmptyReporterRefs,
    },
}

/// Apply the key availability transition table to one calculated evidence value.
pub(crate) fn transition_key_availability(
    evidence: KeyAvailabilityEvidence,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
    departure_grace: Duration,
    prior: PriorKeyAvailability<'_>,
) -> KeyAvailability {
    let runtime_now = runtime_clock.time_at(observed_at);
    match evidence {
        KeyAvailabilityEvidence::Present(present_evidence) => {
            KeyAvailability::Present(present_evidence)
        },
        KeyAvailabilityEvidence::ConfirmedAbsent(established_by) => confirmed_absence_availability(
            established_by,
            observed_at,
            runtime_clock,
            runtime_now,
            departure_grace,
            prior,
        ),
        KeyAvailabilityEvidence::AwaitingFirstReport(reporters) => {
            KeyAvailability::AwaitingFirstReport {
                since: prior_since(
                    prior,
                    |availability| {
                        matches!(availability, KeyAvailability::AwaitingFirstReport { .. })
                    },
                    runtime_now,
                ),
                reporters,
            }
        },
        KeyAvailabilityEvidence::Unconfirmed(basis) => KeyAvailability::Unconfirmed {
            since: prior_since(
                prior,
                |availability| matches!(availability, KeyAvailability::Unconfirmed { .. }),
                runtime_now,
            ),
            basis,
        },
        KeyAvailabilityEvidence::Unreachable { since, reporters } => KeyAvailability::Unreachable {
            since: prior_since(
                prior,
                |availability| matches!(availability, KeyAvailability::Unreachable { .. }),
                since,
            ),
            reporters,
        },
    }
}

fn same_present_contributors(first: &PresentEvidence, second: &PresentEvidence) -> bool {
    let first = first.contributors().as_slice();
    let second = second.contributors().as_slice();
    first.len() == second.len()
        && first.iter().zip(second).all(|(first, second)| {
            first.reporter == second.reporter && first.presence == second.presence
        })
}

const fn same_unconfirmed_basis(first: &UnconfirmedBasis, second: &UnconfirmedBasis) -> bool {
    match (first, second) {
        (UnconfirmedBasis::NoFreshEvidence, UnconfirmedBasis::NoFreshEvidence) => true,
        (
            UnconfirmedBasis::UncoveredAbsence {
                reporter: first_reporter,
                ..
            },
            UnconfirmedBasis::UncoveredAbsence {
                reporter: second_reporter,
                ..
            },
        ) => first_reporter.get() == second_reporter.get(),
        (UnconfirmedBasis::NoFreshEvidence, UnconfirmedBasis::UncoveredAbsence { .. })
        | (UnconfirmedBasis::UncoveredAbsence { .. }, UnconfirmedBasis::NoFreshEvidence) => false,
    }
}

fn confirmed_absence_availability(
    established_by: RetirementEvidence,
    observed_at: Instant,
    runtime_clock: RiggingRuntimeClock,
    runtime_now: RiggingRuntimeTime,
    departure_grace: Duration,
    prior: PriorKeyAvailability<'_>,
) -> KeyAvailability {
    match prior {
        PriorKeyAvailability::Published(KeyAvailability::Present(_)) => {
            KeyAvailability::DepartureGrace {
                since:    runtime_now,
                deadline: runtime_clock.time_at(
                    observed_at
                        .checked_add(departure_grace)
                        .unwrap_or(observed_at),
                ),
                evidence: established_by,
            }
        },
        PriorKeyAvailability::Published(KeyAvailability::DepartureGrace {
            since,
            deadline,
            evidence,
        }) if runtime_now.elapsed() < deadline.elapsed() => KeyAvailability::DepartureGrace {
            since:    *since,
            deadline: *deadline,
            evidence: if evidence.reporter == established_by.reporter {
                *evidence
            } else {
                established_by
            },
        },
        PriorKeyAvailability::Published(KeyAvailability::DepartureGrace { evidence, .. }) => {
            KeyAvailability::Absent {
                since:          runtime_now,
                established_by: if evidence.reporter == established_by.reporter {
                    *evidence
                } else {
                    established_by
                },
            }
        },
        PriorKeyAvailability::Published(KeyAvailability::Absent {
            since,
            established_by,
        }) => KeyAvailability::Absent {
            since:          *since,
            established_by: *established_by,
        },
        PriorKeyAvailability::NeverPublished
        | PriorKeyAvailability::Published(
            KeyAvailability::AwaitingFirstReport { .. }
            | KeyAvailability::Unconfirmed { .. }
            | KeyAvailability::Unreachable { .. },
        ) => KeyAvailability::Absent {
            since: runtime_now,
            established_by,
        },
    }
}

fn prior_since(
    prior: PriorKeyAvailability<'_>,
    same_state: impl FnOnce(&KeyAvailability) -> bool,
    fallback: RiggingRuntimeTime,
) -> RiggingRuntimeTime {
    match prior {
        PriorKeyAvailability::Published(availability) if same_state(availability) => {
            match availability {
                KeyAvailability::DepartureGrace { since, .. }
                | KeyAvailability::AwaitingFirstReport { since, .. }
                | KeyAvailability::Unconfirmed { since, .. }
                | KeyAvailability::Unreachable { since, .. }
                | KeyAvailability::Absent { since, .. } => *since,
                KeyAvailability::Present(_) => fallback,
            }
        },
        PriorKeyAvailability::NeverPublished | PriorKeyAvailability::Published(_) => fallback,
    }
}

/// Which keyed device one reported platform handle names in a reconcile pass.
///
/// A handle two reporters attached to different keys names no device: joining an evidence-only
/// record to whichever key happened to be ingested last is exactly the plausible fallback that
/// exact-match identity exists to forbid.
#[derive(Debug, PartialEq, Eq, Reflect)]
pub(crate) enum HandleOwner {
    /// Every keyed record carrying this handle reported the same key.
    OneKey(DeviceKey),
    /// Keyed records disagree about which key this handle belongs to.
    SeveralKeys(HashSet<DeviceKey>),
}

/// Owned conclusions produced together by one reconcile pass.
///
/// Grouping these pass-scoped collections keeps `Devices::replace_reconciled` from assigning
/// meaning by argument position and makes every conclusion advance with the reconciled device set.
#[derive(Debug, Default)]
pub(crate) struct ReconcilePassConclusions {
    pub(crate) availability:           HashMap<DeviceKey, KeyAvailability>,
    pub(crate) duplicate_keys:         HashSet<DeviceKey>,
    pub(crate) reported_handle_owners: HashMap<ReportedId, HandleOwner>,
    pub(crate) unregistered_schemes:   HashSet<SchemeName>,
    pub(crate) withdrawn_reporters:    HashSet<ReporterId>,
}

/// The kernel's recorded state for every device, keyed by the handle it issued.
///
/// Durable state cannot live only on entities: retirement by key runs during startup and
/// immediately after a departure, when no entity is alive. The entity projection mirrors this
/// registry so queries and the Bevy Remote Protocol can read the same facts.
///
/// State is keyed by `DeviceId` rather than by `DeviceKey` because the durable key is two strings
/// and hashing both on every policy query from application code is the cost that matters. One map
/// resolves a durable key to a handle; everything else is keyed by the copyable handle.
#[derive(Debug, Default, Resource, Reflect)]
#[reflect(Resource)]
pub struct Devices {
    availability:           HashMap<DeviceKey, KeyAvailability>,
    ids:                    HashMap<DeviceKey, DeviceId>,
    state:                  HashMap<DeviceId, ReconciledDeviceState>,
    /// How many times each retained device's reconciled state has actually changed.
    ///
    /// Kept beside `Self::state` rather than on `ReconciledDeviceState` because the counter
    /// describes the history of a handle, not what the current pass concluded about the unit: a
    /// state a reporter supplies has no revision to carry, and the comparison that advances the
    /// counter would otherwise have to exclude a field of the value it is comparing.
    revision:               HashMap<DeviceId, DeviceRevision>,
    entity:                 HashMap<DeviceId, Entity>,
    /// Issues `DeviceId`. Monotonic, never reused within a process, so a retired handle dangles
    /// instead of denoting a later device.
    next:                   u64,
    duplicate_keys:         HashSet<DeviceKey>,
    reported_handle_owners: HashMap<ReportedId, HandleOwner>,
    unregistered_schemes:   HashSet<SchemeName>,
    /// Reporters whose retained records the latest reconciliation excluded from current evidence.
    withdrawn_reporters:    HashSet<ReporterId>,
}

impl Devices {
    /// Read the retained availability conclusion for one durable key.
    pub(crate) fn key_availability(&self, key: &DeviceKey) -> PriorKeyAvailability<'_> {
        self.availability.get(key).map_or(
            PriorKeyAvailability::NeverPublished,
            PriorKeyAvailability::Published,
        )
    }

    /// Iterate keys that already have a retained availability conclusion.
    pub(crate) fn availability_keys(&self) -> impl Iterator<Item = &DeviceKey> {
        self.availability.keys()
    }

    /// Report whether the latest reconciliation already withdrew one reporter's retained records.
    pub(crate) fn reporter_records_are_withdrawn(&self, reporter: ReporterId) -> bool {
        self.withdrawn_reporters.contains(&reporter)
    }

    /// Report whether a departure-grace deadline has reached the current runtime time.
    pub(crate) fn departure_grace_due(&self, runtime_now: RiggingRuntimeTime) -> bool {
        self.availability.values().any(|availability| {
            matches!(
                availability,
                KeyAvailability::DepartureGrace { deadline, .. }
                    if deadline.elapsed() <= runtime_now.elapsed()
            )
        })
    }

    /// Turn one durable key into the handle this process issued for it.
    ///
    /// Lookup is exact or nothing. There is deliberately no nearest-match, no first-of-kind, and
    /// no fallback to a primary device: every live defect in this area came from a fallback
    /// returning something plausible instead of nothing.
    #[must_use]
    pub fn resolve(&self, key: &DeviceKey) -> DeviceResolution {
        self.ids
            .get(key)
            .map_or(DeviceResolution::NotResolved, |device_id| {
                DeviceResolution::Resolved(*device_id)
            })
    }

    /// Read what the latest reconcile pass concluded about one handle.
    #[must_use]
    pub fn state(&self, device_id: DeviceId) -> DeviceStateLookup<'_> {
        self.state
            .get(&device_id)
            .map_or(DeviceStateLookup::Retired, DeviceStateLookup::Retained)
    }

    /// Read how many times the latest reconcile passes have changed one handle's state.
    ///
    /// This is the counter an in-flight attempt is re-validated against, so a reporter's scan can
    /// only abandon attempts on the devices that scan actually changed.
    #[must_use]
    #[cfg(feature = "test-support")]
    pub fn revision(&self, device_id: DeviceId) -> DeviceRevisionLookup {
        self.revision
            .get(&device_id)
            .map_or(DeviceRevisionLookup::Retired, |device_revision| {
                DeviceRevisionLookup::Retained(*device_revision)
            })
    }

    #[must_use]
    #[cfg(not(feature = "test-support"))]
    pub(crate) fn revision(&self, device_id: DeviceId) -> DeviceRevisionLookup {
        self.revision
            .get(&device_id)
            .map_or(DeviceRevisionLookup::Retired, |device_revision| {
                DeviceRevisionLookup::Retained(*device_revision)
            })
    }

    /// How many devices the latest reconcile pass retained.
    #[must_use]
    pub fn count(&self) -> usize { self.state.len() }

    /// Keys that arrived more than once from a single reporter in the latest reconcile pass.
    ///
    /// This is one set per pass, not one fact per device, and it is replaced on every pass that
    /// ingests reports. Reconciliation draws no conclusion from it: the identity verdict stage
    /// turns each key into an unverified verdict, which is what stops a weak scheme — two
    /// identical webcams under one device name, neither reporting a serial — from presenting as
    /// proven.
    #[must_use]
    pub const fn duplicate_keys(&self) -> &HashSet<DeviceKey> { &self.duplicate_keys }

    /// Identity spaces the latest reconcile pass rejected at its ingest boundary.
    ///
    /// A reported key whose scheme no provider registered during app construction never becomes
    /// device state, because an unregistered name is a typo rather than an identity space. The
    /// rejected names are retained here so the mistake is visible in a report instead of silently
    /// producing a device that no consumer can address.
    #[must_use]
    pub const fn unregistered_schemes(&self) -> &HashSet<SchemeName> { &self.unregistered_schemes }

    /// Resolve a reported platform handle through keyed records from the latest reconcile pass.
    ///
    /// The answer is replaced with the device set on every completed pass. `NoKeyedRecord` means
    /// the latest pass contained no keyed record carrying `reported_id`; it does not claim that the
    /// platform handle or hardware is absent.
    #[must_use]
    pub fn resolve_reported_handle(&self, reported_id: &ReportedId) -> ReportedHandleResolution {
        match self.reported_handle_owners.get(reported_id) {
            Some(HandleOwner::OneKey(device_key)) => {
                ReportedHandleResolution::OneKey(device_key.clone())
            },
            Some(HandleOwner::SeveralKeys(device_keys)) => {
                ReportedHandleResolution::SeveralKeys(device_keys.clone())
            },
            None => ReportedHandleResolution::NoKeyedRecord,
        }
    }

    /// Replace the reconciled set with the current pass's conclusions.
    ///
    /// `reconciled` arrives roots first so presence is already folded down each parent chain.
    /// A device retains its handle and reconciled state through every unavailable conclusion except
    /// `KeyAvailability::Absent`. Entry into `Absent` is the only transition that removes the
    /// reconciled state and retires the runtime handle and entity.
    pub(crate) fn replace_reconciled(
        &mut self,
        reconciled: Vec<ReconciledDeviceState>,
        conclusions: ReconcilePassConclusions,
    ) -> ReconciledDeviceReplacement {
        let ReconcilePassConclusions {
            availability,
            duplicate_keys,
            reported_handle_owners,
            unregistered_schemes,
            withdrawn_reporters,
        } = conclusions;
        let mut ids = HashMap::with_capacity(reconciled.len());
        let mut state = HashMap::with_capacity(reconciled.len());
        let mut revision = HashMap::with_capacity(reconciled.len());
        let mut changes = ReconciledDeviceChanges::default();

        for reconciled_device_state in reconciled {
            let device_id = self
                .ids
                .get(&reconciled_device_state.key)
                .copied()
                .unwrap_or_else(|| self.issue());
            let key_availability = &availability[&reconciled_device_state.key];
            revision.insert(
                device_id,
                self.advanced_revision(device_id, &reconciled_device_state, key_availability),
            );
            let dispute_changed = self
                .state
                .get(&device_id)
                .map_or(!reconciled_device_state.disputed.is_empty(), |held| {
                    held.disputed != reconciled_device_state.disputed
                });
            if dispute_changed {
                changes.disputes_changed.push(device_id);
            }
            ids.insert(reconciled_device_state.key.clone(), device_id);
            state.insert(device_id, reconciled_device_state);
        }

        for (key, next) in &availability {
            if let Some(previous) = self.availability.get(key)
                && !same_key_availability_conclusion(previous, next)
            {
                changes.availability.push(DeviceAvailabilityChange {
                    key:  key.clone(),
                    from: previous.clone(),
                    to:   next.clone(),
                });
            }
        }
        let device_register_change_detection = if self.ids == ids
            && self.state.len() == state.len()
            && state.iter().all(|(device_id, reported)| {
                self.state
                    .get(device_id)
                    .is_some_and(|retained| retained.holds_same_facts(reported))
            })
            && self.revision == revision
            && same_key_availability_conclusions(&self.availability, &availability)
            && self.duplicate_keys == duplicate_keys
            && self.reported_handle_owners == reported_handle_owners
            && self.unregistered_schemes == unregistered_schemes
            && self.withdrawn_reporters == withdrawn_reporters
            && self
                .entity
                .keys()
                .all(|device_id| state.contains_key(device_id))
        {
            DeviceRegisterChangeDetection::Preserve
        } else {
            DeviceRegisterChangeDetection::MarkChanged
        };
        self.entity.retain(|device_id, entity| {
            let retained = state.contains_key(device_id);
            if !retained {
                changes.orphaned_entities.push(*entity);
            }

            retained
        });
        self.ids = ids;
        self.state = state;
        self.revision = revision;
        self.availability = availability;
        self.duplicate_keys = duplicate_keys;
        self.reported_handle_owners = reported_handle_owners;
        self.unregistered_schemes = unregistered_schemes;
        self.withdrawn_reporters = withdrawn_reporters;

        ReconciledDeviceReplacement {
            changes,
            device_register_change_detection,
        }
    }

    /// Record which entity mirrors one handle, so the next pass updates that entity instead of
    /// spawning a second one for the same device.
    pub(crate) fn project_entity(&mut self, device_id: DeviceId, entity: Entity) {
        self.entity.insert(device_id, entity);
    }

    /// Read every retained state, so a consumer that did not author the keys can list what the
    /// kernel currently holds.
    ///
    /// Iteration order follows the state map and is therefore unspecified: a caller that needs the
    /// order reporters supplied should key off `ReconciledDeviceState::key` instead.
    pub fn states(&self) -> impl Iterator<Item = &ReconciledDeviceState> { self.state.values() }

    /// Clear the identity debt one unit carries, because a human answered the question about it.
    ///
    /// `crate::IdentityDecisions` is the only caller. Until the debt is cleared, every later pass
    /// reports `crate::IdentityVerdict::Displaced` or `crate::IdentityVerdict::WrongUnit` again
    /// from the retained value, and both refuse every authorization — which is what made a
    /// displaced unit unusable for the life of the process.
    ///
    /// The verdict is concluded again here rather than left to the next pass, through the same
    /// `IdentityVerdict::concluded_from_scan` the merge reaches for. `Self::in_service_state` gates
    /// on the verdict, so clearing the debt alone would leave the unit a human has just adopted
    /// refusing every authorization until a reporter happened to scan again — an hour, on a
    /// reporter that only rescans when the operating system says the hardware moved.
    pub(crate) fn discharge_identity_decision(&mut self, key: &DeviceKey) {
        let Some(device_id) = self.ids.get(key).copied() else {
            return;
        };
        let verdict = IdentityVerdict::concluded_from_scan(key, &self.duplicate_keys);
        if let Some(reconciled_device_state) = self.state.get_mut(&device_id) {
            reconciled_device_state.decision_owed = IdentityDecisionOwed::Nothing;
            reconciled_device_state.verdict = verdict;
        }
    }

    /// Find the entity mirroring one handle, so a caller holding a durable key can reach the
    /// components the projection inserted without scanning every device entity.
    #[must_use]
    pub(crate) fn entity(&self, device_id: DeviceId) -> DeviceEntityLookup {
        self.entity
            .get(&device_id)
            .map_or(DeviceEntityLookup::NotProjected, |entity| {
                DeviceEntityLookup::Projected(*entity)
            })
    }

    /// Authorize one device to be put in service.
    ///
    /// The only in-service decision point in the kernel. Every check is against the merged view,
    /// so a co-reported device resolves most-restrictive-wins: one reporter seeing an idle camera
    /// does not authorize capture when another watched a second application open it.
    ///
    /// `disputed` is deliberately ignored. A unit whose reporters contradict each other about one
    /// capability is still correct about every other one, so refusing the whole device would take a
    /// Stream Deck dark over a disagreement about its LED brightness range. A consumer that must
    /// not act on a contested capability reads `ReconciledDeviceState::disputed`, which holds the
    /// contested capability types for that device.
    ///
    /// # Errors
    ///
    /// Returns the `ApplyAuthorizationError` naming the first check that refused: an unknown
    /// handle, an identity that was never proven, a unit that is not reachable, a claim another
    /// process holds, or an authored entry the application marked offline.
    #[cfg(feature = "test-support")]
    pub fn authorize_service(
        &self,
        device_id: DeviceId,
    ) -> Result<ApplyPermit, ApplyAuthorizationError> {
        self.in_service_state(device_id)?;

        Ok(ApplyPermit::authorized())
    }

    #[cfg(not(feature = "test-support"))]
    pub(crate) fn authorize_service(
        &self,
        device_id: DeviceId,
    ) -> Result<ApplyPermit, ApplyAuthorizationError> {
        self.in_service_state(device_id)?;

        Ok(ApplyPermit::authorized())
    }

    /// Run every device-wide service check and return the state used to authorize the permit, so
    /// the handle is resolved only once.
    fn in_service_state(
        &self,
        device_id: DeviceId,
    ) -> Result<&ReconciledDeviceState, ApplyAuthorizationError> {
        let reconciled_device_state = self.authorized_state(device_id)?;
        match reconciled_device_state.verdict {
            IdentityVerdict::Proven | IdentityVerdict::Presumed | IdentityVerdict::Authored => {
                Ok(reconciled_device_state)
            },
            _ => Err(ApplyAuthorizationError::IdentityNotProven {
                key: reconciled_device_state.key.clone(),
            }),
        }
    }

    /// Run the checks every predicate shares: the handle resolves, the authored mode permits driver
    /// work, the unit is reachable, and this process may use it.
    fn authorized_state(
        &self,
        device_id: DeviceId,
    ) -> Result<&ReconciledDeviceState, ApplyAuthorizationError> {
        let DeviceStateLookup::Retained(reconciled_device_state) = self.state(device_id) else {
            return Err(ApplyAuthorizationError::DeviceRetired { device_id });
        };
        if reconciled_device_state.mode == ConfiguredDeviceMode::Offline {
            return Err(ApplyAuthorizationError::Offline {
                key: reconciled_device_state.key.clone(),
            });
        }
        if !matches!(
            self.key_availability(&reconciled_device_state.key),
            PriorKeyAvailability::Published(KeyAvailability::Present(_))
        ) {
            return Err(ApplyAuthorizationError::NotPresent {
                key: reconciled_device_state.key.clone(),
            });
        }
        match reconciled_device_state.claim {
            Claim::Held | Claim::Free | Claim::NotApplicable => {},
            Claim::Contended { .. } | Claim::Blocked { .. } => {
                return Err(ApplyAuthorizationError::ClaimUnavailable {
                    key: reconciled_device_state.key.clone(),
                });
            },
        }

        Ok(reconciled_device_state)
    }

    /// Carry one handle's revision into this pass, advancing it only for a device that moved.
    ///
    /// A device the kernel is meeting for the first time starts at `DeviceRevision::default()`:
    /// there is no retained state for the incoming one to differ from, so nothing has changed yet.
    /// A pass that reports a retained device exactly as it already stands hands back the same
    /// counter, which is what keeps routine scanning from abandoning the attempts in flight on it.
    fn advanced_revision(
        &self,
        device_id: DeviceId,
        reported: &ReconciledDeviceState,
        availability: &KeyAvailability,
    ) -> DeviceRevision {
        let Some(retained) = self.state.get(&device_id) else {
            return DeviceRevision::default();
        };
        let device_revision = self.revision.get(&device_id).copied().unwrap_or_default();
        if retained.holds_same_facts(reported)
            && self
                .availability
                .get(&reported.key)
                .is_some_and(|retained| same_key_availability_conclusion(retained, availability))
        {
            device_revision
        } else {
            device_revision.advanced()
        }
    }

    const fn issue(&mut self) -> DeviceId {
        let device_id = DeviceId::new(self.next);
        self.next += 1;

        device_id
    }
}

fn same_key_availability_conclusions(
    first: &HashMap<DeviceKey, KeyAvailability>,
    second: &HashMap<DeviceKey, KeyAvailability>,
) -> bool {
    first.len() == second.len()
        && first.iter().all(|(key, first)| {
            second
                .get(key)
                .is_some_and(|second| same_key_availability_conclusion(first, second))
        })
}

fn same_key_availability_conclusion(first: &KeyAvailability, second: &KeyAvailability) -> bool {
    match (first, second) {
        (KeyAvailability::Present(first), KeyAvailability::Present(second)) => {
            same_present_contributors(first, second)
        },
        (
            KeyAvailability::DepartureGrace {
                since: first_since,
                deadline: first_deadline,
                evidence: first_evidence,
            },
            KeyAvailability::DepartureGrace {
                since: second_since,
                deadline: second_deadline,
                evidence: second_evidence,
            },
        ) => {
            first_since == second_since
                && first_deadline == second_deadline
                && first_evidence.reporter == second_evidence.reporter
        },
        (
            KeyAvailability::AwaitingFirstReport {
                since: first_since,
                reporters: first_reporters,
            },
            KeyAvailability::AwaitingFirstReport {
                since: second_since,
                reporters: second_reporters,
            },
        )
        | (
            KeyAvailability::Unreachable {
                since: first_since,
                reporters: first_reporters,
            },
            KeyAvailability::Unreachable {
                since: second_since,
                reporters: second_reporters,
            },
        ) => first_since == second_since && first_reporters == second_reporters,
        (
            KeyAvailability::Unconfirmed {
                since: first_since,
                basis: first_basis,
            },
            KeyAvailability::Unconfirmed {
                since: second_since,
                basis: second_basis,
            },
        ) => first_since == second_since && same_unconfirmed_basis(first_basis, second_basis),
        (
            KeyAvailability::Absent {
                since: first_since,
                established_by: first_evidence,
            },
            KeyAvailability::Absent {
                since: second_since,
                established_by: second_evidence,
            },
        ) => first_since == second_since && first_evidence.reporter == second_evidence.reporter,
        _ => false,
    }
}

/// The kernel's recorded state for one device.
///
/// Durable and entity-free, so retirement by key runs with no entity alive. Capability *values*
/// stay with the reporter registry that retains them — `Box<dyn Reflect>` is neither clonable nor
/// reflectable, and a second copy could drift from the reporter's. What lands here is the
/// normalized conclusions the kernel itself drew, plus the one piece of reporter evidence a later
/// pass has to read back: `Self::attachment`, without which a returning unit could never be judged
/// against the slot the departed one occupied.
#[derive(Clone, Debug, Reflect)]
pub struct ReconciledDeviceState {
    /// The durable name, so a handle can be turned back into one without a reverse scan of the
    /// key-to-handle map.
    pub key:           DeviceKey,
    /// Whether this live unit corresponds to its durable key, and therefore what it may be asked
    /// to do.
    ///
    /// Computed here rather than reported: a verdict a reporter supplied would be a claim about
    /// identity that the merge is the only thing able to check.
    pub verdict:       IdentityVerdict,
    /// The `Displaced` or `WrongUnit` verdict a human still has to resolve, held separately from
    /// `Self::verdict` so a pass that reports a scan observation instead does not destroy it.
    ///
    /// A key duplicated within one scan is the case that separates the two: the duplicate must be
    /// reportable while the scan shows it and must clear when the scan stops, so it cannot be
    /// stored in the same slot as a verdict that outlives every scan.
    pub decision_owed: IdentityDecisionOwed,
    /// The authored operation mode for this key, `ConfiguredDeviceMode::Managed` when the
    /// application authored no inventory entry for it.
    ///
    /// Stamped during reconciliation so the authorization predicates answer from one value instead
    /// of asking every caller to carry the inventory to the decision point and combine the two
    /// rules themselves.
    pub mode:          ConfiguredDeviceMode,
    /// Where the contributors observed this unit attached.
    ///
    /// Retained across passes because the displaced-unit rule compares a departed key's slot with
    /// the slot a newly arrived unit occupies, and by the time the arrival is judged the departed
    /// reporter record is gone.
    pub attachment:    AttachmentPath,
    /// What this device hangs off. Drives the conjunctive presence fold and the retirement of
    /// descendants by key.
    pub parent:        ReportedParent,
    /// Reachability after folding every contributor's report against this device's parent chain.
    ///
    /// Compared by variant, never by value: `crate::Presence::Unreachable` carries the reporter's
    /// elapsed time, which grows on every scan, so comparing values would report a change forever
    /// and defeat the once-per-change rule the entity projection depends on.
    pub presence:      Presence,
    /// Exclusive ownership, retained separately from `presence` because a camera can be present
    /// while another process owns its capture stream.
    pub claim:         Claim,
    /// Every reporter contributing to this device, in the order their sets were ingested.
    ///
    /// The freshness lease reads this to find whose devices to mark unreachable when one reporter
    /// goes stale. A `Vec` rather than a small-vector type: reflection support for those is
    /// feature-gated in Bevy, and a device rarely has more than two contributors.
    pub contributors:  Vec<ReporterId>,
    /// Capability component types the contributors declared for this device.
    pub declared:      HashSet<TypeId>,
    /// Capability component types whose values the contributors disagree about.
    ///
    /// Facts only; the values stay with the reporters that own them, because the erased capability
    /// payload is neither clonable nor reflectable and copying it would create a second
    /// authoritative record that can drift from the reporter's.
    pub disputed:      HashSet<TypeId>,
}

impl ReconciledDeviceState {
    /// Report whether a newly reported state says the same thing about this device as the retained
    /// one. `Devices::advanced_revision` advances the device's [`DeviceRevision`] when this returns
    /// `false`.
    ///
    /// `Self::presence` is compared by variant for the reason its own documentation gives:
    /// `Presence::Unreachable` carries an elapsed time that grows on every scan, so comparing
    /// values would report a change every pass and advance the counter at scan rate — which is the
    /// kernel-wide abandonment the per-device counter exists to stop.
    fn holds_same_facts(&self, reported: &Self) -> bool {
        self.key == reported.key
            && self.verdict == reported.verdict
            && self.decision_owed == reported.decision_owed
            && self.mode == reported.mode
            && self.attachment == reported.attachment
            && self.parent == reported.parent
            && self.presence.is_same_variant(reported.presence)
            && self.claim == reported.claim
            && self.contributors == reported.contributors
            && self.declared == reported.declared
            && self.disputed == reported.disputed
    }
}

/// Whether a reconciliation result should advance the `Devices` resource's change tick.
///
/// Kept distinct from `ReconciledDeviceChanges`: a new device or a changed claim mutates
/// `Devices` without necessarily producing a projection-side departure or dispute, while an
/// authored connection change can require projection work without changing `Devices`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DeviceRegisterChangeDetection {
    /// Every retained device still carries the same meaningful facts, so consumers should keep
    /// sleeping even though reconciliation refreshed the register's internal representation.
    Preserve,
    /// At least one retained fact, durable key, revision, or diagnostic set changed.
    MarkChanged,
}

/// The register and projection outcomes of replacing one reconciled device generation.
#[derive(Debug)]
pub(crate) struct ReconciledDeviceReplacement {
    /// Differences the entity and event projection must apply.
    pub(crate) changes:                          ReconciledDeviceChanges,
    /// Whether this replacement should wake consumers watching `Devices` through change detection.
    pub(crate) device_register_change_detection: DeviceRegisterChangeDetection,
}

/// What the latest reconcile pass changed about the retained device set, held until the entity
/// projection has applied it.
///
/// A named record rather than a tuple of vectors, because a reader has to learn from the type that
/// these devices left, these entities have nothing behind them any more, and these devices' report
/// disagreements moved. `Devices::replace_reconciled` rebuilds its maps in place, so nothing else
/// can recover the previous generation once it returns.
///
/// It is a resource because the projection runs as its own system after `reconcile` — device
/// entities cannot be spawned from inside the merge, which holds borrows of every reporter's
/// retained set. The projection clears it once applied, so a settled frame that never reaches the
/// merge finds nothing left to re-apply.
#[derive(Debug, Default, Resource)]
pub(crate) struct ReconciledDeviceChanges {
    /// Availability edges produced by the retained transition table.
    pub(crate) availability:      Vec<DeviceAvailabilityChange>,
    /// Entities that were mirroring a newly absent device and now need despawning.
    pub(crate) orphaned_entities: Vec<Entity>,
    /// Handles whose contributors changed what they disagree about, including a device whose first
    /// pass already found a disagreement.
    pub(crate) disputes_changed:  Vec<DeviceId>,
    /// Authored inventory keys whose connection conclusion this pass changed.
    ///
    /// Only the changed keys travel: `HardwareInventory` is written through mutable resource
    /// access, which marks the resource changed whether or not any value differs, so a pass that
    /// concluded nothing new must leave this empty rather than rewrite what is already there.
    pub(crate) connections:       Vec<ConfiguredDeviceConnectionChange>,
}

/// One retained key availability edge and both published conclusions.
#[derive(Clone, Debug)]
pub(crate) struct DeviceAvailabilityChange {
    pub(crate) key:  DeviceKey,
    pub(crate) from: KeyAvailability,
    pub(crate) to:   KeyAvailability,
}

/// Departures and connection conclusions held for the event stage after the projection consumed
/// them.
///
/// `ReconciledDeviceChanges` is taken whole by the entity projection, which despawns the entities a
/// departure orphaned; by the time the event stage runs there is nothing left to read. The two
/// facts a consumer still needs are moved here rather than left in place, because re-reading them
/// from the projection's own resource would mean the projection could not clear it and a settled
/// frame would re-announce the last departure forever.
#[derive(Debug, Default, Resource)]
pub(crate) struct DeviceChangeAnnouncements {
    /// Availability edges retained until the event stage publishes them.
    pub(crate) availability: Vec<DeviceAvailabilityChange>,
    /// Authored inventory keys whose connection conclusion this pass changed.
    pub(crate) connections:  Vec<ConfiguredDeviceConnectionChange>,
}

/// One authored inventory key and the connection conclusion this pass reached for it.
#[derive(Debug)]
pub(crate) struct ConfiguredDeviceConnectionChange {
    pub(crate) key:        DeviceKey,
    pub(crate) connection: ConfiguredDeviceConnection,
}

/// Result of resolving one durable key into the current process-local handle.
///
/// A named result rather than an optional handle: the two outcomes lead to different work, and a
/// caller that reads "no handle" as "nothing to do" would silently skip a device that is merely
/// waiting for its reporter's first complete scan.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub enum DeviceResolution {
    /// No reporter has contributed a record under this key during this process, so the key names
    /// nothing that can be queried, claimed, or driven right now.
    NotResolved,
    /// The key names a device the kernel currently retains, and this handle addresses it.
    Resolved(DeviceId),
}

/// Result of resolving one reported platform handle through the latest reconcile pass.
///
/// The result names what keyed records in the registry establish. It makes no claim about whether
/// the operating-system handle or the underlying hardware still exists.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum ReportedHandleResolution {
    /// No keyed record in the latest reconcile pass carried this handle.
    NoKeyedRecord,
    /// Every keyed record carrying the handle named this one durable device key.
    OneKey(DeviceKey),
    /// Keyed records in the latest reconcile pass attached the handle to different device keys.
    SeveralKeys(HashSet<DeviceKey>),
}

/// Result of asking which entity mirrors one handle.
///
/// A named result rather than an optional entity, because the absent case means reconciliation has
/// not projected this device yet — the handle is live, its components are simply not queryable this
/// frame. A caller that read "no entity" as "no device" would drop a unit the kernel retains.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum DeviceEntityLookup {
    /// The handle names a device, but no entity currently mirrors it.
    NotProjected,
    /// This entity carries the device's mirrored identity, presence, and capability components.
    Projected(Entity),
}

/// Why the kernel refused to authorize an endpoint operation on one device.
///
/// Named for the refusal rather than for the caller, because both predicates return it and the
/// reader needs to know which check failed, not which function asked. Every variant carries the
/// durable key rather than the handle wherever one exists, so a refusal stays readable in a log
/// after the process that issued the handle exits.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum ApplyAuthorizationError {
    /// The handle addresses no retained device, so there is nothing to authorize.
    #[error("device handle `{device_id:?}` addresses no retained device")]
    DeviceRetired {
        /// The handle that resolved to nothing.
        device_id: DeviceId,
    },
    /// The application authored this device offline, so no driver operation may touch it — not
    /// even a restore. Passive discovery still reports its presence.
    #[error("device `{key:?}` is configured offline")]
    Offline {
        /// Durable key of the authored entry whose mode refused the operation.
        key: DeviceKey,
    },
    /// The unit is not reachable right now, so an operation would address hardware that is absent
    /// or whose reachability the kernel cannot establish.
    #[error("device `{key:?}` is not present")]
    NotPresent {
        /// Durable key of the unreachable device.
        key: DeviceKey,
    },
    /// Another process owns the unit, or the platform blocked access to it, so an apply would fail
    /// at the driver.
    #[error("device `{key:?}` claim does not permit use by this process")]
    ClaimUnavailable {
        /// Durable key of the device whose claim refused the operation.
        key: DeviceKey,
    },
    /// The identity verdict does not authorize this operation: in-service use requires `Proven` or
    /// `Presumed`, or `Authored`.
    #[error("device `{key:?}` identity does not authorize this operation")]
    IdentityNotProven {
        /// Durable key of the device whose verdict refused the operation.
        key: DeviceKey,
    },
}

/// Result of reading the kernel's recorded state for one handle.
#[derive(Clone, Copy, Debug)]
pub enum DeviceStateLookup<'a> {
    /// The handle addresses no retained device: it was issued for a device that has since departed
    /// or its key was never reported in this process.
    Retired,
    /// The kernel retains this device and its latest reconciled state.
    Retained(&'a ReconciledDeviceState),
}

/// One global revision, combining every reporter's own revision.
///
/// It advances once per reconcile pass that accepts at least one complete set whose records differ
/// from that reporter's retained set. An unchanged complete set may still refresh reporter
/// freshness and run reconciliation, but it does not advance this counter.
///
/// Reacquisition does not read this counter. It compares each device's presence reading from one
/// pass to the next, and attempt staleness keys on the per-device [`DeviceRevision`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Resource, Reflect)]
#[reflect(opaque)]
#[reflect(Resource)]
pub struct RiggingRevision(u64);

impl RiggingRevision {
    /// Report the number of reconcile passes that ingested at least one changed complete set.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }

    pub(crate) const fn advance(&mut self) { self.0 += 1; }
}

/// How many times one device's own reconciled state has changed since the kernel issued its handle.
///
/// Separate from `RiggingRevision` because the two answer different questions: the global counter
/// says how many reconcile passes accepted a changed complete set from any reporter, while this
/// says whether *this* unit changed. An attempt validated against the global counter is abandoned
/// by a change from a reporter that never names its device; validated against this counter it
/// survives every pass that leaves this device's reconciled state unchanged.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Reflect)]
#[reflect(opaque)]
pub struct DeviceRevision(u64);

impl DeviceRevision {
    /// Report how many reconcile passes changed this device's reconciled state.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }

    /// The counter one pass that found a real change hands to the next.
    ///
    /// Returned rather than mutated in place because `Devices::replace_reconciled` builds the next
    /// pass's map beside the retained one instead of editing it.
    pub(crate) const fn advanced(self) -> Self { Self(self.0 + 1) }
}

/// Result of reading how many times one device has changed.
///
/// A named result rather than an `Option<DeviceRevision>`: a retired handle has no revision at all,
/// and reading that absence as revision zero would say the device is at its original state when in
/// fact there is no device. A retry gate that stamped zero for a departed unit would then never
/// reopen, because the fresh handle a returning unit receives also starts at zero.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum DeviceRevisionLookup {
    /// The handle addresses no retained device, so nothing has a revision to report.
    Retired,
    /// The kernel retains this device and its state has changed this many times.
    Retained(DeviceRevision),
}

/// Monotonic issuer for process-local attempt identifiers.
///
/// `next` starts at 1, not 0: `AttemptRef` derives `Default`, so `AttemptRef::default()` is
/// `AttemptRef(0)`. An issued identifier equal to a defaulted or reflection-round-tripped one
/// would collide with the value used wherever a field was left unset.
#[derive(Debug, Resource, Reflect)]
#[reflect(Resource)]
pub(crate) struct Attempts {
    next: u64,
}

impl Default for Attempts {
    fn default() -> Self {
        Self {
            next: FIRST_ISSUED_ATTEMPT,
        }
    }
}

impl Attempts {
    /// Advance the counter and hand back the identifier the next attempt record must carry.
    ///
    /// # Errors
    ///
    /// Returns `AttemptIssueError::SequenceExhausted` when the counter cannot advance without
    /// wrapping back to `AttemptRef::default()` and reusing an earlier identifier.
    pub(crate) fn issue(&mut self) -> Result<AttemptRef, AttemptIssueError> {
        let next = self
            .next
            .checked_add(1)
            .ok_or(AttemptIssueError::SequenceExhausted)?;
        let attempt = AttemptRef::new(self.next);
        self.next = next;

        Ok(attempt)
    }
}

/// Failure from asking the attempt registry for another identifier.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub(crate) enum AttemptIssueError {
    /// The registry cannot issue again without reusing an identifier a driver may still poll.
    #[error("attempt identifier sequence is exhausted")]
    SequenceExhausted,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    reason = "tests should panic on unexpected values"
)]
mod tests {
    use std::any::TypeId;
    use std::collections::HashSet;
    use std::error::Error;
    use std::time::Duration;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::ecs::reflect::ReflectResource;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::reflect::FromReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::ApplyAuthorizationError;
    use super::Attempts;
    use super::Device;
    use super::DeviceRegisterChangeDetection;
    use super::DeviceResolution;
    use super::DeviceRevision;
    use super::DeviceRevisionLookup;
    use super::DeviceStateLookup;
    use super::Devices;
    use super::PresentWithUsableClaim;
    use super::PriorKeyAvailability;
    use super::ReconcilePassConclusions;
    use super::ReconciledDeviceState;
    use super::RiggingRevision;
    use crate::AttachmentPath;
    use crate::AttemptRef;
    use crate::BatchRef;
    use crate::Claim;
    use crate::ClaimHolder;
    use crate::ConfiguredDeviceMode;
    use crate::ContributorView;
    use crate::DeviceId;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DiscoveryBatchId;
    use crate::IdentityDecisionOwed;
    use crate::IdentityVerdict;
    use crate::KeyAvailability;
    use crate::NonEmptyContributors;
    use crate::NonEmptyReporterRefs;
    use crate::PermissionGate;
    use crate::Presence;
    use crate::PresenceView;
    use crate::PresentEvidence;
    use crate::ReportedId;
    use crate::ReportedParent;
    use crate::ReporterId;
    use crate::ReporterRef;
    use crate::RetirementEvidence;
    use crate::RiggingRuntimeTime;
    use crate::SchemeName;
    use crate::UnverifiedReason;

    /// Capability type the contributing reporters disagree about in the authorization tests.
    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct DisputedCapability;

    fn reported_key(value: &str) -> Result<DeviceKey, Box<dyn Error>> {
        Ok(DeviceKey {
            kind: DeviceKind::Display,
            id:   DeviceIdSource::Reported {
                scheme: SchemeName::new("edid-serial")?,
                value:  ReportedId::new(value)?,
            },
        })
    }

    fn reconciled(key: DeviceKey) -> ReconciledDeviceState {
        ReconciledDeviceState {
            key,
            verdict: IdentityVerdict::Proven,
            decision_owed: IdentityDecisionOwed::Nothing,
            mode: ConfiguredDeviceMode::Managed,
            attachment: AttachmentPath::PlatformHasNoConcept,
            parent: ReportedParent::Root,
            presence: Presence::Present,
            claim: Claim::NotApplicable,
            contributors: Vec::new(),
            declared: HashSet::new(),
            disputed: HashSet::new(),
        }
    }

    fn conclusions_for(states: &[ReconciledDeviceState]) -> ReconcilePassConclusions {
        let reporter = ReporterId(0);
        let reporter_ref = ReporterRef::from_reporter_id(reporter);
        let batch = BatchRef::from_batch_id(DiscoveryBatchId(0));
        let availability = states
            .iter()
            .map(|state| {
                let availability = match state.presence {
                    Presence::Present => KeyAvailability::Present(PresentEvidence::new(
                        NonEmptyContributors::from_contributors(vec![ContributorView::new(
                            reporter_ref,
                            batch,
                            PresenceView::Present,
                        )])
                        .expect("one contributor is non-empty"),
                    )),
                    Presence::Unreachable { since } => KeyAvailability::Unreachable {
                        since:     RiggingRuntimeTime::from_elapsed(since),
                        reporters: NonEmptyReporterRefs::from_first_and_rest(reporter, &[]),
                    },
                    Presence::Absent => KeyAvailability::Absent {
                        since:          RiggingRuntimeTime::from_elapsed(Duration::ZERO),
                        established_by: RetirementEvidence::new(reporter_ref, batch),
                    },
                };
                (state.key.clone(), availability)
            })
            .collect();
        ReconcilePassConclusions {
            availability,
            ..ReconcilePassConclusions::default()
        }
    }

    fn replace(
        devices: &mut Devices,
        states: Vec<ReconciledDeviceState>,
    ) -> super::ReconciledDeviceReplacement {
        let mut conclusions = conclusions_for(&states);
        for (key, availability) in &mut conclusions.availability {
            let PriorKeyAvailability::Published(previous) = devices.key_availability(key) else {
                continue;
            };
            if std::mem::discriminant(previous) == std::mem::discriminant(availability) {
                *availability = previous.clone();
            }
        }
        devices.replace_reconciled(states, conclusions)
    }

    #[test]
    fn resolution_distinguishes_an_unknown_key_from_a_retained_handle() -> Result<(), Box<dyn Error>>
    {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let absent_key = reported_key("DELL-U2723QE-OTHER")?;
        let mut devices = Devices::default();

        assert_eq!(devices.resolve(&key), DeviceResolution::NotResolved);

        replace(&mut devices, vec![reconciled(key.clone())]);

        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            panic!("an ingested key must resolve to the handle the registry issued");
        };
        assert!(matches!(
            devices.state(device_id),
            DeviceStateLookup::Retained(state) if state.key == key
        ));
        assert_eq!(devices.resolve(&absent_key), DeviceResolution::NotResolved);

        Ok(())
    }

    #[test]
    fn a_rescan_reporting_the_same_state_leaves_the_device_revision_alone()
    -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        let first_replacement = replace(&mut devices, vec![reconciled(key.clone())]);
        assert_eq!(
            first_replacement.device_register_change_detection,
            DeviceRegisterChangeDetection::MarkChanged
        );
        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            panic!("an ingested key must resolve to the handle the registry issued");
        };
        let issued = devices.revision(device_id);
        assert_eq!(
            issued,
            DeviceRevisionLookup::Retained(DeviceRevision::default())
        );

        for _ in 0..3 {
            let replacement = replace(&mut devices, vec![reconciled(key.clone())]);
            assert_eq!(
                replacement.device_register_change_detection,
                DeviceRegisterChangeDetection::Preserve
            );
        }

        // The whole point of counting real changes: a reporter that keeps reporting the same unit
        // is not a reason to abandon the attempts running against it.
        assert_eq!(devices.revision(device_id), issued);

        let mut unreachable = reconciled(key.clone());
        unreachable.presence = Presence::Unreachable {
            since: Duration::from_secs(9),
        };
        let unreachable_replacement = replace(&mut devices, vec![unreachable]);
        assert_eq!(
            unreachable_replacement.device_register_change_detection,
            DeviceRegisterChangeDetection::MarkChanged
        );

        assert_eq!(
            devices.revision(device_id),
            DeviceRevisionLookup::Retained(DeviceRevision::default().advanced())
        );

        // `Presence::Unreachable` carries an elapsed time that grows on every scan, so a device
        // that stays unreachable would advance forever if presence were compared by value.
        let mut later = reconciled(key);
        later.presence = Presence::Unreachable {
            since: Duration::from_secs(30),
        };
        let later_replacement = replace(&mut devices, vec![later]);
        assert_eq!(
            later_replacement.device_register_change_detection,
            DeviceRegisterChangeDetection::Preserve
        );

        assert_eq!(
            devices.revision(device_id),
            DeviceRevisionLookup::Retained(DeviceRevision::default().advanced())
        );

        Ok(())
    }

    #[test]
    fn a_retired_handle_reports_no_revision_rather_than_a_first_one() -> Result<(), Box<dyn Error>>
    {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        replace(&mut devices, vec![reconciled(key.clone())]);
        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            panic!("an ingested key must resolve to the handle the registry issued");
        };

        devices.replace_reconciled(Vec::new(), ReconcilePassConclusions::default());

        assert_eq!(devices.revision(device_id), DeviceRevisionLookup::Retired);

        Ok(())
    }

    #[test]
    fn a_returning_key_never_reuses_the_retired_handle() -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        replace(&mut devices, vec![reconciled(key.clone())]);
        let DeviceResolution::Resolved(first) = devices.resolve(&key) else {
            panic!("an ingested key must resolve");
        };

        devices.replace_reconciled(Vec::new(), ReconcilePassConclusions::default());
        assert!(matches!(devices.state(first), DeviceStateLookup::Retired));

        replace(&mut devices, vec![reconciled(key.clone())]);
        let DeviceResolution::Resolved(second) = devices.resolve(&key) else {
            panic!("a returning key must resolve again");
        };

        assert_ne!(first, second);

        Ok(())
    }

    #[test]
    fn an_unchanged_key_keeps_its_handle_across_passes() -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        replace(&mut devices, vec![reconciled(key.clone())]);
        let first = devices.resolve(&key);

        replace(&mut devices, vec![reconciled(key.clone())]);

        assert_eq!(devices.resolve(&key), first);

        Ok(())
    }

    #[test]
    fn reflection_cannot_construct_a_rigging_revision() {
        let mut dynamic_rigging_revision = DynamicTupleStruct::default();
        dynamic_rigging_revision.insert(0_u64);

        assert!(RiggingRevision::from_reflect(&dynamic_rigging_revision).is_none());
    }

    #[test]
    fn device_registry_types_register_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        for type_id in [
            TypeId::of::<Device>(),
            TypeId::of::<PresentWithUsableClaim>(),
        ] {
            assert!(
                type_registry
                    .get_type_data::<ReflectComponent>(type_id)
                    .is_some()
            );
        }
        for type_id in [TypeId::of::<Devices>(), TypeId::of::<RiggingRevision>()] {
            assert!(
                type_registry
                    .get_type_data::<ReflectResource>(type_id)
                    .is_some()
            );
        }

        drop(type_registry);
    }

    #[test]
    fn an_availability_edge_names_the_key_and_both_conclusions() -> Result<(), Box<dyn Error>> {
        let unplugged = reported_key("UNPLUGGED")?;
        let mut devices = Devices::default();
        replace(&mut devices, vec![reconciled(unplugged.clone())]);
        let unplugged_handle = handle(&devices, &unplugged);
        let mut absent = reconciled(unplugged.clone());
        absent.presence = Presence::Absent;

        let replacement = replace(&mut devices, vec![absent]);
        let [change] = replacement.changes.availability.as_slice() else {
            panic!("one availability edge must be retained");
        };
        assert_eq!(change.key, unplugged);
        assert!(matches!(change.from, KeyAvailability::Present(_)));
        assert!(matches!(change.to, KeyAvailability::Absent { .. }));
        assert_eq!(
            devices.resolve(&unplugged),
            DeviceResolution::Resolved(unplugged_handle)
        );
        assert!(replacement.changes.orphaned_entities.is_empty());

        Ok(())
    }

    // --- the authorization predicates ---

    /// Resolve one retained key to its handle, so an authorization test names the device it is
    /// asking about rather than the order the registry happened to issue handles in.
    fn handle(devices: &Devices, key: &DeviceKey) -> DeviceId {
        match devices.resolve(key) {
            DeviceResolution::Resolved(device_id) => device_id,
            DeviceResolution::NotResolved => panic!("key `{key:?}` must resolve after ingest"),
        }
    }

    #[test]
    fn proven_presumed_and_authored_devices_each_authorize_service() -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        for verdict in [
            IdentityVerdict::Proven,
            IdentityVerdict::Presumed,
            IdentityVerdict::Authored,
        ] {
            let mut state = reconciled(key.clone());
            state.verdict = verdict;
            let mut devices = Devices::default();
            replace(&mut devices, vec![state]);

            devices.authorize_service(handle(&devices, &key))?;
        }

        Ok(())
    }

    #[test]
    fn every_refusal_names_the_check_that_actually_failed() -> Result<(), Box<dyn Error>> {
        let offline = reported_key("OFFLINE")?;
        let absent = reported_key("ABSENT")?;
        let contended = reported_key("CONTENDED")?;
        let blocked = reported_key("BLOCKED")?;
        let unverified = reported_key("UNVERIFIED")?;
        let mut offline_state = reconciled(offline.clone());
        offline_state.mode = ConfiguredDeviceMode::Offline;
        let mut absent_state = reconciled(absent.clone());
        absent_state.presence = Presence::Absent;
        let mut contended_state = reconciled(contended.clone());
        contended_state.claim = Claim::Contended {
            holder: ClaimHolder::Unidentified,
        };
        let mut blocked_state = reconciled(blocked.clone());
        blocked_state.claim = Claim::Blocked {
            gate: PermissionGate::CameraAccess,
        };
        let mut unverified_state = reconciled(unverified.clone());
        unverified_state.verdict = IdentityVerdict::Unverified(UnverifiedReason::NotUniqueInScan);
        let mut devices = Devices::default();
        replace(
            &mut devices,
            vec![
                offline_state,
                absent_state,
                contended_state,
                blocked_state,
                unverified_state,
            ],
        );

        let retired = DeviceId::new(u64::MAX);
        assert_eq!(
            devices.authorize_service(retired).err(),
            Some(ApplyAuthorizationError::DeviceRetired { device_id: retired })
        );
        for (key, expected) in [
            (
                &offline,
                ApplyAuthorizationError::Offline {
                    key: offline.clone(),
                },
            ),
            (
                &absent,
                ApplyAuthorizationError::NotPresent {
                    key: absent.clone(),
                },
            ),
            (
                &contended,
                ApplyAuthorizationError::ClaimUnavailable {
                    key: contended.clone(),
                },
            ),
            (
                &blocked,
                ApplyAuthorizationError::ClaimUnavailable {
                    key: blocked.clone(),
                },
            ),
            (
                &unverified,
                ApplyAuthorizationError::IdentityNotProven {
                    key: unverified.clone(),
                },
            ),
        ] {
            let device_id = handle(&devices, key);
            assert_eq!(devices.authorize_service(device_id).err(), Some(expected));
        }

        Ok(())
    }

    #[test]
    fn an_offline_authored_entry_refuses_service() -> Result<(), Box<dyn Error>> {
        let key = reported_key("WITHDRAWN")?;
        let mut offline = reconciled(key.clone());
        offline.mode = ConfiguredDeviceMode::Offline;
        let mut devices = Devices::default();
        replace(&mut devices, vec![offline]);
        let device_id = handle(&devices, &key);

        assert_eq!(
            devices.authorize_service(device_id).err(),
            Some(ApplyAuthorizationError::Offline { key })
        );

        Ok(())
    }

    #[test]
    fn a_disputed_capability_does_not_block_device_wide_service() -> Result<(), Box<dyn Error>> {
        let key = reported_key("STREAMDECK-XL-A00")?;
        let mut disputed_state = reconciled(key.clone());
        disputed_state.disputed = HashSet::from([TypeId::of::<DisputedCapability>()]);
        let mut devices = Devices::default();
        replace(&mut devices, vec![disputed_state]);
        let device_id = handle(&devices, &key);

        assert!(devices.authorize_service(device_id).is_ok());

        Ok(())
    }

    // --- attempt identifier issuance ---

    #[test]
    fn successive_issued_attempt_identifiers_differ_and_none_equals_the_default() {
        let mut attempts = Attempts::default();

        let first = attempts
            .issue()
            .expect("a fresh counter can issue an identifier");
        let second = attempts
            .issue()
            .expect("a fresh counter can issue a second identifier");

        assert_ne!(first, second);
        assert_ne!(first, AttemptRef::default());
        assert_ne!(second, AttemptRef::default());
    }
}
