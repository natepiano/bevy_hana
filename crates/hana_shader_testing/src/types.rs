//! Stable resource type spelling for linked shader contracts.

use naga::AddressSpace;
use naga::ArraySize;
use naga::Handle;
use naga::ImageClass;
use naga::ImageDimension;
use naga::Module;
use naga::Scalar;
use naga::ScalarKind;
use naga::ShaderStage;
use naga::StorageAccess;
use naga::StorageFormat;
use naga::Type;
use naga::TypeInner;

fn scalar_name(scalar: Scalar) -> Result<&'static str, String> {
    match (scalar.kind, scalar.width) {
        (ScalarKind::Bool, _) => Ok("bool"),
        (ScalarKind::Sint, 4) => Ok("i32"),
        (ScalarKind::Sint, 8) => Ok("i64"),
        (ScalarKind::Uint, 4) => Ok("u32"),
        (ScalarKind::Uint, 8) => Ok("u64"),
        (ScalarKind::Float, 2) => Ok("f16"),
        (ScalarKind::Float, 4) => Ok("f32"),
        (ScalarKind::Float, 8) => Ok("f64"),
        _ => Err("unsupported scalar kind or width".into()),
    }
}

fn access_name(access: StorageAccess) -> Result<&'static str, String> {
    match access.bits() {
        1 => Ok("read"),
        2 => Ok("write"),
        3 => Ok("read_write"),
        4..=7 => Ok("atomic"),
        _ => Err("unsupported storage access".into()),
    }
}

pub(super) fn address_space(space: AddressSpace) -> Result<String, String> {
    Ok(match space {
        AddressSpace::Function => "function".into(),
        AddressSpace::Private => "private".into(),
        AddressSpace::WorkGroup => "workgroup".into(),
        AddressSpace::Uniform => "uniform".into(),
        AddressSpace::Storage { access } => format!("storage,{}", access_name(access)?),
        AddressSpace::Handle => "handle".into(),
        AddressSpace::Immediate => "immediate".into(),
        AddressSpace::TaskPayload => "task_payload".into(),
        AddressSpace::RayPayload => "ray_payload".into(),
        AddressSpace::IncomingRayPayload => "incoming_ray_payload".into(),
    })
}

pub(super) const fn stage_name(stage: ShaderStage) -> &'static str {
    match stage {
        ShaderStage::Vertex => "vertex",
        ShaderStage::Task => "task",
        ShaderStage::Mesh => "mesh",
        ShaderStage::Fragment => "fragment",
        ShaderStage::Compute => "compute",
        ShaderStage::RayGeneration => "ray_generation",
        ShaderStage::Miss => "miss",
        ShaderStage::AnyHit => "any_hit",
        ShaderStage::ClosestHit => "closest_hit",
    }
}

fn array_size(module: &Module, size: ArraySize) -> Result<String, String> {
    match size {
        ArraySize::Constant(count) => Ok(count.to_string()),
        ArraySize::Dynamic => Ok("runtime".into()),
        ArraySize::Pending(handle) => {
            let pending = &module.overrides[handle];
            match (&pending.name, pending.id) {
                (_, Some(id)) => Ok(format!("override({id})")),
                (Some(name), None) => Ok(format!("override({name})")),
                (None, None) => Err("array override has no name or id".into()),
            }
        },
    }
}

fn image_type(dim: ImageDimension, arrayed: bool, class: ImageClass) -> Result<String, String> {
    let dimension = match (dim, arrayed) {
        (ImageDimension::D1, false) => "1d",
        (ImageDimension::D2, false) => "2d",
        (ImageDimension::D2, true) => "2d_array",
        (ImageDimension::D3, false) => "3d",
        (ImageDimension::Cube, false) => "cube",
        (ImageDimension::Cube, true) => "cube_array",
        _ => return Err("unsupported texture dimension".into()),
    };
    match class {
        ImageClass::Sampled { kind, multi } => {
            let scalar = scalar_name(Scalar { kind, width: 4 })?;
            let sampling = if multi { "multisampled_" } else { "" };
            Ok(format!("texture_{sampling}{dimension}<{scalar}>"))
        },
        ImageClass::Depth { multi } => {
            let sampling = if multi { "multisampled_" } else { "" };
            Ok(format!("texture_depth_{sampling}{dimension}"))
        },
        ImageClass::External => Ok("texture_external".into()),
        ImageClass::Storage { format, access } => Ok(format!(
            "texture_storage_{dimension}<{},{}>",
            storage_format(format),
            access_name(access)?
        )),
    }
}

const fn storage_format(format: StorageFormat) -> &'static str {
    match format {
        StorageFormat::R8Unorm => "r8unorm",
        StorageFormat::R8Snorm => "r8snorm",
        StorageFormat::R8Uint => "r8uint",
        StorageFormat::R8Sint => "r8sint",
        StorageFormat::R16Uint => "r16uint",
        StorageFormat::R16Sint => "r16sint",
        StorageFormat::R16Float => "r16float",
        StorageFormat::Rg8Unorm => "rg8unorm",
        StorageFormat::Rg8Snorm => "rg8snorm",
        StorageFormat::Rg8Uint => "rg8uint",
        StorageFormat::Rg8Sint => "rg8sint",
        StorageFormat::R32Uint => "r32uint",
        StorageFormat::R32Sint => "r32sint",
        StorageFormat::R32Float => "r32float",
        StorageFormat::Rg16Uint => "rg16uint",
        StorageFormat::Rg16Sint => "rg16sint",
        StorageFormat::Rg16Float => "rg16float",
        StorageFormat::Rgba8Unorm => "rgba8unorm",
        StorageFormat::Rgba8Snorm => "rgba8snorm",
        StorageFormat::Rgba8Uint => "rgba8uint",
        StorageFormat::Rgba8Sint => "rgba8sint",
        StorageFormat::Bgra8Unorm => "bgra8unorm",
        StorageFormat::Rgb10a2Uint => "rgb10a2uint",
        StorageFormat::Rgb10a2Unorm => "rgb10a2unorm",
        StorageFormat::Rg11b10Ufloat => "rg11b10ufloat",
        StorageFormat::R64Uint => "r64uint",
        StorageFormat::Rg32Uint => "rg32uint",
        StorageFormat::Rg32Sint => "rg32sint",
        StorageFormat::Rg32Float => "rg32float",
        StorageFormat::Rgba16Uint => "rgba16uint",
        StorageFormat::Rgba16Sint => "rgba16sint",
        StorageFormat::Rgba16Float => "rgba16float",
        StorageFormat::Rgba32Uint => "rgba32uint",
        StorageFormat::Rgba32Sint => "rgba32sint",
        StorageFormat::Rgba32Float => "rgba32float",
        StorageFormat::R16Unorm => "r16unorm",
        StorageFormat::R16Snorm => "r16snorm",
        StorageFormat::Rg16Unorm => "rg16unorm",
        StorageFormat::Rg16Snorm => "rg16snorm",
        StorageFormat::Rgba16Unorm => "rgba16unorm",
        StorageFormat::Rgba16Snorm => "rgba16snorm",
    }
}

pub(super) fn binding_type(module: &Module, handle: Handle<Type>) -> Result<String, String> {
    type_name(module, handle).map_err(|error| {
        format!(
            "type {}: {error}",
            module.types[handle].name.as_deref().unwrap_or("unnamed")
        )
    })
}

fn type_name(module: &Module, handle: Handle<Type>) -> Result<String, String> {
    match &module.types[handle].inner {
        TypeInner::Scalar(scalar) => Ok(scalar_name(*scalar)?.into()),
        TypeInner::Vector { size, scalar } => {
            Ok(format!("vec{}<{}>", *size as u8, scalar_name(*scalar)?))
        },
        TypeInner::Matrix {
            columns,
            rows,
            scalar,
        } => Ok(format!(
            "mat{}x{}<{}>",
            *columns as u8,
            *rows as u8,
            scalar_name(*scalar)?
        )),
        TypeInner::Atomic(scalar) => Ok(format!("atomic<{}>", scalar_name(*scalar)?)),
        TypeInner::Array { base, size, stride } => Ok(format!(
            "array<{},{};stride={stride}>",
            binding_type(module, *base)?,
            array_size(module, *size)?
        )),
        TypeInner::BindingArray { base, size } => Ok(format!(
            "binding_array<{},{}>",
            binding_type(module, *base)?,
            array_size(module, *size)?
        )),
        TypeInner::Struct { members, span } => {
            let fields = members
                .iter()
                .map(|member| {
                    Ok(format!(
                        "{}@{}:{}",
                        member.name.as_deref().unwrap_or(""),
                        member.offset,
                        binding_type(module, member.ty)?
                    ))
                })
                .collect::<Result<Vec<_>, String>>()?
                .join(",");
            Ok(format!("struct<span={span};{fields}>"))
        },
        TypeInner::Pointer { base, space } => pointer_type(*space, &binding_type(module, *base)?),
        TypeInner::ValuePointer {
            size,
            scalar,
            space,
        } => {
            let base = match size {
                Some(size) => format!("vec{}<{}>", *size as u8, scalar_name(*scalar)?),
                None => scalar_name(*scalar)?.into(),
            };
            pointer_type(*space, &base)
        },
        TypeInner::Image {
            dim,
            arrayed,
            class,
        } => image_type(*dim, *arrayed, *class),
        TypeInner::Sampler { comparison: false } => Ok("sampler".into()),
        TypeInner::Sampler { comparison: true } => Ok("sampler_comparison".into()),
        TypeInner::CooperativeMatrix { .. } => Err("unsupported cooperative_matrix".into()),
        TypeInner::AccelerationStructure { .. } => Err("unsupported acceleration_structure".into()),
        TypeInner::RayQuery { .. } => Err("unsupported ray_query".into()),
    }
}

fn pointer_type(space: AddressSpace, base: &str) -> Result<String, String> {
    match space {
        AddressSpace::Storage { access } => {
            Ok(format!("ptr<storage,{base},{}>", access_name(access)?))
        },
        _ => Ok(format!("ptr<{},{base}>", address_space(space)?)),
    }
}
