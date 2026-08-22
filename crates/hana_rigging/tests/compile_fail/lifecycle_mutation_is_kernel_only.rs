use hana_rigging::ReadyRole;

fn cannot_record_a_driver_outcome(mut ready_role: ReadyRole<'_>) {
    ready_role.record_capture(unsafe { std::mem::MaybeUninit::uninit().assume_init() });
}

fn cannot_start_an_apply_from_the_ready_role(ready_role: ReadyRole<'_>) {
    let _ = ready_role.start_requested_apply(
        unsafe { std::mem::MaybeUninit::uninit().assume_init() },
        unsafe { std::mem::MaybeUninit::uninit().assume_init() },
        unsafe { std::mem::MaybeUninit::uninit().assume_init() },
    );
}

fn main() {}
