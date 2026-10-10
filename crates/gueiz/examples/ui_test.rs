use std::error::Error;
use std::sync::Arc;
use log::info;
use wgpu::Color;
use wgpu::hal::DynCommandEncoder;
use winit::application::ApplicationHandler;
use winit::cursor::CursorIcon;
use winit::dpi::LogicalSize;
use winit::event::{ButtonSource, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use gueiz::gpu::camera::ScaleMode;
use gueiz::gpu::msaa::DEFAULT_MULTISAMPLE;
use gueiz::gpu::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::label::{Label, LabelFont, LabelRegisterName, LabelTheme};
use gueiz_2d::gui::window_frame::{DeviceId, Gap, Quad, ResizeHandle, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::instance::create_instance;
use gueiz_2d::object::Object;
use gueiz_2d::resource::Resources;

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
    /// いま出している指の形。変わったときだけ窓に伝える。
    cursor: CursorIcon,
}

impl Application {
    fn new(renderer_backend: RendererBackend) -> Self {
        let mut renderer = Renderer::new(renderer_backend);
        renderer.set_sample_count(DEFAULT_MULTISAMPLE);

        Self {
            renderer,
            window: None,
            scene: None,
            cursor: CursorIcon::Default,
        }
    }

    /// 窓の縁の上なら、大きさを変える形の指にする。
    ///
    /// 引いているあいだは変えない。指が縁から外れても、引いている向きの形のままにする。
    fn update_cursor(&mut self, x: f32, y: f32) {
        let (Some(window), Some(scene)) = (self.window.as_ref(), self.scene.as_ref()) else {
            return;
        };

        if scene.window_frame.is_resizing() {
            return;
        }

        let cursor = match scene.window_frame.resize_handle_at(&scene.draw_manager, x, y) {
            Some(ResizeHandle::Left | ResizeHandle::Right) => CursorIcon::EwResize,
            Some(ResizeHandle::Top | ResizeHandle::Bottom) => CursorIcon::NsResize,
            Some(ResizeHandle::TopLeft | ResizeHandle::BottomRight) => CursorIcon::NwseResize,
            Some(ResizeHandle::TopRight | ResizeHandle::BottomLeft) => CursorIcon::NeswResize,
            None => CursorIcon::Default,
        };

        if cursor != self.cursor {
            window.set_cursor(cursor.into());
            self.cursor = cursor;
        }
    }

    fn request_redraw(&self) {
        if let Some(window) = self.window.as_ref() {
            window.request_redraw();
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
        let scale_factor = window.scale_factor() as f32;

        log::info!("surface {}x{} (scale {scale_factor})", size.width, size.height);

        if let Err(error) = self.renderer.create_surface(window.clone(), surface_size) {
            log::error!("failed to create the surface: {error}");
            event_loop.exit();
            return;
        }

        match Scene::new(&self.renderer, surface_size, scale_factor) {
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
        match &event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::SurfaceResized(size) => {
                let surface_size = SurfaceSize::new(size.width, size.height);
                self.renderer.resize(surface_size);

                if let Some(scene) = self.scene.as_mut() {
                    scene.resize(surface_size);
                }
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let Some(window) = self.window.as_ref() else {
                    return;
                };

                let size = window.surface_size();
                let surface_size = SurfaceSize::new(size.width, size.height);

                self.renderer.resize(surface_size);

                if let Some(scene) = self.scene.as_mut() {
                    scene.rescale(surface_size, *scale_factor as f32);
                }

                window.request_redraw();
            }

            WindowEvent::RedrawRequested => self.draw_frame(),

            // macOS では最初の描画が窓の出る前に来て、`Occluded` で飛ばされる。
            // 常時回していないので、見えるようになったら描き直しを頼む。
            WindowEvent::Occluded(false) => self.request_redraw(),

            _ => {}
        }

        // 窓が動いたら描き直す。常時回していないので、これが無いと
        // 掴んで引いても絵が変わらない。
        let touched = self
            .scene
            .as_mut()
            .is_some_and(|scene| scene.window_event(&event));

        if touched {
            self.request_redraw();
        }

        if let WindowEvent::PointerMoved { position, .. } = &event {
            self.update_cursor(position.x as f32, position.y as f32);
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
    /// 窓が使う書体などを預かる。窓は取っ手だけを持つ。
    resources: Resources,
    window_frame: WindowFrame,
    scale_factor: f32,
}

impl Scene {
    fn new(
        renderer: &Renderer,
        surface_size: SurfaceSize,
        scale_factor: f32,
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

        let font_path = if cfg!(target_os = "windows") {
            "C:/Windows/Fonts/YuGothM.ttc".to_string()
        } else {
            "/System/Library/Fonts/Avenir Next.ttc".to_string()
        };

        let mut resources = Resources::new();
        let font_handle = resources.load_font_file("font", font_path)?;

        let mut window_frame = WindowFrame::new(
            ScaleMode::Stretch,
            "TestWindow1".to_string(),
            Quad {
                x: 100.0,
                y: 100.0,
                width: 500.0,
                height: 200.0,
            },
            30.0,
            Gap {
                x: 5.0,
                y: 0.0,
            },
            WindowFont {
                base_font: font_handle,
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
        // window_frame.add_window_item(
        //     Box::new(
        //         Label::new(
        //             "TestLabel1".to_string(),
        //             32.0,
        //             Gap {
        //                 x: 0.0,
        //                 y: 0.0,
        //             },
        //             LabelFont {
        //                 base_font: font_handle,
        //             },
        //             LabelTheme {
        //                 label_color: ThemeColor {
        //                     r: 0.0,
        //                     g: 0.0,
        //                     b: 0.0,
        //                     a: 1.0,
        //                 },
        //             }
        //         )
        //     )
        // );
        window_frame.create_object(
            &mut draw_manager,
            &resources,
            "TestWindowObject",
            surface_size,
            surface_size.to_logical(scale_factor),
        );

        Ok(Self {
            draw_manager,
            resources,
            window_frame,
            scale_factor,
        })
    }

    /// 指の入力を窓へ渡す。**窓が受け取ったら `true`。**
    ///
    /// 位置はサーフェスの画素なので、そのまま渡せます。
    /// 図形ごとの座標へ戻すのは窓の側の仕事です
    /// （カメラが図形ごとに違うので、呼ぶ側では決められません）。
    fn window_event(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::PointerButton {
                state,
                position,
                button,
                ..
            } => match (button, state) {
                (ButtonSource::Mouse(MouseButton::Left), ElementState::Pressed) => {
                    self.window_frame.mouse_left_pressed(
                        &self.draw_manager,
                        position.x as f32,
                        position.y as f32,
                    )
                }

                (ButtonSource::Mouse(MouseButton::Left), ElementState::Released) => {
                    self.window_frame.mouse_left_released()
                }

                (ButtonSource::Mouse(MouseButton::Right), ElementState::Pressed) => {
                    self.window_frame.mouse_right_pressed(
                        &self.draw_manager,
                        position.x as f32,
                        position.y as f32,
                    )
                }

                (ButtonSource::Mouse(MouseButton::Right), ElementState::Released) => {
                    self.window_frame.mouse_right_released()
                }

                _ => false,
            },

            WindowEvent::PointerMoved { position, .. } => self.window_frame.mouse_moved(
                &mut self.draw_manager,
                &self.resources,
                position.x as f32,
                position.y as f32,
            ),

            _ => false,
        }
    }

    fn resize(&mut self, surface_size: SurfaceSize) {
        self.window_frame.resize(
            &mut self.draw_manager,
            surface_size,
            surface_size.to_logical(self.scale_factor),
        );
    }

    fn rescale(&mut self, surface_size: SurfaceSize, scale_factor: f32) {
        self.scale_factor = scale_factor;
        self.resize(surface_size);
    }
}