use bevy::prelude::App;
use bevy::prelude::On;
use hana_rigging::prelude::ContributorView;
use hana_rigging::prelude::EndedRegistrationLifetime;
use hana_rigging::prelude::KeyAvailability;
use hana_rigging::prelude::NonEmptyContributors;
use hana_rigging::prelude::PresenceView;
use hana_rigging::prelude::PresentEvidence;
use hana_rigging::prelude::RegistrationAttemptEnded;
use hana_rigging::prelude::RoleStatusAfterChange;
use hana_rigging::prelude::RoleStatusBeforeChange;

fn observe_registration_attempt_ending(event: On<RegistrationAttemptEnded>) {
    match event.lifetime {
        EndedRegistrationLifetime::Displaced => {},
        EndedRegistrationLifetime::Retired => {},
        EndedRegistrationLifetime::RetirementBlocked => {},
    }
}

fn read_role_status_change(before: &RoleStatusBeforeChange, after: &RoleStatusAfterChange) {
    std::hint::black_box((before.view(), after.view()));
}

fn read_present_evidence(present_evidence: &PresentEvidence) {
    let contributors: &NonEmptyContributors = present_evidence.contributors();
    let [contributor, ..] = contributors.as_slice() else {
        return;
    };
    let contributor: &ContributorView = contributor;
    let presence: PresenceView = contributor.presence;
    std::hint::black_box(presence);
}

fn read_key_availability(key_availability: KeyAvailability) {
    if let KeyAvailability::Present(present_evidence) = key_availability {
        read_present_evidence(&present_evidence);
    }
}

fn main() {
    let mut app = App::new();
    app.add_observer(observe_registration_attempt_ending);
    std::hint::black_box(
        read_role_status_change as fn(&RoleStatusBeforeChange, &RoleStatusAfterChange),
    );
    std::hint::black_box(read_key_availability as fn(KeyAvailability));
}
