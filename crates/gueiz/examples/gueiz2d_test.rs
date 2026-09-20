//! `gueiz-2d` でいちばん短い絵。**画面いっぱいの四角形を 1 つ描く。**
//!
//! ```rust,ignore
//! let mut square = object::create_object("Square");
//!
//! square.begin(PaintType::Fill);
//! square.put_vertex(Vertex::new_position_color(x, y, 0.0, r, g, b, a));   // 4 つ
//! square.end();
//!
//! square.camera(Camera::orthographic_2d(width, height));
//! square.scale(width, height, 1.0);
//! square.instance(instance::create_instance());
//!
//! draw_manager.register(square);
//! ```
//!
//! 座標はピクセル。カメラが左上原点の正射影を張るので、`(0, 0)` が画面左上、
//! `(width, height)` が右下になる。
//!
//! # 画面いっぱいに保つ
//!
//! 頂点は **0..1 の割合**で置いて、実際の大きさは
//! [`Object::scale`](gueiz_2d::object::Object::scale) で決めている。
//! 窓が変わったら拡大だけ更新すればよく、**三角形を組み直さずに済む**。
//!
//! 実寸で頂点を置くと、窓を引っぱるたびに形が変わったことになり、
//! そのたびに全図形の三角形を積み直すことになる。
//!
//! 頂点は**回る向きを揃えなくてよい**。テッセレータが包含関係で
//! 表裏を決めるので、時計回りでも反時計回りでも塗れる。
//!
//! ```sh
//! RUST_LOG=info cargo run -p gueiz --example gueiz2d_test
//! RUST_LOG=info cargo run -p gueiz --example gueiz2d_test -- dx12
//! ```

use std::error::Error;
use std::sync::Arc;

use gueiz_2d::camera::Camera;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::object::{self, instance};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use gueiz::gpu::camera::ScaleMode;

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let renderer_backend = match std::env::args().nth(1) {
        Some(argument) => argument.parse::<RendererBackend>()?,
        None => RendererBackend::Auto,
    };

    let event_loop = EventLoop::new()?;
    event_loop.set_control_flow(ControlFlow::Wait);
    event_loop.run_app(Application::new(renderer_backend))?;

    Ok(())
}

struct Application {
    renderer: Renderer,
    window: Option<Arc<dyn Window>>,
    scene: Option<Scene>,
}

impl Application {
    fn new(renderer_backend: RendererBackend) -> Self {
        Self {
            renderer: Renderer::new(renderer_backend),
            window: None,
            scene: None,
        }
    }

    fn draw_frame(&mut self) {
        let Some(scene) = self.scene.as_mut() else {
            return;
        };

        let (Some(device), Some(queue)) = (self.renderer.device(), self.renderer.queue()) else {
            return;
        };

        if let Err(error) = scene.draw_manager.prepare(device, queue) {
            log::error!("failed to prepare the frame: {error}");
            return;
        }
        
        self.renderer.render(wgpu::Color::BLACK, |render_pass| {
            scene.draw_manager.draw(render_pass);
        });
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let attributes = WindowAttributes::default()
            .with_title("gueiz 2d")
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

        match Scene::new(&self.renderer, surface_size) {
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

            WindowEvent::RedrawRequested => self.draw_frame(),

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
    square: String,
}

impl Scene {
    fn new(renderer: &Renderer, surface_size: SurfaceSize) -> Result<Self, Box<dyn Error>> {
        let (Some(device), Some(queue), Some(format)) = (
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
        ) else {
            return Err("the renderer has no surface yet".into());
        };

        let mut draw_manager =
            DrawManager::new(device, queue, format, &DrawManagerDescriptor::default())?;

        let mut square = object::create_object("Square");

        square.begin(PaintType::Fill);

        // 頂点は **0..1 の割合**で置く。実際の大きさは拡大で決める。
        //
        // 実寸（`0..width`）で置くと、窓が変わるたびに頂点を書き換えることになり、
        // そのたびに三角形を組み直して積み直すことになる。拡大なら形は変わらない。
        square.put_vertex(Vertex::new_position_color(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
        square.put_vertex(Vertex::new_position_color(1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0));
        square.put_vertex(Vertex::new_position_color(1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0));
        square.put_vertex(Vertex::new_position_color(0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0));

        square.end();
        square.camera(camera_for(surface_size));
        square.scale(
            surface_size.width as f32,
            surface_size.height as f32,
            1.0,
        );

        square.instance(instance::create_instance());

        let square = draw_manager.register(square);

        log::info!("scene: 1 object, 1 instance, draw calls: 1");

        Ok(Self {
            draw_manager,
            square,
        })
    }

    /// 窓の大きさが変わっても画面いっぱいのままにする。
    ///
    /// カメラだけ貼り直すと、四角形は作ったときの大きさのまま残る
    /// （1 単位 = 1 画素なので）。**拡大も合わせて更新する。**
    fn resize(&mut self, surface_size: SurfaceSize) {
        if let Some(object) = self.draw_manager.object_mut(&self.square) {
            object.camera(camera_for(surface_size));
            object.scale(
                surface_size.width as f32,
                surface_size.height as f32,
                1.0,
            );
        }
    }
}

fn camera_for(surface_size: SurfaceSize) -> Camera {
    let camera = Camera::orthographic_2d(surface_size.width as f32, surface_size.height as f32);
    camera.with_scale_mode(ScaleMode::Fixed)
}
