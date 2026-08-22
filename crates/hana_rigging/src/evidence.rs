use bevy::prelude::Reflect;

use crate::ReportedId;

/// The operating system's process-local identifier for a unit, retained only while this process
/// is running.
///
/// `PlatformDeviceHandle` distinguishes a platform that has no identifier for a device category
/// from a platform API that normally provides one but returned no value for this unit. Neither case
/// is a durable identity or a substitute for `crate::DeviceKey`.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum PlatformDeviceHandle {
    /// The platform supplied an identifier such as a `CGDirectDisplayID` or an `IOKit` registry
    /// entry that can join reports during this process.
    Reported(ReportedId),
    /// This platform has no operating-system identifier for this device category, as with a
    /// network node reported through a provider API that exposes no local device handle.
    PlatformHasNoConcept,
    /// This platform normally exposes an operating-system identifier, but the scan returned no
    /// value for this unit, as when a transient device record lacks its `IOKit` registry entry.
    PlatformReportedNothing,
}

/// Where a provider observes a unit attached, such as a USB port path, display slot, or network
/// node address.
///
/// `AttachmentPath` is reconciliation evidence for detecting a displaced unit. Its absence is an
/// enum because a device category without attachment paths differs from a scan that failed to
/// report a path on a platform that supports them.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum AttachmentPath {
    /// The provider observed a path such as `usb/1-3.2`, `display/2`, or `node/rack-a/7`.
    Reported(ReportedId),
    /// This platform does not model an attachment location for this device category, as with a
    /// virtual camera that has no host port or display slot.
    PlatformHasNoConcept,
    /// The platform can report attachment locations but supplied none for this unit during the
    /// completed scan, as when a USB device disconnects while its record is being read.
    PlatformReportedNothing,
}

/// Vendor, product, and model text that a provider retains for synthesized identity and
/// reconciliation diagnostics.
///
/// The descriptor alone never identifies a unit: identical webcams often publish the same three
/// strings. `DeviceDescriptor` retains whether the platform cannot express descriptors at all or
/// merely returned none for this scan, preventing those distinct observations from becoming an
/// empty string.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum DeviceDescriptor {
    /// The provider received all descriptor values from the unit or its platform API.
    Reported {
        /// Vendor text, such as the manufacturer name in a USB or EDID descriptor.
        vendor:  ReportedId,
        /// Product text, such as a model-family name supplied by the device descriptor.
        product: ReportedId,
        /// Model text, such as a panel or camera model string used as digest input.
        model:   ReportedId,
    },
    /// This platform has no descriptor model for this device category, as with a provider whose
    /// remote inventory contains only a device key.
    PlatformHasNoConcept,
    /// The platform supports descriptors but returned no vendor, product, or model data for this
    /// unit, as when a remote node sent an incomplete inventory record.
    PlatformReportedNothing,
}

#[cfg(test)]
mod tests {
    use super::AttachmentPath;
    use super::DeviceDescriptor;
    use super::PlatformDeviceHandle;

    #[test]
    fn evidence_types_distinguish_unsupported_concepts_from_missing_reports() {
        assert_ne!(
            PlatformDeviceHandle::PlatformHasNoConcept,
            PlatformDeviceHandle::PlatformReportedNothing,
        );

        assert_ne!(
            AttachmentPath::PlatformHasNoConcept,
            AttachmentPath::PlatformReportedNothing,
        );

        assert_ne!(
            DeviceDescriptor::PlatformHasNoConcept,
            DeviceDescriptor::PlatformReportedNothing,
        );
    }
}
