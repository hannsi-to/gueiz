//! ラスタライザと深度・ステンシルの設定まわりの wgpu 型ラッパー。

use crate::texture::TextureFormat;

#[derive(Copy, Clone)]
pub enum PrimitiveTopology {
    PointList = 0,
    LineList = 1,
    LineStrip = 2,
    TriangleList = 3,
    TriangleStrip = 4,
}

impl PrimitiveTopology {
    pub fn is_strip(&self) -> bool {
        let wgpu_primitive_topology: wgpu::PrimitiveTopology = (*self).into();
        wgpu_primitive_topology.is_strip()
    }

    pub fn is_triangles(&self) -> bool {
        let wgpu_primitive_topology: wgpu::PrimitiveTopology = (*self).into();
        wgpu_primitive_topology.is_triangles()
    }
}

impl From<PrimitiveTopology> for wgpu::PrimitiveTopology {
    fn from(value: PrimitiveTopology) -> Self {
        match value {
            PrimitiveTopology::PointList => {
                wgpu::PrimitiveTopology::PointList
            }
            PrimitiveTopology::LineList => {
                wgpu::PrimitiveTopology::LineList
            }
            PrimitiveTopology::LineStrip => {
                wgpu::PrimitiveTopology::LineStrip
            }
            PrimitiveTopology::TriangleList => {
                wgpu::PrimitiveTopology::TriangleList
            }
            PrimitiveTopology::TriangleStrip => {
                wgpu::PrimitiveTopology::TriangleStrip
            }
        }
    }
}

impl From<wgpu::PrimitiveTopology> for PrimitiveTopology {
    fn from(value: wgpu::PrimitiveTopology) -> Self {
        match value {
            wgpu::PrimitiveTopology::PointList => {
                PrimitiveTopology::PointList
            }
            wgpu::PrimitiveTopology::LineList => {
                PrimitiveTopology::LineList
            }
            wgpu::PrimitiveTopology::LineStrip => {
                PrimitiveTopology::LineStrip
            }
            wgpu::PrimitiveTopology::TriangleList => {
                PrimitiveTopology::TriangleList
            }
            wgpu::PrimitiveTopology::TriangleStrip => {
                PrimitiveTopology::TriangleStrip
            }
        }
    }
}

pub enum IndexFormat {
    Uint16 = 0,
    Uint32 = 1,
}

impl From<IndexFormat> for wgpu::IndexFormat {
    fn from(value: IndexFormat) -> Self {
        match value {
            IndexFormat::Uint16 => {
                wgpu::IndexFormat::Uint16
            }
            IndexFormat::Uint32 => {
                wgpu::IndexFormat::Uint32
            }
        }
    }
}

impl From<wgpu::IndexFormat> for IndexFormat {
    fn from(value: wgpu::IndexFormat) -> Self {
        match value {
            wgpu::IndexFormat::Uint16 => {
                IndexFormat::Uint16
            }
            wgpu::IndexFormat::Uint32 => {
                IndexFormat::Uint32
            }
        }
    }
}

pub enum FrontFace {
    Ccw = 0,
    Cw = 1,
}

impl From<wgpu::FrontFace> for FrontFace {
    fn from(value: wgpu::FrontFace) -> Self {
        match value {
            wgpu::FrontFace::Ccw => {
                FrontFace::Ccw
            }
            wgpu::FrontFace::Cw => {
                FrontFace::Cw
            }
        }
    }
}

pub enum Face {
    Front = 0,
    Back = 1,
}

impl From<Face> for wgpu::Face {
    fn from(value: Face) -> Self {
        match value {
            Face::Front => {
                wgpu::Face::Front
            }
            Face::Back => {
                wgpu::Face::Back
            }
        }
    }
}

impl From<wgpu::Face> for Face {
    fn from(value: wgpu::Face) -> Self {
        match value {
            wgpu::Face::Front => {
                Face::Front
            }
            wgpu::Face::Back => {
                Face::Back
            }
        }
    }
}

pub enum PolygonMode {
    Fill = 0,
    Line = 1,
    Point = 2,
}

impl From<PolygonMode> for wgpu::PolygonMode {
    fn from(value: PolygonMode) -> Self {
        match value {
            PolygonMode::Fill => {
                wgpu::PolygonMode::Fill
            }
            PolygonMode::Line => {
                wgpu::PolygonMode::Line
            }
            PolygonMode::Point => {
                wgpu::PolygonMode::Point
            }
        }
    }
}

impl From<wgpu::PolygonMode> for PolygonMode {
    fn from(value: wgpu::PolygonMode) -> Self {
        match value {
            wgpu::PolygonMode::Fill => {
                PolygonMode::Fill
            }
            wgpu::PolygonMode::Line => {
                PolygonMode::Line
            }
            wgpu::PolygonMode::Point => {
                PolygonMode::Point
            }
        }
    }
}

pub struct PrimitiveState {
    pub primitive_topology: PrimitiveTopology,
    pub strip_index_format: Option<IndexFormat>,
    pub front_face: FrontFace,
    pub cull_mode: Option<Face>,
    pub unclipped_depth: bool,
    pub polygon_mode: PolygonMode,
    pub conservative: bool,
}

pub type ImmediateSize = u32;

pub struct MultisampleState {
    pub count: u32,
    pub mask: u64,
    pub alpha_to_coverage_enabled: bool,
}

pub enum CompareFunction {
    Never = 0,
    Less = 1,
    Equal = 2,
    LessEqual = 3,
    Greater = 4,
    NotEqual = 5,
    GreaterEqual = 6,
    Always = 7,
}

impl From<wgpu::CompareFunction> for CompareFunction {
    fn from(value: wgpu::CompareFunction) -> Self {
        match value {
            wgpu::CompareFunction::Never => {
                CompareFunction::Never
            }
            wgpu::CompareFunction::Less => {
                CompareFunction::Less
            }
            wgpu::CompareFunction::Equal => {
                CompareFunction::Equal
            }
            wgpu::CompareFunction::LessEqual => {
                CompareFunction::LessEqual
            }
            wgpu::CompareFunction::Greater => {
                CompareFunction::Greater
            }
            wgpu::CompareFunction::NotEqual => {
                CompareFunction::NotEqual
            }
            wgpu::CompareFunction::GreaterEqual => {
                CompareFunction::GreaterEqual
            }
            wgpu::CompareFunction::Always => {
                CompareFunction::Always
            }
        }
    }
}

impl From<CompareFunction> for wgpu::CompareFunction {
    fn from(value: CompareFunction) -> Self {
        match value {
            CompareFunction::Never => {
                wgpu::CompareFunction::Never
            }
            CompareFunction::Less => {
                wgpu::CompareFunction::Less
            }
            CompareFunction::Equal => {
                wgpu::CompareFunction::Equal
            }
            CompareFunction::LessEqual => {
                wgpu::CompareFunction::LessEqual
            }
            CompareFunction::Greater => {
                wgpu::CompareFunction::Greater
            }
            CompareFunction::NotEqual => {
                wgpu::CompareFunction::NotEqual
            }
            CompareFunction::GreaterEqual => {
                wgpu::CompareFunction::GreaterEqual
            }
            CompareFunction::Always => {
                wgpu::CompareFunction::Always
            }
        }
    }
}

pub struct StencilFaceState {
    pub compare: CompareFunction,
    pub fail_op: StencilOperation,
    pub depth_fail_op: StencilOperation,
    pub pass_op: StencilOperation,
}

pub enum StencilOperation {
    Keep = 0,
    Zero = 1,
    Replace = 2,
    Invert = 3,
    IncrementClamp = 4,
    DecrementClamp = 5,
    IncrementWrap = 6,
    DecrementWrap = 7,
}

impl From<wgpu::StencilOperation> for StencilOperation {
    fn from(value: wgpu::StencilOperation) -> Self {
        match value {
            wgpu::StencilOperation::Keep => {
                StencilOperation::Keep
            }
            wgpu::StencilOperation::Zero => {
                StencilOperation::Zero
            }
            wgpu::StencilOperation::Replace => {
                StencilOperation::Replace
            }
            wgpu::StencilOperation::Invert => {
                StencilOperation::Invert
            }
            wgpu::StencilOperation::IncrementClamp => {
                StencilOperation::IncrementClamp
            }
            wgpu::StencilOperation::DecrementClamp => {
                StencilOperation::DecrementClamp
            }
            wgpu::StencilOperation::IncrementWrap => {
                StencilOperation::IncrementWrap
            }
            wgpu::StencilOperation::DecrementWrap => {
                StencilOperation::DecrementWrap
            }
        }
    }
}

impl From<StencilOperation> for wgpu::StencilOperation {
    fn from(value: StencilOperation) -> Self {
        match value {
            StencilOperation::Keep => {
                wgpu::StencilOperation::Keep
            }
            StencilOperation::Zero => {
                wgpu::StencilOperation::Zero
            }
            StencilOperation::Replace => {
                wgpu::StencilOperation::Replace
            }
            StencilOperation::Invert => {
                wgpu::StencilOperation::Invert
            }
            StencilOperation::IncrementClamp => {
                wgpu::StencilOperation::IncrementClamp
            }
            StencilOperation::DecrementClamp => {
                wgpu::StencilOperation::DecrementClamp
            }
            StencilOperation::IncrementWrap => {
                wgpu::StencilOperation::IncrementWrap
            }
            StencilOperation::DecrementWrap => {
                wgpu::StencilOperation::DecrementWrap
            }
        }
    }
}

pub struct StencilState {
    pub front: StencilFaceState,
    pub back: StencilFaceState,
    pub read_mask: u32,
    pub write_mask: u32,
}

pub struct DepthStencilState {
    pub format: TextureFormat,
    pub depth_write_enabled: Option<bool>,
    pub depth_compare: Option<CompareFunction>,
    pub stencil: StencilState,
}
