//! Display state: mirrors the per-kind interaction sources, speed, and live
//! zoom direction the panel highlights. lagrange debounces the reported sources
//! (see `hana_lagrange::CameraInputReportingDebounce`), and the panel holds
//! released sources briefly so highlights can fade after input ends.

use std::time::Duration;

use bevy::prelude::Component;
use hana_lagrange::ControlSpeed;
use hana_lagrange::FreeCamActiveDirections;
use hana_lagrange::FreeCamInteractionKind;
use hana_lagrange::FreeCamInteractionState;
use hana_lagrange::InteractionSources;
use hana_lagrange::OrbitCamInteractionKind;
use hana_lagrange::OrbitCamInteractionState;
use hana_lagrange::ZoomDirection;

use super::constants::HIGHLIGHT_RELEASE_HOLD;
use super::constants::HOME_HIGHLIGHT_HOLD;
use super::guidance::CameraGuidanceAction;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum RenderState {
    #[default]
    Idle,
    Pending,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct ReleaseCountdowns {
    orbit:     Option<Duration>,
    pan:       Option<Duration>,
    zoom:      Option<Duration>,
    look:      Option<Duration>,
    translate: Option<Duration>,
    roll:      Option<Duration>,
    home:      Option<Duration>,
}

impl ReleaseCountdowns {
    const fn empty() -> Self {
        Self {
            orbit:     None,
            pan:       None,
            zoom:      None,
            look:      None,
            translate: None,
            roll:      None,
            home:      None,
        }
    }
}

#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(super) struct CameraGuidanceDisplayState {
    orbit:                   InteractionSources,
    pan:                     InteractionSources,
    zoom:                    InteractionSources,
    look:                    InteractionSources,
    translate:               InteractionSources,
    roll:                    InteractionSources,
    home:                    InteractionSources,
    orbit_speed:             Option<ControlSpeed>,
    pan_speed:               Option<ControlSpeed>,
    zoom_speed:              Option<ControlSpeed>,
    look_speed:              Option<ControlSpeed>,
    translate_speed:         Option<ControlSpeed>,
    roll_speed:              Option<ControlSpeed>,
    zoom_direction:          Option<ZoomDirection>,
    free_directions:         FreeCamActiveDirections,
    slow_mode_active:        bool,
    releases:                ReleaseCountdowns,
    pub(super) render_state: RenderState,
}

impl CameraGuidanceDisplayState {
    pub(super) const fn from_display(display: CameraGuidanceDisplay) -> Self {
        Self {
            orbit:            display.orbit,
            pan:              display.pan,
            zoom:             display.zoom,
            look:             display.look,
            translate:        display.translate,
            roll:             display.roll,
            home:             display.home,
            orbit_speed:      display.orbit_speed,
            pan_speed:        display.pan_speed,
            zoom_speed:       display.zoom_speed,
            look_speed:       display.look_speed,
            translate_speed:  display.translate_speed,
            roll_speed:       display.roll_speed,
            zoom_direction:   display.zoom_direction,
            free_directions:  display.free_directions,
            slow_mode_active: display.slow_mode_active,
            releases:         ReleaseCountdowns::empty(),
            render_state:     RenderState::Idle,
        }
    }

    pub(super) const fn display(self) -> CameraGuidanceDisplay {
        CameraGuidanceDisplay {
            orbit:            self.orbit,
            pan:              self.pan,
            zoom:             self.zoom,
            look:             self.look,
            translate:        self.translate,
            roll:             self.roll,
            home:             self.home,
            orbit_speed:      self.orbit_speed,
            pan_speed:        self.pan_speed,
            zoom_speed:       self.zoom_speed,
            look_speed:       self.look_speed,
            translate_speed:  self.translate_speed,
            roll_speed:       self.roll_speed,
            zoom_direction:   self.zoom_direction,
            free_directions:  self.free_directions,
            slow_mode_active: self.slow_mode_active,
        }
    }

    /// Mirrors an action's reported interaction sources and holds the displayed
    /// sources through the panel release window.
    pub(super) fn set_sources(
        &mut self,
        action: CameraGuidanceAction,
        sources: InteractionSources,
    ) {
        let (slot, release) = match action {
            CameraGuidanceAction::Orbit => (&mut self.orbit, &mut self.releases.orbit),
            CameraGuidanceAction::Pan => (&mut self.pan, &mut self.releases.pan),
            CameraGuidanceAction::Zoom
            | CameraGuidanceAction::ZoomIn
            | CameraGuidanceAction::ZoomOut => (&mut self.zoom, &mut self.releases.zoom),
            CameraGuidanceAction::Look => (&mut self.look, &mut self.releases.look),
            CameraGuidanceAction::Translate => (&mut self.translate, &mut self.releases.translate),
            CameraGuidanceAction::Roll => (&mut self.roll, &mut self.releases.roll),
            CameraGuidanceAction::Home | CameraGuidanceAction::Other => return,
        };

        if sources.is_empty() {
            if !slot.is_empty() {
                *release = Some(HIGHLIGHT_RELEASE_HOLD);
            }
            return;
        }

        if *slot != sources {
            *slot = sources;
            self.render_state = RenderState::Pending;
        }
        *release = None;
    }

    /// Lights the home row and holds it for [`HOME_HIGHLIGHT_HOLD`], re-arming on
    /// each home invocation. The eased glide to the home pose has no discrete end
    /// event, so the row fades on this timer rather than on an interaction-ended
    /// signal.
    pub(super) const fn pulse_home(&mut self, sources: InteractionSources) {
        if sources.is_empty() {
            return;
        }
        self.home = sources;
        self.releases.home = Some(HOME_HIGHLIGHT_HOLD);
        self.render_state = RenderState::Pending;
    }

    /// Returns the home-row highlight (sources plus release countdown). A rebuild
    /// re-derives every interaction row from live camera state, but the home
    /// pulse is panel-side and has no camera-state source, so it is carried
    /// across the rebuild explicitly with [`Self::restore_home_highlight`].
    pub(super) const fn home_highlight(&self) -> (InteractionSources, Option<Duration>) {
        (self.home, self.releases.home)
    }

    pub(super) const fn restore_home_highlight(
        &mut self,
        home: InteractionSources,
        release: Option<Duration>,
    ) {
        self.home = home;
        self.releases.home = release;
    }

    pub(super) fn tick_highlight_release(&mut self, delta: Duration) {
        let orbit_expired =
            tick_release_countdown(&mut self.orbit, &mut self.releases.orbit, delta);
        let pan_expired = tick_release_countdown(&mut self.pan, &mut self.releases.pan, delta);
        let zoom_expired = tick_release_countdown(&mut self.zoom, &mut self.releases.zoom, delta);
        let look_expired = tick_release_countdown(&mut self.look, &mut self.releases.look, delta);
        let translate_expired =
            tick_release_countdown(&mut self.translate, &mut self.releases.translate, delta);
        let roll_expired = tick_release_countdown(&mut self.roll, &mut self.releases.roll, delta);
        let home_expired = tick_release_countdown(&mut self.home, &mut self.releases.home, delta);

        if orbit_expired
            || pan_expired
            || zoom_expired
            || look_expired
            || translate_expired
            || roll_expired
            || home_expired
        {
            self.render_state = RenderState::Pending;
        }
    }

    /// Records the live zoom direction read from the camera input when a zoom
    /// interaction begins or changes. A `None` argument (zero-delta frame) keeps
    /// the last direction so the highlighted row stays correct while the zoom is
    /// reported.
    pub(super) fn set_zoom_direction(&mut self, zoom_direction: Option<ZoomDirection>) {
        let Some(zoom_direction) = zoom_direction else {
            return;
        };
        if self.zoom_direction != Some(zoom_direction) {
            self.zoom_direction = Some(zoom_direction);
            self.render_state = RenderState::Pending;
        }
    }

    /// Records the live `FreeCam` move directions read from the bound camera. An
    /// empty set (idle or debounce-hold frame) keeps the last directions so a
    /// highlighted row stays lit through the panel's release window rather than
    /// blinking off before its source fades.
    pub(super) fn set_free_directions(&mut self, directions: FreeCamActiveDirections) {
        if directions.is_empty() {
            return;
        }
        if self.free_directions != directions {
            self.free_directions = directions;
            self.render_state = RenderState::Pending;
        }
    }

    pub(super) fn set_speed(&mut self, action: CameraGuidanceAction, speed: Option<ControlSpeed>) {
        let slot = match action {
            CameraGuidanceAction::Orbit => &mut self.orbit_speed,
            CameraGuidanceAction::Pan => &mut self.pan_speed,
            CameraGuidanceAction::Zoom
            | CameraGuidanceAction::ZoomIn
            | CameraGuidanceAction::ZoomOut => &mut self.zoom_speed,
            CameraGuidanceAction::Look => &mut self.look_speed,
            CameraGuidanceAction::Translate => &mut self.translate_speed,
            CameraGuidanceAction::Roll => &mut self.roll_speed,
            CameraGuidanceAction::Home | CameraGuidanceAction::Other => return,
        };
        if *slot != speed {
            *slot = speed;
            self.render_state = RenderState::Pending;
        }
    }

    pub(super) const fn set_slow_mode_active(&mut self, active: bool) {
        if self.slow_mode_active != active {
            self.slow_mode_active = active;
            self.render_state = RenderState::Pending;
        }
    }
}

impl Default for CameraGuidanceDisplayState {
    fn default() -> Self { Self::from_display(CameraGuidanceDisplay::default()) }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct CameraGuidanceDisplay {
    pub(super) orbit:            InteractionSources,
    pub(super) pan:              InteractionSources,
    pub(super) zoom:             InteractionSources,
    pub(super) look:             InteractionSources,
    pub(super) translate:        InteractionSources,
    pub(super) roll:             InteractionSources,
    pub(super) home:             InteractionSources,
    pub(super) orbit_speed:      Option<ControlSpeed>,
    pub(super) pan_speed:        Option<ControlSpeed>,
    pub(super) zoom_speed:       Option<ControlSpeed>,
    pub(super) look_speed:       Option<ControlSpeed>,
    pub(super) translate_speed:  Option<ControlSpeed>,
    pub(super) roll_speed:       Option<ControlSpeed>,
    pub(super) zoom_direction:   Option<ZoomDirection>,
    pub(super) free_directions:  FreeCamActiveDirections,
    pub(super) slow_mode_active: bool,
}

impl CameraGuidanceDisplay {
    pub(super) const fn from_orbit_camera_state(
        state: OrbitCamInteractionState,
        slow_mode_active: bool,
    ) -> Self {
        Self {
            orbit: state.orbit_sources(),
            pan: state.pan_sources(),
            zoom: state.zoom_sources(),
            look: InteractionSources::NONE,
            translate: InteractionSources::NONE,
            roll: InteractionSources::NONE,
            home: InteractionSources::NONE,
            orbit_speed: state.speed(OrbitCamInteractionKind::Orbit),
            pan_speed: state.speed(OrbitCamInteractionKind::Pan),
            zoom_speed: state.speed(OrbitCamInteractionKind::Zoom),
            look_speed: None,
            translate_speed: None,
            roll_speed: None,
            zoom_direction: state.zoom_direction(),
            free_directions: FreeCamActiveDirections::NONE,
            slow_mode_active,
        }
    }

    pub(super) const fn from_free_camera_state(
        state: FreeCamInteractionState,
        slow_mode_active: bool,
    ) -> Self {
        Self {
            orbit: InteractionSources::NONE,
            pan: InteractionSources::NONE,
            zoom: InteractionSources::NONE,
            look: state.look_sources(),
            translate: state.translate_sources(),
            roll: state.roll_sources(),
            home: InteractionSources::NONE,
            orbit_speed: None,
            pan_speed: None,
            zoom_speed: None,
            look_speed: state.speed(FreeCamInteractionKind::Look),
            translate_speed: state.speed(FreeCamInteractionKind::Translate),
            roll_speed: state.speed(FreeCamInteractionKind::Roll),
            zoom_direction: None,
            free_directions: state.directions(),
            slow_mode_active,
        }
    }

    pub(super) const fn sources(self, action: CameraGuidanceAction) -> InteractionSources {
        match action {
            CameraGuidanceAction::Orbit => self.orbit,
            CameraGuidanceAction::Pan => self.pan,
            CameraGuidanceAction::Zoom
            | CameraGuidanceAction::ZoomIn
            | CameraGuidanceAction::ZoomOut => self.zoom,
            CameraGuidanceAction::Look => self.look,
            CameraGuidanceAction::Translate => self.translate,
            CameraGuidanceAction::Roll => self.roll,
            CameraGuidanceAction::Home => self.home,
            CameraGuidanceAction::Other => InteractionSources::NONE,
        }
    }

    pub(super) const fn zoom_direction(self) -> Option<ZoomDirection> { self.zoom_direction }

    pub(super) const fn free_directions(self) -> FreeCamActiveDirections { self.free_directions }

    pub(super) const fn slow_mode_active(self) -> bool { self.slow_mode_active }

    pub(super) const fn speed(self, action: CameraGuidanceAction) -> Option<ControlSpeed> {
        match action {
            CameraGuidanceAction::Orbit => self.orbit_speed,
            CameraGuidanceAction::Pan => self.pan_speed,
            CameraGuidanceAction::Zoom
            | CameraGuidanceAction::ZoomIn
            | CameraGuidanceAction::ZoomOut => self.zoom_speed,
            CameraGuidanceAction::Look => self.look_speed,
            CameraGuidanceAction::Translate => self.translate_speed,
            CameraGuidanceAction::Roll => self.roll_speed,
            // Home has no slow variant; report Normal so its row is never gated
            // out of the Normal speed block when the home pulse lights it.
            CameraGuidanceAction::Home => Some(ControlSpeed::Normal),
            CameraGuidanceAction::Other => None,
        }
    }
}

fn tick_release_countdown(
    sources: &mut InteractionSources,
    release: &mut Option<Duration>,
    delta: Duration,
) -> bool {
    let Some(remaining) = release.as_mut() else {
        return false;
    };
    let next = remaining.saturating_sub(delta);
    if next == Duration::ZERO {
        *sources = InteractionSources::NONE;
        *release = None;
        return true;
    }

    *remaining = next;
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_zoom_direction_updates_and_survives_zero_delta_frames() {
        let mut display = CameraGuidanceDisplayState::default();

        display.set_zoom_direction(Some(ZoomDirection::In));
        assert_eq!(display.display().zoom_direction, Some(ZoomDirection::In));
        assert_eq!(display.render_state, RenderState::Pending);

        // A zero-delta frame reports `None` and must keep the captured direction
        // so the highlighted row stays correct while the zoom is reported.
        display.render_state = RenderState::Idle;
        display.set_zoom_direction(None);
        assert_eq!(display.display().zoom_direction, Some(ZoomDirection::In));
        assert_eq!(display.render_state, RenderState::Idle);

        // Reversing direction (release zoom-in, press zoom-out) flips the tag and
        // requests a rebuild even though the source bit is unchanged.
        display.set_zoom_direction(Some(ZoomDirection::Out));
        assert_eq!(display.display().zoom_direction, Some(ZoomDirection::Out));
        assert_eq!(display.render_state, RenderState::Pending);
    }

    #[test]
    fn set_sources_marks_pending_only_on_change() {
        let mut display = CameraGuidanceDisplayState::default();

        display.set_sources(CameraGuidanceAction::Orbit, InteractionSources::NONE);
        assert_eq!(display.releases.orbit, None);
        assert_eq!(display.render_state, RenderState::Idle);

        display.set_sources(CameraGuidanceAction::Orbit, InteractionSources::MOUSE);
        assert_eq!(display.display().orbit, InteractionSources::MOUSE);
        assert_eq!(display.render_state, RenderState::Pending);

        // Re-reporting the same sources does not request another rebuild.
        display.render_state = RenderState::Idle;
        display.set_sources(CameraGuidanceAction::Orbit, InteractionSources::MOUSE);
        assert_eq!(display.render_state, RenderState::Idle);

        // The release hold keeps the displayed sources stable until its
        // countdown elapses.
        display.set_sources(CameraGuidanceAction::Orbit, InteractionSources::NONE);
        assert_eq!(display.display().orbit, InteractionSources::MOUSE);
        assert_eq!(display.releases.orbit, Some(HIGHLIGHT_RELEASE_HOLD));
        assert_eq!(display.render_state, RenderState::Idle);

        display.set_sources(CameraGuidanceAction::Orbit, InteractionSources::MOUSE);
        assert_eq!(display.display().orbit, InteractionSources::MOUSE);
        assert_eq!(display.releases.orbit, None);
        assert_eq!(display.render_state, RenderState::Idle);
    }

    #[test]
    fn release_countdown_clears_sources_and_marks_pending() {
        let mut display = CameraGuidanceDisplayState::default();

        display.set_sources(CameraGuidanceAction::Pan, InteractionSources::KEYBOARD);
        display.render_state = RenderState::Idle;
        display.set_sources(CameraGuidanceAction::Pan, InteractionSources::NONE);

        let partial_delta = HIGHLIGHT_RELEASE_HOLD / 2;
        display.tick_highlight_release(partial_delta);
        assert_eq!(display.display().pan, InteractionSources::KEYBOARD);
        assert_eq!(
            display.releases.pan,
            Some(HIGHLIGHT_RELEASE_HOLD.saturating_sub(partial_delta)),
        );
        assert_eq!(display.render_state, RenderState::Idle);

        display.tick_highlight_release(HIGHLIGHT_RELEASE_HOLD);
        assert!(display.display().pan.is_empty());
        assert_eq!(display.releases.pan, None);
        assert_eq!(display.render_state, RenderState::Pending);
    }

    #[test]
    fn slow_mode_active_marks_pending_only_on_change() {
        let mut display = CameraGuidanceDisplayState::default();

        display.set_slow_mode_active(false);
        assert!(!display.display().slow_mode_active);
        assert_eq!(display.render_state, RenderState::Idle);

        display.set_slow_mode_active(true);
        assert!(display.display().slow_mode_active);
        assert_eq!(display.render_state, RenderState::Pending);

        display.render_state = RenderState::Idle;
        display.set_slow_mode_active(true);
        assert_eq!(display.render_state, RenderState::Idle);
    }

    #[test]
    fn home_pulse_lights_and_survives_the_rebuild_carry() {
        let mut display = CameraGuidanceDisplayState::default();

        display.pulse_home(InteractionSources::GAMEPAD);
        assert_eq!(display.display().home, InteractionSources::GAMEPAD);
        assert_eq!(display.render_state, RenderState::Pending);

        // A same-camera rebuild re-derives interaction rows from camera state but
        // must keep the panel-side home pulse across the reset.
        let (home, release) = display.home_highlight();
        display = CameraGuidanceDisplayState::from_display(
            CameraGuidanceDisplay::from_free_camera_state(
                FreeCamInteractionState::default(),
                false,
            ),
        );
        assert!(display.display().home.is_empty());
        display.restore_home_highlight(home, release);
        assert_eq!(display.display().home, InteractionSources::GAMEPAD);

        // Once the hold elapses the row clears.
        display.tick_highlight_release(HOME_HIGHLIGHT_HOLD);
        assert!(display.display().home.is_empty());
    }
}
