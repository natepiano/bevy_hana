use std::fmt::Display;
use std::fmt::Formatter;
use std::fmt::Result as FormatResult;
use std::num::NonZeroUsize;

use bevy::ecs::reflect::ReflectComponent;
use bevy::ecs::system::SystemParam;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use hana_rigging::prelude::DeviceKey;
use thiserror::Error;

use super::DisplayIdentity;
use super::MonitorDescriptor;

/// Current Clerestory monitor lifetime projected onto a kernel device entity.
#[derive(Component, Clone, PartialEq, Reflect)]
#[reflect(Component, PartialEq)]
#[type_path = "hana_clerestory::monitors"]
pub struct LiveDisplayEndpoint {
    /// Bevy monitor entity for the current operating-system monitor lifetime.
    pub monitor:         Entity,
    /// Geometry and adapter data for the current monitor lifetime.
    pub descriptor:      MonitorDescriptor,
    /// Pre-v5 display identity retained only for old-file migration.
    pub legacy_identity: DisplayIdentity,
}

/// Device-entity relationship to the current monitor lifetime behind a display endpoint.
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq, Reflect)]
#[relationship(relationship_target = LiveDisplayDevices)]
#[reflect(Component, PartialEq)]
#[type_path = "hana_clerestory::monitors"]
pub struct LiveDisplayMonitor(Entity);

impl LiveDisplayMonitor {
    pub(crate) const fn new(monitor: Entity) -> Self { Self(monitor) }

    /// Current Bevy monitor entity related to this kernel device entity.
    #[must_use]
    pub const fn monitor(self) -> Entity { self.0 }
}

/// Kernel device entities currently projected for one Bevy monitor entity.
///
/// Bevy maintains this target from [`LiveDisplayMonitor`], so code that already holds the monitor
/// entity can reach its device without scanning every [`LiveDisplayEndpoint`]. A real monitor has
/// exactly one device; retaining a collection also represents contradictory synthetic evidence
/// without corrupting relationship bookkeeping.
#[derive(Component, Debug, Reflect)]
#[relationship_target(relationship = LiveDisplayMonitor)]
#[reflect(Component)]
#[type_path = "hana_clerestory::monitors"]
pub struct LiveDisplayDevices(Vec<Entity>);

impl LiveDisplayDevices {
    /// Builds one exact monitor-to-device association for downstream regression tests.
    #[cfg(any(test, feature = "test"))]
    #[must_use]
    pub fn single_device_for_test(device: Entity) -> Self { Self(vec![device]) }

    /// Resolve the one kernel device entity projected for this monitor lifetime.
    ///
    /// # Errors
    ///
    /// Returns [`LiveDisplayMatchError::NoLiveDisplayMatches`] when no source remains and
    /// [`LiveDisplayMatchError::SeveralLiveDisplaysMatch`] when contradictory evidence relates
    /// more than one device to the monitor.
    pub fn device(&self) -> Result<Entity, LiveDisplayMatchError> {
        let mut devices = self.0.iter().copied();
        let Some(first) = devices.next() else {
            return Err(LiveDisplayMatchError::NoLiveDisplayMatches);
        };
        let Some(_second) = devices.next() else {
            return Ok(first);
        };
        let count = LiveDisplayContradictionCount::from_second_and_remaining(devices.count());
        Err(LiveDisplayMatchError::SeveralLiveDisplaysMatch { count })
    }
}

/// Validated number of device entities contradictorily related to one monitor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Reflect)]
pub struct LiveDisplayContradictionCount(NonZeroUsize);

impl LiveDisplayContradictionCount {
    /// Counts a second matching device plus however many followed it.
    ///
    /// Callers reach this after a match iterator has already yielded two
    /// entries, so `remaining` counts only what is left, and the total is two
    /// or more by construction.
    #[must_use]
    pub fn from_second_and_remaining(remaining: usize) -> Self {
        let count = remaining.saturating_add(2);
        Self(NonZeroUsize::new(count).unwrap_or(NonZeroUsize::MIN))
    }

    #[cfg(test)]
    fn try_new(count: usize) -> Result<Self, LiveDisplayContradictionCountError> {
        if count < 2 {
            return Err(LiveDisplayContradictionCountError { supplied: count });
        }
        Ok(Self(NonZeroUsize::new(count).unwrap_or(NonZeroUsize::MIN)))
    }

    /// Number of contradictory device entities.
    #[must_use]
    pub const fn get(self) -> usize { self.0.get() }
}

impl Display for LiveDisplayContradictionCount {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> FormatResult { self.get().fmt(formatter) }
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
#[error("a live-display contradiction requires at least two candidates, got {supplied}")]
struct LiveDisplayContradictionCountError {
    supplied: usize,
}

/// Failure to resolve exactly one live display projection.
#[derive(Clone, Copy, Debug, Error, PartialEq, Eq)]
pub enum LiveDisplayMatchError {
    /// No projected display capability matches the supplied evidence.
    #[error("no live display matches")]
    NoLiveDisplayMatches,
    /// More than one projected display capability matches the supplied evidence.
    #[error("{count} live displays match")]
    SeveralLiveDisplaysMatch {
        /// Validated number of matching projected device entities.
        count: LiveDisplayContradictionCount,
    },
}

/// Exact reverse lookups over live display capabilities projected by the rigging kernel.
#[derive(SystemParam)]
pub struct LiveDisplayEndpointLookup<'w, 's> {
    endpoints: Query<'w, 's, (&'static DeviceKey, &'static LiveDisplayEndpoint)>,
}

impl LiveDisplayEndpointLookup<'_, '_> {
    /// Find the one live device key carrying `descriptor`.
    ///
    /// # Errors
    ///
    /// Returns [`LiveDisplayMatchError::NoLiveDisplayMatches`] when none match and
    /// [`LiveDisplayMatchError::SeveralLiveDisplaysMatch`] when more than one matches.
    pub fn key_for_descriptor(
        &self,
        descriptor: MonitorDescriptor,
    ) -> Result<DeviceKey, LiveDisplayMatchError> {
        self.key_matching(|endpoint| endpoint.descriptor == descriptor)
    }

    /// Find the one live device key carrying `legacy_identity`.
    ///
    /// Anonymous identities never match: equality between two anonymous values does not establish
    /// that they describe the same physical display.
    ///
    /// # Errors
    ///
    /// Returns [`LiveDisplayMatchError::NoLiveDisplayMatches`] when none match and
    /// [`LiveDisplayMatchError::SeveralLiveDisplaysMatch`] when more than one matches.
    pub fn key_for_legacy_identity(
        &self,
        legacy_identity: DisplayIdentity,
    ) -> Result<DeviceKey, LiveDisplayMatchError> {
        match legacy_identity {
            DisplayIdentity::Fingerprinted(_) => {
                self.key_matching(|endpoint| endpoint.legacy_identity == legacy_identity)
            },
            DisplayIdentity::Anonymous => Err(LiveDisplayMatchError::NoLiveDisplayMatches),
        }
    }

    fn key_matching(
        &self,
        predicate: impl Fn(&LiveDisplayEndpoint) -> bool,
    ) -> Result<DeviceKey, LiveDisplayMatchError> {
        let mut matching = self
            .endpoints
            .iter()
            .filter(|(_, endpoint)| predicate(endpoint))
            .map(|(device_key, _)| device_key.clone());
        let Some(first) = matching.next() else {
            return Err(LiveDisplayMatchError::NoLiveDisplayMatches);
        };
        let Some(_second) = matching.next() else {
            return Ok(first);
        };
        let count = LiveDisplayContradictionCount::from_second_and_remaining(matching.count());
        Err(LiveDisplayMatchError::SeveralLiveDisplaysMatch { count })
    }
}

#[cfg(test)]
mod contradiction_count_tests {
    use super::LiveDisplayContradictionCount;

    #[test]
    fn contradiction_count_rejects_zero_and_one() {
        assert!(LiveDisplayContradictionCount::try_new(0).is_err());
        assert!(LiveDisplayContradictionCount::try_new(1).is_err());
        assert_eq!(
            LiveDisplayContradictionCount::try_new(2).map(LiveDisplayContradictionCount::get),
            Ok(2)
        );
    }
}
