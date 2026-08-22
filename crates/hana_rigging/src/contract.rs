use std::any::Any;
use std::any::type_name;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::Weak;

use bevy::prelude::Component;
use bevy::prelude::Reflect;
use bevy::prelude::World;
use thiserror::Error;

use crate::ApplyPermit;
use crate::AttemptId;
use crate::AttemptProgress;
use crate::DeviceAccessError;
use crate::DeviceEndpoint;
use crate::DeviceScan;
use crate::LastKnownGoodConfiguration;

/// Reports the whole current set of hardware devices from an integration crate.
///
/// This trait is an external extension point: a monitor integration reports display records while
/// a camera integration reports camera records, and the kernel must not name either type. An
/// implementation prepares work on the main thread without receiving the application `World`,
/// then hands one owned enumeration job to the kernel. Kernel dispatch temporarily removes the
/// reporter registry while `discover` and a returned `MainThreadDiscoveryJob` run. The main-thread
/// job may read `!Send` integration state, but it must not mutate kernel-owned resources, device
/// entities, binding entities, or their kernel components; returning `DeviceScan` is the only
/// reporting path. A successful scan may retain a `ReportAcceptanceProjection` that the kernel
/// invokes only when it accepts that exact result. The projection may mutate only
/// integration-owned state and remains subject to the same prohibition on kernel authority.
///
/// An implementation panic is fatal, and the kernel does not catch it. Continuing after a
/// partially prepared discovery run would leave the reporter's device set and progress
/// untrustworthy.
pub trait DeviceReporter: Send + Sync + 'static {
    /// Prepare one complete discovery run when the kernel schedules this reporter.
    ///
    /// This method runs on the main thread, receives no `World`, and must return without blocking.
    /// A main-thread-only reporter returns a `MainThreadDiscoveryJob` that the kernel immediately
    /// invokes with `World`. A reporter that needs a slow probe captures sendable data it already
    /// owns and returns `DiscoveryWork::Background`.
    fn discover(&mut self) -> DiscoveryWork;
}

/// Work a `DeviceReporter` prepared after the kernel selected one discovery opportunity.
pub enum DiscoveryWork {
    /// Owned work the kernel immediately invokes with `World` on the main thread.
    Immediate(MainThreadDiscoveryJob),
    /// Owned enumeration that Bevy may run on `IoTaskPool` without access to `World`.
    Background(DiscoveryJob),
}

/// Owned device enumeration that the kernel immediately runs on the main thread.
///
/// This is the only discovery job that receives the application `World`. Its closure may read
/// `!Send` resources and must return an owned whole-set `DeviceScan` before scheduler admission
/// continues. The kernel never stores this job in reporter runtime state or submits it to
/// `IoTaskPool`.
pub struct MainThreadDiscoveryJob(Box<dyn FnOnce(&mut World) -> DeviceScan + 'static>);

impl MainThreadDiscoveryJob {
    /// Store one main-thread discovery closure for immediate synchronous execution.
    #[must_use]
    pub fn new(run: impl FnOnce(&mut World) -> DeviceScan + 'static) -> Self { Self(Box::new(run)) }

    pub(crate) fn run(self, world: &mut World) -> DeviceScan { self.0(world) }
}

/// Owned device enumeration that the kernel can move onto Bevy's I/O task pool.
///
/// The closure cannot receive a `World`, `Devices`, bindings, or a reporter identifier. It can
/// only send descriptive progress and return an owned `DeviceScan`, leaving kernel mutation,
/// accepted integration-state projection, and reporter revision assignment on the main thread.
pub struct DiscoveryJob(
    Mutex<Box<dyn FnOnce(DiscoveryProgressSender) -> DeviceScan + Send + 'static>>,
);

impl DiscoveryJob {
    /// Store one sendable discovery closure for a later `IoTaskPool` submission.
    #[must_use]
    pub fn new(run: impl FnOnce(DiscoveryProgressSender) -> DeviceScan + Send + 'static) -> Self {
        Self(Mutex::new(Box::new(run)))
    }

    pub(crate) fn run(self, discovery_progress_sender: DiscoveryProgressSender) -> DeviceScan {
        let run = self
            .0
            .into_inner()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        run(discovery_progress_sender)
    }
}

/// Send-only handle a background discovery job uses to describe its current work.
///
/// The sender deliberately carries no reporter identity. The scheduler associates each update
/// with the reporter and discovery batch that submitted the job. Unread updates coalesce in one
/// latest-value slot, so sending replaces any progress the scheduler has not collected yet.
#[derive(Clone)]
pub struct DiscoveryProgressSender(Weak<DiscoveryProgressMailbox>);

struct DiscoveryProgressMailbox {
    pending: Mutex<PendingDiscoveryProgress>,
}

pub(crate) struct DiscoveryProgressReceiver(Arc<DiscoveryProgressMailbox>);

pub(crate) enum PendingDiscoveryProgress {
    NoUpdate,
    Latest(DiscoveryProgress),
}

impl DiscoveryProgressSender {
    pub(crate) fn scheduler_mailbox() -> (Self, DiscoveryProgressReceiver) {
        let discovery_progress_mailbox = Arc::new(DiscoveryProgressMailbox {
            pending: Mutex::new(PendingDiscoveryProgress::NoUpdate),
        });
        (
            Self(Arc::downgrade(&discovery_progress_mailbox)),
            DiscoveryProgressReceiver(discovery_progress_mailbox),
        )
    }

    /// Replace any unread progress with the job's latest observed progress.
    ///
    /// Sending uses constant storage and briefly synchronizes on the single-slot mailbox lock
    /// while replacing unread progress. It neither queues earlier updates nor waits for the
    /// scheduler to collect them.
    ///
    /// # Errors
    ///
    /// Returns `DiscoveryProgressSendError::SchedulerStopped` after the kernel has stopped
    /// retaining this job, such as during application shutdown.
    pub fn send(
        &self,
        discovery_progress: DiscoveryProgress,
    ) -> Result<(), DiscoveryProgressSendError> {
        let discovery_progress_mailbox = self
            .0
            .upgrade()
            .ok_or(DiscoveryProgressSendError::SchedulerStopped)?;
        *discovery_progress_mailbox
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            PendingDiscoveryProgress::Latest(discovery_progress);

        Ok(())
    }
}

impl DiscoveryProgressReceiver {
    pub(crate) fn take_latest(&self) -> PendingDiscoveryProgress {
        let mut pending = self
            .0
            .pending
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        std::mem::replace(&mut *pending, PendingDiscoveryProgress::NoUpdate)
    }
}

/// Progress state a background discovery job can report without exposing a bare optional count.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum DiscoveryProgress {
    /// The job can identify the current operation but cannot count its remaining devices.
    Indeterminate,
    /// The job counted device operations, such as opening three of eight camera descriptors.
    Measured {
        /// Number of completed device operations from this discovery run.
        completed: u32,
        /// Total device operations known to this discovery run; it cannot be zero when measured.
        total:     NonZeroU32,
    },
}

/// Failure returned when a background job sends progress after scheduler retention ended.
#[derive(Debug, Error)]
pub enum DiscoveryProgressSendError {
    /// The scheduler released this job's progress receiver during application shutdown.
    #[error("discovery scheduler stopped receiving background progress")]
    SchedulerStopped,
}

/// Drives one endpoint from an integration crate after the kernel authorizes an apply.
///
/// This trait is an external extension point separate from `DeviceReporter`: a monitor reporter
/// only scans, while a window driver captures and applies window state for the same display. An
/// implementation may touch hardware through every method here. An
/// implementation must not insert, remove, or read the driver registry while these methods run,
/// and it must not mutate kernel-owned resources, device entities, binding entities, or their
/// kernel components through the supplied `World`. Kernel dispatch temporarily removes the
/// driver registry during each call, so accessing it would panic; the method result is the
/// only report back to the kernel.
///
/// An implementation panic is fatal, and the kernel does not catch it. Continuing after a
/// partially completed hardware operation would leave the endpoint and its attempt state
/// untrustworthy.
pub trait EndpointDriver: Send + Sync + 'static {
    /// The driver-specific configuration the kernel captures, erases, and later returns here.
    ///
    /// Driver authors must add `#[derive(Component, Reflect)]` and `#[reflect(Component)]` to the
    /// concrete configuration type. `Reflect` permits erased configuration retention, while
    /// `#[reflect(Component)]` creates the `ReflectComponent` metadata the kernel needs to mirror
    /// the concrete configuration on the binding entity. The trait bounds alone do not create that
    /// metadata.
    type Configuration: Reflect + Component;

    /// Read the live endpoint configuration for a later restore or diagnostic inspection.
    ///
    /// The kernel calls `capture` only for a present endpoint with no in-flight attempt. A driver
    /// does not repeat those kernel checks because the decision can change after this call.
    fn capture(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
    ) -> CaptureOutcome<Self::Configuration>;

    /// Start applying `configuration` to `endpoint` and return before the device operation ends.
    ///
    /// An apply is absolute: a window driver cannot rely on its current placement, and a camera
    /// driver cannot rely on its current stream settings. Drivers that need blocking hardware
    /// work start it on their own thread and report its result through later `poll` calls.
    fn start_apply(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
        configuration: &Self::Configuration,
        attempt: AttemptId,
        permit: ApplyPermit,
    );

    /// Report whether an apply has reached the driver's own arrival condition.
    ///
    /// The kernel owns the end-to-end deadline while the driver defines arrival: a delivered
    /// camera frame, stable window position, or completed panel image write. A driver returns
    /// `AttemptProgress::Pending` between its own steps and reports a substituted endpoint when
    /// its comparison finds a different physical target.
    fn poll(&mut self, world: &mut World, attempt: AttemptId) -> AttemptProgress;
}

/// Result of asking an endpoint driver to read its current configuration.
///
/// This generic return value never reflects, lands on an entity, or enters a resource. It keeps
/// the driver's configuration type until the erased driver entry receives it, which is why this
/// exception to the kernel's non-generic-type rule is safe.
pub enum CaptureOutcome<Configuration> {
    /// The endpoint exposed a configuration value that the kernel may retain.
    Read(Configuration),
    /// The endpoint is reachable but has no readable configuration, such as a display API that
    /// reports geometry without exposing the current window arrangement.
    ///
    /// This is permanent for the endpoint, so the kernel stops calling `EndpointDriver::capture`.
    NotReadable,
    /// A transient read failed while the endpoint remained reachable, such as a camera that is
    /// switching modes.
    ///
    /// The kernel retains its prior `LastKnownGoodConfiguration` and tries again during a later
    /// reconcile.
    ReadFailed(DeviceAccessError),
}

/// Failure at the erased driver boundary before a typed `EndpointDriver` method can run.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum DriverContractError {
    /// A binding addressed a `DriverId` that no registration call issued during this process.
    #[error("endpoint driver `{driver_id:?}` is not registered")]
    DriverNotRegistered {
        /// The process-local route that did not select a registered endpoint driver.
        driver_id: crate::DriverId,
    },
    /// A driver registry entry's erased value differs from the concrete driver type installed in
    /// its dispatch functions.
    ///
    /// Registration constructs the value and functions together, but erased dispatch keeps this
    /// internal contract failure recoverable so one invalid entry does not terminate the app.
    #[error("endpoint driver registry entry expected concrete driver `{expected_driver}`")]
    DriverTypeMismatch {
        /// The concrete `EndpointDriver` type required by the installed dispatch function.
        expected_driver: &'static str,
    },
    /// A binding or capture value reached a driver whose `Configuration` type differs from the
    /// concrete value selected by a state-issued driver request.
    ///
    /// This remains recoverable because two drivers can serve one device while accepting distinct
    /// configuration types; terminating the app would turn one authored routing error into loss
    /// of every endpoint.
    #[error(
        "endpoint driver expected configuration `{expected_configuration}` but received `{received_configuration}`"
    )]
    ConfigurationTypeMismatch {
        /// The concrete `EndpointDriver::Configuration` type the registered driver accepts.
        expected_configuration: &'static str,
        /// The reflected type path stored in the supplied requested or established value.
        received_configuration: String,
    },
    /// A state-issued restore request no longer has its checked readback value at dispatch time.
    ///
    /// Normal request ownership prevents this result: the request retains the binding borrow until
    /// dispatch completes. It remains recoverable so a future kernel caller cannot commit an
    /// applying state after a malformed internal request.
    #[error("state-issued apply request for role `{role}` has no last-known-good configuration")]
    LastKnownGoodConfigurationUnavailable {
        /// Binding role whose restore request no longer selected a safe readback value.
        role: crate::RoleKey,
    },
}

type ErasedDriver = dyn Any + Send + Sync;
type CaptureFunction =
    fn(
        &mut ErasedDriver,
        &mut World,
        &DeviceEndpoint,
    ) -> Result<CaptureOutcome<LastKnownGoodConfiguration>, DriverContractError>;
type StartApplyFunction = fn(
    &mut ErasedDriver,
    &mut World,
    &DeviceEndpoint,
    &dyn Reflect,
    AttemptId,
    ApplyPermit,
) -> Result<(), DriverContractError>;
type PollFunction =
    fn(&mut ErasedDriver, &mut World, AttemptId) -> Result<AttemptProgress, DriverContractError>;

/// Erased driver value and the typed functions the kernel routes by `DriverId`.
pub(crate) struct DriverEntry {
    driver:      Box<ErasedDriver>,
    capture:     CaptureFunction,
    start_apply: StartApplyFunction,
    poll:        PollFunction,
}

impl DriverEntry {
    pub(crate) fn new<Driver>(driver: Driver) -> Self
    where
        Driver: EndpointDriver,
    {
        Self {
            driver:      Box::new(driver),
            capture:     capture_driver::<Driver>,
            start_apply: start_apply_driver::<Driver>,
            poll:        poll_driver::<Driver>,
        }
    }

    pub(crate) fn capture(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
    ) -> Result<CaptureOutcome<LastKnownGoodConfiguration>, DriverContractError> {
        (self.capture)(self.driver.as_mut(), world, endpoint)
    }

    pub(crate) fn start_apply(
        &mut self,
        world: &mut World,
        endpoint: &DeviceEndpoint,
        configuration: &dyn Reflect,
        attempt: AttemptId,
        permit: ApplyPermit,
    ) -> Result<(), DriverContractError> {
        (self.start_apply)(
            self.driver.as_mut(),
            world,
            endpoint,
            configuration,
            attempt,
            permit,
        )
    }

    pub(crate) fn poll(
        &mut self,
        world: &mut World,
        attempt: AttemptId,
    ) -> Result<AttemptProgress, DriverContractError> {
        (self.poll)(self.driver.as_mut(), world, attempt)
    }
}

fn capture_driver<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    endpoint: &DeviceEndpoint,
) -> Result<CaptureOutcome<LastKnownGoodConfiguration>, DriverContractError>
where
    Driver: EndpointDriver,
{
    let driver = typed_driver_mut::<Driver>(driver)?;

    Ok(match driver.capture(world, endpoint) {
        CaptureOutcome::Read(configuration) => {
            CaptureOutcome::Read(LastKnownGoodConfiguration::known(configuration))
        },
        CaptureOutcome::NotReadable => CaptureOutcome::NotReadable,
        CaptureOutcome::ReadFailed(error) => CaptureOutcome::ReadFailed(error),
    })
}

fn start_apply_driver<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    endpoint: &DeviceEndpoint,
    configuration: &dyn Reflect,
    attempt: AttemptId,
    permit: ApplyPermit,
) -> Result<(), DriverContractError>
where
    Driver: EndpointDriver,
{
    let driver = typed_driver_mut::<Driver>(driver)?;
    let Some(configuration) = configuration
        .as_any()
        .downcast_ref::<Driver::Configuration>()
    else {
        return Err(DriverContractError::ConfigurationTypeMismatch {
            expected_configuration: type_name::<Driver::Configuration>(),
            received_configuration: configuration.reflect_type_path().to_owned(),
        });
    };

    driver.start_apply(world, endpoint, configuration, attempt, permit);

    Ok(())
}

fn poll_driver<Driver>(
    driver: &mut ErasedDriver,
    world: &mut World,
    attempt: AttemptId,
) -> Result<AttemptProgress, DriverContractError>
where
    Driver: EndpointDriver,
{
    Ok(typed_driver_mut::<Driver>(driver)?.poll(world, attempt))
}

fn typed_driver_mut<Driver>(driver: &mut ErasedDriver) -> Result<&mut Driver, DriverContractError>
where
    Driver: EndpointDriver,
{
    driver
        .downcast_mut::<Driver>()
        .ok_or_else(|| DriverContractError::DriverTypeMismatch {
            expected_driver: type_name::<Driver>(),
        })
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;
    use std::any::type_name;
    use std::error::Error;
    use std::num::NonZeroU32;
    use std::rc::Rc;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectComponent;
    use bevy::prelude::Component;
    use bevy::prelude::Reflect;
    use bevy::prelude::Resource;
    use bevy::prelude::World;

    use super::CaptureOutcome;
    use super::DiscoveryProgressSender;
    use super::DriverContractError;
    use super::DriverEntry;
    use super::EndpointDriver;
    use super::MainThreadDiscoveryJob;
    use super::PendingDiscoveryProgress;
    use crate::ApplyPermit;
    use crate::AttemptId;
    use crate::AttemptProgress;
    use crate::DeviceEndpoint;
    use crate::DeviceIdSource;
    use crate::DeviceKey;
    use crate::DeviceKind;
    use crate::DeviceScan;
    use crate::DiscoveryProgress;
    use crate::DiscoveryProgressSendError;
    use crate::EndpointId;
    use crate::LastKnownGoodConfiguration;
    use crate::ReportedId;
    use crate::SchemeName;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct TestConfiguration;

    #[derive(Component, Reflect)]
    #[reflect(Component)]
    struct OtherConfiguration;

    #[derive(Resource)]
    struct Running;

    struct MainThreadDiscoverySource {
        name: Rc<str>,
    }

    struct TestDriver;

    impl EndpointDriver for TestDriver {
        type Configuration = TestConfiguration;

        fn capture(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
        ) -> CaptureOutcome<Self::Configuration> {
            CaptureOutcome::NotReadable
        }

        fn start_apply(
            &mut self,
            _: &mut World,
            _: &DeviceEndpoint,
            _: &Self::Configuration,
            _: AttemptId,
            _: ApplyPermit,
        ) {
        }

        fn poll(&mut self, _: &mut World, _: AttemptId) -> AttemptProgress {
            AttemptProgress::Pending
        }
    }

    #[test]
    fn main_thread_discovery_job_reads_non_send_resource_and_returns_owned_whole_set() {
        let mut world = World::new();
        world.insert_non_send(MainThreadDiscoverySource {
            name: Rc::from("monitor-api"),
        });
        let main_thread_discovery_job = MainThreadDiscoveryJob::new(|world| {
            let main_thread_discovery_source = world.non_send::<MainThreadDiscoverySource>();
            assert_eq!(main_thread_discovery_source.name.as_ref(), "monitor-api");

            DeviceScan::Complete(Vec::new())
        });

        let device_scan = main_thread_discovery_job.run(&mut world);

        assert!(matches!(
            device_scan,
            DeviceScan::Complete(device_records) if device_records.is_empty()
        ));
    }

    #[test]
    fn progress_mailbox_coalesces_to_latest_measured_and_indeterminate_updates() {
        const UPDATE_COUNT: u32 = 1_000;

        let (discovery_progress_sender, discovery_progress_receiver) =
            DiscoveryProgressSender::scheduler_mailbox();
        let total = NonZeroU32::new(UPDATE_COUNT).unwrap_or(NonZeroU32::MIN);
        for completed in 0..UPDATE_COUNT {
            assert!(
                discovery_progress_sender
                    .send(DiscoveryProgress::Measured { completed, total })
                    .is_ok(),
                "scheduler must retain the progress receiver"
            );
        }

        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::Latest(DiscoveryProgress::Measured {
                completed,
                total,
            }) if completed == UPDATE_COUNT - 1 && total.get() == UPDATE_COUNT
        ));
        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::NoUpdate
        ));

        for completed in 0..UPDATE_COUNT {
            assert!(
                discovery_progress_sender
                    .send(DiscoveryProgress::Measured { completed, total })
                    .is_ok(),
                "scheduler must retain the progress receiver"
            );
        }
        assert!(
            discovery_progress_sender
                .send(DiscoveryProgress::Indeterminate)
                .is_ok(),
            "scheduler must retain the progress receiver"
        );
        assert!(matches!(
            discovery_progress_receiver.take_latest(),
            PendingDiscoveryProgress::Latest(DiscoveryProgress::Indeterminate)
        ));
    }

    #[test]
    fn progress_sender_reports_stopped_after_scheduler_releases_receiver() {
        let (discovery_progress_sender, discovery_progress_receiver) =
            DiscoveryProgressSender::scheduler_mailbox();
        drop(discovery_progress_receiver);

        assert!(matches!(
            discovery_progress_sender.send(DiscoveryProgress::Indeterminate),
            Err(DiscoveryProgressSendError::SchedulerStopped)
        ));
    }

    #[test]
    fn wrong_configuration_type_returns_contract_error_without_stopping_the_app()
    -> Result<(), Box<dyn Error>> {
        let mut app = App::new();
        let mut driver_entry = DriverEntry::new(TestDriver);
        let configuration = LastKnownGoodConfiguration::known(OtherConfiguration);
        let result = driver_entry.start_apply(
            app.world_mut(),
            &display_endpoint()?,
            configuration
                .as_reflect()
                .map_err(|_| "missing configuration")?,
            AttemptId::default(),
            ApplyPermit::restore_only(),
        );

        assert!(matches!(
            result,
            Err(DriverContractError::ConfigurationTypeMismatch { .. })
        ));
        app.insert_resource(Running);
        assert!(app.world().contains_resource::<Running>());

        Ok(())
    }

    #[test]
    fn wrong_driver_type_returns_contract_error_without_stopping_the_app() {
        let mut app = App::new();
        let mut driver_entry = DriverEntry::new(TestDriver);
        driver_entry.driver = Box::new(());

        let result = driver_entry.poll(app.world_mut(), AttemptId::default());

        assert!(matches!(
            result,
            Err(DriverContractError::DriverTypeMismatch { expected_driver })
                if expected_driver == type_name::<TestDriver>()
        ));
        app.insert_resource(Running);
        assert!(app.world().contains_resource::<Running>());
    }

    #[test]
    fn driver_configuration_registers_reflect_component_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();
        let type_id = TypeId::of::<TestConfiguration>();

        assert!(type_registry.contains(type_id));
        assert!(
            type_registry
                .get_type_data::<ReflectComponent>(type_id)
                .is_some()
        );

        drop(type_registry);
    }

    fn display_endpoint() -> Result<DeviceEndpoint, Box<dyn Error>> {
        Ok(DeviceEndpoint {
            device: DeviceKey {
                kind: DeviceKind::Display,
                id:   DeviceIdSource::Reported {
                    scheme: SchemeName::new("edid-serial")?,
                    value:  ReportedId::new("DELL-U2723QE-9J4K2H3")?,
                },
            },
            id:     EndpointId::Whole,
        })
    }
}
