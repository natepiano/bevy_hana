use std::time::Duration;

use bevy::ecs::reflect::ReflectResource;
use bevy::prelude::Reflect;
use bevy::prelude::Resource;

const DEFAULT_APPLY_DEADLINE: Duration = Duration::from_secs(10);
const DEFAULT_APPLY_OVERRUN: Duration = Duration::from_secs(5);
const DEFAULT_DEPARTURE_GRACE: Duration = Duration::from_secs(5);
const DEFAULT_REPORT_GRACE: Duration = Duration::from_secs(30);

/// Timing allowances the kernel applies to endpoint attempts and to reporter freshness.
///
/// These are settable fields rather than crate constants because a slow projector or a remote
/// camera bridge is diagnosed by widening one allowance at runtime, and reflection makes both
/// readable and writable over the Bevy Remote Protocol without a rebuild. Insert the resource
/// before [`RiggingPlugin`](crate::RiggingPlugin) to replace the defaults; the plugin leaves an
/// existing value alone.
#[derive(Clone, Debug, PartialEq, Eq, Resource, Reflect)]
#[reflect(Resource)]
pub struct RiggingLimits {
    /// How long one attempt has to reach its target, measured end to end rather than per step.
    ///
    /// End to end because a driver that must reset a panel and then render to it has both steps
    /// inside one attempt with one settle result, and a per-step budget would let it report done
    /// between them.
    ///
    /// Defaults to 10 seconds, which covers a display mode change and a camera stream open
    /// without letting an unresponsive device hold a role long enough for an operator to notice.
    pub apply_deadline:  Duration,
    /// How far past an attempt's own deadline the kernel lets it keep running before abandoning
    /// it.
    ///
    /// Bounded overrun rather than a hard cut: a projector or a camera that is still converging
    /// gets extra time, while an attempt that never finishes must not run forever.
    ///
    /// Defaults to 5 seconds — small next to any realistic hardware deadline, large enough to
    /// cover one slow poll.
    pub apply_overrun:   Duration,
    /// How long confirmed absence remains retained before the device is retired.
    ///
    /// Authorization ends when this interval begins. A matching present report cancels retirement;
    /// continued confirmed absence retires the device when the deadline arrives.
    pub departure_grace: Duration,
    /// How far past its **own declared cadence** a reporter may fall before the kernel stops
    /// treating its last report as current.
    ///
    /// Past `cadence + report_grace`, that reporter's devices become
    /// [`Presence::Unreachable`](crate::Presence::Unreachable) and its in-flight attempts are
    /// abandoned. This is grace on top of the declared interval, never an absolute age: cameras
    /// re-enumerate every two seconds and HID every ten, while a display reporter scans only when
    /// the display configuration changes and can stay silent for hours without anything being
    /// wrong. A fixed maximum age would mark a healthy event-driven reporter's monitors
    /// unreachable. A reporter that declared no cadence at all is never stale by the clock; it
    /// reports trouble by failing a scan.
    ///
    /// Defaults to 30 seconds, which puts a two-second camera reporter's lease at 32 seconds and
    /// a ten-second HID reporter's at 40, so one slow cycle does not trip it.
    pub report_grace:    Duration,
}

impl Default for RiggingLimits {
    fn default() -> Self {
        Self {
            apply_deadline:  DEFAULT_APPLY_DEADLINE,
            apply_overrun:   DEFAULT_APPLY_OVERRUN,
            departure_grace: DEFAULT_DEPARTURE_GRACE,
            report_grace:    DEFAULT_REPORT_GRACE,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::any::TypeId;

    use bevy::app::App;
    use bevy::ecs::reflect::AppTypeRegistry;
    use bevy::ecs::reflect::ReflectResource;

    use super::DEFAULT_APPLY_DEADLINE;
    use super::DEFAULT_APPLY_OVERRUN;
    use super::DEFAULT_DEPARTURE_GRACE;
    use super::DEFAULT_REPORT_GRACE;
    use super::RiggingLimits;

    #[test]
    fn default_limits_use_the_documented_overrun_and_grace() {
        let rigging_limits = RiggingLimits::default();

        assert_eq!(rigging_limits.apply_deadline, DEFAULT_APPLY_DEADLINE);
        assert_eq!(rigging_limits.apply_overrun, DEFAULT_APPLY_OVERRUN);
        assert_eq!(rigging_limits.departure_grace, DEFAULT_DEPARTURE_GRACE);
        assert_eq!(rigging_limits.report_grace, DEFAULT_REPORT_GRACE);
    }

    #[test]
    fn rigging_limits_registers_resource_reflection_metadata() {
        let app = App::new();
        let type_registry = app.world().resource::<AppTypeRegistry>().read();

        assert!(
            type_registry
                .get_type_data::<ReflectResource>(TypeId::of::<RiggingLimits>())
                .is_some()
        );

        drop(type_registry);
    }
}
