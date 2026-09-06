use std::time::Duration;

use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::World;

use crate::AttachmentPath;
use crate::Capabilities;
use crate::Claim;
use crate::DeviceDescriptor;
use crate::DeviceKey;
use crate::PlatformDeviceHandle;
use crate::ReportedSerial;

/// Reporter observation of whether a unit can be reached in its most recently completed device
/// set.
///
/// [`Presence`] is an entity component because reporters update it as hardware appears, departs,
/// or becomes unreachable. [`Self::Unreachable`] is not [`Self::Absent`]: treating a silent remote
/// node as removed can retire output still attached to a live device.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub enum Presence {
    /// The reporter observed the unit and can use it, such as a connected display or camera.
    Present,
    /// The reporter established that the unit is gone from its whole current device set.
    Absent,
    /// The reporter cannot determine whether the unit remains available, such as when a remote
    /// node stopped responding or its transport disconnected.
    Unreachable {
        /// Time elapsed since the reporter first observed this unit as unreachable.
        since: Duration,
    },
}

impl Presence {
    /// Report whether two observations are the same kind of reachability, ignoring how long an
    /// [`Self::Unreachable`] one has lasted.
    ///
    /// Departure detection and the device-entity mirror both need this instead of `==`: the
    /// elapsed time inside [`Self::Unreachable`] grows on every scan, so value equality would
    /// report a change on every frame and make a once-per-change consumer fire forever. One
    /// function serves both callers, so the diff and the mirror classify a presence change the
    /// same way.
    pub(crate) const fn is_same_variant(self, other: Self) -> bool {
        matches!(
            (self, other),
            (Self::Present, Self::Present)
                | (Self::Absent, Self::Absent)
                | (Self::Unreachable { .. }, Self::Unreachable { .. })
        )
    }
}

/// Name status a reporter assigns to the unit represented by one [`DeviceRecord`].
///
/// The two variants replace `Option<DeviceKey>` because a reporter that can name a unit durably
/// participates in key reconciliation, while a reporter with only operating-system match evidence
/// must not fabricate a durable identity.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum ReportedAs {
    /// The reporter derived a durable key from evidence that can identify this unit across runs.
    Keyed(DeviceKey),
    /// The reporter listed the unit but supplied no durable name, as with a display API report
    /// that can join another reporter through [`DeviceRecord::platform_device_handle`] only.
    ///
    /// Reconciliation keeps this evidence only when it joins a keyed record. A report with no
    /// keyed match creates no device entity and cannot expose a fabricated [`DeviceKey`].
    MatchEvidenceOnly,
}

/// How a reporter reaches one unit: directly, or through another device it already names.
///
/// The variants replace an optional parent key because a root and a child are different reports,
/// not a value and its absence. The field is called a parent rather than a transport: a transport
/// would name a bus or protocol, which a kernel that performs no input or output must never model,
/// while the value is the [`DeviceKey`] of another reported device.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum ReportedParent {
    /// The reporter reaches this unit directly, as with a display panel enumerated by the window
    /// system. A root sits at the top of one reporter's forest.
    Root,
    /// The reporter reaches this unit through the device named by this key, as a camera reached
    /// through the capture card it is plugged into.
    ///
    /// Presence is conjunctive down the chain: reconciliation never reports a child as more
    /// reachable than the device it hangs off, because a capture card that stopped responding
    /// cannot deliver frames from a camera behind it.
    ChildOf(DeviceKey),
}

/// One unit in a reporter's completed whole-set report.
///
/// [`DeviceRecord`] carries observed evidence but no [`IdentityVerdict`](crate::IdentityVerdict)
/// or [`DeviceId`](crate::DeviceId). Reconciliation creates those conclusions after comparing this
/// report with saved keys; allowing a reporter to supply either would let it assert identity for a
/// unit that supplied no evidence.
pub struct DeviceRecord {
    /// Durable naming status for this report, including the evidence-only case with no device key.
    pub reported_as:            ReportedAs,
    /// Parent link that places a child device below the device it is reached through, and names
    /// every directly reached unit a root.
    pub parent:                 ReportedParent,
    /// Reporter observation of whether this unit is present, absent, or unreachable.
    pub presence:               Presence,
    /// Reporter observation of exclusive ownership, independent from whether the unit is present.
    pub claim:                  Claim,
    /// Component values that describe what this unit can do, as this reporter reports them.
    pub capabilities:           Capabilities,
    /// Serial evidence supplied by the unit or the reason no serial value was available to the
    /// reporter.
    pub serial:                 ReportedSerial,
    /// Process-local operating-system handle that can join reports without becoming persisted
    /// identity.
    pub platform_device_handle: PlatformDeviceHandle,
    /// Observed attachment location that reconciliation compares when a saved unit was displaced.
    pub attachment:             AttachmentPath,
    /// Vendor, product, and model evidence the reporter uses for synthesized identity and
    /// diagnostics.
    pub descriptor:             DeviceDescriptor,
}

impl DeviceRecord {
    /// Build a keyed report while requiring every reporter-owned evidence axis.
    #[must_use]
    pub const fn keyed(
        key: DeviceKey,
        parent: ReportedParent,
        presence: Presence,
        claim: Claim,
        capabilities: Capabilities,
        serial: ReportedSerial,
        platform_device_handle: PlatformDeviceHandle,
        attachment: AttachmentPath,
        descriptor: DeviceDescriptor,
    ) -> Self {
        Self {
            reported_as: ReportedAs::Keyed(key),
            parent,
            presence,
            claim,
            capabilities,
            serial,
            platform_device_handle,
            attachment,
            descriptor,
        }
    }
}

/// Whether an accepted whole-set report differs from the reporter's retained records.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RetainedSetChange {
    /// Every retained record has an equal record in the accepted set.
    Unchanged,
    /// At least one record was added, removed, or changed.
    Changed,
}

/// Compare two whole-set reports without assigning meaning to their enumeration order.
///
/// Sets are equal when they have the same length and every retained record has a distinct matching
/// accepted record, so record order never decides the result. Durable keys and
/// [`DeviceDescriptor`] values use typed structural equality, and each capability declaration uses
/// the typed equality function stored when `Capabilities::add` erased its component value. A
/// capability pair with different concrete types or unequal `PartialEq` values counts as
/// [`RetainedSetChange::Changed`]. The other reconciliation evidence is compared with typed
/// equality, except that two [`Presence::Unreachable`] values are equal here regardless of their
/// elapsed `since` values.
pub(crate) fn retained_set_change(
    retained: &[DeviceRecord],
    accepted: &[DeviceRecord],
) -> RetainedSetChange {
    if retained.len() != accepted.len() {
        return RetainedSetChange::Changed;
    }

    let mut unmatched_accepted: Vec<_> = accepted.iter().collect();
    for retained_record in retained {
        let matching_index = unmatched_accepted
            .iter()
            .position(|accepted_record| device_records_are_equal(retained_record, accepted_record));
        let Some(matching_index) = matching_index else {
            return RetainedSetChange::Changed;
        };
        unmatched_accepted.remove(matching_index);
    }

    RetainedSetChange::Unchanged
}

fn device_records_are_equal(left: &DeviceRecord, right: &DeviceRecord) -> bool {
    left.reported_as == right.reported_as
        && left.parent == right.parent
        && left.presence.is_same_variant(right.presence)
        && left.claim == right.claim
        && capabilities_are_equal(&left.capabilities, &right.capabilities)
        && left.serial == right.serial
        && left.platform_device_handle == right.platform_device_handle
        && left.attachment == right.attachment
        && left.descriptor == right.descriptor
}

fn capabilities_are_equal(left: &Capabilities, right: &Capabilities) -> bool {
    let mut unmatched_right: Vec<_> = right.declarations().collect();
    if left.declarations().count() != unmatched_right.len() {
        return false;
    }

    for left_declaration in left.declarations() {
        let matching_index = unmatched_right.iter().position(|right_declaration| {
            left_declaration.value().reflect_type_path()
                == right_declaration.value().reflect_type_path()
                && left_declaration.equals(right_declaration.value())
        });
        let Some(matching_index) = matching_index else {
            return false;
        };
        unmatched_right.remove(matching_index);
    }

    true
}

/// Reporter result for one scheduled attempt to enumerate its devices.
///
/// Not calling a reporter is the unchanged case: cadence and activation state record why it was
/// not due. A completed scan always contains the reporter's whole current set, so a missing record
/// is meaningful evidence of departure.
pub enum DeviceScan {
    /// The reporter scanned and supplied every currently visible device record.
    ///
    /// The reporter registry files this list under the registry-issued [`ReporterId`]; reporter
    /// implementations cannot supply that handle themselves.
    Complete(Vec<DeviceRecord>),
    /// The reporter scanned every visible device and retained one integration-state projection for
    /// the acceptance of this exact result.
    ///
    /// The kernel invokes the retained [`ReportAcceptanceProjection`] on the main thread only when
    /// it accepts this complete set. Finishing the producer job does not by itself publish
    /// integration state.
    CompleteWithProjection {
        /// Every currently visible device record from this run.
        devices:                      Vec<DeviceRecord>,
        /// Integration-owned state update bound to this run's acceptance.
        report_acceptance_projection: ReportAcceptanceProjection,
    },
    /// A startup prerequisite is not ready, so the reporter's preceding complete set stays current.
    Deferred(crate::ReporterDeferral),
    /// Enumeration failed before the reporter could establish its whole current set.
    ///
    /// The registry retains the preceding complete set, because treating an I/O failure like an
    /// empty device list would report every still-connected camera or display as departed.
    Failed(crate::DeviceAccessError),
}

/// One type-erased integration-state update bound to an accepted successful report.
///
/// The kernel invokes this action exactly once on the main thread when it accepts the same
/// [`DeviceScan::CompleteWithProjection`] result, whether or not the result changes the reporter's
/// retained record set. The action may mutate only state owned by the reporter's integration crate.
/// It must not mutate reporter or driver registries, device or binding entities, kernel components,
/// or any other kernel authority.
pub struct ReportAcceptanceProjection(Box<dyn FnOnce(&mut World) + Send + Sync + 'static>);

impl ReportAcceptanceProjection {
    /// Retain one integration-state update until the successful report that owns it is accepted.
    #[must_use]
    pub fn new(publish: impl FnOnce(&mut World) + Send + Sync + 'static) -> Self {
        Self(Box::new(publish))
    }

    pub(crate) fn publish(self, world: &mut World) { self.0(world); }
}

/// Whole current device set the reporter registry prepared after one completed scan.
pub(crate) struct DeviceSet {
    /// Every currently visible device from the reporter, with absent devices omitted. Parent links
    /// form a forest that reconciliation ingests from roots toward children.
    pub(crate) devices: Vec<DeviceRecord>,
}

/// Opaque process-local handle that the reporter registry issues in registration order.
///
/// [`ReporterId`] has no public constructor because reporters receive it from the reporter
/// registry; permitting a reporter to fabricate one would let it overwrite another reporter's whole
/// device set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Reflect)]
#[reflect(opaque)]
pub struct ReporterId(pub(crate) u32);

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::reflect::FromReflect;
    use bevy::reflect::tuple_struct::DynamicTupleStruct;

    use super::DeviceRecord;
    use super::Presence;
    use super::ReportedAs;
    use super::ReportedParent;
    use super::ReporterId;
    use super::RetainedSetChange;
    use super::retained_set_change;
    use crate::AttachmentPath;
    use crate::Capabilities;
    use crate::Claim;
    use crate::DeviceDescriptor;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::Digest;
    use crate::PlatformDeviceHandle;
    use crate::ReportedId;
    use crate::ReportedIdError;
    use crate::ReportedSerial;

    #[derive(Component, PartialEq, Reflect)]
    struct TestCapability(u32);

    fn test_record(digest: u64, presence: Presence, capability: u32) -> DeviceRecord {
        DeviceRecord {
            reported_as: ReportedAs::Keyed(DeviceKey {
                kind: DeviceKind::ControlSurface,
                id:   DeviceIdSource::Synthesized {
                    digest: Digest::new(digest),
                },
            }),
            parent: ReportedParent::Root,
            presence,
            claim: Claim::NotApplicable,
            capabilities: Capabilities::new().with(TestCapability(capability)),
            serial: ReportedSerial::PlatformCannotReport,
            platform_device_handle: PlatformDeviceHandle::PlatformHasNoConcept,
            attachment: AttachmentPath::PlatformHasNoConcept,
            descriptor: DeviceDescriptor::PlatformHasNoConcept,
        }
    }

    fn match_evidence_record(model: &str) -> Result<DeviceRecord, ReportedIdError> {
        Ok(DeviceRecord {
            reported_as: ReportedAs::MatchEvidenceOnly,
            descriptor: DeviceDescriptor::Reported {
                vendor:  ReportedId::new("test vendor")?,
                product: ReportedId::new("test product")?,
                model:   ReportedId::new(model)?,
            },
            ..test_record(1, Presence::Present, 10)
        })
    }

    #[test]
    fn unreachable_presence_retains_when_the_reporter_lost_contact() {
        let since = Duration::from_secs(6);
        let presence = Presence::Unreachable { since };

        assert_eq!(presence, Presence::Unreachable { since });
    }

    #[test]
    fn evidence_only_record_has_no_reported_device_key() {
        let device_record = DeviceRecord {
            reported_as:            ReportedAs::MatchEvidenceOnly,
            parent:                 ReportedParent::Root,
            presence:               Presence::Present,
            claim:                  Claim::NotApplicable,
            capabilities:           Capabilities::new(),
            serial:                 ReportedSerial::NotExposedByUnit,
            platform_device_handle: PlatformDeviceHandle::PlatformReportedNothing,
            attachment:             AttachmentPath::PlatformHasNoConcept,
            descriptor:             DeviceDescriptor::PlatformReportedNothing,
        };

        assert!(matches!(
            device_record.reported_as,
            ReportedAs::MatchEvidenceOnly
        ));
        assert_eq!(device_record.parent, ReportedParent::Root);
    }

    #[test]
    fn retained_set_comparison_ignores_order_and_unreachable_elapsed_time() {
        let retained = vec![
            test_record(
                1,
                Presence::Unreachable {
                    since: Duration::from_secs(1),
                },
                10,
            ),
            test_record(2, Presence::Present, 20),
        ];
        let accepted = vec![
            test_record(2, Presence::Present, 20),
            test_record(
                1,
                Presence::Unreachable {
                    since: Duration::from_secs(30),
                },
                10,
            ),
        ];

        assert_eq!(
            retained_set_change(&retained, &accepted),
            RetainedSetChange::Unchanged
        );
    }

    #[test]
    fn retained_set_comparison_matches_evidence_only_records_by_the_full_record()
    -> Result<(), ReportedIdError> {
        let retained = vec![
            match_evidence_record("first")?,
            match_evidence_record("second")?,
        ];
        let accepted = vec![
            match_evidence_record("second")?,
            match_evidence_record("first")?,
        ];

        assert_eq!(
            retained_set_change(&retained, &accepted),
            RetainedSetChange::Unchanged
        );

        Ok(())
    }

    #[test]
    fn retained_set_comparison_reports_a_capability_difference() {
        let retained = vec![test_record(1, Presence::Present, 10)];
        let accepted = vec![test_record(1, Presence::Present, 11)];

        assert_eq!(
            retained_set_change(&retained, &accepted),
            RetainedSetChange::Changed
        );
    }

    #[test]
    fn retained_set_comparison_ignores_capability_declaration_order() {
        let mut retained_record = test_record(1, Presence::Present, 10);
        retained_record.capabilities = Capabilities::new()
            .with(TestCapability(10))
            .with(TestCapability(20));
        let mut accepted_record = test_record(1, Presence::Present, 10);
        accepted_record.capabilities = Capabilities::new()
            .with(TestCapability(20))
            .with(TestCapability(10));

        assert_eq!(
            retained_set_change(&[retained_record], &[accepted_record]),
            RetainedSetChange::Unchanged
        );
    }

    #[test]
    fn reflection_cannot_construct_reporter_id() {
        let mut dynamic_reporter_id = DynamicTupleStruct::default();
        dynamic_reporter_id.insert(0_u32);

        assert!(ReporterId::from_reflect(&dynamic_reporter_id).is_none());
    }
}
