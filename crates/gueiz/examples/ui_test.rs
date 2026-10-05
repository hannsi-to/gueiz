use std::error::Error;
use std::sync::Arc;
use wgpu::Color;
use wgpu::hal::DynCommandEncoder;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use gueiz::gpu::camera::ScaleMode;
use gueiz::gpu::msaa::DEFAULT_MULTISAMPLE;
use gueiz::gpu::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::window_frame::{Gap, Quad, ThemeColor, WindowFrame, WindowTheme};
use gueiz_2d::instance::create_instance;
use gueiz_2d::object::Object;

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
        let mut renderer = Renderer::new(renderer_backend);
        renderer.set_sample_count(DEFAULT_MULTISAMPLE);

        Self {
            renderer,
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

        if let Err(error) = scene.draw_manager.prepare(device,queue) {
            log::error!("failed to prepare the frame: {error}");
            return;
        }

        self.renderer.render(wgpu::Color{
            r: 0.13,
            g: 0.14,
            b: 0.16,
            a: 1.0,
        }, |render_pass| {
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

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
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
    window_frame: WindowFrame,
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

        let descriptor = DrawManagerDescriptor {
            sample_count: renderer.sample_count(),
            ..DrawManagerDescriptor::default()
        };

        let mut draw_manager = DrawManager::new(device, queue, format, &descriptor)?;

        let mut window_frame = WindowFrame::new(
            ScaleMode::Fixed,
            "TestWindow1".to_string(),
            Quad {
                x: 100.0,
                y: 100.0,
                width: 500.0,
                height: 200.0,
            },
            20.0,
            Gap {
                x: 5.0,
                y: 0.0,
            },
            WindowTheme {
                frame_fill_color: ThemeColor {
                    r: 0.1,
                    g: 0.1,
                    b: 0.1,
                    a: 1.0,
                },
                frame_outline_color: ThemeColor {
                    r: 0.5,
                    g: 0.5,
                    b: 0.5,
                    a: 1.0,
                },
                title_bar_fill_color: ThemeColor {
                    r: 0.1,
                    g: 0.1,
                    b: 0.1,
                    a: 1.0,
                },
                title_bar_slice_line_color: ThemeColor {
                    r: 0.24,
                    g: 0.24,
                    b: 0.24,
                    a: 1.0,
                },
                text_title_color: ThemeColor {
                    r: 1.0,
                    g: 1.0,
                    b: 1.0,
                    a: 1.0,
                }
            }
        );
        window_frame.create_object("TestWindowObject", surface_size);
        window_frame.register_draw_manager(&mut draw_manager);

        Ok(Self {
            draw_manager,
            window_frame,
        })
    }

    fn resize(&mut self, surface_size: SurfaceSize) {
        self.window_frame.resize(&mut self.draw_manager, surface_size);
    }
}