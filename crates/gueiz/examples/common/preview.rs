//! 読み戻した画素を窓に出す。
//!
//! # 使い方
//!
//! 例の中で、読み戻した画像を [`Preview::capture`] に渡していきます。
//! 最後に [`Preview::show`] を呼ぶと、`--window` が付いているときだけ窓が開きます。
//!
//! ```ignore
//! let mut preview = Preview::new("text_test");
//! // ...
//! preview.capture("`o` の穴", SIZE, SIZE, &pixels);
//! // ...
//! preview.show()?;
//! ```
//!
//! 窓では**矢印キーか Space で切り替え**られます。題名は窓の枠に出ます。
//!
//! # 色をいじらない
//!
//! 読み戻した並びをそのまま出します。テクスチャはサーフェスと同じ形式で作るので、
//! sRGB の変換が入っても行きと帰りで打ち消し合い、**確認したバイトがそのまま**出ます。
//! 見えているものと、assert が見ているものがずれません。

use std::error::Error;
use std::sync::Arc;

use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::wgpu;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

/// 窓を出すかどうか。`--window` が付いていれば出す。
pub fn window_requested() -> bool {
    std::env::args().any(|argument| argument == "--window" || argument == "-w")
}

/// 読み戻した 1 枚。
pub struct Capture {
    pub label: String,
    pub width: u32,
    pub height: u32,
    /// BGRA の並び。読み戻したそのまま。
    pub pixels: Vec<u8>,
}

/// 撮った画像をためて、最後に窓へ出す。
pub struct Preview {
    title: String,
    captures: Vec<Capture>,
}

impl Preview {
    pub fn new(title: &str) -> Self {
        Self {
            title: String::from(title),
            captures: Vec::new(),
        }
    }

    /// 1 枚ためる。窓を出さないときは何もしない（複製を作らない）。
    pub fn capture(&mut self, label: &str, width: u32, height: u32, pixels: &[u8]) {
        if !window_requested() {
            return;
        }

        self.captures.push(Capture {
            label: String::from(label),
            width,
            height,
            pixels: pixels.to_vec(),
        });
    }

    /// `--window` が付いていれば窓を開く。付いていなければ何もしない。
    ///
    /// 窓を閉じるまで戻りません。
    pub fn show(self) -> Result<(), Box<dyn Error>> {
        if !window_requested() || self.captures.is_empty() {
            if window_requested() {
                println!("\n（出せる画像がありません）");
            }

            return Ok(());
        }

        println!(
            "\n窓を開きます（{} 枚）。← → か Space で切り替え、Esc で閉じる。",
            self.captures.len(),
        );

        let event_loop = EventLoop::new()?;
        // 切り替えたときだけ描けばよい。
        event_loop.set_control_flow(ControlFlow::Wait);
        event_loop.run_app(PreviewApplication::new(self.title, self.captures))?;

        Ok(())
    }
}

struct PreviewApplication {
    title: String,
    captures: Vec<Capture>,
    current: usize,
    window: Option<Arc<dyn Window>>,
    surface: Option<Surface>,
}

impl PreviewApplication {
    fn new(title: String, captures: Vec<Capture>) -> Self {
        Self {
            title,
            captures,
            current: 0,
            window: None,
            surface: None,
        }
    }

    fn step(&mut self, forward: bool) {
        let count = self.captures.len();

        self.current = if forward {
            (self.current + 1) % count
        } else {
            (self.current + count - 1) % count
        };

        self.refresh();
    }

    /// 題名を貼り直して、描き直す。
    fn refresh(&mut self) {
        let Some(capture) = self.captures.get(self.current) else {
            return;
        };

        if let Some(window) = self.window.as_ref() {
            window.set_title(&format!(
                "{} [{}/{}] {}",
                self.title,
                self.current + 1,
                self.captures.len(),
                capture.label,
            ));
            window.request_redraw();
        }
    }

    fn draw(&mut self) {
        let (Some(surface), Some(capture)) =
            (self.surface.as_mut(), self.captures.get(self.current))
        else {
            return;
        };

        surface.draw(capture);
    }
}

impl ApplicationHandler for PreviewApplication {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        let first = &self.captures[0];

        let attributes = WindowAttributes::default()
            .with_title(&self.title)
            .with_surface_size(LogicalSize::new(first.width, first.height));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::<dyn Window>::from(window),
            Err(error) => {
                log::error!("failed to create the window: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.surface_size();

        match Surface::new(
            window.clone(),
            SurfaceSize::new(size.width.max(1), size.height.max(1)),
        ) {
            Ok(surface) => self.surface = Some(surface),
            Err(error) => {
                log::error!("failed to set up the preview surface: {error}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
        self.refresh();
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                if let Some(surface) = self.surface.as_mut() {
                    surface.resize(SurfaceSize::new(size.width.max(1), size.height.max(1)));
                }

                if let Some(window) = self.window.as_ref() {
                    window.request_redraw();
                }
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state != ElementState::Pressed {
                    return;
                }

                match event.logical_key.as_ref() {
                    Key::Named(NamedKey::Escape) => event_loop.exit(),
                    Key::Named(NamedKey::ArrowRight) => self.step(true),
                    Key::Named(NamedKey::ArrowLeft) => self.step(false),
                    // Space は名前付きではなく文字として来る。
                    Key::Character(" ") => self.step(true),
                    _ => {}
                }
            }

            WindowEvent::RedrawRequested => self.draw(),

            _ => {}
        }
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.surface = None;
    }
}

/// 画素をそのまま画面いっぱいに貼るだけの、最小の描画。
struct Surface {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    configuration: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
}

impl Surface {
    fn new(
        target: impl Into<wgpu::SurfaceTarget<'static>>,
        size: SurfaceSize,
    ) -> Result<Self, Box<dyn Error>> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(target)?;

        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("preview"),
                ..Default::default()
            }))?;

        let configuration = surface
            .get_default_config(&adapter, size.width, size.height)
            .ok_or("the adapter does not support this surface")?;
        surface.configure(&device, &configuration);

        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("preview shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("preview layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("preview pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });

        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("preview pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: configuration.format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        // 拡大しても元の画素が分かるように、ぼかさない。
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("preview sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        Ok(Self {
            surface,
            device,
            queue,
            configuration,
            pipeline,
            layout,
            sampler,
        })
    }

    fn resize(&mut self, size: SurfaceSize) {
        if size.is_empty() || SurfaceSize::new(self.configuration.width, self.configuration.height) == size
        {
            return;
        }

        self.configuration.width = size.width;
        self.configuration.height = size.height;
        self.surface.configure(&self.device, &self.configuration);
    }

    fn draw(&mut self, capture: &Capture) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame)
            | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => frame,
            other => {
                log::debug!("skipped a preview frame: {other:?}");
                self.surface.configure(&self.device, &self.configuration);
                return;
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        // **サーフェスと同じ形式で作る。** そうすれば sRGB の変換が
        // 行きと帰りで打ち消し合い、読み戻したバイトがそのまま出る。
        let texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("preview image"),
            size: wgpu::Extent3d {
                width: capture.width,
                height: capture.height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: self.configuration.format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &to_surface_order(&capture.pixels, self.configuration.format),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(capture.width * 4),
                rows_per_image: Some(capture.height),
            },
            wgpu::Extent3d {
                width: capture.width,
                height: capture.height,
                depth_or_array_layers: 1,
            },
        );

        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("preview bind group"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(
                        &texture.create_view(&wgpu::TextureViewDescriptor::default()),
                    ),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("preview"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("preview pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // 画像より窓が広いときに、はみ出しが見えるように。
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.08,
                            g: 0.08,
                            b: 0.10,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });

            render_pass.set_pipeline(&self.pipeline);
            render_pass.set_bind_group(0, &bind_group, &[]);
            render_pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(encoder.finish()));
        self.queue.present(frame);
    }
}

/// 読み戻した BGRA を、サーフェスの並びに合わせる。
///
/// 読み戻しは BGRA 固定だが、サーフェスが RGBA のこともある。
fn to_surface_order(pixels: &[u8], format: wgpu::TextureFormat) -> Vec<u8> {
    let swap = matches!(
        format,
        wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb
    );

    if !swap {
        return pixels.to_vec();
    }

    let mut swapped = pixels.to_vec();

    for pixel in swapped.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }

    swapped
}

const SHADER: &str = r#"
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
}

// 画面を覆う三角形 1 枚。頂点バッファは要らない。
@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> VertexOutput {
    let uv = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));

    var output: VertexOutput;
    output.uv = uv;
    output.position = vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, 0.0, 1.0);
    return output;
}

@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, input.uv);
}
"#;
