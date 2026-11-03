//! Paired screenshots requested before the same render extraction.

use std::sync::mpsc;

use bevy::prelude::*;
use bevy::render::render_resource::PollType;
use bevy::render::renderer::RenderDevice;
use bevy::render::view::screenshot::Screenshot;
use bevy::render::view::screenshot::ScreenshotCaptured;

use crate::drawn_support::CaseApp;

/// Reads both views of the first frame after the caller's world edit.
pub(crate) fn capture_pair(
    app: &mut CaseApp,
    targets: [Handle<Image>; 2],
) -> Result<[Vec<u8>; 2], String> {
    let (sender, receiver) = mpsc::channel();
    for (view, target) in targets.into_iter().enumerate() {
        let sender = sender.clone();
        app.world_mut().spawn(Screenshot::image(target)).observe(
            move |event: On<ScreenshotCaptured>| {
                let pixels = event
                    .image
                    .data
                    .clone()
                    .ok_or("screenshot has no pixel data");
                let _ = sender.send((view, pixels));
            },
        );
    }
    let mut frames = [Vec::new(), Vec::new()];
    let mut captured = 0;
    // Both requests enter the first extraction after the world edit.
    // Later updates deliver those readbacks; they do not select a later frame.
    loop {
        app.update();
        app.world()
            .resource::<RenderDevice>()
            .wgpu_device()
            .poll(PollType::Wait {
                submission_index: None,
                timeout:          None,
            })
            .map_err(|error| format!("GPU screenshot readback failed: {error}"))?;
        while let Ok((view, pixels)) = receiver.try_recv() {
            frames[view] = pixels.map_err(str::to_owned)?;
            captured += 1;
        }
        if captured == 2 {
            return Ok(frames);
        }
    }
}
