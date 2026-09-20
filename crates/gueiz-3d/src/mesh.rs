//! 3D の形。頂点と索引の組。
//!
//! # 2D との違い
//!
//! 2D は輪郭を受け取って三角形に開きます（テッセレーション）。3D は
//! **三角形が最初からある**ので、開く工程がありません。代わりに索引を持ちます。
//!
//! 索引がある理由は、3D では 1 つの頂点が何枚もの三角形に共有されるからです。
//! 立方体なら 8 頂点で 12 枚の三角形が作れます。索引を使わないと 36 頂点要ります。

use std::f32::consts::TAU;

/// 3D の頂点。
///
/// 2D の [`gueiz_gpu::vertex::Vertex`] と違い、**法線を実際に使います**。
/// 2D 側はこの分を持っていながら 1 度も読んでいませんでした。
///
/// WGSL のアラインメント規則で `vec3` は 16 バイト境界に揃うので、
/// 並びを合わせるため成分をばらして持つ（2D の頂点プールと同じ手）。
#[repr(C)]
#[derive(Copy, Clone)]
#[derive(PartialEq)]
#[derive(Debug)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex3d {
    pub x: f32,
    pub y: f32,
    pub z: f32,
    pub normal_x: f32,
    pub normal_y: f32,
    pub normal_z: f32,
    pub u: f32,
    pub v: f32,
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Vertex3d {
    pub fn new(
        position: [f32; 3],
        normal: [f32; 3],
        uv: [f32; 2],
        color: [f32; 4],
    ) -> Self {
        Self {
            x: position[0],
            y: position[1],
            z: position[2],
            normal_x: normal[0],
            normal_y: normal[1],
            normal_z: normal[2],
            u: uv[0],
            v: uv[1],
            r: color[0],
            g: color[1],
            b: color[2],
            a: color[3],
        }
    }

    pub fn position(&self) -> [f32; 3] {
        [self.x, self.y, self.z]
    }

    pub fn normal(&self) -> [f32; 3] {
        [self.normal_x, self.normal_y, self.normal_z]
    }

    /// 頂点バッファの並び。2D と違い、**本物の頂点バッファとして読む**。
    ///
    /// 2D は図形ごとに頂点数が違うのでストレージバッファに積んで番号で引いて
    /// いますが、3D は索引付き描画なので `base_vertex` を GPU が足してくれます。
    /// そのぶん素直に書けます。
    pub fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<Self>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 12,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x3,
                },
                wgpu::VertexAttribute {
                    offset: 24,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x2,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// 三角形の集まり。
#[derive(Clone)]
#[derive(Default)]
#[derive(Debug)]
pub struct Mesh {
    vertices: Vec<Vertex3d>,
    indices: Vec<u32>,
}

impl Mesh {
    pub fn new() -> Self {
        Self::default()
    }

    /// 頂点と索引から直接組む。索引は 3 つで 1 枚の三角形。
    pub fn from_parts(vertices: Vec<Vertex3d>, indices: Vec<u32>) -> Self {
        Self { vertices, indices }
    }

    pub fn vertices(&self) -> &[Vertex3d] {
        &self.vertices
    }

    pub fn indices(&self) -> &[u32] {
        &self.indices
    }

    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    pub fn is_empty(&self) -> bool {
        self.indices.is_empty()
    }

    /// ローカル原点からいちばん遠い頂点までの距離。錐台カリングの境界球。
    pub fn bounding_radius(&self) -> f32 {
        self.vertices
            .iter()
            .map(|vertex| {
                (vertex.x * vertex.x + vertex.y * vertex.y + vertex.z * vertex.z).sqrt()
            })
            .fold(0.0_f32, f32::max)
    }

    /// 原点を中心とした 1 辺 `size` の立方体。
    ///
    /// 面ごとに法線が違うので、頂点は共有できず 24 個になる。
    /// 索引が効くのは面の中の 2 枚だけ（4 頂点で 2 枚）。
    pub fn cube(size: f32, color: [f32; 4]) -> Self {
        let half = size * 0.5;

        // (法線, 面の右方向, 面の上方向)
        let faces = [
            ([0.0, 0.0, 1.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([0.0, 0.0, -1.0], [-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
            ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ([-1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
            ([0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
            ([0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]),
        ];

        let mut vertices = Vec::with_capacity(24);
        let mut indices = Vec::with_capacity(36);

        for (normal, right, up) in faces {
            let base = vertices.len() as u32;

            for (corner, uv) in [
                ([-1.0, -1.0], [0.0, 1.0]),
                ([1.0, -1.0], [1.0, 1.0]),
                ([1.0, 1.0], [1.0, 0.0]),
                ([-1.0, 1.0], [0.0, 0.0]),
            ] {
                let position = [
                    (normal[0] + right[0] * corner[0] + up[0] * corner[1]) * half,
                    (normal[1] + right[1] * corner[0] + up[1] * corner[1]) * half,
                    (normal[2] + right[2] * corner[0] + up[2] * corner[1]) * half,
                ];

                vertices.push(Vertex3d::new(position, normal, uv, color));
            }

            indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
        }

        Self { vertices, indices }
    }

    /// 原点を中心とした半径 `radius` の球。経線 `segments` 本、緯線 `rings` 本。
    ///
    /// 球は頂点をよく共有するので、索引の効きがいちばん分かりやすい。
    pub fn sphere(radius: f32, segments: u32, rings: u32, color: [f32; 4]) -> Self {
        let segments = segments.max(3);
        let rings = rings.max(2);

        let mut vertices = Vec::new();
        let mut indices = Vec::new();

        for ring in 0..=rings {
            let phi = ring as f32 / rings as f32 * std::f32::consts::PI;
            let (sin_phi, cos_phi) = phi.sin_cos();

            for segment in 0..=segments {
                let theta = segment as f32 / segments as f32 * TAU;
                let (sin_theta, cos_theta) = theta.sin_cos();

                let normal = [sin_phi * cos_theta, cos_phi, sin_phi * sin_theta];
                let position = [normal[0] * radius, normal[1] * radius, normal[2] * radius];
                let uv = [segment as f32 / segments as f32, ring as f32 / rings as f32];

                vertices.push(Vertex3d::new(position, normal, uv, color));
            }
        }

        let stride = segments + 1;

        for ring in 0..rings {
            for segment in 0..segments {
                let a = ring * stride + segment;
                let b = a + stride;

                indices.extend_from_slice(&[a, b, a + 1, a + 1, b, b + 1]);
            }
        }

        Self { vertices, indices }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 頂点バッファの並びはシェーダと揃っていないといけない。
    #[test]
    fn the_vertex_layout_matches_the_struct() {
        assert_eq!(size_of::<Vertex3d>(), 48);

        let layout = Vertex3d::layout();
        assert_eq!(layout.array_stride, 48);
        assert_eq!(layout.attributes.len(), 4);
        // 位置(12) + 法線(12) + UV(8) + 色(16) = 48
        assert_eq!(layout.attributes[3].offset, 32);
    }

    #[test]
    fn a_cube_has_six_faces() {
        let cube = Mesh::cube(2.0, [1.0; 4]);

        // 面ごとに法線が違うので頂点は共有できない。
        assert_eq!(cube.vertices().len(), 24);
        assert_eq!(cube.triangle_count(), 12);

        // 索引が範囲に収まっていること。外れると GPU が読み違える。
        for &index in cube.indices() {
            assert!((index as usize) < cube.vertices().len());
        }
    }

    /// 1 辺 2 の立方体の角までの距離は sqrt(3)。
    #[test]
    fn the_bounding_radius_reaches_the_corner() {
        let cube = Mesh::cube(2.0, [1.0; 4]);
        let expected = 3.0_f32.sqrt();

        assert!((cube.bounding_radius() - expected).abs() < 1e-5);
    }

    /// 法線は長さ 1。狂うと明るさが変わる。
    #[test]
    fn every_normal_is_a_unit_vector() {
        for mesh in [Mesh::cube(1.0, [1.0; 4]), Mesh::sphere(1.0, 12, 8, [1.0; 4])] {
            for vertex in mesh.vertices() {
                let [x, y, z] = vertex.normal();
                let length = (x * x + y * y + z * z).sqrt();

                assert!((length - 1.0).abs() < 1e-4, "{length}");
            }
        }
    }

    /// 球は頂点をよく共有する。索引の数は頂点の数よりずっと多くなる。
    #[test]
    fn a_sphere_reuses_its_vertices() {
        let sphere = Mesh::sphere(1.0, 16, 8, [1.0; 4]);

        assert_eq!(sphere.triangle_count(), 16 * 8 * 2);
        assert!(
            sphere.indices().len() > sphere.vertices().len() * 2,
            "索引が効いていない",
        );

        for &index in sphere.indices() {
            assert!((index as usize) < sphere.vertices().len());
        }
    }

    /// 球の頂点はすべて半径の上にある。
    #[test]
    fn a_sphere_keeps_its_radius() {
        let sphere = Mesh::sphere(2.5, 20, 12, [1.0; 4]);

        for vertex in sphere.vertices() {
            let [x, y, z] = vertex.position();
            let distance = (x * x + y * y + z * z).sqrt();

            assert!((distance - 2.5).abs() < 1e-4, "{distance}");
        }
    }
}
