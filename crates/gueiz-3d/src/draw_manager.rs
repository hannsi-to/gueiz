//! 登録したメッシュを GPU-driven + インダイレクト描画でまとめて描く。
//!
//! # 流れ
//!
//! 1. CPU はメッシュと複製の素データ（TRS と色）を上げる。行列は計算しない。
//! 2. コンピュートパスが複製ごとに 1 スレッド走り、
//!    - 世界行列を組み立て（`object * instance`）
//!    - **世界空間で**境界球を錐台の 6 面と比べて捨て
//!    - 生き残りを詰めて出力バッファに書き
//!    - `DrawIndexedIndirectArgs.instance_count` を atomicAdd で増やす
//! 3. `multi_draw_indexed_indirect` が、GPU が書いた引数どおりに描く。
//!
//! # 2D と本当に違うところ
//!
//! 素直に書いた結果、2D と違ったのは 4 点でした。どれもパラメータでは
//! 吸収できず、ここを共通化しようとすると両方が歪みます。
//!
//! | | 2D | 3D |
//! |---|---|---|
//! | 頂点の渡し方 | ストレージバッファに積んで番号で引く | **本物の頂点バッファ + 索引** |
//! | カリング | 境界円 × クリップ矩形（畳んだ行列で） | **境界球 × 錐台 6 面（世界空間で）** |
//! | 奥行き | 深度バッファ無し。z は並べ替えの鍵 | **深度バッファ。z は奥行き** |
//! | 行列の畳み方 | `camera * object` を CPU で 1 本に | **世界行列とカメラを分ける** |
//!
//! 頂点の渡し方が違うのは、索引付き描画だと `base_vertex` を GPU が足して
//! くれるからです。2D は図形ごとに頂点数が違う非索引描画なので、番号で
//! 引くしかありませんでした。
//!
//! 行列を分けるのは、錐台カリングが世界座標を要るからです。2D のように
//! 畳むと、カリングに使う座標が取り出せません。

use std::collections::HashMap;

use gueiz_gpu::buffer::{Allocation, BufferHeap, BufferHeapDescriptor};
use gueiz_gpu::error::Gueiz2DError as GpuError;
use gueiz_gpu::instance::{InstanceSource, InstanceUploader};
use gueiz_gpu::msaa::NO_MULTISAMPLE;
use gueiz_gpu::pool::{Pool, PoolSource};
use gueiz_gpu::texture::TextureFormat;

use crate::camera::Camera3d;
use crate::mesh::Vertex3d;
use crate::object::{Instance3d, Object3d};

/// コンピュートシェーダのワークグループサイズ。シェーダ側と揃える。
const CULL_WORKGROUP_SIZE: u32 = 64;

const DEFAULT_MAX_VERTICES: u32 = 256 * 1024;
const DEFAULT_MAX_INDICES: u32 = 512 * 1024;
const DEFAULT_MAX_OBJECTS: u32 = 1024;
const DEFAULT_MAX_INSTANCES: u32 = 64 * 1024;

/// 深度バッファの形式。3D にはこれが要る。2D には無い。
pub const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// [`DrawManager3d`] の作成パラメータ。
pub struct DrawManager3dDescriptor {
    pub max_vertices: u32,
    pub max_indices: u32,
    pub max_objects: u32,
    pub max_instances: u32,
    /// 錐台の外の複製を GPU で捨てる。
    pub culling: bool,
    /// 縁のギザギザを均す点の数。1 で均さない、4 が無難。
    ///
    /// **色の描き先と深度バッファの両方**が同じ数でなければならない。
    /// 深度は [`DrawManager3d`] が持っているので、こちらで合わせる。
    pub sample_count: u32,
}

impl Default for DrawManager3dDescriptor {
    fn default() -> Self {
        Self {
            max_vertices: DEFAULT_MAX_VERTICES,
            max_indices: DEFAULT_MAX_INDICES,
            max_objects: DEFAULT_MAX_OBJECTS,
            max_instances: DEFAULT_MAX_INSTANCES,
            culling: true,
            sample_count: NO_MULTISAMPLE,
        }
    }
}

/// メッシュ 1 つぶんの記述。コンピュートシェーダが読む。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct ObjectRaw {
    /// 世界へ写す行列。**カメラは掛かっていない。**
    world: [[f32; 4]; 4],
    instance_base: u32,
    instance_count: u32,
    /// 世界空間での境界球の半径（図形の拡大まで込み）。
    bounding_radius: f32,
    _padding: u32,
}

/// 複製の素データ。行列にする前の TRS と色。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceIn {
    translation: [f32; 3],
    _padding0: f32,
    rotation: [f32; 3],
    _padding1: f32,
    scale: [f32; 3],
    _padding2: f32,
    tint: [f32; 4],
}

/// コンピュートパスが書き出す、生き残った複製。頂点バッファとして読む。
///
/// 2D と違い**世界行列**を持つ。カメラは描画側のユニフォームから掛ける。
/// こうしないと、法線を世界空間へ回せない。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct InstanceOut {
    world: [[f32; 4]; 4],
    tint: [f32; 4],
}

impl InstanceOut {
    fn layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: size_of::<InstanceOut>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                // mat4 は vec4 4 本に割って渡す。
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 4,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 16,
                    shader_location: 5,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 32,
                    shader_location: 6,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 48,
                    shader_location: 7,
                    format: wgpu::VertexFormat::Float32x4,
                },
                wgpu::VertexAttribute {
                    offset: 64,
                    shader_location: 8,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }
}

/// カメラとライト。コンピュートと描画の両方が読む。
#[repr(C)]
#[derive(Clone, Copy)]
#[derive(bytemuck::Pod, bytemuck::Zeroable)]
struct SceneUniform {
    view_projection: [[f32; 4]; 4],
    /// 錐台の 6 面。世界空間で、内側が正。
    frustum: [[f32; 4]; 6],
    /// 平行光の向き（光が進む向きの逆）。
    light_direction: [f32; 4],
    /// 影の側の明るさ。
    ambient: [f32; 4],
    object_count: u32,
    culling: u32,
    _padding: [u32; 2],
}

/// メッシュから測っておく値。置き場の持ち分（[`Pool`]）とは別に持つ。
///
/// `base_vertex` も `first_index` も 2 本の [`Pool`] が覚えているので、
/// ここには**3D 固有のぶんだけ**残る。
#[derive(Clone, Copy)]
#[derive(Debug, Default)]
struct MeshMetrics {
    /// ローカル原点からいちばん遠い頂点までの距離。錐台カリングの境界球。
    bounding_radius: f32,
}

/// 登録したメッシュをまとめて描く。
pub struct DrawManager3d {
    render_pipeline: wgpu::RenderPipeline,
    cull_pipeline: wgpu::ComputePipeline,
    render_bind_group: wgpu::BindGroup,
    cull_bind_group: wgpu::BindGroup,

    vertex_heap: BufferHeap,
    index_heap: BufferHeap,
    object_heap: BufferHeap,
    instance_input_heap: BufferHeap,
    instance_output_heap: BufferHeap,
    indirect_heap: BufferHeap,
    scene_buffer: wgpu::Buffer,

    vertices: Allocation,
    indices: Allocation,
    objects_allocation: Allocation,
    instance_input: Allocation,
    instance_output: Allocation,
    indirect: Allocation,

    depth: Option<DepthBuffer>,

    objects: Vec<Object3d>,
    /// 名前 → 番号。名前はここでひとつに保たれる。
    names: HashMap<String, usize>,
    /// 全メッシュの頂点と索引をつないだ置き場。**2D と共通の仕組み**。
    /// 2D は 1 本だが、索引付き描画なので 3D は 2 本使う。
    vertex_pool: Pool<Vertex3d>,
    index_pool: Pool<u32>,
    mesh_metrics: Vec<MeshMetrics>,
    /// 複製の持ち分と、変わったぶんだけの送り直し。**2D と共通の仕組み**。
    instance_uploader: InstanceUploader<InstanceIn>,
    meshes_dirty: bool,

    camera: Camera3d,
    light_direction: [f32; 3],
    ambient: f32,
    culling: bool,
    max_objects: u32,
    sample_count: u32,

    object_scratch: Vec<ObjectRaw>,
    indirect_scratch: Vec<wgpu::util::DrawIndexedIndirectArgs>,

    draw_count: u32,
}

/// 深度バッファ。大きさが変わったら作り直す。
struct DepthBuffer {
    view: wgpu::TextureView,
    width: u32,
    height: u32,
}

impl DrawManager3d {
    pub fn new(
        device: &wgpu::Device,
        surface_format: TextureFormat,
        descriptor: &DrawManager3dDescriptor,
    ) -> Result<Self, GpuError> {
        let mut vertex_heap = heap(
            device,
            "gueiz3d vertices",
            descriptor.max_vertices as u64 * size_of::<Vertex3d>() as u64,
            wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        );
        let vertices = vertex_heap.allocate(vertex_heap.size())?;

        let mut index_heap = heap(
            device,
            "gueiz3d indices",
            descriptor.max_indices as u64 * size_of::<u32>() as u64,
            wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
        );
        let indices = index_heap.allocate(index_heap.size())?;

        let mut object_heap = heap(
            device,
            "gueiz3d objects",
            descriptor.max_objects as u64 * size_of::<ObjectRaw>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let objects_allocation = object_heap.allocate(object_heap.size())?;

        let mut instance_input_heap = heap(
            device,
            "gueiz3d instance input",
            descriptor.max_instances as u64 * size_of::<InstanceIn>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST,
        );
        let instance_input = instance_input_heap.allocate(instance_input_heap.size())?;

        let mut instance_output_heap = heap(
            device,
            "gueiz3d instance output",
            descriptor.max_instances as u64 * size_of::<InstanceOut>() as u64,
            wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::VERTEX,
        );
        let instance_output = instance_output_heap.allocate(instance_output_heap.size())?;

        let mut indirect_heap = heap(
            device,
            "gueiz3d indirect args",
            descriptor.max_objects as u64
                * size_of::<wgpu::util::DrawIndexedIndirectArgs>() as u64,
            wgpu::BufferUsages::STORAGE
                | wgpu::BufferUsages::INDIRECT
                | wgpu::BufferUsages::COPY_DST
                | wgpu::BufferUsages::COPY_SRC,
        );
        let indirect = indirect_heap.allocate(indirect_heap.size())?;

        let scene_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("gueiz3d scene"),
            size: size_of::<SceneUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // --- コンピュート側 ---
        let cull_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gueiz3d cull shader"),
            source: wgpu::ShaderSource::Wgsl(CULL_SHADER.into()),
        });

        let cull_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gueiz3d cull layout"),
            entries: &[
                storage_entry(0, true),
                storage_entry(1, true),
                storage_entry(2, false),
                storage_entry(3, false),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let cull_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gueiz3d cull bind group"),
            layout: &cull_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: object_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: instance_input_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: instance_output_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: indirect_heap.buffer().as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: scene_buffer.as_entire_binding(),
                },
            ],
        });

        let cull_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gueiz3d cull pipeline layout"),
                bind_group_layouts: &[Some(&cull_layout)],
                immediate_size: 0,
            });

        let cull_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("gueiz3d cull pipeline"),
            layout: Some(&cull_pipeline_layout),
            module: &cull_shader,
            entry_point: Some("cull_main"),
            compilation_options: Default::default(),
            cache: None,
        });

        // --- 描画側 ---
        let render_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gueiz3d mesh shader"),
            source: wgpu::ShaderSource::Wgsl(RENDER_SHADER.into()),
        });

        let render_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gueiz3d scene layout"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });

        let render_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gueiz3d scene bind group"),
            layout: &render_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: scene_buffer.as_entire_binding(),
            }],
        });

        let render_pipeline_layout =
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gueiz3d mesh pipeline layout"),
                bind_group_layouts: &[Some(&render_layout)],
                immediate_size: 0,
            });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gueiz3d mesh pipeline"),
            layout: Some(&render_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &render_shader,
                entry_point: Some("vs_main"),
                // スロット 0 = メッシュの頂点、スロット 1 = 生き残った複製。
                buffers: &[Some(Vertex3d::layout()), Some(InstanceOut::layout())],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                // 3D は裏面を捨てる。2D には表裏が無いので設定していない。
                cull_mode: Some(wgpu::Face::Back),
                ..Default::default()
            },
            // ここが 2D といちばん違う。奥行きを深度で解く。
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH_FORMAT,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Less),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: descriptor.sample_count.max(1),
                ..Default::default()
            },
            fragment: Some(wgpu::FragmentState {
                module: &render_shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format.into(),
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        log::info!(
            "3d draw manager: GPU-driven, culling {}, up to {} objects / {} instances",
            if descriptor.culling { "on" } else { "off" },
            descriptor.max_objects,
            descriptor.max_instances,
        );

        Ok(Self {
            render_pipeline,
            cull_pipeline,
            render_bind_group,
            cull_bind_group,
            vertex_heap,
            index_heap,
            object_heap,
            instance_input_heap,
            instance_output_heap,
            indirect_heap,
            scene_buffer,
            vertices,
            indices,
            objects_allocation,
            instance_input,
            instance_output,
            indirect,
            depth: None,
            objects: Vec::new(),
            names: HashMap::new(),
            vertex_pool: Pool::new(),
            index_pool: Pool::new(),
            mesh_metrics: Vec::new(),
            instance_uploader: InstanceUploader::new(descriptor.max_instances),
            meshes_dirty: true,
            camera: Camera3d::default(),
            light_direction: [-0.4, -1.0, -0.6],
            ambient: 0.25,
            culling: descriptor.culling,
            max_objects: descriptor.max_objects,
            sample_count: descriptor.sample_count.max(1),
            object_scratch: Vec::new(),
            indirect_scratch: Vec::new(),
            draw_count: 0,
        })
    }

    /// メッシュを預ける。**返るのは、実際に付いた名前。**
    ///
    /// 渡した名前がすでに使われていたら後ろに数が足される
    /// （`Cube` → `Cube 2`）ので、返ってきたほうを使ってください。
    /// 以降の書き換えは [`DrawManager3d::object_mut`] から。
    pub fn register(&mut self, mut object: Object3d) -> String {
        if self.objects.len() as u32 >= self.max_objects {
            log::warn!(
                "the object limit ({}) is reached; '{}' will not be drawn",
                self.max_objects,
                object.name(),
            );
        }

        let index = self.objects.len();

        // 名前はここでひとつに保つ。重なっていたら後ろに数を足す。
        if self.names.contains_key(object.name()) {
            let unique = self.unique_name(object.name());

            log::warn!(
                "two objects are called '{}'; the new one is now '{}'",
                object.name(),
                unique,
            );

            object.rename(unique);
        }

        let name = String::from(object.name());
        self.names.insert(name.clone(), index);

        self.objects.push(object);
        self.mesh_metrics.push(MeshMetrics::default());
        self.meshes_dirty = true;
        self.instance_uploader.invalidate_layout();

        name
    }

    /// まだ使われていない名前を作る。`Cube` → `Cube 2` → `Cube 3`。
    fn unique_name(&self, wanted: &str) -> String {
        unique_name(wanted, |candidate| self.names.contains_key(candidate))
    }

    /// 名前で引く。名前は [`DrawManager3d::register`] がひとつに保つので、
    /// **必ずメッシュひとつに決まります。**
    pub fn object(&self, name: &str) -> Option<&Object3d> {
        self.objects.get(*self.names.get(name)?)
    }

    /// 名前で引いて書き換える。
    pub fn object_mut(&mut self, name: &str) -> Option<&mut Object3d> {
        let index = *self.names.get(name)?;

        self.objects.get_mut(index)
    }

    /// その名前のメッシュがあるか。
    pub fn contains(&self, name: &str) -> bool {
        self.names.contains_key(name)
    }

    /// 預かっている名前を全部。順番は決まっていない。
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.names.keys().map(String::as_str)
    }

    pub fn object_count(&self) -> usize {
        self.objects.len()
    }

    pub fn camera(&self) -> &Camera3d {
        &self.camera
    }

    pub fn set_camera(&mut self, camera: Camera3d) {
        self.camera = camera;
    }

    /// 平行光の向き。光が**進む**向き。
    pub fn set_light(&mut self, direction: [f32; 3], ambient: f32) {
        self.light_direction = direction;
        self.ambient = ambient;
    }

    /// 錐台の外の複製を GPU で捨てるか。
    pub fn set_culling(&mut self, culling: bool) {
        self.culling = culling;
    }

    pub fn draw_count(&self) -> u32 {
        self.draw_count
    }

    /// GPU が書いたインダイレクト引数。読み戻して生き残り数を見るときに使う。
    pub fn indirect_buffer(&self) -> &wgpu::Buffer {
        self.indirect_heap.buffer()
    }

    pub fn indirect_offset(&self) -> u64 {
        self.indirect.offset()
    }

    /// メッシュが変わっていれば積み直し、コンピュートパスを投げる。
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) -> Result<(), GpuError> {
        if self.meshes_dirty || self.objects.iter().any(Object3d::is_mesh_dirty) {
            self.rebuild_meshes(queue)?;
        }

        self.upload_frame(queue);

        if self.draw_count == 0 {
            return Ok(());
        }

        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gueiz3d cull"),
        });

        {
            let mut cull_pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("gueiz3d cull pass"),
                timestamp_writes: None,
            });

            cull_pass.set_pipeline(&self.cull_pipeline);
            cull_pass.set_bind_group(0, &self.cull_bind_group, &[]);

            let groups_x = self
                .instance_uploader
                .max_instances_per_object()
                .div_ceil(CULL_WORKGROUP_SIZE);
            cull_pass.dispatch_workgroups(groups_x.max(1), self.draw_count, 1);
        }

        queue.submit(Some(encoder.finish()));

        Ok(())
    }

    /// 深度バッファを用意する。大きさが変わっていたら作り直す。
    pub fn depth_view(
        &mut self,
        device: &wgpu::Device,
        width: u32,
        height: u32,
    ) -> &wgpu::TextureView {
        let fits = self
            .depth
            .as_ref()
            .is_some_and(|depth| depth.width == width && depth.height == height);

        if !fits {
            let texture = device.create_texture(&wgpu::TextureDescriptor {
                label: Some("gueiz3d depth"),
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                // 色の描き先と同じ数でないと wgpu が弾く。
                sample_count: self.sample_count,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH_FORMAT,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            });

            log::debug!("3d depth buffer: {width}x{height}");

            self.depth = Some(DepthBuffer {
                view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
                width,
                height,
            });
        }

        &self.depth.as_ref().expect("すぐ上で用意した").view
    }

    /// 場面を描く。**深度が要るのでパスごと組む。**
    ///
    /// 2D は `draw(&mut RenderPass)` で呼び出し側のパスに乗るが、3D は
    /// 深度アタッチメントが要るので、パスの形が決まってしまう。
    pub fn draw(
        &mut self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        color_view: &wgpu::TextureView,
        width: u32,
        height: u32,
        clear_color: wgpu::Color,
    ) {
        // 借用が重なるので、先に深度を用意して取り出す。
        let depth_view = self.depth_view(device, width, height).clone();

        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gueiz3d render pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: color_view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &depth_view,
                depth_ops: Some(wgpu::Operations {
                    // 手前が小さい値なので、いちばん奥で消す。
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            ..Default::default()
        });

        if self.draw_count == 0 {
            return;
        }

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(0, &self.render_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_heap.slice(&self.vertices));
        render_pass.set_vertex_buffer(1, self.instance_output_heap.slice(&self.instance_output));
        render_pass.set_index_buffer(
            self.index_heap.slice(&self.indices),
            wgpu::IndexFormat::Uint32,
        );
        render_pass.multi_draw_indexed_indirect(
            self.indirect_heap.buffer(),
            self.indirect.offset(),
            self.draw_count,
        );
    }

    /// 全メッシュの頂点と索引を 1 本ずつにつないで積み直す。
    ///
    /// つなぐところは共通の [`Pool`] に任せ、ここは 3D 固有の
    /// [`MeshMetrics`]（境界球）だけを測る。
    fn rebuild_meshes(&mut self, queue: &wgpu::Queue) -> Result<(), GpuError> {
        {
            let vertices = MeshVertices(&self.objects);
            self.vertex_pool.rebuild(&vertices);

            let indices = MeshIndices(&self.objects);
            self.index_pool.rebuild(&indices);
        }

        self.mesh_metrics.resize(self.objects.len(), MeshMetrics::default());

        for (index, object) in self.objects.iter_mut().enumerate() {
            self.mesh_metrics[index] = MeshMetrics {
                bounding_radius: object.mesh().bounding_radius(),
            };

            object.clear_mesh_dirty();
        }

        self.vertex_pool
            .upload(queue, &self.vertex_heap, &self.vertices)?;
        self.index_pool
            .upload(queue, &self.index_heap, &self.indices)?;

        self.meshes_dirty = false;

        log::debug!(
            "3d meshes: {} objects, {} vertices, {} indices",
            self.objects.len(),
            self.vertex_pool.len(),
            self.index_pool.len(),
        );

        Ok(())
    }

    /// 今フレームの記述・複製・インダイレクト引数・場面の設定を上げる。
    ///
    /// 複製の送り方は **2D とまったく同じ仕組み**にしてある。並びが変わらない
    /// フレームでは、書き換えられた図形の持ち分だけを送り直す。
    /// ここは引き上げられる（そうと分かるのが、素直に 2 度書いてみた収穫）。
    fn upload_frame(&mut self, queue: &wgpu::Queue) {
        let object_limit = self.objects.len().min(self.max_objects as usize);

        // 複製を送る。並びが変わっていなければ、書き換えられた図形のぶんだけ。
        // 仕組みは 2D と共通（[`InstanceUploader`]）。
        {
            let mut source = Instances(&mut self.objects[..object_limit]);
            self.instance_uploader.upload(
                queue,
                self.instance_input_heap.buffer(),
                self.instance_input.offset(),
                &mut source,
            );
        }

        self.object_scratch.clear();
        self.indirect_scratch.clear();

        for index in 0..object_limit {
            // 索引の持ち分がそのまま `first_index` / `index_count`、
            // 頂点の持ち分の先頭がそのまま `base_vertex` になる。
            let vertex_range = self.vertex_pool.range(index);
            let index_range = self.index_pool.range(index);
            let metrics = self.mesh_metrics[index];
            let instance_range = self.instance_uploader.range(index);
            let object = &self.objects[index];

            self.object_scratch.push(ObjectRaw {
                world: object.world_transform().to_columns(),
                instance_base: instance_range.base,
                instance_count: instance_range.count,
                // 図形の拡大まで込みの半径。複製側の拡大はシェーダで掛ける。
                bounding_radius: metrics.bounding_radius * object.max_scale(),
                _padding: 0,
            });

            self.indirect_scratch.push(wgpu::util::DrawIndexedIndirectArgs {
                index_count: index_range.count,
                instance_count: 0,
                first_index: index_range.base,
                // 索引はメッシュの中の番号のまま積んであるので、GPU がここを足す。
                base_vertex: vertex_range.base as i32,
                first_instance: instance_range.base,
            });
        }

        self.draw_count = self.object_scratch.len() as u32;

        if self.draw_count == 0 {
            return;
        }

        queue.write_buffer(
            self.object_heap.buffer(),
            self.objects_allocation.offset(),
            bytemuck::cast_slice(&self.object_scratch),
        );
        queue.write_buffer(
            self.indirect_heap.buffer(),
            self.indirect.offset(),
            indexed_args_bytes(&self.indirect_scratch),
        );

        let [lx, ly, lz] = self.light_direction;

        queue.write_buffer(
            &self.scene_buffer,
            0,
            bytemuck::bytes_of(&SceneUniform {
                view_projection: self.camera.view_projection().to_columns(),
                frustum: self.camera.frustum_planes(),
                light_direction: [lx, ly, lz, 0.0],
                ambient: [self.ambient; 4],
                object_count: self.draw_count,
                culling: u32::from(self.culling),
                _padding: [0; 2],
            }),
        );
    }
}

/// 頂点の積み元。[`Pool`] が覗くだけの薄い型。
struct MeshVertices<'a>(&'a [Object3d]);

impl PoolSource for MeshVertices<'_> {
    type Item = Vertex3d;

    fn object_count(&self) -> usize {
        self.0.len()
    }

    fn elements(&self, object: usize) -> &[Vertex3d] {
        self.0[object].mesh().vertices()
    }
}

/// 索引の積み元。頂点とは別の置き場になる。
struct MeshIndices<'a>(&'a [Object3d]);

impl PoolSource for MeshIndices<'_> {
    type Item = u32;

    fn object_count(&self) -> usize {
        self.0.len()
    }

    fn elements(&self, object: usize) -> &[u32] {
        self.0[object].mesh().indices()
    }
}

/// 複製の送り元。[`InstanceUploader`] が覗くだけの薄い型。
///
/// 2D 側とまったく同じ形。中で作る素データの型だけが違う。
struct Instances<'a>(&'a mut [Object3d]);

impl InstanceSource for Instances<'_> {
    type Raw = InstanceIn;

    fn object_count(&self) -> usize {
        self.0.len()
    }

    fn instance_count(&self, object: usize) -> usize {
        self.0[object].instances().len()
    }

    fn is_dirty(&self, object: usize) -> bool {
        self.0[object].is_instances_dirty()
    }

    fn write(&self, object: usize, count: usize, out: &mut Vec<InstanceIn>) {
        let instances = self.0[object].instances();

        if instances.is_empty() {
            // 複製を 1 つも足さなかった図形。変換なしの複製で埋める。
            out.extend(std::iter::repeat_n(to_raw(&Instance3d::new()), count));
            return;
        }

        out.extend(instances.iter().take(count).map(to_raw));
    }

    fn clear_dirty(&mut self) {
        for object in self.0.iter_mut() {
            object.clear_instances_dirty();
        }
    }
}

fn to_raw(instance: &Instance3d) -> InstanceIn {
    InstanceIn {
        translation: instance.translation(),
        _padding0: 0.0,
        rotation: instance.rotation_angles(),
        _padding1: 0.0,
        scale: instance.scale_factors(),
        _padding2: 0.0,
        tint: instance.tint(),
    }
}

/// `DrawIndexedIndirectArgs` は `Pod` ではないので、バイト列として見る。
///
/// `#[repr(C)]` の 32 ビット 5 つで、詰め物も不正なビット列も持たない。
fn indexed_args_bytes(args: &[wgpu::util::DrawIndexedIndirectArgs]) -> &[u8] {
    // SAFETY: 上記のとおり。読み出し専用のスライスとして見るだけ。
    unsafe { std::slice::from_raw_parts(args.as_ptr().cast::<u8>(), std::mem::size_of_val(args)) }
}

fn heap(device: &wgpu::Device, label: &str, size: u64, usage: wgpu::BufferUsages) -> BufferHeap {
    BufferHeap::new(
        device,
        &BufferHeapDescriptor {
            label: Some(label),
            size: size.max(256),
            frame_size: 0,
            frames_in_flight: 1,
            usage,
            alignment: 256,
        },
    )
}

fn storage_entry(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

const CULL_SHADER: &str = r#"
struct ObjectDesc {
    world: mat4x4<f32>,
    instance_base: u32,
    instance_count: u32,
    bounding_radius: f32,
    padding0: u32,
}

struct InstanceIn {
    translation: vec3<f32>,
    rotation: vec3<f32>,
    scale: vec3<f32>,
    tint: vec4<f32>,
}

struct InstanceOut {
    world: mat4x4<f32>,
    tint: vec4<f32>,
}

// `wgpu::util::DrawIndexedIndirectArgs` と同じ並び。2D の 4 つに対して 5 つ。
struct DrawArgs {
    index_count: u32,
    instance_count: atomic<u32>,
    first_index: u32,
    base_vertex: i32,
    first_instance: u32,
}

struct Scene {
    view_projection: mat4x4<f32>,
    frustum: array<vec4<f32>, 6>,
    light_direction: vec4<f32>,
    ambient: vec4<f32>,
    object_count: u32,
    culling: u32,
    padding0: u32,
    padding1: u32,
}

@group(0) @binding(0) var<storage, read>       objects: array<ObjectDesc>;
@group(0) @binding(1) var<storage, read>       instances_in: array<InstanceIn>;
@group(0) @binding(2) var<storage, read_write> instances_out: array<InstanceOut>;
@group(0) @binding(3) var<storage, read_write> draws: array<DrawArgs>;
@group(0) @binding(4) var<uniform>             scene: Scene;

fn rotation_matrix(rotation: vec3<f32>) -> mat3x3<f32> {
    let cx = cos(rotation.x);
    let sx = sin(rotation.x);
    let cy = cos(rotation.y);
    let sy = sin(rotation.y);
    let cz = cos(rotation.z);
    let sz = sin(rotation.z);

    let rx = mat3x3<f32>(
        vec3<f32>(1.0, 0.0, 0.0),
        vec3<f32>(0.0, cx, sx),
        vec3<f32>(0.0, -sx, cx),
    );
    let ry = mat3x3<f32>(
        vec3<f32>(cy, 0.0, -sy),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(sy, 0.0, cy),
    );
    let rz = mat3x3<f32>(
        vec3<f32>(cz, sz, 0.0),
        vec3<f32>(-sz, cz, 0.0),
        vec3<f32>(0.0, 0.0, 1.0),
    );

    return rx * ry * rz;
}

/// 平行移動 -> 回転 -> 拡大。CPU 側の Instance3d::transform と同じ順。
fn model_matrix(instance: InstanceIn) -> mat4x4<f32> {
    let rotation = rotation_matrix(instance.rotation);

    return mat4x4<f32>(
        vec4<f32>(rotation[0] * instance.scale.x, 0.0),
        vec4<f32>(rotation[1] * instance.scale.y, 0.0),
        vec4<f32>(rotation[2] * instance.scale.z, 0.0),
        vec4<f32>(instance.translation, 1.0),
    );
}

// x = 図形内の複製番号、y = 図形番号。
@compute @workgroup_size(64, 1, 1)
fn cull_main(@builtin(global_invocation_id) id: vec3<u32>) {
    let object_index = id.y;
    if (object_index >= scene.object_count) {
        return;
    }

    let object = objects[object_index];
    let local_index = id.x;
    if (local_index >= object.instance_count) {
        return;
    }

    let instance = instances_in[object.instance_base + local_index];

    // ここが 2D といちばん違う。カメラを掛けず、世界空間で持つ。
    let world = object.world * model_matrix(instance);

    if (scene.culling != 0u) {
        let centre = (world * vec4<f32>(0.0, 0.0, 0.0, 1.0)).xyz;

        // 複製側の拡大も効かせる。いちばん大きい軸で見積もる。
        let instance_scale = max(
            max(abs(instance.scale.x), abs(instance.scale.y)),
            abs(instance.scale.z),
        );
        let radius = object.bounding_radius * instance_scale;

        // 6 面のどれか 1 つでも外側なら、見えない。
        for (var face = 0u; face < 6u; face = face + 1u) {
            let plane = scene.frustum[face];
            let distance = dot(plane.xyz, centre) + plane.w;

            if (distance < -radius) {
                return;
            }
        }
    }

    let slot = atomicAdd(&draws[object_index].instance_count, 1u);

    var output: InstanceOut;
    output.world = world;
    output.tint = instance.tint;
    instances_out[object.instance_base + slot] = output;
}
"#;

const RENDER_SHADER: &str = r#"
struct Scene {
    view_projection: mat4x4<f32>,
    frustum: array<vec4<f32>, 6>,
    light_direction: vec4<f32>,
    ambient: vec4<f32>,
    object_count: u32,
    culling: u32,
    padding0: u32,
    padding1: u32,
}

@group(0) @binding(0) var<uniform> scene: Scene;

// スロット 0 = メッシュの頂点。索引付き描画なので本物の頂点バッファ。
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) uv: vec2<f32>,
    @location(3) color: vec4<f32>,
}

// スロット 1 = コンピュートが書いた、生き残った複製。
struct InstanceInput {
    @location(4) world_0: vec4<f32>,
    @location(5) world_1: vec4<f32>,
    @location(6) world_2: vec4<f32>,
    @location(7) world_3: vec4<f32>,
    @location(8) tint: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) world_normal: vec3<f32>,
}

@vertex
fn vs_main(vertex: VertexInput, instance: InstanceInput) -> VertexOutput {
    let world = mat4x4<f32>(
        instance.world_0,
        instance.world_1,
        instance.world_2,
        instance.world_3,
    );

    let world_position = world * vec4<f32>(vertex.position, 1.0);

    var output: VertexOutput;
    output.clip_position = scene.view_projection * world_position;
    output.color = vertex.color * instance.tint;

    // 法線は世界へ回すだけ。平行移動は乗せない（w = 0）。
    // 非一様な拡大をすると狂うが、そこは逆転置行列が要る話。
    output.world_normal = normalize((world * vec4<f32>(vertex.normal, 0.0)).xyz);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let normal = normalize(input.world_normal);
    // light_direction は光が進む向き。面から光源へ向かう向きに直す。
    let to_light = normalize(-scene.light_direction.xyz);

    let diffuse = max(dot(normal, to_light), 0.0);
    let brightness = scene.ambient.x + (1.0 - scene.ambient.x) * diffuse;

    let lit = input.color.rgb * brightness;

    // 2D と同じく、出すときだけ乗算済みアルファに直す。
    return vec4<f32>(lit * input.color.a, input.color.a);
}
"#;

/// まだ使われていない名前を作る。`Cube` → `Cube 2` → `Cube 3`。
///
/// 2 から始めるのは、`Cube` と `Cube 2` のほうが
/// `Cube 1` と `Cube 2` より「どちらが元か」が分かるためです。
///
/// [`DrawManager3d`] は GPU が無いと組み立てられないので、
/// **名前の作り方だけをここに出して**単体で試せるようにしてあります。
fn unique_name(wanted: &str, taken: impl Fn(&str) -> bool) -> String {
    (2..)
        .map(|suffix| format!("{wanted} {suffix}"))
        .find(|candidate| !taken(candidate))
        .expect("番号は尽きない")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 重なった名前には数が付く。元の名前はそのまま残る。
    #[test]
    fn a_repeated_name_gets_a_number() {
        let taken: std::collections::HashSet<String> =
            ["Cube", "Cube 2"].iter().map(|name| String::from(*name)).collect();

        assert_eq!(unique_name("Cube", |name| taken.contains(name)), "Cube 3");
        assert_eq!(unique_name("Ball", |name| taken.contains(name)), "Ball 2");
        assert!(taken.contains("Cube"), "元の名前が消えている");
    }

    /// WGSL 側と並びが合っていないと、まるごと化ける。
    #[test]
    fn the_gpu_layouts_are_what_the_shaders_expect() {
        assert_eq!(size_of::<ObjectRaw>(), 80);
        assert_eq!(size_of::<InstanceIn>(), 64);
        assert_eq!(size_of::<InstanceOut>(), 80);
        // view_projection 64 + frustum 96 + light 16 + ambient 16 + 末尾 16
        assert_eq!(size_of::<SceneUniform>(), 208);
    }

    /// 索引付きの引数は 2D の 4 つに対して 5 つ。読み戻しの位置がずれる。
    #[test]
    fn indexed_args_have_five_fields() {
        assert_eq!(size_of::<wgpu::util::DrawIndexedIndirectArgs>(), 20);
    }

    #[test]
    fn the_instance_layout_starts_after_the_vertex_one() {
        let vertex = Vertex3d::layout();
        let instance = InstanceOut::layout();

        // 場所が重なるとどちらかが読めない。
        let vertex_last = vertex.attributes.last().expect("属性がある").shader_location;
        let instance_first = instance.attributes[0].shader_location;

        assert!(instance_first > vertex_last, "{instance_first} <= {vertex_last}");
        assert_eq!(instance.step_mode, wgpu::VertexStepMode::Instance);
    }
}
