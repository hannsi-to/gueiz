//! 文字を描く**いちばん短い形**。窓に日本語を 1 行出す。
//!
//! ```rust,ignore
//! let font = Font::from_bytes(&data)?;
//!
//! let mut text = TextRenderer::new("text");
//! text.camera(Camera::orthographic_2d(width, height));
//! text.color(1.0, 1.0, 1.0, 1.0);
//! text.write(&mut draw_manager, &font, "こんにちは、世界", &TextStyle::new(48.0), 40.0, 40.0)?;
//! ```
//!
//! あとは図形と同じで、`prepare` して `draw` するだけです。
//! 字は形として [`DrawManager`] に載るので、描く手順は増えません。
//!
//! - 座標はピクセルで、`(x, y)` は字の**左上**
//! - 同じ字は形を 1 つだけ作り、出てくるたびにインスタンスを足す
//! - 縁は 4 点で均す。均さないと、小さい字の細い画が途切れる
//!
//! ```sh
//! cargo run -p gueiz --example text_minimal
//! ```

use std::alloc::Layout;
use std::error::Error;
use std::sync::Arc;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::font::Font;
use gueiz_2d::msaa::DEFAULT_MULTISAMPLE;
use gueiz_2d::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::text::{TextLayoutData, TextLocation, TextRenderer, TextStyle};
use gueiz_2d::wgpu;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// 日本語の字を持つ書体を探す。
#[path = "common/font.rs"]
mod example_font;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    // 書体はずっと使うので漏らす。`Font` はバイト列を借りるだけなので、
    // こうしておけば寿命を気にせず持ち回せる。
    let (path, data) = example_font::read_japanese_font()?;
    let font = Font::from_bytes(data.leak())?;
    println!("書体: {path}");

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(Application::new(font))?;

    Ok(())
}

struct Application {
    renderer: Renderer,
    font: Font<'static>,
    window: Option<Arc<dyn Window>>,
    scene: Option<Scene>,
}

impl Application {
    fn new(font: Font<'static>) -> Self {
        let mut renderer = Renderer::new(RendererBackend::Auto);
        // サーフェスを作る前に決める。
        renderer.set_sample_count(DEFAULT_MULTISAMPLE);

        Self {
            renderer,
            font,
            window: None,
            scene: None,
        }
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("gueiz text")
            .with_surface_size(LogicalSize::new(640.0, 240.0));

        let window = match event_loop.create_window(attributes) {
            Ok(window) => Arc::<dyn Window>::from(window),
            Err(error) => {
                log::error!("failed to create the window: {error}");
                event_loop.exit();
                return;
            }
        };

        let size = window.surface_size();
        let surface_size = SurfaceSize::new(size.width, size.height);

        if let Err(error) = self.renderer.create_surface(window.clone(), surface_size) {
            log::error!("failed to create the surface: {error}");
            event_loop.exit();
            return;
        }

        match Scene::new(&self.renderer, &self.font, surface_size) {
            Ok(scene) => self.scene = Some(scene),
            Err(error) => {
                log::error!("failed to build the scene: {error}");
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
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                let surface_size = SurfaceSize::new(size.width, size.height);
                self.renderer.resize(surface_size);

                if let Some(scene) = self.scene.as_mut() {
                    scene.resize(surface_size);
                }
            }

            WindowEvent::RedrawRequested => {
                let Some(scene) = self.scene.as_mut() else {
                    return;
                };

                if let Err(error) = scene.draw_manager.prepare() {
                    log::error!("failed to prepare the frame: {error}");
                    return;
                }

                self.renderer.render(wgpu::Color::BLACK, |render_pass| {
                    scene.draw_manager.draw(render_pass);
                });
            }

            _ => {}
        }
    }

    fn destroy_surfaces(&mut self, _event_loop: &dyn ActiveEventLoop) {
        self.scene = None;
        self.renderer.destroy_surface();
        self.window = None;
    }
}

struct Scene {
    draw_manager: DrawManager,
    text: TextRenderer,
}

impl Scene {
    fn new(
        renderer: &Renderer,
        font: &Font,
        surface_size: SurfaceSize,
    ) -> Result<Self, Box<dyn Error>> {
        let (Some(device), Some(queue), Some(format)) = (
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
        ) else {
            return Err("the renderer has no surface yet".into());
        };

        // 描き先と同じ数で均す。食い違うと wgpu が弾く。
        let descriptor = DrawManagerDescriptor {
            sample_count: renderer.sample_count(),
            ..DrawManagerDescriptor::default()
        };
        let mut draw_manager = DrawManager::new(device, queue, format, &descriptor)?;

        // 他の図形と同じく、名前をつけて作る。字の形はこの名前を頭につけて登録される。
        let mut text = TextRenderer::new("text");
        // カメラは字の形を作る前に決める。
        text.camera(camera_for(surface_size));

        text.color(1.0, 1.0, 1.0, 1.0);
        // text.write(&mut draw_manager, font, "こんにちは、世界", &TextStyle::new(48.0), 40.0, 40.0)?;
        text.text_layout_data(TextLayoutData {
            text: "こんにちは、世界".to_string(),
            style: TextStyle::new(48.0),
            text_location: TextLocation::Coordinate {
                x: 40.0,
                y: 40.0,
            }
        });
        text.register_draw_manager(&mut draw_manager, font)?;

        text.color(0.6, 0.8, 1.0, 1.0);
        text.text_layout_data(TextLayoutData {
            text: "漢字・かな・English 0123".to_string(),
            style: TextStyle::new(24.0),
            text_location: TextLocation::Coordinate {
                x: 40.0,
                y: 120.0,
            }
        });
        text.register_draw_manager(&mut draw_manager, font)?;

        text.color(0.7, 0.7, 0.7, 1.0);
        text.text_layout_data(TextLayoutData {
            text: "小さい字（13 px）も均せば途切れない".to_string(),
            style: TextStyle::new(13.0),
            text_location: TextLocation::Coordinate {
                x: 40.0,
                y: 170.0,
            }
        });
        text.register_draw_manager(&mut draw_manager, font)?;

        Ok(Self { draw_manager, text })
    }

    /// 窓が変わったら、カメラだけ貼り直す。字の形は作り直さない。
    fn resize(&mut self, surface_size: SurfaceSize) {
        let camera = camera_for(surface_size);

        // これから作る字の形にも効かせる。
        self.text.camera(camera);

        for name in self.text.shape_ids() {
            if let Some(object) = self.draw_manager.object_mut(name) {
                object.camera(camera);
            }
        }
    }
}

/// 左上が `(0, 0)`、右下が `(width, height)` のピクセル座標。
fn camera_for(surface_size: SurfaceSize) -> Camera {
    Camera::orthographic_2d(surface_size.width as f32, surface_size.height as f32)
}
