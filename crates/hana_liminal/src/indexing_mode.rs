use bevy_render::batching::gpu_preprocessing::IndirectParametersCpuMetadata;
use bevy_render::batching::gpu_preprocessing::UntypedPhaseIndirectParametersBuffers;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum IndexingMode {
    Indexed,
    NonIndexed,
}

impl IndexingMode {
    pub(crate) fn write_metadata(
        self,
        buffers: &mut UntypedPhaseIndirectParametersBuffers,
        offset: u32,
        metadata: IndirectParametersCpuMetadata,
    ) {
        match self {
            Self::Indexed => buffers.indexed.set(offset, metadata),
            Self::NonIndexed => buffers.non_indexed.set(offset, metadata),
        }
    }
}

impl From<bool> for IndexingMode {
    fn from(indexed: bool) -> Self {
        if indexed {
            Self::Indexed
        } else {
            Self::NonIndexed
        }
    }
}
