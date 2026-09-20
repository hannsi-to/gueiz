use std::fmt::{Display, Formatter};
use std::str::FromStr;

use crate::error::Gueiz2DError;
use crate::msaa::{MultisampleTarget, NO_MULTISAMPLE};
use crate::post::{PostChain, PostProcessor};
use crate::texture::TextureFormat;
use crate::error::Gueiz2DError::{
    AdapterNotFoundError, DeviceCreationError, SurfaceCreationError, UnsupportedSurfaceError,
};

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum RendererBackend {
    #[default]
    Auto,
    Vulkan,
    DirectX12,
    Metal,
    OpenGl,
}

impl RendererBackend {
    pub const ALL: [Self; 5] = [
        Self::Auto,
        Self::Vulkan,
        Self::DirectX12,
        Self::Metal,
        Self::OpenGl,
    ];

    pub fn available() -> Vec<Self> {
        Self::ALL
            .into_iter()
            .filter(Self::is_available)
            .collect()
    }

    pub fn is_available(&self) -> bool {
        if *self == Self::Auto {
            return true;
        }

        wgpu::Instance::enabled_backend_features().contains(self.to_backends())
    }

    pub fn to_backends(self) -> wgpu::Backends {
        match self {
            Self::Auto => wgpu::Backends::all(),
            Self::Vulkan => wgpu::Backends::VULKAN,
            Self::DirectX12 => wgpu::Backends::DX12,
            Self::Metal => wgpu::Backends::METAL,
            Self::OpenGl => wgpu::Backends::GL,
        }
    }

    pub fn from_backend(backend: wgpu::Backend) -> Option<Self> {
        match backend {
            wgpu::Backend::Vulkan => Some(Self::Vulkan),
            wgpu::Backend::Dx12 => Some(Self::DirectX12),
            wgpu::Backend::Metal => Some(Self::Metal),
            wgpu::Backend::Gl => Some(Self::OpenGl),
            wgpu::Backend::Noop | wgpu::Backend::BrowserWebGpu => None,
        }
    }
}

impl Display for RendererBackend {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Auto => "auto",
            Self::Vulkan => "vulkan",
            Self::DirectX12 => "directx12",
            Self::Metal => "metal",
            Self::OpenGl => "opengl",
        };

        f.write_str(name)
    }
}

impl FromStr for RendererBackend {
    type Err = Gueiz2DError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "auto" | "" => Ok(Self::Auto),
            "vulkan" | "vk" => Ok(Self::Vulkan),
            "directx12" | "direct3d12" | "dx12" | "d3d12" => Ok(Self::DirectX12),
            "metal" => Ok(Self::Metal),
            "opengl" | "gl" | "gles" => Ok(Self::OpenGl),
            _ => Err(Gueiz2DError::UnknownBackendError(String::from(s))),
        }
    }
}

#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub struct SurfaceSize {
    pub width: u32,
    pub height: u32,
}

impl SurfaceSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn is_empty(&self) -> bool {
        self.width == 0 || self.height == 0
    }
}


/// 透ける窓に描くときの、色の重ね方。
///
/// **[`Renderer::create_surface`] より前に決めること。** 後から変えても、
/// すでに作ったサーフェスには効きません。
///
/// 既定は [`SurfaceAlphaMode::Auto`]、つまりサーフェスに任せる。多くの環境で
/// これは不透明になり、**シェーダが書いたアルファは捨てられます。** 窓を
/// 透かしたいなら [`SurfaceAlphaMode::PreMultiplied`] を選ぶ。
///
/// 選んだ重ね方をサーフェスが持っていないときは、警告を出して既定に戻ります。
/// 何が使えるかは起動時のログ（`alpha : supported ...`）に出ます。Windows の
/// DirectX12 は `PreMultiplied` を出さないことがあるので、そのときは
/// [`RendererBackend::Vulkan`] を指定すると通ることが多い。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug, Default)]
pub enum SurfaceAlphaMode {
    /// サーフェスに任せる。たいてい不透明になる。
    #[default]
    Auto,
    /// アルファを捨てて不透明にする。
    Opaque,
    /// 色にあらかじめアルファを掛けた状態で重ねる。透ける窓ならこれ。
    PreMultiplied,
    /// 色とアルファを別々に持ったまま重ねる。
    PostMultiplied,
    /// 窓の側の設定に従う。
    Inherit,
}

impl SurfaceAlphaMode {
    /// wgpu の指定へ。`Auto` は「触らない」なので `None` を返す。
    fn to_composite_alpha_mode(self) -> Option<wgpu::CompositeAlphaMode> {
        match self {
            Self::Auto => None,
            Self::Opaque => Some(wgpu::CompositeAlphaMode::Opaque),
            Self::PreMultiplied => Some(wgpu::CompositeAlphaMode::PreMultiplied),
            Self::PostMultiplied => Some(wgpu::CompositeAlphaMode::PostMultiplied),
            Self::Inherit => Some(wgpu::CompositeAlphaMode::Inherit),
        }
    }
}
pub struct Renderer {
    instance: wgpu::Instance,
    renderer_backend: RendererBackend,
    render_surface: Option<RenderSurface>,
    post_chain: PostChain,
    /// 縁のギザギザを均す点の数。サーフェスを作る前に決めること。
    sample_count: u32,
    /// 透ける窓に描くときの色の重ね方。サーフェスを作る前に決めること。
    surface_alpha_mode: SurfaceAlphaMode,
}

impl Renderer {
    pub fn new(renderer_backend: RendererBackend) -> Self {
        let mut instance_descriptor = wgpu::InstanceDescriptor::new_without_display_handle_from_env();

        if renderer_backend != RendererBackend::Auto {
            if !renderer_backend.is_available() {
                log::warn!("{renderer_backend} is not enabled in this build of wgpu");
            }

            instance_descriptor.backends = renderer_backend.to_backends();
        }

        let instance = wgpu::Instance::new(instance_descriptor);

        Self {
            instance,
            renderer_backend,
            render_surface: None,
            post_chain: PostChain::new(),
            sample_count: NO_MULTISAMPLE,
            surface_alpha_mode: SurfaceAlphaMode::default(),
        }
    }

    pub fn create_surface(
        &mut self,
        surface_target: impl Into<wgpu::SurfaceTarget<'static>>,
        surface_size: SurfaceSize,
    ) -> Result<(), Gueiz2DError> {
        self.render_surface = None;

        let surface = self
            .instance
            .create_surface(surface_target)
            .map_err(SurfaceCreationError)?;

        let adapter = pollster::block_on(self.instance.request_adapter(
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                ..Default::default()
            },
        ))
        .map_err(AdapterNotFoundError)?;

        let adapter_info = adapter.get_info();
        log::info!("backend : {:?}", adapter_info.backend);
        log::info!("adapter : {} ({:?})", adapter_info.name, adapter_info.device_type);
        log::info!("driver  : {} {}", adapter_info.driver, adapter_info.driver_info);

        // インダイレクト描画で `first_instance` を使うのに要る。
        // 無い環境でも起動できるよう、adapter が持っているときだけ要求する。
        let optional_features = wgpu::Features::INDIRECT_FIRST_INSTANCE;
        let required_features = optional_features & adapter.features();

        for feature in optional_features.iter() {
            if !required_features.contains(feature) {
                log::warn!("{feature:?} is not available; GPU-driven drawing will be disabled");
            }
        }

        // device = 論理デバイス、queue = コマンド投入先。
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gueiz device"),
            required_features,
            ..Default::default()
        }))
        .map_err(DeviceCreationError)?;

        let surface_capabilities = surface.get_capabilities(&adapter);

        let mut surface_configuration = surface
            .get_default_config(&adapter, surface_size.width, surface_size.height)
            .ok_or(UnsupportedSurfaceError)?;

        // 透ける窓に描くなら、ここで重ね方を選んでおく必要がある。既定の設定は
        // たいてい不透明で、シェーダが書いたアルファは捨てられてしまう。
        if let Some(composite_alpha_mode) = self.surface_alpha_mode.to_composite_alpha_mode() {
            if surface_capabilities.alpha_modes.contains(&composite_alpha_mode) {
                surface_configuration.alpha_mode = composite_alpha_mode;
            } else {
                log::warn!(
                    "{:?} is not supported by this surface; falling back to {:?}",
                    composite_alpha_mode,
                    surface_configuration.alpha_mode,
                );
            }
        }

        surface.configure(&device, &surface_configuration);

        log::info!(
            "surface : {:?}, {:?}, {}x{}",
            surface_configuration.format,
            surface_configuration.present_mode,
            surface_configuration.width,
            surface_configuration.height,
        );
        log::info!(
            "alpha   : {:?} (supported: {:?})",
            surface_configuration.alpha_mode,
            surface_capabilities.alpha_modes,
        );

        // 画面全体のパスを走らせる側。サーフェスの形式に合わせて組む。
        let post_processor = PostProcessor::new(&device, surface_configuration.format);

        self.render_surface = Some(RenderSurface {
            surface,
            adapter_info,
            device,
            queue,
            surface_configuration,
            last_submission_index: None,
            post_processor,
            multisample: MultisampleTarget::new(self.sample_count),
        });

        Ok(())
    }

    pub fn destroy_surface(&mut self) {
        self.render_surface = None;
    }

    pub fn has_surface(&self) -> bool {
        self.render_surface.is_some()
    }

    pub fn renderer_backend(&self) -> RendererBackend {
        self.renderer_backend
    }

    pub fn backend_in_use(&self) -> Option<RendererBackend> {
        self.adapter_info()
            .and_then(|adapter_info| RendererBackend::from_backend(adapter_info.backend))
    }

    pub fn resize(&mut self, surface_size: SurfaceSize) {
        if surface_size.is_empty() {
            return;
        }

        let Some(render_surface) = self.render_surface.as_mut() else {
            return;
        };

        if render_surface.size() == surface_size {
            return;
        }

        render_surface.surface_configuration.width = surface_size.width;
        render_surface.surface_configuration.height = surface_size.height;
        render_surface.reconfigure();
    }

    pub fn surface_size(&self) -> Option<SurfaceSize> {
        self.render_surface.as_ref().map(RenderSurface::size)
    }

    /// サーフェスのピクセル形式。クレート自前の [`TextureFormat`] で返す。
    pub fn surface_format(&self) -> Option<TextureFormat> {
        self.render_surface
            .as_ref()
            .map(|render_surface| render_surface.surface_configuration.format.into())
    }

    pub fn adapter_info(&self) -> Option<&wgpu::AdapterInfo> {
        self.render_surface
            .as_ref()
            .map(|render_surface| &render_surface.adapter_info)
    }

    pub fn device(&self) -> Option<&wgpu::Device> {
        self.render_surface
            .as_ref()
            .map(|render_surface| &render_surface.device)
    }

    /// 直前の [`Renderer::render`] が投げた submission。
    ///
    /// `BufferHeap::end_frame` に渡して、フレームスライスの再利用待ちに使う。
    pub fn last_submission_index(&self) -> Option<wgpu::SubmissionIndex> {
        self.render_surface
            .as_ref()
            .and_then(|render_surface| render_surface.last_submission_index.clone())
    }

    pub fn queue(&self) -> Option<&wgpu::Queue> {
        self.render_surface
            .as_ref()
            .map(|render_surface| &render_surface.queue)
    }

    /// 縁のギザギザを均す点の数。1 で均さない、4 が無難。
    ///
    /// **[`Renderer::create_surface`] より前に決めること。** 後から変えても、
    /// すでに作ったサーフェスには効きません。
    ///
    /// ここで決めた数を、図形を描く側（`DrawManager` の `sample_count`）にも
    /// 同じ値で渡す必要があります。食い違うと wgpu が弾きます。
    pub fn set_sample_count(&mut self, sample_count: u32) {
        self.sample_count = sample_count.max(1);
    }

    pub fn sample_count(&self) -> u32 {
        self.sample_count
    }

    /// 透ける窓に描くときの色の重ね方。
    ///
    /// **[`Renderer::create_surface`] より前に決めること。** 後から変えても、
    /// すでに作ったサーフェスには効きません。
    ///
    /// 窓を透かすには三つ揃える必要がある。窓を `transparent` で作ること、
    /// ここを [`SurfaceAlphaMode::PreMultiplied`] にすること、そして
    /// [`Renderer::render`] に渡す消す色のアルファを 0 にすること。
    pub fn set_surface_alpha_mode(&mut self, surface_alpha_mode: SurfaceAlphaMode) {
        self.surface_alpha_mode = surface_alpha_mode;
    }

    pub fn surface_alpha_mode(&self) -> SurfaceAlphaMode {
        self.surface_alpha_mode
    }

    /// 画面全体に掛けるエフェクトの列。
    pub fn post_chain(&self) -> &PostChain {
        &self.post_chain
    }

    /// 画面全体に掛けるエフェクトをまるごと差し替える。
    ///
    /// 空なら、場面はこれまでどおりサーフェスに直接描かれる。オフスクリーンの
    /// テクスチャは作られず、追加のコストはゼロ。
    pub fn set_post_chain(&mut self, post_chain: PostChain) {
        self.post_chain = post_chain;
    }

    /// 画面全体に掛けるエフェクトを書き換える。順番の入れ替えもここから。
    ///
    /// ```no_run
    /// # use gueiz_gpu::post::PostEffect;
    /// # fn run(renderer: &mut gueiz_gpu::renderer::Renderer) {
    /// renderer.edit_post_chain(|chain| {
    ///     chain.push(PostEffect::Glow { threshold: 0.7, intensity: 0.8, radius: 4.0 });
    ///     chain.push(PostEffect::Vignette { amount: 0.5, softness: 0.4 });
    /// });
    /// # }
    /// ```
    pub fn edit_post_chain(&mut self, edit: impl FnOnce(&mut PostChain)) {
        edit(&mut self.post_chain);
    }

    pub fn clear(&mut self, clear_color: wgpu::Color) -> FrameOutcome {
        self.render(clear_color, |_render_pass| {})
    }

    pub fn render(
        &mut self,
        clear_color: wgpu::Color,
        draw: impl FnOnce(&mut wgpu::RenderPass<'_>),
    ) -> FrameOutcome {
        let Some(render_surface) = self.render_surface.as_mut() else {
            return FrameOutcome::NoSurface;
        };

        let frame = match render_surface.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => frame,

            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                render_surface.reconfigure();
                frame
            }

            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                log::debug!("surface outdated or lost; reconfiguring");
                render_surface.reconfigure();
                return FrameOutcome::Skipped;
            }

            other => {
                log::debug!("skipped a frame: {other:?}");
                return FrameOutcome::Skipped;
            }
        };

        let surface_view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder =
            render_surface
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("gueiz frame"),
                });

        // ポストが 1 つも無ければ、これまでどおりサーフェスに直接描く。
        // オフスクリーンのテクスチャも作らないので、追加のコストはゼロ。
        let scene_view = if self.post_chain.pass_count() == 0 {
            surface_view.clone()
        } else {
            render_surface
                .post_processor
                .scene_view(
                    &render_surface.device,
                    render_surface.surface_configuration.width,
                    render_surface.surface_configuration.height,
                    render_surface.surface_configuration.format,
                )
                .clone()
        };

        // 均すときは、多点の描き先に描いて `scene_view` へ書き出す。
        // 均さないときは `scene_view` に直接描く。呼ぶ側の書き方は変わらない。
        render_surface.multisample.ensure(
            &render_surface.device,
            render_surface.surface_configuration.width,
            render_surface.surface_configuration.height,
            render_surface.surface_configuration.format,
        );

        {
            let attachment = render_surface
                .multisample
                .color_attachment(&scene_view, wgpu::LoadOp::Clear(clear_color));

            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("gueiz render pass"),
                color_attachments: &[Some(attachment)],
                ..Default::default()
            });

            draw(&mut render_pass);
        }

        // 最後の 1 パスがサーフェスに書く。間は 2 枚を行き来する。
        render_surface.post_processor.run(
            &render_surface.queue,
            &mut encoder,
            &self.post_chain,
            &surface_view,
        );

        let submission_index = render_surface.queue.submit(Some(encoder.finish()));
        render_surface.queue.present(frame);
        render_surface.last_submission_index = Some(submission_index);

        FrameOutcome::Presented
    }
}

struct RenderSurface {
    surface: wgpu::Surface<'static>,
    adapter_info: wgpu::AdapterInfo,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface_configuration: wgpu::SurfaceConfiguration,
    last_submission_index: Option<wgpu::SubmissionIndex>,
    post_processor: PostProcessor,
    multisample: MultisampleTarget,
}

impl RenderSurface {
    fn size(&self) -> SurfaceSize {
        SurfaceSize::new(
            self.surface_configuration.width,
            self.surface_configuration.height,
        )
    }

    fn reconfigure(&self) {
        self.surface
            .configure(&self.device, &self.surface_configuration);
    }
}

/// 1 フレーム分の結果。ループを回すだけなら無視してよい。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Debug)]
pub enum FrameOutcome {
    /// 描いて提示した。
    Presented,
    /// 今回は描けなかった。次のフレームで取り直せばよい。
    Skipped,
    /// サーフェスがまだ無い。[`Renderer::create_surface`] を呼ぶ必要がある。
    NoSurface,
}
