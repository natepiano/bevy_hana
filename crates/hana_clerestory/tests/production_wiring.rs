//! Verifies that display injection requires explicit [`DisplayTestAdapter`] installation.
#![cfg(feature = "test")]

use bevy::prelude::App;
use hana_clerestory::DisplayEnumerationSource;
use hana_clerestory::DisplayTestAdapter;
use hana_clerestory::DisplayTestAdapterInstallation;
use hana_clerestory::WindowManagerPlugin;

#[test]
fn production_wiring_does_not_activate_display_injection() {
    let mut production_app = App::new();
    production_app.add_plugins(WindowManagerPlugin);

    assert_eq!(
        *production_app
            .world()
            .resource::<DisplayEnumerationSource>(),
        DisplayEnumerationSource::LiveWinit
    );
    assert_eq!(
        DisplayTestAdapter::installation(production_app.world()),
        DisplayTestAdapterInstallation::NotInstalled
    );

    let mut test_harness_app = App::new();
    test_harness_app.add_plugins((DisplayTestAdapter::new(), WindowManagerPlugin));

    assert_eq!(
        *test_harness_app
            .world()
            .resource::<DisplayEnumerationSource>(),
        DisplayEnumerationSource::ExplicitTestAdapter
    );
    assert_eq!(
        DisplayTestAdapter::installation(test_harness_app.world()),
        DisplayTestAdapterInstallation::ExplicitlyInstalled
    );
}
