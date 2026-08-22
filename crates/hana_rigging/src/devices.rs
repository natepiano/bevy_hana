use std::any::TypeId;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Duration;

use bevy::ecs::entity::Entity;
use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;
use thiserror::Error;

use crate::ApplyPermit;
use crate::AttachmentPath;
use crate::Attempt;
use crate::AttemptId;
use crate::Claim;
use crate::ConfiguredDeviceConnection;
use crate::ConfiguredDeviceMode;
use crate::DeviceId;
use crate::DeviceKey;
use crate::IdentityDecisionOwed;
use crate::IdentityVerdict;
use crate::Presence;
use crate::ReportedParent;
use crate::ReporterId;
use crate::RoleKey;
use crate::SchemeName;
use crate::attempt::AttemptDeadlineStatus;
#[cfg(test)]
use crate::binding::Bindings;
use crate::reconcile::FrameClockReading;

/// First identifier `Attempts` issues, chosen so no issued value equals `AttemptId::default()`.
const FIRST_ISSUED_ATTEMPT: u64 = 1;

/// Marker for the entity that mirrors one reconciled device.
///
/// Queries and the Bevy Remote Protocol reach device state through entities, so the kernel keeps
/// one entity per reconciled device alongside the `Devices` registry. The marker exists so a query
/// can select devices without naming every component the projection inserts.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub struct Device;

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

/// What the kernel currently believes about every device, keyed by the handle it issued.
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
    ids:                  HashMap<DeviceKey, DeviceId>,
    state:                HashMap<DeviceId, ReconciledDeviceState>,
    /// How many times each retained device's reconciled state has actually changed.
    ///
    /// Kept beside `Self::state` rather than on `ReconciledDeviceState` because the counter
    /// describes the history of a handle, not what the current pass concluded about the unit: a
    /// state a reporter supplies has no revision to carry, and the comparison that advances the
    /// counter would otherwise have to exclude a field of the value it is comparing.
    revision:             HashMap<DeviceId, DeviceRevision>,
    entity:               HashMap<DeviceId, Entity>,
    /// Issues `DeviceId`. Monotonic, never reused within a process, so a retired handle dangles
    /// instead of denoting a later device.
    next:                 u64,
    duplicate_keys:       HashSet<DeviceKey>,
    unregistered_schemes: HashSet<SchemeName>,
}

impl Devices {
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
    pub fn revision(&self, device_id: DeviceId) -> DeviceRevisionLookup {
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

    /// Replace the reconciled set with the current pass's conclusions.
    ///
    /// `reconciled` arrives roots first so presence is already folded down each parent chain.
    /// Keys absent from it are retired by key: their handle, their state, and their entity mapping
    /// are dropped, and a device that returns later receives a newly issued handle.
    ///
    /// A key a reporter still names while it stops reporting the unit `Presence::Present` is the
    /// other way a device leaves. It is recorded through `Presence::is_same_variant`, the one
    /// comparison the entity mirror also uses, and it retires nothing: the unplugged monitor keeps
    /// its handle, its state, and its entity while the roles bound to it learn it is gone.
    pub(crate) fn replace_reconciled(
        &mut self,
        reconciled: Vec<ReconciledDeviceState>,
        duplicate_keys: HashSet<DeviceKey>,
        unregistered_schemes: HashSet<SchemeName>,
    ) -> ReconciledDeviceReplacement {
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
            revision.insert(
                device_id,
                self.advanced_revision(device_id, &reconciled_device_state),
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
            if self.state.get(&device_id).is_some_and(|held| {
                held.presence == Presence::Present
                    && !held
                        .presence
                        .is_same_variant(reconciled_device_state.presence)
            }) {
                changes.departed.push(DepartedDevice {
                    key:       reconciled_device_state.key.clone(),
                    departure: DeviceDeparture::RetainedButNotPresent,
                });
            }
            ids.insert(reconciled_device_state.key.clone(), device_id);
            state.insert(device_id, reconciled_device_state);
        }

        for (key, device_id) in &self.ids {
            if !state.contains_key(device_id) {
                changes.departed.push(DepartedDevice {
                    key:       key.clone(),
                    departure: DeviceDeparture::KeyLeftTheSet,
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
            && self.duplicate_keys == duplicate_keys
            && self.unregistered_schemes == unregistered_schemes
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
        self.duplicate_keys = duplicate_keys;
        self.unregistered_schemes = unregistered_schemes;

        ReconciledDeviceReplacement {
            changes,
            device_register_change_detection,
        }
    }

    /// Remember which entity mirrors one handle, so the next pass updates that entity instead of
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

    /// Decide whether one device may be put in service.
    ///
    /// The only in-service decision point in the kernel. Every check is against the merged view,
    /// so a co-reported device resolves most-restrictive-wins: one reporter seeing an idle camera
    /// does not authorize capture when another watched a second application open it.
    ///
    /// `disputed` is deliberately ignored. A unit whose reporters contradict each other about one
    /// capability is still correct about every other one, so refusing the whole device would take a
    /// Stream Deck dark over a disagreement about its LED brightness range. A consumer that must
    /// not act on a contested capability reads the `crate::CapabilitiesDisputed` event, which names
    /// the contested types for that device.
    ///
    /// # Errors
    ///
    /// Returns the `ApplyAuthorizationError` naming the first check that refused: an unknown
    /// handle, an identity that was never proven, a unit that is not reachable, a claim another
    /// process holds, or an authored entry the application marked offline.
    pub fn authorize_service(
        &self,
        device_id: DeviceId,
    ) -> Result<ApplyPermit, ApplyAuthorizationError> {
        self.in_service_state(device_id)?;

        Ok(ApplyPermit::in_service())
    }

    /// Decide whether one device may receive a saved configuration back.
    ///
    /// The weaker gate: it additionally accepts `IdentityVerdict::RestoreOnly`, so a window can go
    /// back to the monitor a synthesized key names. It still refuses an offline authored entry,
    /// because returning a saved layout to hardware the application withdrew is still an automatic
    /// action on that hardware.
    ///
    /// # Errors
    ///
    /// Returns the `ApplyAuthorizationError` naming the first check that refused, on the same
    /// terms as `Self::authorize_service` except that a restore-only identity passes.
    pub fn authorize_restore(
        &self,
        device_id: DeviceId,
    ) -> Result<ApplyPermit, ApplyAuthorizationError> {
        let reconciled_device_state = self.authorized_state(device_id)?;
        if !reconciled_device_state.verdict.identified() {
            return Err(ApplyAuthorizationError::IdentityNotProven {
                key: reconciled_device_state.key.clone(),
            });
        }

        Ok(ApplyPermit::restore_only())
    }

    /// Run every device-wide in-service check and hand back the state both in-service predicates
    /// read, so neither of them resolves the same handle twice.
    fn in_service_state(
        &self,
        device_id: DeviceId,
    ) -> Result<&ReconciledDeviceState, ApplyAuthorizationError> {
        let reconciled_device_state = self.authorized_state(device_id)?;
        match reconciled_device_state.verdict {
            IdentityVerdict::Proven | IdentityVerdict::Authored => Ok(reconciled_device_state),
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
        if reconciled_device_state.presence != Presence::Present {
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
    ) -> DeviceRevision {
        let Some(retained) = self.state.get(&device_id) else {
            return DeviceRevision::default();
        };
        let device_revision = self.revision.get(&device_id).copied().unwrap_or_default();
        if retained.holds_same_facts(reported) {
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

/// What the kernel currently believes about one device.
///
/// Durable and entity-free, so retirement by key runs with no entity alive. Capability *values*
/// stay with the reporter registry that retains them — `Box<dyn Reflect>` is neither clonable nor
/// reflectable, and a second copy could drift from the reporter's. What lands here is the
/// normalized conclusions the kernel itself drew, plus the one piece of reporter evidence a later
/// pass has to remember: `Self::attachment`, without which a returning unit could never be judged
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
    /// one, which is what decides whether its `DeviceRevision` advances.
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
    /// Devices that stopped being usable this pass, whether their key left the reported set or a
    /// reporter still names them while no longer reporting them present.
    pub(crate) departed:          Vec<DepartedDevice>,
    /// Entities that were mirroring a departed device and now need despawning.
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

/// One device that left service this pass, and which of the two ways it left by.
///
/// The cause travels with the key because the two lead to different work: both make every
/// `crate::RecoveryPolicy::ReapplyOnReturn` role owe its restoration, but only a key that left the
/// reported set retires a handle, despawns an entity, and drops a `crate::ResolvedToDevice` link.
#[derive(Debug)]
pub(crate) struct DepartedDevice {
    pub(crate) key:       DeviceKey,
    pub(crate) departure: DeviceDeparture,
}

/// How one device stopped being usable.
///
/// Public because it is the payload of `crate::DeviceDeparted`: a consumer that must tell an
/// unplugged unit from one a reporter still enumerates but no longer reports present cannot get
/// that from the key alone, and after a `Self::KeyLeftTheSet` there is no entity left to read it
/// from either.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Reflect)]
pub enum DeviceDeparture {
    /// No reporter named the key this pass, so its handle, state, and entity are retired.
    KeyLeftTheSet,
    /// A reporter still names the key but no longer reports the unit present, so everything keyed
    /// to it stays while the hardware itself is gone.
    RetainedButNotPresent,
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
pub(crate) struct DepartureAnnouncements {
    /// Devices that left service this pass, still carrying which of the two departures it was.
    pub(crate) departed:    Vec<DepartedDevice>,
    /// Authored inventory keys whose connection conclusion this pass changed.
    pub(crate) connections: Vec<ConfiguredDeviceConnectionChange>,
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
    /// `Authored`, and a restore additionally accepts `RestoreOnly`.
    #[error("device `{key:?}` identity does not authorize this operation")]
    IdentityNotProven {
        /// Durable key of the device whose verdict refused the operation.
        key: DeviceKey,
    },
}

/// Result of reading the kernel's belief about one handle.
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
/// It advances once per reconcile pass in which any reporter returned a complete scan, whether or
/// not the contents changed. Counting completed scans rather than content changes keeps a rapid
/// absent-then-present cycle visible to a consumer watching this value: the set looks identical at
/// both ends, so a content hash would report no change and a panel watching the revision would
/// stay blank.
///
/// Reacquisition does not read this counter. It compares each device's presence reading from one
/// pass to the next, and attempt staleness keys on the per-device [`DeviceRevision`].
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Default, Resource, Reflect)]
#[reflect(opaque)]
#[reflect(Resource)]
pub struct RiggingRevision(u64);

impl RiggingRevision {
    /// Report the number of reconcile passes that ingested at least one completed scan.
    #[must_use]
    pub const fn get(self) -> u64 { self.0 }

    pub(crate) const fn advance(&mut self) { self.0 += 1; }
}

/// How many times one device's own reconciled state has changed since the kernel issued its handle.
///
/// Separate from `RiggingRevision` because the two answer different questions: the global counter
/// says how many scans have landed anywhere, which is a fact about the reporters, while this says
/// whether *this* unit moved. An attempt validated against the global counter is abandoned by any
/// reporter's routine scan, including one that never names its device; validated against this
/// counter it survives every pass that reports it unchanged.
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

/// In-flight endpoint attempts, keyed by the identifier this registry issued.
///
/// The registry is the only issuer of `AttemptId`, and its counter is monotonic and never reused,
/// so a driver poll that arrives after its attempt finished resolves to nothing rather than to a
/// later attempt for another role.
///
/// `next` starts at 1, not 0: `AttemptId` derives `Default`, so `AttemptId::default()` is
/// `AttemptId(0)`. An issued identifier equal to a defaulted or reflection-round-tripped one
/// would make "a late poll for a finished attempt resolves to nothing" unenforceable, because the
/// zero value appears wherever a field was left unset.
#[derive(Debug, Resource, Reflect)]
#[reflect(Resource)]
pub struct Attempts {
    in_flight:       HashMap<AttemptId, Attempt>,
    /// Which attempt each role currently has in flight.
    ///
    /// Retirement and replacement both destroy the `crate::RoleState::Applying` that named the
    /// in-flight attempt before the abort path runs, and `crate::binding::BindingTransition`
    /// carries only the role. Without this index an abandoned attempt would keep polling a
    /// driver for a role that no longer exists.
    attempt_by_role: HashMap<RoleKey, AttemptId>,
    next:            u64,
}

impl Default for Attempts {
    fn default() -> Self {
        Self {
            in_flight:       HashMap::new(),
            attempt_by_role: HashMap::new(),
            next:            FIRST_ISSUED_ATTEMPT,
        }
    }
}

impl Attempts {
    /// Read the record the kernel retained for one issued identifier.
    #[must_use]
    pub fn in_flight(&self, attempt: AttemptId) -> AttemptLookup<'_> {
        self.in_flight
            .get(&attempt)
            .map_or(AttemptLookup::Finished, AttemptLookup::InFlight)
    }

    /// Advance the counter and hand back the identifier the next attempt record must carry.
    ///
    /// # Errors
    ///
    /// Returns `AttemptIssueError::SequenceExhausted` when the counter cannot advance without
    /// wrapping back to `AttemptId::default()` and reusing identifiers a late driver poll could
    /// still resolve.
    pub(crate) fn issue(&mut self) -> Result<AttemptId, AttemptIssueError> {
        let next = self
            .next
            .checked_add(1)
            .ok_or(AttemptIssueError::SequenceExhausted)?;
        let attempt = AttemptId::new(self.next);
        self.next = next;

        Ok(attempt)
    }

    /// Hand back an identifier whose dispatch never committed, so the counter does not advance.
    ///
    /// Minting has to happen before erased dispatch, because the request the driver receives
    /// carries the identifier — but a driver that is unregistered or refuses the configuration
    /// contract returns without ever seeing it. Without this, a role whose driver refuses would
    /// consume one identifier on every dispatch, and the sequence guarding against reuse would be
    /// spent by attempts that never ran.
    ///
    /// Only the most recently issued identifier is reclaimed, and only while it is not retained:
    /// rewinding past a retained attempt would reissue an identifier a driver may still poll.
    fn release(&mut self, attempt: AttemptId) {
        if self.in_flight.contains_key(&attempt) {
            return;
        }
        if self.next == attempt.value().saturating_add(1) {
            self.next = attempt.value();
        }
    }

    /// Remove and reclaim the provisional attempt for a driver start that did not commit.
    ///
    /// Only the most recently issued identifier can be provisional. Checking that identifier before
    /// removing anything prevents a late rollback from ending older work that a driver may still
    /// poll. The normal attempt-ending paths use [`Self::end`] and never return an identifier to
    /// the counter.
    pub(crate) fn rollback_dispatch(&mut self, attempt: AttemptId) {
        let Some(next) = attempt.value().checked_add(1) else {
            return;
        };
        if self.next != next {
            return;
        }
        self.end(attempt);
        self.release(attempt);
    }

    /// How many attempts are still in flight.
    #[must_use]
    pub fn len(&self) -> usize { self.in_flight.len() }

    /// Report whether no attempt is in flight.
    #[must_use]
    pub fn is_empty(&self) -> bool { self.in_flight.is_empty() }

    /// Begin retaining one attempt record until its driver reports a terminal outcome.
    ///
    /// Named for the act rather than for the storage: every other `retain` in this crate filters a
    /// collection, and a reader meeting `Attempts::retain` would expect this call to drop records.
    pub(crate) fn begin(&mut self, attempt: Attempt) {
        self.attempt_by_role
            .insert(attempt.role.clone(), attempt.id);
        self.in_flight.insert(attempt.id, attempt);
    }

    /// Stop retaining one attempt, whether it succeeded, failed, or was abandoned.
    ///
    /// Removing the role index here rather than at each ending site is what keeps a later poll from
    /// resolving a role to an attempt the kernel already finished.
    pub(crate) fn end(&mut self, attempt: AttemptId) {
        if let Some(ended) = self.in_flight.remove(&attempt)
            && self.attempt_by_role.get(&ended.role) == Some(&attempt)
        {
            self.attempt_by_role.remove(&ended.role);
        }
    }

    /// Iterate every attempt the kernel currently retains.
    ///
    /// The apply systems re-validate each one before any driver poll, so the registry has to be
    /// walkable rather than only addressable by identifier.
    pub(crate) fn in_flight_attempts(&self) -> impl Iterator<Item = &Attempt> {
        self.in_flight.values()
    }

    /// Ask which attempt is in flight for one application role.
    ///
    /// `bindings` is consulted only to separate a bound role that happens to be idle from a role
    /// that has no binding at all: a test that conflated the two would pass while a registration
    /// silently failed.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn in_flight_for(&self, role: &RoleKey, bindings: &Bindings) -> RoleAttemptLookup {
        self.attempt_by_role.get(role).copied().map_or_else(
            || {
                if bindings.binding(role).is_ok() {
                    RoleAttemptLookup::Idle
                } else {
                    RoleAttemptLookup::NoSuchRole
                }
            },
            RoleAttemptLookup::InFlight,
        )
    }

    /// Report where one attempt stands against its own deadline and the bounded overrun budget.
    ///
    /// Being overdue is neither an error nor a failure, so the kernel keeps polling through
    /// `AttemptDeadlineStatus::OverdueWithinOverrun` and only abandons the attempt once the budget
    /// is spent. A clock that has not advanced past application startup answers
    /// `AttemptDeadlineStatus::WithinDeadline`, because a first frame carries no elapsed time an
    /// overrun could be computed from.
    #[must_use]
    pub(crate) fn deadline_status(
        &self,
        attempt: AttemptId,
        now: FrameClockReading,
        apply_overrun: Duration,
    ) -> AttemptDeadlineStatus {
        let Some(retained) = self.in_flight.get(&attempt) else {
            return AttemptDeadlineStatus::NoSuchAttempt;
        };
        let FrameClockReading::Measurable(now) = now else {
            return AttemptDeadlineStatus::WithinDeadline;
        };
        if now <= retained.deadline {
            return AttemptDeadlineStatus::WithinDeadline;
        }
        let past_deadline = now.duration_since(retained.deadline);
        if past_deadline > apply_overrun {
            AttemptDeadlineStatus::OverrunExhausted { past_deadline }
        } else {
            AttemptDeadlineStatus::OverdueWithinOverrun { past_deadline }
        }
    }
}

/// Result of asking which attempt one application role currently has in flight.
///
/// A named result rather than an optional identifier: a test that could not tell "this role is
/// bound and idle" from "there is no such role" would treat a failed registration as a settled one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg(test)]
pub(crate) enum RoleAttemptLookup {
    /// No binding exists for this role, so nothing could have been in flight for it.
    NoSuchRole,
    /// The role is bound and has no driver operation in flight.
    Idle,
    /// This attempt is in flight for the role and is the one an abort must end.
    InFlight(AttemptId),
}

/// Result of looking one issued identifier up in the attempt registry.
///
/// A named result rather than an optional record: "no record" is the terminal answer a late poll
/// must receive, and a caller that read it as "not ready yet" would keep polling a driver whose
/// attempt already ended.
#[derive(Clone, Copy, Debug)]
pub enum AttemptLookup<'a> {
    /// No record remains for this identifier, so the attempt it named has already ended.
    Finished,
    /// The kernel retains this attempt and the authorization it was started under.
    InFlight(&'a Attempt),
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
    use bevy::platform::time::Instant;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::reflect::FromReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::ApplyAuthorizationError;
    use super::AttemptLookup;
    use super::Attempts;
    use super::Device;
    use super::DeviceDeparture;
    use super::DeviceRegisterChangeDetection;
    use super::DeviceResolution;
    use super::DeviceRevision;
    use super::DeviceRevisionLookup;
    use super::DeviceStateLookup;
    use super::Devices;
    use super::PresentWithUsableClaim;
    use super::ReconciledDeviceState;
    use super::RiggingRevision;
    use super::RoleAttemptLookup;
    use crate::ApplyDeadline;
    use crate::AttachmentPath;
    use crate::Attempt;
    use crate::AttemptId;
    use crate::Claim;
    use crate::ClaimHolder;
    use crate::ConfiguredDeviceMode;
    use crate::DeviceId;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::EndpointId;
    use crate::IdentityDecisionOwed;
    use crate::IdentityVerdict;
    use crate::PermissionGate;
    use crate::Presence;
    use crate::RecoveryPolicy;
    use crate::ReportedId;
    use crate::ReportedParent;
    use crate::RetryOn;
    use crate::SchemeName;
    use crate::UnverifiedReason;
    use crate::attempt::AttemptDeadlineStatus;
    use crate::reconcile::FrameClockReading;

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

    #[test]
    fn resolution_distinguishes_an_unknown_key_from_a_retained_handle() -> Result<(), Box<dyn Error>>
    {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let absent_key = reported_key("DELL-U2723QE-OTHER")?;
        let mut devices = Devices::default();

        assert_eq!(devices.resolve(&key), DeviceResolution::NotResolved);

        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );

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
        let first_replacement = devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
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
            let replacement = devices.replace_reconciled(
                vec![reconciled(key.clone())],
                HashSet::new(),
                HashSet::new(),
            );
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
        let unreachable_replacement =
            devices.replace_reconciled(vec![unreachable], HashSet::new(), HashSet::new());
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
        let later_replacement =
            devices.replace_reconciled(vec![later], HashSet::new(), HashSet::new());
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
        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
        let DeviceResolution::Resolved(device_id) = devices.resolve(&key) else {
            panic!("an ingested key must resolve to the handle the registry issued");
        };

        devices.replace_reconciled(Vec::new(), HashSet::new(), HashSet::new());

        assert_eq!(devices.revision(device_id), DeviceRevisionLookup::Retired);

        Ok(())
    }

    #[test]
    fn a_returning_key_never_reuses_the_retired_handle() -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
        let DeviceResolution::Resolved(first) = devices.resolve(&key) else {
            panic!("an ingested key must resolve");
        };

        devices.replace_reconciled(Vec::new(), HashSet::new(), HashSet::new());
        assert!(matches!(devices.state(first), DeviceStateLookup::Retired));

        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
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
        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
        let first = devices.resolve(&key);

        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );

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
    fn both_ways_a_device_leaves_are_reported_and_stay_distinguishable()
    -> Result<(), Box<dyn Error>> {
        let unplugged = reported_key("UNPLUGGED")?;
        let removed = reported_key("REMOVED")?;
        let mut devices = Devices::default();
        devices.replace_reconciled(
            vec![reconciled(unplugged.clone()), reconciled(removed.clone())],
            HashSet::new(),
            HashSet::new(),
        );
        let unplugged_handle = handle(&devices, &unplugged);
        let mut absent = reconciled(unplugged.clone());
        absent.presence = Presence::Absent;

        let replacement = devices.replace_reconciled(vec![absent], HashSet::new(), HashSet::new());

        let departures: Vec<(DeviceKey, DeviceDeparture)> = replacement
            .changes
            .departed
            .iter()
            .map(|departed_device| (departed_device.key.clone(), departed_device.departure))
            .collect();
        assert!(departures.contains(&(unplugged.clone(), DeviceDeparture::RetainedButNotPresent)));
        assert!(departures.contains(&(removed, DeviceDeparture::KeyLeftTheSet)));
        // The unit that stopped being present keeps everything keyed to it, so its roles stay bound
        // to the same handle while the hardware is away.
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
    fn a_proven_present_unclaimed_device_authorizes_both_service_and_restore()
    -> Result<(), Box<dyn Error>> {
        let key = reported_key("DELL-U2723QE-9J4K2H3")?;
        let mut devices = Devices::default();
        devices.replace_reconciled(
            vec![reconciled(key.clone())],
            HashSet::new(),
            HashSet::new(),
        );
        let device_id = handle(&devices, &key);

        assert!(
            devices
                .authorize_service(device_id)?
                .allows_in_service_use()
        );
        assert!(
            !devices
                .authorize_restore(device_id)?
                .allows_in_service_use()
        );

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
        devices.replace_reconciled(
            vec![
                offline_state,
                absent_state,
                contended_state,
                blocked_state,
                unverified_state,
            ],
            HashSet::new(),
            HashSet::new(),
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
            assert_eq!(
                devices.authorize_service(device_id).err(),
                Some(expected.clone())
            );
            assert_eq!(devices.authorize_restore(device_id).err(), Some(expected));
        }

        Ok(())
    }

    #[test]
    fn a_restore_only_verdict_returns_a_saved_configuration_and_drives_nothing()
    -> Result<(), Box<dyn Error>> {
        let key = reported_key("SERIAL-LESS-PANEL")?;
        let mut restore_only = reconciled(key.clone());
        restore_only.verdict = IdentityVerdict::RestoreOnly;
        let mut devices = Devices::default();
        devices.replace_reconciled(vec![restore_only], HashSet::new(), HashSet::new());
        let device_id = handle(&devices, &key);

        assert_eq!(
            devices.authorize_service(device_id).err(),
            Some(ApplyAuthorizationError::IdentityNotProven { key })
        );
        assert!(
            !devices
                .authorize_restore(device_id)?
                .allows_in_service_use()
        );

        Ok(())
    }

    #[test]
    fn an_offline_authored_entry_refuses_a_restore_as_well_as_service() -> Result<(), Box<dyn Error>>
    {
        let key = reported_key("WITHDRAWN")?;
        let mut offline = reconciled(key.clone());
        offline.mode = ConfiguredDeviceMode::Offline;
        let mut devices = Devices::default();
        devices.replace_reconciled(vec![offline], HashSet::new(), HashSet::new());
        let device_id = handle(&devices, &key);

        assert_eq!(
            devices.authorize_restore(device_id).err(),
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
        devices.replace_reconciled(vec![disputed_state], HashSet::new(), HashSet::new());
        let device_id = handle(&devices, &key);

        assert!(devices.authorize_service(device_id).is_ok());
        assert!(devices.authorize_restore(device_id).is_ok());

        Ok(())
    }

    // --- the attempt registry ---

    #[test]
    fn successive_issued_attempt_identifiers_differ_and_none_equals_the_default() {
        let mut attempts = Attempts::default();

        let first = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        let second = attempts
            .issue()
            .expect("a fresh registry can issue a second identifier");

        assert_ne!(first, second);
        assert_ne!(first, AttemptId::default());
        assert_ne!(second, AttemptId::default());
    }

    #[test]
    fn an_identifier_whose_dispatch_never_committed_goes_back_on_offer() {
        let mut attempts = Attempts::default();
        let first = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");

        attempts.release(first);

        assert_eq!(
            attempts
                .issue()
                .expect("the released identifier is on offer again"),
            first
        );

        // A retained attempt is never reclaimed, because a driver may still poll it, and neither is
        // an identifier the counter has already moved past.
        let retained = attempts
            .issue()
            .expect("a fresh registry can issue a second identifier");
        attempts.begin(test_attempt(retained, bevy::platform::time::Instant::now()));
        attempts.release(retained);
        attempts.release(first);

        assert_ne!(
            attempts
                .issue()
                .expect("the registry can still issue after two refused releases"),
            retained
        );
    }

    #[test]
    fn a_provisional_dispatch_rollback_removes_the_attempt_and_its_role_index() {
        let mut attempts = Attempts::default();
        let attempt = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        attempts.begin(test_attempt(attempt, bevy::platform::time::Instant::now()));

        attempts.rollback_dispatch(attempt);

        assert!(matches!(
            attempts.in_flight(attempt),
            AttemptLookup::Finished
        ));
        assert!(attempts.attempt_by_role.is_empty());
        assert_eq!(
            attempts
                .issue()
                .expect("the provisional identifier is on offer again"),
            attempt
        );
    }

    #[test]
    fn a_dispatch_rollback_can_reclaim_only_the_most_recently_issued_identifier() {
        let mut attempts = Attempts::default();
        let retained = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        attempts.begin(test_attempt(retained, bevy::platform::time::Instant::now()));
        let provisional = attempts
            .issue()
            .expect("the registry can issue beside retained work");

        attempts.rollback_dispatch(retained);

        assert!(matches!(
            attempts.in_flight(retained),
            AttemptLookup::InFlight(_)
        ));
        attempts.rollback_dispatch(provisional);
        assert_eq!(
            attempts
                .issue()
                .expect("only the provisional identifier was reclaimed"),
            provisional
        );
    }

    #[test]
    fn a_retained_attempt_is_in_flight_while_an_unissued_identifier_is_finished() {
        let mut attempts = Attempts::default();
        let attempt = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        attempts.begin(Attempt {
            id:                 attempt,
            role:               crate::RoleKey::new("window/main")
                .expect("`window/main` is a valid role handle"),
            endpoint:           crate::DeviceEndpoint {
                device: DeviceKey {
                    kind: DeviceKind::Display,
                    id:   DeviceIdSource::Authored {
                        value: crate::AuthoredId::new("panel")
                            .expect("`panel` is a valid authored identifier"),
                    },
                },
                id:     EndpointId::Whole,
            },
            permit:             crate::ApplyPermit::in_service(),
            binding_generation: crate::BindingGeneration::default(),
            expected_device_id: DeviceId::new(0),
            device_revision:    DeviceRevision::default(),
            deadline:           bevy::platform::time::Instant::now(),
        });

        assert!(matches!(
            attempts.in_flight(attempt),
            AttemptLookup::InFlight(_)
        ));
        assert!(matches!(
            attempts.in_flight(AttemptId::default()),
            AttemptLookup::Finished
        ));
    }

    #[test]
    fn the_attempt_registry_registers_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<Attempts>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectResource>(type_id)
                .is_some()
        );

        drop(type_registry);
    }

    fn test_attempt(id: AttemptId, deadline: Instant) -> Attempt {
        Attempt {
            id,
            role: crate::RoleKey::new("window/main").expect("`window/main` is a valid role handle"),
            endpoint: test_endpoint(),
            permit: crate::ApplyPermit::in_service(),
            binding_generation: crate::BindingGeneration::default(),
            expected_device_id: DeviceId::new(0),
            device_revision: DeviceRevision::default(),
            deadline,
        }
    }

    fn test_endpoint() -> crate::DeviceEndpoint {
        crate::DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Authored {
                    value: crate::AuthoredId::new("panel")
                        .expect("`panel` is a valid authored identifier"),
                },
            },
            id:     EndpointId::Whole,
        }
    }

    #[test]
    fn the_deadline_query_separates_healthy_overdue_exhausted_and_unknown_attempts() {
        let mut attempts = Attempts::default();
        let attempt = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        let now = bevy::platform::time::Instant::now();
        let overrun = Duration::from_secs(5);
        attempts.begin(test_attempt(attempt, now + Duration::from_secs(10)));
        let reading = |elapsed| FrameClockReading::Measurable(now + Duration::from_secs(elapsed));

        assert_eq!(
            attempts.deadline_status(attempt, reading(1), overrun),
            AttemptDeadlineStatus::WithinDeadline
        );
        assert_eq!(
            attempts.deadline_status(attempt, reading(12), overrun),
            AttemptDeadlineStatus::OverdueWithinOverrun {
                past_deadline: Duration::from_secs(2),
            }
        );
        assert_eq!(
            attempts.deadline_status(attempt, reading(20), overrun),
            AttemptDeadlineStatus::OverrunExhausted {
                past_deadline: Duration::from_secs(10),
            }
        );
        assert_eq!(
            attempts.deadline_status(AttemptId::default(), reading(20), overrun),
            AttemptDeadlineStatus::NoSuchAttempt
        );
    }

    #[test]
    fn a_clock_that_has_not_advanced_never_reports_an_attempt_overdue() {
        let mut attempts = Attempts::default();
        let attempt = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        // A deadline already in the past: only the missing clock reading keeps this within it.
        let long_past = bevy::platform::time::Instant::now()
            .checked_sub(Duration::from_mins(1))
            .expect("the process started after the clock's origin");
        attempts.begin(test_attempt(attempt, long_past));

        assert_eq!(
            attempts.deadline_status(
                attempt,
                crate::reconcile::FrameClockReading::NotYetAdvanced,
                Duration::ZERO,
            ),
            AttemptDeadlineStatus::WithinDeadline
        );
    }

    #[test]
    fn the_role_keyed_lookup_separates_an_unbound_role_from_an_idle_and_a_working_one()
    -> Result<(), Box<dyn std::error::Error>> {
        let role = crate::RoleKey::new("window/main")?;
        let mut attempts = Attempts::default();
        let mut bindings = crate::binding::Bindings::default();

        assert_eq!(
            attempts.in_flight_for(&role, &bindings),
            RoleAttemptLookup::NoSuchRole
        );

        bindings.register(crate::Binding {
            role:            role.clone(),
            endpoint:        test_endpoint(),
            driver:          crate::DriverId(0),
            recovery:        RecoveryPolicy::Forget,
            retry:           RetryOn::NewRevision,
            on_abort:        crate::OnAbort::default(),
            on_loss:         crate::OnSessionLoss::default(),
            state:           crate::RoleState::default(),
            requested:       crate::RequestedConfiguration::new(RoleAttemptTestConfiguration(3)),
            last_known_good: crate::LastKnownGoodConfiguration::default(),
            apply_deadline:  ApplyDeadline::ProcessDefault,
        })?;

        assert_eq!(
            attempts.in_flight_for(&role, &bindings),
            RoleAttemptLookup::Idle
        );

        let attempt = attempts
            .issue()
            .expect("a fresh registry can issue an identifier");
        attempts.begin(test_attempt(attempt, bevy::platform::time::Instant::now()));

        assert_eq!(
            attempts.in_flight_for(&role, &bindings),
            RoleAttemptLookup::InFlight(attempt)
        );

        // Retirement takes the binding while the driver is still working. The lookup is keyed by
        // role and not by binding for exactly this case: the kernel has to be able to find the
        // orphaned attempt in order to abandon it deliberately.
        bindings.retire(&role)?;

        assert_eq!(
            attempts.in_flight_for(&role, &bindings),
            RoleAttemptLookup::InFlight(attempt)
        );

        Ok(())
    }

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct RoleAttemptTestConfiguration(u32);
}
