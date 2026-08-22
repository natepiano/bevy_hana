/// Generates a `bevy_enhanced_input` `InputAction` struct.
///
/// # Examples
///
/// ```ignore
/// use hana_rubric::action;
///
/// action!(CameraHome);
/// ```
///
/// Expands to:
///
/// ```ignore
/// #[derive(InputAction)]
/// #[action_output(bool)]
/// pub struct CameraHome;
/// ```
#[macro_export]
#[doc(hidden)]
macro_rules! action {
    ($(#[$meta:meta])* $action:ident) => {
        $(#[$meta])*
        #[derive(InputAction)]
        #[action_output(bool)]
        pub struct $action;
    };
}
