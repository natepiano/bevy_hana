use bevy::prelude::Vec4;
use bevy::render::render_resource::ShaderType;
use bytemuck::Pod;
use bytemuck::Zeroable;

use super::constants::OWNER_DATA_RESERVED_CHANNEL;
use super::extract::ExtractedOutline;

#[derive(Debug, Clone, ShaderType, Pod, Zeroable, Copy)]
#[repr(C)]
pub(crate) struct OutlineUniform {
    intensity:  f32,
    width:      f32,
    priority:   f32,
    overlap:    f32,
    color:      Vec4,
    owner_data: Vec4,
}

impl OutlineUniform {
    /// Builds the `owner_data` channel layout: x = owner ID, y = shell-mode
    /// shader factor, z/w = reserved padding.
    const fn owner_data_for(owner_id: f32, shell_mode: f32) -> Vec4 {
        Vec4::new(
            owner_id,
            shell_mode,
            OWNER_DATA_RESERVED_CHANNEL,
            OWNER_DATA_RESERVED_CHANNEL,
        )
    }
}

impl From<&ExtractedOutline> for OutlineUniform {
    fn from(outline: &ExtractedOutline) -> Self {
        let shell_mode = outline.outline_method.as_shell_mode_factor();
        Self {
            intensity:  outline.intensity,
            width:      outline.width,
            priority:   outline.priority,
            overlap:    outline.overlap,
            color:      outline.color,
            owner_data: Self::owner_data_for(outline.owner_id, shell_mode),
        }
    }
}
