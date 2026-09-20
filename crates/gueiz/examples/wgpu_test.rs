//! wgpu の最小構成。ウィンドウを 1 枚開き、画面をクリアするだけ。
//!
//! instance → surface → adapter → device / queue → surface config → render pass、
//! という wgpu 初期化の最短経路をひと通り通す。パイプラインも頂点バッファもまだ無い。
//!
//! ```sh
//! RUST_LOG=info cargo run -p gueiz --example wgpu_test
//! ```
//!
//! バックエンド（Vulkan / DX12 / Metal / GL）は wgpu が実行時に選ぶ。
//! `WGPU_BACKEND=dx12` のような環境変数で固定できる。

use std::error::Error;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let event_loop = EventLoop::new()?;
    // クリア色をアニメーションさせるので毎フレーム回す。
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop.run_app(WgpuApplication::default())?;

    Ok(())
}

#[derive(Default)]
struct WgpuApplication {
    renderer: Option<WgpuRenderer>,
    /// `Box` ではなく `Arc` で持つ。wgpu の `Surface` にクローンを渡して所有権を
    /// 共有させるため（`WgpuRenderer::new` のコメント参照）。
    window: Option<Arc<dyn Window>>,
    frame_index: u64,
}

impl ApplicationHandler for WgpuApplication {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attributes = WindowAttributes::default()
            .with_title("gueiz wgpu")
            .with_surface_size(LogicalSize::new(1280.0, 720.0));

        let window = match event_loop.create_window(window_attributes) {
            Ok(window) => Arc::<dyn Window>::from(window),
            Err(error) => {
                log::error!("failed to create window: {error}");
                event_loop.exit();
                return;
            }
        };

        match WgpuRenderer::new(window.clone()) {
            Ok(renderer) => self.renderer = Some(renderer),
            Err(error) => {
                log::error!("failed to initialize wgpu: {error}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(
        &mut self,
        event_loop: &dyn ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        let (Some(window), Some(renderer)) = (self.window.as_ref(), self.renderer.as_mut()) else {
            return;
        };

        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                renderer.resize(size.width, size.height);
                window.request_redraw();
            }

            WindowEvent::RedrawRequested => {
                // 最小化中はサイズ 0 になる。サーフェスを構成できないので飛ばす。
                let surface_size = window.surface_size();
                if surface_size.width == 0 || surface_size.height == 0 {
                    return;
                }

                // 時間とともに色相が回るクリア色。描画できている証拠になる。
                let phase = self.frame_index as f32 * 0.01;
                let clear_color = wgpu::Color {
                    r: phase.sin().mul_add(0.5, 0.5) as f64,
                    g: (phase + 2.094).sin().mul_add(0.5, 0.5) as f64,
                    b: (phase + 4.189).sin().mul_add(0.5, 0.5) as f64,
                    a: 1.0,
                };
                self.frame_index += 1;

                window.pre_present_notify();

                if let FrameOutcome::NeedsReconfigure = renderer.draw(clear_color) {
                    renderer.reconfigure();
                }
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &dyn ActiveEventLoop) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
        }
    }

    /// サスペンド時などにサーフェスが失われる。winit 0.31 では `exiting` ではなく
    /// これが「サーフェスを捨てろ」の合図。次の `can_create_surfaces` で作り直す。
    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.renderer = None;
        self.window = None;
    }
}

/// wgpu のオブジェクト一式。すべて内部で参照カウントされているので、
/// ash や glutin と違って破棄順を自分で組み立てる必要が無い。
struct WgpuRenderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    /// リサイズのたびに書き換えて `configure` し直す。
    surface_configuration: wgpu::SurfaceConfiguration,
}

impl WgpuRenderer {
    fn new(window: Arc<dyn Window>) -> Result<Self, Box<dyn Error>> {
        let surface_size = window.surface_size();

        // instance = wgpu 本体への入口。ここではまだ GPU に触っていない。
        // `_from_env` は WGPU_BACKEND などの環境変数を読んでくれる版。
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        // `Arc<dyn Window>` をそのまま渡せる。winit 0.31 の `Window` トレイトが
        // `HasWindowHandle + HasDisplayHandle + Send + Sync` なので、wgpu の
        // `DisplayAndWindowHandle` を満たすため。
        //
        // wgpu 側が Arc のクローンを抱えるので、サーフェスがウィンドウより長生きしても
        // ダングリングしない。だから `Surface<'static>` として持てるし、
        // Vulkan/OpenGL 版でやっていた「宣言順 = drop 順」の細工が要らない。
        let surface = instance.create_surface(window)?;

        // adapter = 物理 GPU + バックエンドの組み合わせ。
        // `compatible_surface` を渡さないと、そのサーフェスに出せない GPU を選びうる。
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))?;

        let adapter_info = adapter.get_info();
        log::info!("backend : {:?}", adapter_info.backend);
        log::info!("adapter : {} ({:?})", adapter_info.name, adapter_info.device_type);
        log::info!("driver  : {} {}", adapter_info.driver, adapter_info.driver_info);

        // device = 論理デバイス、queue = コマンド投入先。Vulkan と同じ二分割。
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gueiz device"),
            ..Default::default()
        }))?;

        // スワップチェーンに相当するものは wgpu が内部で持つ。こちらは構成を渡すだけ。
        let surface_configuration = surface
            .get_default_config(&adapter, surface_size.width, surface_size.height)
            .ok_or("the adapter does not support this surface")?;
        surface.configure(&device, &surface_configuration);

        log::info!(
            "surface : {:?}, {:?}, {}x{}",
            surface_configuration.format,
            surface_configuration.present_mode,
            surface_configuration.width,
            surface_configuration.height,
        );

        Ok(Self { surface, device, queue, surface_configuration })
    }

    /// ウィンドウサイズが変わったらサーフェスを構成し直す。
    fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            // 最小化。サイズ 0 のサーフェスは構成できないので何もしない。
            return;
        }

        self.surface_configuration.width = width;
        self.surface_configuration.height = height;
        self.reconfigure();
    }

    fn reconfigure(&self) {
        self.surface.configure(&self.device, &self.surface_configuration);
    }

    fn draw(&self, clear_color: wgpu::Color) -> FrameOutcome {
        // 次に描き込むテクスチャを借りる。Vulkan の acquire_next_image 相当。
        // wgpu 30 から Result ではなく専用の enum を返すようになった。
        let (frame, outcome) = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(frame) => (frame, FrameOutcome::Presented),

            // 取得はできたが構成が古い。今回は描いた上で、次に構成し直す。
            wgpu::CurrentSurfaceTexture::Suboptimal(frame) => {
                (frame, FrameOutcome::NeedsReconfigure)
            }

            // Vulkan の OUT_OF_DATE_KHR / SURFACE_LOST_KHR 相当。
            // スワップチェーンは wgpu が持っているので、構成し直すだけでよい
            // （Lost が続くようならサーフェス自体の作り直しが要る）。
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                return FrameOutcome::NeedsReconfigure;
            }

            // 最小化・遮蔽・タイムアウト・検証エラー。次のフレームで取り直す。
            other => {
                log::debug!("skipped a frame: {other:?}");
                return FrameOutcome::Skipped;
            }
        };

        let view = frame.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gueiz frame") });

        // クリアするだけのレンダーパス。`LoadOp::Clear` が glClear 相当で、
        // パスの開始時に GPU がまとめて塗る。
        // `RenderPass` は drop でパスを閉じるので、スコープで括って早めに落とす。
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("clear pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            });
        }

        self.queue.submit(Some(encoder.finish()));
        // wgpu 30 では present は Queue 側にある。
        self.queue.present(frame);

        outcome
    }
}

/// 1 フレーム分の結果。呼び出し側が次に何をすべきかを表す。
enum FrameOutcome {
    /// 描いて提示できた。
    Presented,
    /// サーフェスを構成し直す必要がある。
    NeedsReconfigure,
    /// 今回は描けなかった。次のフレームで取り直せばよい。
    Skipped,
}
