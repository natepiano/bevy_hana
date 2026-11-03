use bevy::ecs::system::SystemParam;
use bevy::prelude::Camera;
use bevy::prelude::ChildOf;
use bevy::prelude::Component;
use bevy::prelude::Entity;
use bevy::prelude::GlobalTransform;
use bevy::prelude::Query;
use bevy::prelude::Reflect;
use bevy::prelude::ReflectResource;
use bevy::prelude::Res;
use bevy::prelude::Resource;
use bevy::prelude::Vec2;
use bevy::prelude::Vec3;
use bevy::prelude::With;

use crate::BoundingBox;
use crate::ComputedScrollRecord;
use crate::DiegeticPanel;
use crate::layout::VisibleScrollRegion;
use crate::widgets::anchor::WidgetAnchorRect;

/// Exempts a panel and its descendants from the widget screen-size gate.
#[derive(Component, Debug, Default)]
pub struct SizeGateExempt;

/// Gates pointer picking by the smallest width of a projected hit rectangle.
///
/// Sizes are logical screen points in the picking camera's viewport. Each
/// evaluation uses the current projected size; no verdict survives the call.
/// The zero default disables the gate. Hosts supply their current preference
/// before `PickingSystems::Backend`; this input stores no picking verdict.
#[derive(Clone, Copy, Debug, Default, Reflect, Resource)]
#[reflect(Resource)]
pub struct WidgetScreenSizeGate {
    /// A widget is inert strictly below this size in logical points.
    inert_below: f32,
}

impl WidgetScreenSizeGate {
    /// Sets the minimum projected size in logical points.
    ///
    /// Zero disables the gate.
    ///
    /// # Panics
    /// Panics if the threshold is negative or nonfinite.
    #[must_use]
    pub const fn new(inert_below: f32) -> Self {
        assert!(
            inert_below.is_finite() && inert_below >= 0.0,
            "size threshold must be finite and nonnegative"
        );
        Self { inert_below }
    }

    /// Whether the measured smaller side meets the threshold.
    ///
    /// The boundary is inclusive. A zero threshold disables the gate.
    #[must_use]
    pub fn accepts(self, logical_smaller_side: f32) -> bool {
        self.disabled()
            || (logical_smaller_side.is_finite() && logical_smaller_side >= self.inert_below)
    }

    fn disabled(self) -> bool { self.inert_below == 0.0 }
}

#[derive(SystemParam)]
pub(in crate::widgets) struct WidgetSizePicking<'w, 's> {
    settings: Option<Res<'w, WidgetScreenSizeGate>>,
    cameras:  Query<'w, 's, (&'static Camera, &'static GlobalTransform)>,
    parents:  Query<'w, 's, &'static ChildOf>,
    exempt:   Query<'w, 's, (), With<SizeGateExempt>>,
}

impl WidgetSizePicking<'_, '_> {
    pub(super) fn accepts(
        &self,
        panel_entity: Entity,
        rect: BoundingBox,
        panel: &DiegeticPanel,
        panel_transform: &GlobalTransform,
        camera: Entity,
    ) -> bool {
        // HoveredScrollTargets can be used without installing WidgetsPlugin.
        let gate = self.settings.as_deref().copied().unwrap_or_default();
        if gate.disabled() || self.is_exempt(panel_entity) {
            return true;
        }
        self.projected_size(rect, panel, panel_transform, camera)
            .is_some_and(|size| gate.accepts(size))
    }

    pub(super) fn accepts_scroll(
        &self,
        panel_entity: Entity,
        record: &ComputedScrollRecord,
        panel: &DiegeticPanel,
        panel_transform: &GlobalTransform,
        camera: Entity,
    ) -> bool {
        let VisibleScrollRegion::Visible(rect) = record.visible_region else {
            return false;
        };
        self.accepts(panel_entity, rect, panel, panel_transform, camera)
    }

    fn is_exempt(&self, panel: Entity) -> bool {
        self.exempt.contains(panel)
            || self
                .parents
                .iter_ancestors(panel)
                .any(|ancestor| self.exempt.contains(ancestor))
    }

    fn projected_size(
        &self,
        rect: BoundingBox,
        panel: &DiegeticPanel,
        panel_transform: &GlobalTransform,
        camera: Entity,
    ) -> Option<f32> {
        let (camera, transform) = self.cameras.get(camera).ok()?;
        projected_smaller_side(rect, panel, panel_transform, camera, transform)
    }
}

/// Project the clipped rectangle in perimeter order. Panel scale, tilt,
/// perspective and the viewport's logical scale all contribute to its width.
fn projected_smaller_side(
    rect: BoundingBox,
    panel: &DiegeticPanel,
    panel_transform: &GlobalTransform,
    camera: &Camera,
    camera_transform: &GlobalTransform,
) -> Option<f32> {
    let anchor_rect = WidgetAnchorRect::new(panel, rect);
    let half = anchor_rect.size() * 0.5;
    let project = |x, y| {
        let local = anchor_rect.panel_offset() + Vec3::new(x, y, 0.0);
        camera
            .world_to_viewport(camera_transform, panel_transform.transform_point(local))
            .ok()
    };
    let corners = [
        project(-half.x, half.y)?,
        project(half.x, half.y)?,
        project(half.x, -half.y)?,
        project(-half.x, -half.y)?,
    ];
    if corners.iter().any(|corner| !corner.is_finite()) {
        return None;
    }
    Some(smallest_quad_width(corners))
}

/// A convex quad's minimum width occurs perpendicular to one of its edges.
/// For a parallelogram this is area / edge length. Taking the farthest vertex
/// from each edge also handles nonparallel opposite edges under perspective.
fn smallest_quad_width(corners: [Vec2; 4]) -> f32 {
    (0..corners.len())
        .map(|index| {
            let start = corners[index];
            let edge = corners[(index + 1) % corners.len()] - start;
            let length = edge.length();
            if length == 0.0 {
                return 0.0;
            }
            corners
                .iter()
                .map(|corner| edge.perp_dot(*corner - start).abs() / length)
                .fold(0.0, f32::max)
        })
        .fold(f32::INFINITY, f32::min)
}
