#[repr(C)]
#[derive(Copy, Clone, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
    pub u: f32,
    pub v: f32,
    pub n_x: f32,
    pub n_y: f32,
    pub n_z: f32,
}

impl Vertex {
    pub fn new_position_color_uv_normal(x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, a: f32, u: f32, v: f32, n_x: f32, n_y: f32, n_z: f32) -> Self {
        Self {
            x,
            y,
            z,
            r,
            g,
            b,
            a,
            u,
            v,
            n_x,
            n_y,
            n_z
        }
    }

    pub fn new_position_color_uv(x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, a: f32, u: f32, v: f32) -> Self {
        Vertex::new_position_color_uv_normal(
            x,
            y,
            z,
            r,
            g,
            b,
            a,
            u,
            v,
            0.0,
            0.0,
            0.0,
        )
    }

    pub fn new_position_color(x: f32, y: f32, z: f32, r: f32, g: f32, b: f32, a: f32) -> Self {
        Vertex::new_position_color_uv(
            x,
            y,
            z,
            r,
            g,
            b,
            a,
            0.0,
            0.0,
        )
    }

    pub fn new_position(x: f32, y: f32, z: f32) -> Self {
        Vertex::new_position_color(
            x,
            y,
            z,
            0.0,
            0.0,
            0.0,
            0.0,
        )
    }

    pub fn get_vertex_buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Vertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3]>() as wgpu::BufferAddress,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3 + 4]>() as wgpu::BufferAddress,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: size_of::<[f32; 3 + 4 + 2]>() as wgpu::BufferAddress,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x3,
                }
            ],
        }
    }
}
