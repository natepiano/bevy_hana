# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

> This crate is based on the work of [Plonq](https://github.com/Plonq) and his
> [bevy_panorbit_camera](https://github.com/Plonq/bevy_panorbit_camera). Thank you
> for graciously allowing this project to build on your foundation.

## [Unreleased]

### Added

- `CameraEventTiming`, `AnimationRejectionReason`, and
  `CameraRequestPreparationError` enrich the established camera lifecycle and
  rejection events. Lifecycle payloads now include source, target, owner,
  direction, and raw timing; move payloads additionally include stable stage
  identity and exact `boundary_elapsed` time.

- `CameraMove::try_to_look_at()` and `CameraMove::try_to_orbital_look_at()`
  fallible constructors, `CameraMoveError` for every rejected authored value,
  and read-only accessors (`destination()`, `duration()`, `easing()`,
  `focus()`, `free_cam_roll_target()`, `orbit_angles()`, `position()`,
  `radius()`).
- `FreeCamRollTarget` with `InheritPrevious` and `Explicit(Roll)`, replacing the
  move's bare optional roll.
- `CameraMoveDestination`, naming which authored form a move used.
- `CameraSequence` and `CameraSequenceError` for retained, never-empty camera
  authoring. A sequence describes its own `SequenceStages`, which is published
  on the sequence entity so shared driver arbitration and transport can resolve
  scopes against it.
- Re-exports of the `hana_kana` sequence and easing types reachable through
  camera signatures: `Easing`, `EasingCurve`, `EasingInput`,
  `EasingInterpolation`, `EasingKnot`, `EasingOutput`, `EasingSlope`,
  `EasingSlopes`, `SequenceCommand`, `SequenceCommandOutcome`,
  `SequenceCommandRejected`, `SequenceCommandResponse`, `SequenceDirection`,
  `SequenceDriver`, `SequenceDriverTakeover`, `SequenceEasing`,
  `SequenceEasingError`, `SequenceEvaluation`, `SequenceMovement`,
  `SequenceOwner`, `SequencePosition`, `SequenceScope`, `SequenceSourceState`,
  `SequenceStageId`, `SequenceStageSpan`, `SequenceStages`, `SequenceTime`.
- `CameraCommands`, `CameraPlaybackObservation`, and `SequenceOwnership` give
  applications one shared command/ownership vocabulary for retained camera and
  fold playback. A missing retained sequence is now distinguishable from native
  and selected-driver ownership.

### Changed

- **Breaking:** `CameraMove` is an opaque struct instead of a public enum.
  Replace `CameraMove::ToLookAt { .. }` with `CameraMove::try_to_look_at(..)`
  and `CameraMove::ToOrbitalLookAt { .. }` with
  `CameraMove::try_to_orbital_look_at(..)`, and read fields through the
  accessors. Coordinates are the semantic types `Focus`, `OrbitAngles`,
  `Position`, `Radius`, and `Roll` rather than raw scalars.
- **Breaking:** `Easing::Curve` owns immutable `EasingCurve` authoring data.
  Camera moves sample it without world or asset access, and `LagrangePlugin`
  no longer requires `AssetPlugin` for easing. Construct `EasingInput`,
  `EasingOutput`, and `EasingSlope` values before building a curve.
- `PlayAnimation` now admits requests in trigger order into one retained
  `CameraSequence` backend after shared driver arbitration. Zero-duration
  fit/look operations follow the same transaction, command ownership,
  controller capture, and raw-boundary event path as timed requests.
- **Breaking:** remove `CameraMoveList` and direct component insertion. Start
  an atomic journey with `PlayAnimation`, retain authored steps with
  `CameraSequence`, and observe or control them through `CameraCommands`.
- Established lifecycle, source, reason, conflict-policy, and `Zoom*` names
  remain unchanged, including the source-plus-target payloads on lifecycle and
  rejection events.
- Retained camera movement now translates every crossed raw sequence boundary
  through the existing `AnimationBegin`, `AnimationEnd`, `AnimationRejected`,
  `CameraMoveBegin`, and `CameraMoveEnd` event surface.
- The existing event surface now distinguishes native admission from raw
  boundary traversal. Zero-duration routing keeps the deterministic order
  `ZoomBegin` (when applicable), `AnimationBegin`, move begin, move end,
  `AnimationEnd`, and `ZoomEnd` (when applicable); no `CameraAnimation*`
  replacement API was added.

## [0.4.0] - 2026-07-30

### Changed

- Renamed from `bevy_lagrange` to `hana_lagrange`. No API changes; update the
  dependency name and any `bevy_lagrange::` paths, including reflected
  `TypePath` strings in serialized scenes. Feature names are unchanged.
  `bevy_lagrange` 0.3.1 is a deprecated re-export shim and the final release
  under the old name.

## [0.3.0] - 2026-07-29

### Added

- `CameraBasis` component with `Y_UP` and `Z_UP` presets for camera coordinate
  bases shared by orbit and free cameras.
- `FreeCam::with_preset()`, `FreeCam::with_bindings()`, and `FreeCam::manual()`
  bundle constructors matching the `OrbitCam` preset-selection API.
- `FreeCam` controller support with keyboard/mouse presets, bindings, BEI
  actions, and `AnimateToFit` / `ZoomToFit` / `LookAt` /
  `LookAtAndZoomToFit` handling.
- `FreeCam::pitch_limited()` and `FreeCam::horizon_locked()` constructors for
  common constrained free-flight setups.
- `FreeCamBindings::builder()`, `FreeCamBindings::validate()`,
  `FreeCamBindingsBuilder`, and `FreeCamTranslateKeys` for named, validated
  FreeCam keyboard/mouse binding construction.
- `FreeCamManualInputWriter` and `FreeCamManualInput` for app-authored
  translate/look/roll intent while a `FreeCam` is in `FreeCamInputMode::Manual`.
- Generic `describe_controls` / `describe_controls_for` entry points plus
  `CameraControlBinding` summaries for camera control UI/help surfaces.
- `ResetOrbitCamToHome` and `ResetFreeCamToHome` trigger events for
  BRP/app-triggerable reset-to-home flows.
- `FreeCamHomePose::from_orbit_home()` for explicit OrbitCam-to-FreeCam mode
  switches that preserve Home behavior.

### Changed

- `OrbitCam::from_pose()` and `FreeCam::from_pose()` now accept
  `impl Into<OrbitAngles>` / `impl Into<LookAngles>` for their angle
  parameters, and `OrbitAngles` / `LookAngles` implement `From<(f32, f32)>` so
  `(yaw, pitch)` tuples can be passed directly.
- Update the fixed FreeCam example with a vertical title bar that switches
  between free-flight, pitch-limited, and horizon-locked behavior presets,
  adjusts the shared pitch clamp with `+` / `-`, and starts inside a generated
  10x10x10 platform grid.
- Historical note: timed FreeCam fit requests used `CameraMoveList` during
  0.3.0. Current callers use `PlayAnimation` or `CameraSequence`; `CameraMove`
  roll authoring and the retained lifecycle names remain compatible.
- Replace `OrbitCam::axis` with the required `CameraBasis` component (breaking).
  Callers that set `orbit_cam.axis = ...` should insert or mutate `CameraBasis`
  on the camera entity instead.
- Replace `OrbitCamInputPhase` with shared `CameraInputPhase` (breaking). Manual
  input systems should run in `CameraInputPhase::WriteManual`.
- Require `CameraKind` implementations to register controller, `AnimateToFit`,
  `ZoomToFit`, and look-at support through
  `CameraKind::add_camera_kind_systems` (breaking for external `CameraKind`
  implementors).
- Rename `OrbitCamManualInput` writer methods to domain verbs (breaking):
  `orbit_pixels` -> `orbit`, `pan_pixels` -> `pan`, `zoom_coarse_amount` ->
  `zoom_coarse`, `zoom_smooth_amount` -> `zoom_smooth`, and the zero-delta
  active markers to `mark_{orbit,pan,zoom}_active`.
- Remove `#[non_exhaustive]` from `OrbitCamBindingsError`; binding validation
  errors are closed enums unless reopened by an explicit future policy
  decision.

### Removed

- Remove `OrbitCam::keyboard()`, `OrbitCam::simple_mouse_keyboard()`, and
  `OrbitCam::blender_like_keyboard()`. Use `OrbitCam::with_preset(...)` with the
  matching `OrbitCamPreset` constructors instead (breaking).
- Remove the unused `OrbitCamReportingDebounce` alias. Use
  `CameraInputReportingDebounce` instead (breaking).

## [0.2.0] - 2026-06-23

### Changed

- Replace `OrbitCam`'s `{orbit,pan,zoom}_sensitivity` and `{orbit,pan,zoom}_smoothness`
  fields with three `orbit`/`pan`/`zoom` `AxisResponse` values bundling per-axis
  `Sensitivity` and `Damping`; read with `.sensitivity()`/`.damping()`, set with
  `.set_sensitivity()`/`.set_damping()` (breaking).
- Rename the input-sensitivity API to input gain (breaking). The per-axis
  `Sensitivity` type on `AxisResponse` is unrelated and unchanged.
  - Types:
    - `InputSensitivity` → `InputGain`
    - `OrbitCamSensitivity` → `OrbitCamInputGain`
    - `MouseSensitivity` → `MouseInputGain`
    - `GamepadSensitivity` → `GamepadInputGain`
    - `SmoothScrollSensitivity` → `SmoothScrollInputGain`
    - `OrbitCamBindingWithSensitivity<T>` → `OrbitCamBindingWithInputGain<T>`
  - Builder methods:
    - `with_sensitivity` → `with_input_gain`
    - `mouse_sensitivity` → `mouse_input_gain`
    - `smooth_scroll_sensitivity` → `smooth_scroll_input_gain`
    - `gamepad_sensitivity` → `gamepad_input_gain`

### Removed

- `reflect-input-modes` cargo feature and its reflected draft/apply types
  (`OrbitCamInputModeDescriptor`, `OrbitCamInputModeDraft`, the `*PresetDraft`
  mirrors, `OrbitCamInputModeApplyStatus`/`OrbitCamInputModeApplied`/`OrbitCamInputModeRejected`).
  Author input modes directly with `OrbitCamInputMode` (`with_preset`, `Bindings`,
  `Manual`); the runtime preset/binding types and their `Reflect` registration are
  unchanged.

## [0.1.0] - 2026-06-22

### Added
- Source-level input sensitivity on preset APIs — presets can carry mouse, gamepad, and smooth-scroll tuning through construction and conversion, preserved across `impl Into<OrbitCamPreset>` builder bridges.
- Binding-level sensitivity in the device adapter pipeline — mouse wheel, trackpad orbit/pan/zoom, pinch, touch, and button-drag inputs are scaled by their active binding sensitivity.

### Changed
- Change `OrbitCamPreset` variants to carry preset payloads; use helper constructors such as `OrbitCamPreset::blender_like()` or `OrbitCamInputMode::with_preset(...)` instead of unit variants like `OrbitCamPreset::BlenderLike` (breaking).
- A disabled input binding no longer affects camera motion or shows up as an active input source.
- Zero or negative input sensitivities are rejected consistently; a zero-sensitive source is disabled rather than applied.

### Fixed
- Clear cached orbit interaction and reported settle state when an input mode is replaced, so mode swaps no longer retain stale source or debounce data (including slow-mode latch reset).

## [0.0.4] - 2026-06-20

### Added
- `OrbitCamSlowMode` and `CameraSlowModeState` — first-class slow control speed;
  BlenderLike preset adapters scale orbit/pan/zoom while a slow modifier is held, and the
  active speed is surfaced through `OrbitCamInteractionState`.
- `OrbitCamScalePolicy` — configurable input-scaling policy for orbit-cam adapters.
- Per-preset marker types — `OrbitCamSimpleMousePreset`, `OrbitCamSimpleMouseKeyboardPreset`,
  `OrbitCamBlenderLikePreset`, `OrbitCamBlenderLikeKeyboardPreset`, `OrbitCamKeyboardPreset` —
  plus preset helper constructors.
- `FitAnchor` event.

### Changed
- Update to Bevy 0.19.0 (stable) from the `0.19.0-rc.2` release candidate.
- Modularize orbit-cam presets into per-preset modules.

### Removed
- Remove `OrbitCamBindingsProfile`, `OrbitCamPresetLayer`, `OrbitCamPresetLayers`, and
  `PresetLayerSet` — superseded by the per-preset types (breaking).

## [0.0.4-rc.1] - 2026-06-05

### Changed
- Update to Bevy 0.19 (`0.19.0-rc.2`) (breaking).
- `OrbitCamInteractionState::speed(kind)` now returns `Option<ControlSpeed>` — `None` until a gamepad's speed settles (breaking).
- Add `target: Option<Entity>` to the `AnimationBegin`, `AnimationEnd`, and `AnimationRejected` events (breaking).
- Collapse `AnimationCancelled` into `AnimationEnd` and `ZoomCancelled` into `ZoomEnd`, each with a `reason` field (breaking).
- Replace owned `bool` input fields with enums (`PinchGestureZoom`, `CameraMotion`, `FocusFrame`) (breaking).
- Rename `ZoomDirection` to `ZoomInversion` (`Reversed` → `Inverted`) and `zoom_direction` builder method/field to `zoom_inversion`; the name `ZoomDirection` now refers to an unrelated new reporting enum (breaking).
- Rebuild input handling on `bevy_enhanced_input` (new dependency).
- Scroll zoom now uses an exponential curve so zoom in and out are symmetric.
- Move the crate into the [hana](https://github.com/natepiano/hana) monorepo; `repository`/`homepage` metadata updated.
- Fit overlay lines are now retained `Core3d` mesh visuals on the source camera's `RenderLayers` instead of gizmos; overlay labels render as UI text targeted via `UiTargetCamera`.
- Replace raw `OrbitCam` input fields with input-mode components (`OrbitCamPreset`, `OrbitCamBindings`, `OrbitCamManual`).
- Replace manual render-to-texture camera setup with `CameraInputRoutingConfig::explicit(...)` and `CameraInputSurfaceMetrics`.
- Replace keyboard target-mutation examples with `OrbitCamBindings`/`OrbitCamManualInputWriter` examples.

### Added
- `OrbitCamReportingDebounce(Duration)` resource (default 100 ms) — debounces the reported `OrbitCamInteractionState` (per-kind sources and gamepad speed) so a control panel does not flicker. Reporting-only; `Duration::ZERO` disables it.
- Source-attributed camera interaction lifecycle events and `OrbitCamInteractionState`.
- Teaching examples for SimpleMouse, BlenderLike, keyboard, gamepad, manual, and custom bindings.
- `ZoomDirection { In, Out }` and a `zoom_direction` tag on `OrbitCamControlRow`; built-in presets emit one zoom row per direction. `OrbitCamMouseWheelZoom` is now a marker struct.
- `OrbitCamInteractionState::zoom_direction()` reports the active zoom's direction, held through the debounce window and flipped at once on a reversal.
- Gamepad input preset.
- `reflect-input-modes` cargo feature (enabled by default) — reflection support for the input-mode components.
- `swapped_axis` example — multi-engine coordinate-convention showcase.

### Removed
- Remove the `bevy_egui` feature, `EguiWantsFocus`, and the egui example (breaking).

### Fixed
- Route camera input by focused window so overlapping windows with stale cursor positions no longer capture input.
- Suppress pinch zoom while BlenderLike modifier keys are held.
- Split `LookAtAndZoomToFit` into look and fit phases; internal fit work no longer emits `ZoomBegin`/`ZoomEnd`.
- Input teardown no longer double-despawns binding entities (logged "Could not despawn entity" on every preset switch).
- Fit overlay visuals no longer intercept picking, and duplicate/orphaned/stale overlay visuals are reconciled away.

## [0.0.3] - 2026-04-15

### Changed
- Reduced the default `OrbitCam` perspective zoom lower limit from `0.05` to `1e-7` so close-up orbiting can zoom much nearer to the target
- Perspective cameras now keep their near clip plane synchronized to orbit radius, with a minimum floor and far-plane clamp, to avoid clipping the focus target during close zoom

### Added
- Utility tests covering perspective near-plane synchronization for radius tracking, minimum clamping, and far-plane clamping

## [0.0.2] - 2026-04-06

### Changed
- Restructured `OrbitCam` input configuration: flat touch/trackpad fields (`touch_enabled`, `touch_controls`, `trackpad_behavior`, `trackpad_pinch_to_zoom_enabled`, `trackpad_sensitivity`) replaced with `Option<InputControl>` containing `Option<TouchInput>` and `Option<TrackpadInput>` — set to `None` to disable all user input
- Animation events and types (`ZoomToFit`, `LookAt`, `PlayAnimation`, `CameraMove`, etc.) are now always available without requiring the `fit_overlay` feature
- Extracted internal types (`ButtonZoomAxis`, `TrackpadBehavior`, `UpsideDownPolicy`, `ZoomDirection`, etc.) as standalone public types

### Fixed
- Prevent panic when closing a second window
- Idle camera no longer triggers Bevy change detection every frame — `&mut Transform` is now only passed when the camera actually moves

## [0.0.1] - 2026-03-28

### Added
- Pan, orbit, and zoom camera controls with smoothing, customizable sensitivity, and configurable key/mouse bindings
- Orthographic and perspective projection support
- Touch controls (one finger orbit, two finger pan, pinch zoom)
- Trackpad support with optional Blender-style orbit/pan/zoom mode
- Multi-viewport and multi-window support
- Render-to-texture camera support
- Optional `bevy_egui` feature to ignore input consumed by egui
- Optional `fit_overlay` feature: zoom-to-fit, retained camera journeys, event-driven camera control (`ZoomToFit`, `LookAt`, `LookAtAndZoomToFit`, `AnimateToFit`, `PlayAnimation`), animation lifecycle events, conflict resolution, and debug overlay with gizmos and screen-space labels
