//! バインドグループの要素まわりの wgpu 型ラッパー。

use std::num::NonZeroU64;

use crate::texture::{StorageTextureAccess, TextureFormat, TextureSampleType, TextureViewDimension};

pub type BindingId = u32;

pub enum ShaderStages {
    All,
    None,
    Vertex,
    Fragment,
    Compute,
    VertexFragment,
    Task,
    Mesh,
    RayGeneration,
    AnyHit,
    ClosestHit,
    Miss,
}

impl From<ShaderStages> for wgpu::ShaderStages {
    fn from(value: ShaderStages) -> Self {
        match value {
            ShaderStages::All => {
                wgpu::ShaderStages::all()
            }
            ShaderStages::None => {
                wgpu::ShaderStages::NONE
            }
            ShaderStages::Vertex => {
                wgpu::ShaderStages::VERTEX
            }
            ShaderStages::Fragment => {
                wgpu::ShaderStages::FRAGMENT
            }
            ShaderStages::Compute => {
                wgpu::ShaderStages::COMPUTE
            }
            ShaderStages::VertexFragment => {
                wgpu::ShaderStages::VERTEX_FRAGMENT
            }
            ShaderStages::Task => {
                wgpu::ShaderStages::TASK
            }
            ShaderStages::Mesh => {
                wgpu::ShaderStages::MESH
            }
            ShaderStages::RayGeneration => {
                wgpu::ShaderStages::RAY_GENERATION
            }
            ShaderStages::AnyHit => {
                wgpu::ShaderStages::ANY_HIT
            }
            ShaderStages::ClosestHit => {
                wgpu::ShaderStages::CLOSEST_HIT
            }
            ShaderStages::Miss => {
                wgpu::ShaderStages::MISS
            }
        }
    }
}

pub type BufferSize = NonZeroU64;

pub enum BindingType {
    Buffer {
        buffer_binding_type: BufferBindingType,
        has_dynamic_offset: bool,
        min_binding_size: Option<BufferSize>,
    },
    Sampler{
        sampler_binding_type: SamplerBindingType,
    },
    Texture {
        texture_sample_type: TextureSampleType,
        texture_view_dimension: TextureViewDimension,
        multisampled: bool,
    },
    StorageTexture {
        access: StorageTextureAccess,
        format: TextureFormat,
        view_dimension: TextureViewDimension,
    },
    AccelerationStructure {
        vertex_return: bool,
    },
    ExternalTexture {

    },
}

impl From<BindingType> for wgpu::BindingType {
    fn from(value: BindingType) -> Self {
        match value {
            BindingType::Buffer {
                buffer_binding_type,
                has_dynamic_offset,
                min_binding_size,
            } => {
                wgpu::BindingType::Buffer {
                    ty: buffer_binding_type.into(),
                    has_dynamic_offset,
                    min_binding_size,
                }
            }
            BindingType::Sampler{
                sampler_binding_type
            } => {
                wgpu::BindingType::Sampler (
                    sampler_binding_type.into()
                )
            }
            BindingType::Texture {
                texture_sample_type,
                texture_view_dimension,
                multisampled,
            } => {
                wgpu::BindingType::Texture {
                    sample_type: texture_sample_type.into(),
                    view_dimension: texture_view_dimension.into(),
                    multisampled,
                }
            }
            BindingType::StorageTexture {
                access,
                format,
                view_dimension,
            } => {
                wgpu::BindingType::StorageTexture {
                    access: access.into(),
                    format: format.into(),
                    view_dimension: view_dimension.into(),
                }
            }
            BindingType::AccelerationStructure {
                vertex_return,
            } => {
                wgpu::BindingType::AccelerationStructure {
                    vertex_return,
                }
            }
            BindingType::ExternalTexture {} => {
                wgpu::BindingType::ExternalTexture {
                }
            }
        }
    }
}

impl From<wgpu::BindingType> for BindingType {
    fn from(value: wgpu::BindingType) -> Self {
        match value {
            wgpu::BindingType::Buffer {
                ty,
                has_dynamic_offset,
                min_binding_size,
            } => {
                BindingType::Buffer {
                    buffer_binding_type: ty.into(),
                    has_dynamic_offset,
                    min_binding_size,
                }
            }
            wgpu::BindingType::Sampler(sampler_binding_type) => {
                BindingType::Sampler {
                    sampler_binding_type: sampler_binding_type.into(),
                }
            }
            wgpu::BindingType::Texture {
                sample_type,
                view_dimension,
                multisampled,
            } => {
                BindingType::Texture {
                    texture_sample_type: sample_type.into(),
                    texture_view_dimension: view_dimension.into(),
                    multisampled,
                }
            }
            wgpu::BindingType::StorageTexture {
                access,
                format,
                view_dimension,
            } => {
                BindingType::StorageTexture {
                    access: access.into(),
                    format: format.into(),
                    view_dimension: view_dimension.into(),
                }
            }
            wgpu::BindingType::AccelerationStructure {
                vertex_return,
            } => {
                BindingType::AccelerationStructure {
                    vertex_return,
                }
            }
            wgpu::BindingType::ExternalTexture => {
                BindingType::ExternalTexture {
                }
            }
        }
    }
}

pub enum SamplerBindingType {
    Filtering,
    NonFiltering,
    Comparison,
}

impl From<SamplerBindingType> for wgpu::SamplerBindingType {
    fn from(value: SamplerBindingType) -> Self {
        match value {
            SamplerBindingType::Filtering => {
                wgpu::SamplerBindingType::Filtering
            }
            SamplerBindingType::NonFiltering => {
                wgpu::SamplerBindingType::NonFiltering
            }
            SamplerBindingType::Comparison => {
                wgpu::SamplerBindingType::Comparison
            }
        }
    }
}

impl From<wgpu::SamplerBindingType> for SamplerBindingType {
    fn from(value: wgpu::SamplerBindingType) -> Self {
        match value {
            wgpu::SamplerBindingType::Filtering => {
                SamplerBindingType::Filtering
            }
            wgpu::SamplerBindingType::NonFiltering => {
                SamplerBindingType::NonFiltering
            }
            wgpu::SamplerBindingType::Comparison => {
                SamplerBindingType::Comparison
            }
        }
    }
}

pub enum BufferBindingType {
    Uniform,
    Storage{
        read_only: bool
    },
}

impl From<BufferBindingType> for wgpu::BufferBindingType {
    fn from(value: BufferBindingType) -> Self {
        match value {
            BufferBindingType::Uniform => {
                wgpu::BufferBindingType::Uniform
            }
            BufferBindingType::Storage { read_only } => {
                wgpu::BufferBindingType::Storage {read_only}
            }
        }
    }
}

impl From<wgpu::BufferBindingType> for BufferBindingType {
    fn from(value: wgpu::BufferBindingType) -> Self {
        match value {
            wgpu::BufferBindingType::Uniform => {
                BufferBindingType::Uniform
            }
            wgpu::BufferBindingType::Storage { read_only } => {
                BufferBindingType::Storage {read_only}
            }
        }
    }
}
