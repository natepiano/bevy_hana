//! Capability: `bevy_brp_extras::BrpExtrasPlugin` configured to display the
//! BRP port in the window title when running on a non-default port.

use bevy::prelude::App;
use bevy_brp_extras::BrpExtrasPlugin;
use bevy_brp_extras::PortDisplay;

use crate::ensure_plugin;

pub(crate) fn install(app: &mut App) {
    ensure_plugin(
        app,
        BrpExtrasPlugin::default().port_in_title(PortDisplay::NonDefault),
    );
}
