use std::cell::RefCell;
use std::marker::PhantomData;

use bevy::prelude::Bundle;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::KeyCode;
use bevy::prelude::default;
use bevy_enhanced_input::prelude::Action;
use bevy_enhanced_input::prelude::ActionSettings;
use bevy_enhanced_input::prelude::ActionSpawner;
use bevy_enhanced_input::prelude::Binding;
use bevy_enhanced_input::prelude::BlockBy;
use bevy_enhanced_input::prelude::Chord;
use bevy_enhanced_input::prelude::InputAction;
use bevy_enhanced_input::prelude::InputModKeys;
use bevy_enhanced_input::prelude::ModKeys;
use bevy_enhanced_input::prelude::Press;
use bevy_enhanced_input::prelude::bindings;

use crate::platform_shortcut_mode::PlatformShortcutMode;

struct ModifierActions {
    families: [(ModKeys, ModifierFamilyActions); 4],
}

impl ModifierActions {
    fn required(&self, mod_keys: ModKeys) -> Vec<Entity> {
        self.families
            .iter()
            .filter_map(|(modifier, actions)| {
                mod_keys.contains(*modifier).then_some(actions.required)
            })
            .collect()
    }

    fn blockers(&self, binding: Binding) -> Vec<Entity> {
        let declared_mod_keys = binding.mod_keys();
        let primary_key = primary_key(binding);
        let mut blockers = Vec::new();

        for (modifier, actions) in &self.families {
            if !declared_mod_keys.contains(*modifier) {
                actions.extend_blockers(primary_key, &mut blockers);
            }
        }

        blockers
    }

    fn matching(&self, mod_keys: ModKeys) -> Vec<Entity> { self.required(mod_keys) }
}

struct ModifierFamilyActions {
    required: Entity,
    left:     ModifierSideAction,
    right:    ModifierSideAction,
}

impl ModifierFamilyActions {
    fn spawn<C: Component, A: InputAction>(
        action_spawner: &mut ActionSpawner<C>,
        action_settings: ActionSettings,
        left_key: KeyCode,
        right_key: KeyCode,
    ) -> Self {
        let required = action_spawner
            .spawn((
                Action::<A>::new(),
                action_settings,
                bindings![left_key, right_key],
            ))
            .id();
        Self {
            required,
            left: ModifierSideAction::spawn(action_spawner, action_settings, left_key),
            right: ModifierSideAction::spawn(action_spawner, action_settings, right_key),
        }
    }

    fn extend_blockers(&self, primary_key: Option<KeyCode>, blockers: &mut Vec<Entity>) {
        match primary_key {
            Some(key_code) if key_code == self.left.key_code => blockers.push(self.right.entity),
            Some(key_code) if key_code == self.right.key_code => blockers.push(self.left.entity),
            Some(_) | None => blockers.extend([self.left.entity, self.right.entity]),
        }
    }
}

struct ModifierSideAction {
    key_code: KeyCode,
    entity:   Entity,
}

impl ModifierSideAction {
    fn spawn<C: Component>(
        action_spawner: &mut ActionSpawner<C>,
        action_settings: ActionSettings,
        key_code: KeyCode,
    ) -> Self {
        let entity = action_spawner
            .spawn((
                Action::<ModifierSide>::new(),
                action_settings,
                bindings![key_code],
            ))
            .id();
        Self { key_code, entity }
    }
}

/// Modifier-aware keybinding builder with platform-specific `Cmd`/`Ctrl` handling.
///
/// [`Self::spawn_key`], [`Self::spawn_shift_key`],
/// [`Self::spawn_platform_key`], and [`Self::spawn_binding`] install ordinary
/// Enhanced Input actions that can fire continuously while held.
/// [`Self::spawn_shortcut`] is the explicit one-shot path for authored runtime
/// bindings that need exact modifier matching.
///
/// # Type parameters
///
/// - `C: Component` is the input context component for the action spawner
///
/// # Examples
///
/// ```ignore
/// use hana_rubric::Keybindings;
///
/// fn setup_bindings(action_spawner: &mut ActionSpawner<MyContext>) {
///     let keybindings = Keybindings::new::<ShiftAction>(action_spawner, ActionSettings::default());
///     keybindings.spawn_key::<JumpAction>(action_spawner, KeyCode::Space);
///     keybindings.spawn_platform_key::<SaveAction>(action_spawner, KeyCode::KeyS);
///     keybindings.spawn_shift_key::<RunAction>(action_spawner, KeyCode::KeyR);
/// }
/// ```
pub struct Keybindings<C: Component> {
    modifier_actions: ModifierActions,
    physical_edges:   RefCell<Vec<(Binding, Entity)>>,
    action_settings:  ActionSettings,
    phantom_data:     PhantomData<C>,
}

impl<C: Component> Keybindings<C> {
    /// Spawns non-consuming modifier actions and returns a keybinding builder.
    ///
    /// `S` is the caller's action type for the Shift family: the entities spawned for it are the
    /// Shift blockers every keybinding spawned from this builder is given a [`BlockBy`] against,
    /// so an unshifted binding stays quiet while Shift is down. Modifier actions report
    /// physically held keys immediately when their context becomes active.
    pub fn new<S: InputAction>(
        action_spawner: &mut ActionSpawner<C>,
        action_settings: ActionSettings,
    ) -> Self {
        let modifier_settings = modifier_action_settings();
        let shift = ModifierFamilyActions::spawn::<C, S>(
            action_spawner,
            modifier_settings,
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
        );
        let control = ModifierFamilyActions::spawn::<C, ControlModifier>(
            action_spawner,
            modifier_settings,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
        );
        let alt = ModifierFamilyActions::spawn::<C, AltModifier>(
            action_spawner,
            modifier_settings,
            KeyCode::AltLeft,
            KeyCode::AltRight,
        );
        let super_ = ModifierFamilyActions::spawn::<C, SuperModifier>(
            action_spawner,
            modifier_settings,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        );

        Self {
            modifier_actions: ModifierActions {
                families: [
                    (ModKeys::CONTROL, control),
                    (ModKeys::SHIFT, shift),
                    (ModKeys::ALT, alt),
                    (ModKeys::SUPER, super_),
                ],
            },
            physical_edges: RefCell::default(),
            action_settings,
            phantom_data: PhantomData,
        }
    }

    /// Spawns a one-shot semantic action for an authored runtime binding.
    ///
    /// Keyboard-capable bindings match their exact declared modifier set.
    /// When a modifier key is the primary input, only that physical side is
    /// exempt from blocking; the opposite side remains an extra modifier.
    /// Gamepad, custom, and other bindings that cannot carry keyboard modifiers
    /// remain independent of keyboard modifier state.
    ///
    /// Returns the semantic action entity so callers can attach installation or
    /// debugging components that retain the original authored [`Binding`].
    pub fn spawn_shortcut<A: InputAction>(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        binding: Binding,
    ) -> Entity {
        let physical_binding = modifier_free_binding(binding);
        let physical_edge = self.physical_edge(action_spawner, physical_binding);
        let mut chord_actions = vec![physical_edge];
        let blockers = if supports_keyboard_modifiers(binding) {
            chord_actions.extend(self.modifier_actions.required(binding.mod_keys()));
            self.modifier_actions.blockers(binding)
        } else {
            Vec::new()
        };

        let mut semantic_action = action_spawner.spawn((
            Action::<A>::new(),
            self.action_settings,
            Chord::new(chord_actions).with_ongoing(false),
            bindings![physical_binding],
        ));
        if !blockers.is_empty() {
            semantic_action.insert(BlockBy::new(blockers));
        }
        semantic_action.id()
    }

    /// Spawns an ordinary action bound to a single key and blocked by modifiers.
    ///
    /// Observe [`Start`](bevy_enhanced_input::prelude::Start) for a discrete
    /// response or [`Fire`](bevy_enhanced_input::prelude::Fire) for a continuous
    /// response while the key remains held.
    pub fn spawn_key<A: InputAction>(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        key_code: KeyCode,
    ) {
        action_spawner.spawn((
            Action::<A>::new(),
            self.action_settings,
            BlockBy::new(self.modifier_actions.matching(held_modifier_keys())),
            bindings![key_code],
        ));
    }

    /// Spawns an ordinary action bound to `Shift + key`.
    ///
    /// Observe [`Start`](bevy_enhanced_input::prelude::Start) for a discrete
    /// response or [`Fire`](bevy_enhanced_input::prelude::Fire) for a continuous
    /// response while the shortcut remains held.
    pub fn spawn_shift_key<A: InputAction>(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        key_code: KeyCode,
    ) {
        action_spawner.spawn((
            Action::<A>::new(),
            self.action_settings,
            BlockBy::new(
                self.modifier_actions
                    .matching(held_modifier_keys() - ModKeys::SHIFT),
            ),
            bindings![key_code.with_mod_keys(ModKeys::SHIFT)],
        ));
    }

    /// Spawns an ordinary action with an opaque binding bundle.
    ///
    /// Observe [`Start`](bevy_enhanced_input::prelude::Start) for a discrete
    /// response or [`Fire`](bevy_enhanced_input::prelude::Fire) for a continuous
    /// response while a binding remains actuated. Use [`Self::spawn_shortcut`]
    /// for exact-modifier one-shot behavior.
    pub fn spawn_binding<A: InputAction, B: Bundle>(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        bindings: B,
    ) {
        action_spawner.spawn((
            Action::<A>::new(),
            self.action_settings,
            BlockBy::new(self.modifier_actions.matching(held_modifier_keys())),
            bindings,
        ));
    }

    /// Spawns an ordinary action with the platform `Cmd`/`Ctrl` modifier.
    ///
    /// Observe [`Start`](bevy_enhanced_input::prelude::Start) for a discrete
    /// response or [`Fire`](bevy_enhanced_input::prelude::Fire) for a continuous
    /// response while the shortcut remains held.
    pub fn spawn_platform_key<A: InputAction>(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        key_code: KeyCode,
    ) {
        let mod_keys = match PlatformShortcutMode::current() {
            PlatformShortcutMode::Command => ModKeys::SUPER,
            PlatformShortcutMode::Control => ModKeys::CONTROL,
        };
        action_spawner.spawn((
            Action::<A>::new(),
            self.action_settings,
            bindings![key_code.with_mod_keys(mod_keys)],
        ));
    }

    fn physical_edge(
        &self,
        action_spawner: &mut ActionSpawner<C>,
        physical_binding: Binding,
    ) -> Entity {
        if let Some((_, entity)) = self
            .physical_edges
            .borrow()
            .iter()
            .find(|(binding, _)| *binding == physical_binding)
        {
            return *entity;
        }

        let entity = action_spawner
            .spawn((
                Action::<PhysicalShortcutEdge>::new(),
                physical_edge_action_settings(),
                Press::default(),
                bindings![physical_binding],
            ))
            .id();
        self.physical_edges
            .borrow_mut()
            .push((physical_binding, entity));
        entity
    }
}

fn modifier_action_settings() -> ActionSettings {
    ActionSettings {
        consume_input: false,
        require_reset: false,
        ..default()
    }
}

fn physical_edge_action_settings() -> ActionSettings {
    ActionSettings {
        consume_input: false,
        require_reset: true,
        ..default()
    }
}

fn held_modifier_keys() -> ModKeys {
    match PlatformShortcutMode::current() {
        PlatformShortcutMode::Command => ModKeys::all(),
        PlatformShortcutMode::Control => ModKeys::CONTROL | ModKeys::SHIFT | ModKeys::ALT,
    }
}

const fn primary_key(binding: Binding) -> Option<KeyCode> {
    let Binding::Keyboard { key, .. } = binding else {
        return None;
    };
    Some(key)
}

fn modifier_free_binding(binding: Binding) -> Binding {
    if supports_keyboard_modifiers(binding) {
        binding.without_mod_keys()
    } else {
        binding
    }
}

const fn supports_keyboard_modifiers(binding: Binding) -> bool {
    matches!(
        binding,
        Binding::Keyboard { .. }
            | Binding::MouseButton { .. }
            | Binding::MouseMotion { .. }
            | Binding::MouseWheel { .. }
    )
}

#[derive(InputAction)]
#[action_output(bool)]
struct PhysicalShortcutEdge;

#[derive(InputAction)]
#[action_output(bool)]
struct ModifierSide;

#[derive(InputAction)]
#[action_output(bool)]
struct ControlModifier;

#[derive(InputAction)]
#[action_output(bool)]
struct AltModifier;

#[derive(InputAction)]
#[action_output(bool)]
struct SuperModifier;

#[cfg(test)]
mod tests {
    use bevy::input::ButtonState;
    use bevy::input::InputPlugin;
    use bevy::input::gamepad::RawGamepadButtonChangedEvent;
    use bevy::input::gamepad::RawGamepadEvent;
    use bevy::input::keyboard::Key;
    use bevy::input::keyboard::KeyboardInput;
    use bevy::input::keyboard::NativeKey;
    use bevy::prelude::App;
    use bevy::prelude::Entity;
    use bevy::prelude::Gamepad;
    use bevy::prelude::GamepadButton;
    use bevy::prelude::KeyCode;
    use bevy::prelude::MinimalPlugins;
    use bevy::prelude::On;
    use bevy::prelude::ResMut;
    use bevy::prelude::Resource;
    use bevy::prelude::SpawnRelated;
    use bevy::prelude::SpawnWith;
    use bevy::prelude::With;
    use bevy::prelude::default;
    use bevy_enhanced_input::prelude::Action;
    use bevy_enhanced_input::prelude::ActionSettings;
    use bevy_enhanced_input::prelude::ActionSpawner;
    use bevy_enhanced_input::prelude::Actions;
    use bevy_enhanced_input::prelude::ContextPriority;
    use bevy_enhanced_input::prelude::EnhancedInputPlugin;
    use bevy_enhanced_input::prelude::Fire;
    use bevy_enhanced_input::prelude::InputContextAppExt;
    use bevy_enhanced_input::prelude::InputModKeys;
    use bevy_enhanced_input::prelude::ModKeys;
    use bevy_enhanced_input::prelude::Start;
    use bevy_enhanced_input::prelude::bindings;

    use super::*;

    #[derive(Component)]
    struct TestContext;

    #[derive(Component)]
    struct HigherContext;

    #[derive(Component)]
    struct LowerContext;

    #[derive(Default, Resource)]
    struct RecordedStarts(Vec<RecordedAction>);

    #[derive(Default, Resource)]
    struct RecordedHeldFires(usize);

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum RecordedAction {
        Bare,
        Modified,
        Control,
        ControlShift,
        Gamepad,
        Higher,
        Lower,
    }

    #[derive(InputAction)]
    #[action_output(bool)]
    struct TestShift;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct BareAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct ModifiedAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct ControlAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct ControlShiftAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct GamepadAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct HigherAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct LowerAction;

    #[derive(InputAction)]
    #[action_output(bool)]
    struct HeldAction;

    fn record_bare(_start: On<Start<BareAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Bare);
    }

    fn record_modified(_start: On<Start<ModifiedAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Modified);
    }

    fn record_control(_start: On<Start<ControlAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Control);
    }

    fn record_control_shift(
        _start: On<Start<ControlShiftAction>>,
        mut recorded: ResMut<RecordedStarts>,
    ) {
        recorded.0.push(RecordedAction::ControlShift);
    }

    fn record_gamepad(_start: On<Start<GamepadAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Gamepad);
    }

    fn record_higher(_start: On<Start<HigherAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Higher);
    }

    fn record_lower(_start: On<Start<LowerAction>>, mut recorded: ResMut<RecordedStarts>) {
        recorded.0.push(RecordedAction::Lower);
    }

    fn record_held_fire(_fire: On<Fire<HeldAction>>, mut recorded: ResMut<RecordedHeldFires>) {
        recorded.0 += 1;
    }

    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, EnhancedInputPlugin))
            .add_input_context::<TestContext>()
            .init_resource::<RecordedStarts>()
            .init_resource::<RecordedHeldFires>()
            .add_observer(record_bare)
            .add_observer(record_modified)
            .add_observer(record_control)
            .add_observer(record_control_shift)
            .add_observer(record_gamepad)
            .add_observer(record_held_fire);
        app.finish();
        app
    }

    fn consuming_settings() -> ActionSettings {
        ActionSettings {
            consume_input: true,
            ..default()
        }
    }

    fn press_key(app: &mut App, window: Entity, key_code: KeyCode) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window,
        });
    }

    fn release_key(app: &mut App, window: Entity, key_code: KeyCode) {
        app.world_mut().write_message(KeyboardInput {
            key_code,
            logical_key: Key::Unidentified(NativeKey::Unidentified),
            state: ButtonState::Released,
            text: None,
            repeat: false,
            window,
        });
    }

    fn clear_recorded(app: &mut App) { app.world_mut().resource_mut::<RecordedStarts>().0.clear(); }

    #[test]
    fn runtime_shortcuts_match_exact_modifiers_without_release_handoff() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                keybindings.spawn_shortcut::<BareAction>(spawner, KeyCode::Enter.into());
                keybindings.spawn_shortcut::<ModifiedAction>(
                    spawner,
                    KeyCode::Enter.with_mod_keys(ModKeys::SHIFT),
                );
                keybindings.spawn_shortcut::<ControlAction>(
                    spawner,
                    KeyCode::Space.with_mod_keys(ModKeys::CONTROL),
                );
                keybindings.spawn_shortcut::<ControlShiftAction>(
                    spawner,
                    KeyCode::Space.with_mod_keys(ModKeys::CONTROL | ModKeys::SHIFT),
                );
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::Modified]
        );

        clear_recorded(&mut app);
        release_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());

        release_key(&mut app, window, KeyCode::Enter);
        press_key(&mut app, window, KeyCode::ControlLeft);
        press_key(&mut app, window, KeyCode::ShiftRight);
        press_key(&mut app, window, KeyCode::Space);
        app.update();
        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::ControlShift]
        );

        clear_recorded(&mut app);
        release_key(&mut app, window, KeyCode::ShiftRight);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());
    }

    #[test]
    fn runtime_shortcut_alternatives_share_one_physical_edge() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                keybindings.spawn_shortcut::<ModifiedAction>(spawner, KeyCode::Enter.into());
                keybindings.spawn_shortcut::<ModifiedAction>(
                    spawner,
                    KeyCode::Enter.with_mod_keys(ModKeys::SHIFT),
                );
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        let world = app.world_mut();
        let mut edges = world.query_filtered::<Entity, With<Action<PhysicalShortcutEdge>>>();
        assert_eq!(edges.iter(world).count(), 1);

        press_key(&mut app, window, KeyCode::ShiftLeft);
        press_key(&mut app, window, KeyCode::Enter);
        app.update();
        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::Modified]
        );

        clear_recorded(&mut app);
        release_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());
    }

    #[test]
    fn spawn_key_fires_on_successive_updates_while_held() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, ActionSettings::default());
                keybindings.spawn_key::<HeldAction>(spawner, KeyCode::ArrowUp);
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        press_key(&mut app, window, KeyCode::ArrowUp);
        app.update();
        assert_eq!(app.world().resource::<RecordedHeldFires>().0, 1);

        app.update();
        assert_eq!(app.world().resource::<RecordedHeldFires>().0, 2);
    }

    #[test]
    fn each_modifier_key_is_a_primary_one_shot_shortcut() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                for key_code in [
                    KeyCode::ShiftLeft,
                    KeyCode::ShiftRight,
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::AltLeft,
                    KeyCode::AltRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ] {
                    keybindings.spawn_shortcut::<ModifiedAction>(spawner, key_code.into());
                }
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        for (expected_starts, key_code) in [
            KeyCode::ShiftLeft,
            KeyCode::ShiftRight,
            KeyCode::ControlLeft,
            KeyCode::ControlRight,
            KeyCode::AltLeft,
            KeyCode::AltRight,
            KeyCode::SuperLeft,
            KeyCode::SuperRight,
        ]
        .into_iter()
        .enumerate()
        {
            press_key(&mut app, window, key_code);
            app.update();
            assert_eq!(
                app.world().resource::<RecordedStarts>().0.len(),
                expected_starts + 1
            );
            release_key(&mut app, window, key_code);
            app.update();
        }
    }

    #[test]
    fn opposite_modifier_side_blocks_primary_modifier_shortcuts() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                for key_code in [
                    KeyCode::ShiftLeft,
                    KeyCode::ShiftRight,
                    KeyCode::ControlLeft,
                    KeyCode::ControlRight,
                    KeyCode::AltLeft,
                    KeyCode::AltRight,
                    KeyCode::SuperLeft,
                    KeyCode::SuperRight,
                ] {
                    keybindings.spawn_shortcut::<ModifiedAction>(spawner, key_code.into());
                }
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        for (primary, opposite) in [
            (KeyCode::ShiftLeft, KeyCode::ShiftRight),
            (KeyCode::ShiftRight, KeyCode::ShiftLeft),
            (KeyCode::ControlLeft, KeyCode::ControlRight),
            (KeyCode::ControlRight, KeyCode::ControlLeft),
            (KeyCode::AltLeft, KeyCode::AltRight),
            (KeyCode::AltRight, KeyCode::AltLeft),
            (KeyCode::SuperLeft, KeyCode::SuperRight),
            (KeyCode::SuperRight, KeyCode::SuperLeft),
        ] {
            press_key(&mut app, window, opposite);
            app.update();
            clear_recorded(&mut app);

            press_key(&mut app, window, primary);
            app.update();
            assert!(app.world().resource::<RecordedStarts>().0.is_empty());

            release_key(&mut app, window, primary);
            release_key(&mut app, window, opposite);
            app.update();
            clear_recorded(&mut app);
        }
    }

    #[test]
    fn primary_modifier_shortcuts_match_additional_modifiers_exactly() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                keybindings.spawn_shortcut::<ControlAction>(
                    spawner,
                    KeyCode::ShiftLeft.with_mod_keys(ModKeys::CONTROL),
                );
                keybindings.spawn_shortcut::<BareAction>(spawner, KeyCode::AltLeft.into());
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        press_key(&mut app, window, KeyCode::ControlRight);
        press_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::Control]
        );

        clear_recorded(&mut app);
        release_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());

        press_key(&mut app, window, KeyCode::ShiftRight);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());

        press_key(&mut app, window, KeyCode::ShiftLeft);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());

        release_key(&mut app, window, KeyCode::ShiftLeft);
        release_key(&mut app, window, KeyCode::ControlRight);
        app.update();
        clear_recorded(&mut app);
        press_key(&mut app, window, KeyCode::AltLeft);
        app.update();
        assert!(app.world().resource::<RecordedStarts>().0.is_empty());
    }

    #[test]
    fn gamepad_shortcut_ignores_held_keyboard_modifiers() {
        let mut app = test_app();
        app.world_mut().spawn((
            TestContext,
            Actions::<TestContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                keybindings.spawn_shortcut::<GamepadAction>(spawner, GamepadButton::South.into());
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        let gamepad = app.world_mut().spawn(Gamepad::default()).id();
        app.update();

        press_key(&mut app, window, KeyCode::ControlLeft);
        press_key(&mut app, window, KeyCode::ShiftLeft);
        app.world_mut()
            .write_message(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
                gamepad,
                GamepadButton::South,
                1.0,
            )));
        app.update();

        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::Gamepad]
        );
    }

    #[test]
    fn consuming_shortcut_blocks_a_lower_priority_context() {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, EnhancedInputPlugin))
            .add_input_context::<HigherContext>()
            .add_input_context::<LowerContext>()
            .init_resource::<RecordedStarts>()
            .add_observer(record_higher)
            .add_observer(record_lower);
        app.finish();
        app.world_mut().spawn((
            HigherContext,
            ContextPriority::<HigherContext>::new(1),
            Actions::<HigherContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                let keybindings = Keybindings::new::<TestShift>(spawner, consuming_settings());
                keybindings.spawn_shortcut::<HigherAction>(spawner, KeyCode::Enter.into());
            })),
        ));
        app.world_mut().spawn((
            LowerContext,
            Actions::<LowerContext>::spawn(SpawnWith(|spawner: &mut ActionSpawner<_>| {
                spawner.spawn((Action::<LowerAction>::new(), bindings![KeyCode::Enter]));
            })),
        ));
        let window = app.world_mut().spawn_empty().id();
        app.update();

        press_key(&mut app, window, KeyCode::Enter);
        app.update();

        assert_eq!(
            app.world().resource::<RecordedStarts>().0,
            vec![RecordedAction::Higher]
        );
    }
}
