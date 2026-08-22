use bevy::prelude::Entity;
#[cfg(test)]
use bevy::prelude::Resource;
#[cfg(test)]
use bevy::prelude::World;

use super::identity::MonitorConfigurationState;
use super::topology::DisplayDeviceEvidence;
use super::topology::InstalledMonitor;
use super::topology::MonitorChanges;
use super::topology::MonitorDescriptor;
use super::topology::MonitorTopologyRevision;
use super::topology::Monitors;
use crate::constants::MONITOR_PROBE_TARGET;

#[derive(Clone, Debug)]
pub(super) struct TopologyProbeRecord {
    frame_count:   u32,
    schedule:      TopologyProducerSchedule,
    configuration: MonitorConfigurationState,
    revision:      MonitorTopologyRevision,
    evidence:      DisplayDeviceEvidence,
    descriptor:    MonitorDescriptor,
    entity:        Entity,
    entity_state:  MonitorEntityState,
    change:        TopologyChangeKind,
}

#[cfg(test)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CapturedTopologyProbeRecord {
    pub(super) frame_count:              u32,
    pub(super) schedule:                 &'static str,
    pub(super) configuration_state:      &'static str,
    pub(super) configuration_generation: Option<u64>,
    pub(super) revision:                 u64,
    pub(super) entity:                   Entity,
    pub(super) change:                   &'static str,
}

#[cfg(test)]
#[derive(Default, Resource)]
pub(super) struct InjectedTopologyProbeRecords {
    pub(super) records: Vec<CapturedTopologyProbeRecord>,
}

#[derive(Clone, Copy, Debug)]
pub(super) enum TopologyProducerSchedule {
    PreStartup,
    Update,
}

impl TopologyProducerSchedule {
    const fn label(self) -> &'static str {
        match self {
            Self::PreStartup => "PreStartup::init_monitors",
            Self::Update => "Update::monitor_topology_producer",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(super) enum TopologyChangeKind {
    Connected,
    Disconnected,
    EvidenceChanged,
    RevalidatedUnchanged,
}

impl TopologyChangeKind {
    const fn label(self) -> &'static str {
        match self {
            Self::Connected => "connected",
            Self::Disconnected => "disconnected",
            Self::EvidenceChanged => "evidence-changed",
            Self::RevalidatedUnchanged => "revalidated-unchanged",
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum MonitorEntityState {
    Current,
    Former,
}

impl MonitorEntityState {
    const fn label(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Former => "former",
        }
    }
}

impl TopologyProbeRecord {
    pub(super) fn emit(&self) {
        let (configuration_state, configuration_generation) = self.configuration.probe_fields();
        tracing::info!(
            target: MONITOR_PROBE_TARGET,
            frame_count = u64::from(self.frame_count),
            producer_schedule = self.schedule.label(),
            configuration = ?self.configuration,
            configuration_state,
            configuration_generation = ?configuration_generation,
            topology_revision = self.revision.get(),
            device_evidence = ?self.evidence,
            monitor_descriptor = ?self.descriptor,
            monitor_entity = ?self.entity,
            monitor_entity_state = self.entity_state.label(),
            topology_change = self.change.label(),
            "installed monitor topology"
        );
    }
}

#[cfg(test)]
pub(super) fn capture_record(world: &mut World, record: &TopologyProbeRecord) {
    let (configuration_state, configuration_generation) = record.configuration.probe_fields();
    if let Some(mut captured) = world.get_resource_mut::<InjectedTopologyProbeRecords>() {
        captured.records.push(CapturedTopologyProbeRecord {
            frame_count: record.frame_count,
            schedule: record.schedule.label(),
            configuration_state,
            configuration_generation,
            revision: record.revision.get(),
            entity: record.entity,
            change: record.change.label(),
        });
    }
}

fn record(
    monitor: &InstalledMonitor,
    frame_count: u32,
    schedule: TopologyProducerSchedule,
    configuration: MonitorConfigurationState,
    revision: MonitorTopologyRevision,
    entity_state: MonitorEntityState,
    change: TopologyChangeKind,
) -> TopologyProbeRecord {
    TopologyProbeRecord {
        frame_count,
        schedule,
        configuration,
        revision,
        evidence: monitor.device_evidence.clone(),
        descriptor: monitor.descriptor,
        entity: monitor.entity,
        entity_state,
        change,
    }
}

pub(super) fn changed_probe_records(
    changes: &MonitorChanges,
    frame_count: u32,
    schedule: TopologyProducerSchedule,
    configuration: MonitorConfigurationState,
    revision: MonitorTopologyRevision,
) -> Vec<TopologyProbeRecord> {
    let mut records = Vec::new();
    records.extend(changes.connected.iter().map(|monitor| {
        record(
            monitor,
            frame_count,
            schedule,
            configuration,
            revision,
            MonitorEntityState::Current,
            TopologyChangeKind::Connected,
        )
    }));
    records.extend(changes.evidence_changed.iter().map(|monitor| {
        record(
            monitor,
            frame_count,
            schedule,
            configuration,
            revision,
            MonitorEntityState::Current,
            TopologyChangeKind::EvidenceChanged,
        )
    }));
    records.extend(changes.disconnected.iter().map(|monitor| {
        record(
            monitor,
            frame_count,
            schedule,
            configuration,
            revision,
            MonitorEntityState::Former,
            TopologyChangeKind::Disconnected,
        )
    }));
    records
}

pub(super) fn current_probe_records(
    monitors: &Monitors,
    frame_count: u32,
    schedule: TopologyProducerSchedule,
    configuration: MonitorConfigurationState,
    revision: MonitorTopologyRevision,
    change: TopologyChangeKind,
) -> Vec<TopologyProbeRecord> {
    monitors
        .live
        .iter()
        .map(|monitor| {
            record(
                monitor,
                frame_count,
                schedule,
                configuration,
                revision,
                MonitorEntityState::Current,
                change,
            )
        })
        .collect()
}
