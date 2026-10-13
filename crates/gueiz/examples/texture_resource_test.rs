//! 画像を [`Resources`] に読み込んで、窓に描く最小の例。
//!
//! 1. 画像ファイルを RGBA に復号する
//! 2. [`Resources::load_texture`] で預ける（GPU はまだ触らない）
//! 3. [`Resources::commit_textures`] でまとめて GPU へ上げる
//! 4. [`Resources::texture`] で引いた場所を四角に貼って描く
//!
//! ```sh
//! cargo run -p gueiz --example texture_resource_test -- path/to/image.png
//! ```

use std::error::Error;
use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

use gueiz::gpu::camera::ScaleMode;
use gueiz::gpu::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::instance::create_instance;
use gueiz_2d::object;
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::resource::Resources;
use gueiz_2d::vertex::Vertex;

/// 貼った四角の名前。窓の大きさが変わったら、これでカメラを貼り直す。
const SPRITE: &str = "Sprite";

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let Some(image_path) = std::env::args().nth(1) else {
        return Err("画像のパスを渡してください: cargo run -p gueiz --example texture_resource_test -- path/to/image.png".into());
    };

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(Application {
        renderer: Renderer::new(RendererBackend::Auto),
        window: None,
        scene: None,
        image_path,
    })?;

    Ok(())
}

struct Application {
    renderer: Renderer,
    window: Option<Arc<dyn Window>>,
    scene: Option<Scene>,
    image_path: String,
}

struct Scene {
    draw_manager: DrawManager,
    scale_factor: f32,
}

impl Scene {
    fn new(
        renderer: &Renderer,
        surface_size: SurfaceSize,
        scale_factor: f32,
        image_path: &str,
    ) -> Result<Self, Box<dyn Error>> {
        let (Some(device), Some(queue), Some(format)) = (
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
        ) else {
            return Err("the renderer has no surface yet".into());
        };

        let descriptor = DrawManagerDescriptor {
            sample_count: renderer.sample_count(),
            ..DrawManagerDescriptor::default()
        };
        let mut draw_manager = DrawManager::new(device, queue, format, &descriptor)?;

        // 1. 画像を復号する。`load_texture` は普通のアルファの RGBA をそのまま受け取る。
        let image = image::open(image_path)?.into_rgba8();
        let (width, height) = image.dimensions();

        // 2. 預ける。ここではまだ GPU を触らない。
        let mut resources = Resources::new();
        let texture = resources.load_texture("image", width, height, image.as_raw())?;

        // 3. 預けた絵をまとめて GPU へ上げる。絵の入ったシートも draw_manager に差される。
        resources.commit_textures(device, queue, &mut draw_manager)?;

        // 4. 上げた場所を引いて、画像と同じ大きさの四角に貼る。
        let region = resources.texture(texture).ok_or("the texture was not committed")?;

        let (x, y) = (20.0, 20.0);
        let (width, height) = (width as f32, height as f32);

        let mut sprite = object::create_object(SPRITE);
        sprite.begin(PaintType::Fill);
        // 左上から時計回り。UV は左上が (0, 0)。
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            sprite.put_vertex(Vertex::new_position_color_uv_normal(
                x + u * width,
                y + v * height,
                0.0,
                1.0, 1.0, 1.0, 1.0,
                u, v,
                0.0, 0.0, 1.0,
            ));
        }
        sprite.end();
        sprite.camera_for(surface_size, surface_size.to_logical(scale_factor), ScaleMode::Stretch);
        sprite.sprite_texture(region);
        sprite.instance(create_instance());

        draw_manager.register(sprite);

        // 上げた絵のシートは draw_manager も持つので、描くだけなら resources はここで手放せる。
        // 後から絵を足すなら、resources も持っておいて load_texture → commit_textures を呼ぶ。
        Ok(Self {
            draw_manager,
            scale_factor,
        })
    }

    fn resize(&mut self, surface_size: SurfaceSize) {
        if let Some(sprite) = self.draw_manager.object_mut(SPRITE) {
            sprite.camera_for(surface_size, surface_size.to_logical(self.scale_factor), ScaleMode::Stretch);
        }
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("texture resource test")
            .with_surface_size(LogicalSize::new(640.0, 480.0));

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

        let scale_factor = window.scale_factor() as f32;
        match Scene::new(&self.renderer, surface_size, scale_factor, &self.image_path) {
            Ok(scene) => self.scene = Some(scene),
            Err(error) => {
                log::error!("failed to build the scene: {error}");
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, _window_id: WindowId, event: WindowEvent) {
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

                self.renderer.render(wgpu::Color { r: 0.13, g: 0.14, b: 0.16, a: 1.0 }, |render_pass| {
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
