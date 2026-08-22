/// Applies the canonical permanent-control mapping through Fairy Dust's shortcut builder.
///
/// Both the public example and the crate-owned headless shortcut tests expand this mapping, so
/// the tested `1`–`4` registrations remain the ones installed by the example.
macro_rules! with_context_state_shortcuts {
    ($builder:expr; $main_menu:path, $running:path, $resting:path, $dimension_lock:path) => {{
        $builder
            .with_shortcut(KeyCode::Digit1, $main_menu)
            .with_shortcut(KeyCode::Digit2, $running)
            .with_shortcut(KeyCode::Digit3, $resting)
            .with_shortcut(KeyCode::Digit4, $dimension_lock)
    }};
}
