use std::error::Error;
use std::sync::Arc;
use wgpu::hal::DynCommandEncoder;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowId};
use gueiz::gpu::renderer::{Renderer, RendererBackend};
use gueiz_2d::draw_manager::DrawManager;

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

        if let Err(error) = scene.draw_manager.prepare(device,queue) {
            log::error!("failed to prepare the frame: {error}");
            return;
        }

        self.renderer.render(wgpu::Color{
            r: 0.0,
            g: 0.0,
            b: 0.0,
            a: 0.0,
        }, |render_pass| {
            scene.draw_manager.draw(render_pass);
        });
    }
}

impl ApplicationHandler for Application {
    fn can_create_surfaces(&mut self, event_loop: &dyn ActiveEventLoop) {
        todo!()
    }

    fn window_event(&mut self, event_loop: &dyn ActiveEventLoop, window_id: WindowId, event: WindowEvent) {
        todo!()
    }
}

struct Scene {
    draw_manager: DrawManager,
}