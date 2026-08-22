use bevy::ecs::reflect::ReflectComponent;
use bevy::prelude::Component;
use bevy::prelude::Reflect;

/// Exclusive-ownership state reported by a provider alongside `crate::Presence`.
///
/// `Claim` records a platform fact, not the policy that Hana should use. A camera can be present
/// while another process owns its capture stream, so reconciliation keeps `Claim` independent of
/// `Presence` before deciding whether output is permitted.
#[derive(Clone, PartialEq, Eq, Debug, Component, Reflect)]
#[reflect(Component, PartialEq)]
pub enum Claim {
    /// The provider's substrate has no exclusive-ownership concept, as with a display panel that
    /// applications may address concurrently.
    NotApplicable,
    /// This process holds the platform claim, such as an opened camera capture device.
    Held,
    /// The device accepts a claim and no process currently holds it, such as an idle audio input.
    Free,
    /// Another process owns the device or owns it in a way that prevents useful use by this
    /// process, such as a camera already open in another application.
    Contended {
        /// Identity information supplied by the operating system for the competing process.
        holder: ClaimHolder,
    },
    /// The platform rejected access until a permission class is granted, as when camera access is
    /// disabled in system privacy settings.
    Blocked {
        /// Permission class that explains which user action can permit a future claim.
        gate: PermissionGate,
    },
}

/// Information the platform provides about a process that contends for a device.
///
/// A missing name is not represented with `Option<String>` because macOS can report camera
/// contention while withholding every process identity; that diagnosis differs from an empty
/// process-name string.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum ClaimHolder {
    /// The platform identified the competing process, such as a capture application named by an
    /// operating-system camera service.
    Named(String),
    /// The platform reports a competing owner but does not identify it, as macOS does for some
    /// camera contention reports.
    Unidentified,
}

/// Permission class that prevented a provider from claiming an otherwise visible device.
///
/// Providers report a `PermissionGate` instead of opaque text because remediation can depend on
/// the class: opening camera privacy settings differs from enabling screen recording.
#[derive(Clone, PartialEq, Eq, Debug, Reflect)]
pub enum PermissionGate {
    /// The operating system denied camera capture permission for a visible camera.
    CameraAccess,
    /// The operating system denied permission to capture displays or windows.
    ScreenRecording,
    /// The operating system denied permission to monitor keyboard or pointer input.
    InputMonitoring,
    /// A provider encountered a permission class outside the shared vocabulary and retained its
    /// platform-specific description for diagnostics.
    Other {
        /// Platform-specific text that identifies the permission class to a provider and its UI.
        detail: String,
    },
}
