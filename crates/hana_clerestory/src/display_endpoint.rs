//! Provider-owned binding addresses for the independent uses of one physical display.

use hana_rigging::prelude::EndpointId;
use hana_rigging::prelude::PartName;
use hana_rigging::prelude::PartNameError;

/// Independent physical roles exposed by a display provider.
///
/// Capture readers and the exclusive output writer address different parts of
/// the same durable display. All output tools use the same output part, so
/// rigging continues to enforce a single writer for each display.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DisplayEndpointPart {
    /// Read the display's current pixels through the screen capture driver.
    Capture,
    /// Drive the display through its exclusive physical output binding.
    Output,
}

impl DisplayEndpointPart {
    /// Returns the provider-defined rigging address for this role.
    ///
    /// # Errors
    ///
    /// Propagates rigging's name validation if a provider spelling is invalid.
    pub fn endpoint_id(self) -> Result<EndpointId, PartNameError> {
        let name = match self {
            Self::Capture => "screen-capture",
            Self::Output => "display-output",
        };
        PartName::new(name).map(EndpointId::Part)
    }
}
