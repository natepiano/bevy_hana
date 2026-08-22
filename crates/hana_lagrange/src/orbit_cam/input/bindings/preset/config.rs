use crate::orbit_cam::input::bindings::BindingsError;
use crate::orbit_cam::input::bindings::OrbitCamBindings;

pub(super) trait OrbitCamPresetConfig: Sized {
    fn build(self) -> Result<OrbitCamBindings, BindingsError>;
}
