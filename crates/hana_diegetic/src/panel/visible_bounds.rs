//! The visible portion of retained panel output, using the renderer's clips.

use super::DiegeticPanel;
use crate::BoundingBox;
use crate::LayoutResult;
use crate::layout::RenderCommand;
use crate::render::clip;

impl DiegeticPanel {
    /// Commands and their visible bounds in layout points, after nested scissor
    /// clips and the panel viewport. Fully clipped commands contribute nothing.
    pub fn visible_command_bounds<'a>(
        &self,
        layout: &'a LayoutResult,
    ) -> impl Iterator<Item = (&'a RenderCommand, BoundingBox)> {
        let viewport = clip::panel_viewport(self);
        layout
            .commands
            .iter()
            .zip(clip::compute_clip_rects(&layout.commands))
            .filter_map(move |(command, scissor)| {
                let active = clip::effective_clip(command.bounds, scissor, viewport)?;
                Some((command, command.bounds.intersect(&active)?))
            })
    }
}
