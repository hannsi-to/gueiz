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
use rand::RngExt;
use gueiz_2d::camera::{Camera, ScaleMode};
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::{self, instance, object};
use gueiz_2d::paint_type::{Dash, JointType, PaintType};
use gueiz_2d::renderer::{Renderer, RendererBackend, SurfaceSize};
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};
use gueiz_2d::effect::Block;
use gueiz_2d::instance::create_instance;
use gueiz_2d::objects::circle::Circle;
use gueiz_2d::objects::lines::{CurveRepresentationType, Lines};
use gueiz_2d::objects::points::{PointType, Points};
use gueiz_2d::objects::polygon_rounded::{CornerType, PolygonRounded};
use gueiz_2d::objects::rect::Rect;
use gueiz_2d::objects::rect_rounded::RectRounded;
use gueiz_2d::objects::triangle::Triangle;
use gueiz_2d::paint_type::PaintType::{Fill, Stroke};
use gueiz_2d::pipeline_state::StencilOperation::Keep;

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

        // let mut square = object::create_object("Square");
        //
        // square.begin(PaintType::Fill);
        //
        // 頂点は **0..1 の割合**で置く。実際の大きさは拡大で決める。
        //
        // 実寸（`0..width`）で置くと、窓が変わるたびに頂点を書き換えることになり、
        // そのたびに三角形を組み直して積み直すことになる。拡大なら形は変わらない。
        // square.put_vertex(Vertex::new_position_color(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
        // square.put_vertex(Vertex::new_position_color(1.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0));
        // square.put_vertex(Vertex::new_position_color(1.0, 1.0, 0.0, 1.0, 1.0, 1.0, 1.0));
        // square.put_vertex(Vertex::new_position_color(0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0));
        //
        // square.end();
        // square.camera(camera_for(surface_size));
        // square.scale(
        //     surface_size.width as f32,
        //     surface_size.height as f32,
        //     1.0,
        // );
        // square.effect(Block::ClipRect {
        //     min: [64.0, 64.0],
        //     max: [192.0, 192.0],
        //     radius: 40.0,      // 角丸
        //     softness: 0.0,     // 0 でも縁は 1 画素なめらか
        //     invert: true,     // 立てると矩形の中が抜ける
        // });
        //
        // square.instance(create_instance());
        //
        // let square = draw_manager.register(square);

        // let mut rect1 = Rect::new("rect1")
        //     .paint_type(PaintType::Stroke {
        //         line_width: 50.0,
        //         joint_type: JointType::Round,
        //         strip: false,
        //     })
        //     .paint_type(Fill)
        //     .from(Vertex::new_position_color(100.0, 100.0, 0.0, 0.0, 0.0, 0.0, 1.0))
        //     .to(Vertex::new_position_color(300.0, 200.0, 0.0, 1.0, 1.0, 1.0, 1.0))
        //     .end();
        // rect1.camera(camera_for(surface_size));
        // rect1.instance(instance::create_instance());
        // draw_manager.register(rect1);

        // let mut triangle1 = Triangle::new("triangle1")
        //     .paint_type(Fill)
        //     .paint_type(Stroke {
        //         line_width: 20.0,
        //         joint_type: JointType::Miter,
        //         strip: false,
        //     })
        //     .vertex1(Vertex::new_position_color(100.0, 100.0, 0.0, 1.0, 0.0, 0.0, 1.0))
        //     .vertex2(Vertex::new_position_color(400.0, 100.0, 0.0, 1.0, 1.0, 0.0, 1.0))
        //     .vertex3(Vertex::new_position_color(150.0, 300.0, 0.0, 1.0, 0.0, 1.0, 1.0))
        //     .end();
        // triangle1.camera(camera_for(surface_size));
        // triangle1.instance(instance::create_instance());
        // draw_manager.register(triangle1);

        // let mut points = Points::new("points1")
        //     .paint_type(Fill)
        //     .point(
        //         PointType::Circle {
        //             radius: 10.0,
        //         }, Vertex::new_position_color(100.0,100.0,0.0,1.0,0.0,0.0,1.0)
        //     )
        //     .point(
        //         PointType::Circle {
        //             radius: 10.0,
        //         }, Vertex::new_position_color(200.0,200.0,0.0,1.0,0.0,0.0,1.0)
        //     )
        //     .point(
        //         PointType::Rect {
        //             width: 10.0,
        //             height: 20.0,
        //         }, Vertex::new_position_color(300.0,300.0,0.0,1.0,0.0,0.0,1.0)
        //     )
        //     .point(
        //         PointType::Circle {
        //             radius: 10.0,
        //         }, Vertex::new_position_color(400.0,400.0,0.0,1.0,0.0,0.0,1.0)
        //     )
        //     .last_point(
        //         PointType::Rect {
        //             width: 10.0,
        //             height: 10.0,
        //         }, Vertex::new_position_color(500.0,500.0,0.0,1.0,0.0,0.0,1.0)
        //     )
        //     .end();
        // points.camera(camera_for(surface_size));
        // points.instance(create_instance());
        // draw_manager.register(points);

        let at = |x, y| Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0);
        // let mut polygon_rounded = PolygonRounded::new("polygon_rounded1")
        //     .paint_type(Fill)
            // .vertex(CornerType::None { }, at(0.0, 0.0))
            // .vertex(CornerType::None { }, at(100.0, 0.0))
            // .vertex(CornerType::None { }, at(100.0,100.0))
            // .last_vertex(CornerType::None { }, at(0.0, 100.0))
            // .vertex(CornerType::Circle { radius: 20.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Circle { radius: 10.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Circle { radius: 20.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Circle { radius: 10.0 }, at(0.0, 100.0))
            // .vertex(CornerType::Ellipse { radius_x: 20.0, radius_y: 10.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Ellipse { radius_x: 10.0, radius_y: 20.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Ellipse { radius_x: 20.0, radius_y: 10.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Ellipse { radius_x: 10.0, radius_y: 20.0 }, at(0.0, 100.0))
            // .vertex(CornerType::Bevel { radius: 20.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Bevel { radius: 10.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Bevel { radius: 20.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Bevel { radius: 10.0 }, at(0.0, 100.0))
            // .vertex(CornerType::Chanfer { radius_x: 20.0, radius_y: 10.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Chanfer { radius_x: 10.0, radius_y: 20.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Chanfer { radius_x: 20.0, radius_y: 10.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Chanfer { radius_x: 10.0, radius_y: 20.0 }, at(0.0, 100.0))
            // .vertex(CornerType::Inset { radius_x: 20.0, radius_y: 10.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Inset { radius_x: 10.0, radius_y: 20.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Inset { radius_x: 20.0, radius_y: 10.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Inset { radius_x: 10.0, radius_y: 20.0 }, at(0.0, 100.0))
            // .vertex(CornerType::Concave { radius_x: 20.0, radius_y: 10.0 }, at(0.0, 0.0))
            // .vertex(CornerType::Concave { radius_x: 10.0, radius_y: 20.0 }, at(100.0, 0.0))
            // .vertex(CornerType::Concave { radius_x: 20.0, radius_y: 10.0 }, at(100.0,100.0))
            // .last_vertex(CornerType::Concave { radius_x: 10.0, radius_y: 20.0 }, at(0.0, 100.0))
            // .end();
        // polygon_rounded.camera(camera_for(surface_size));
        // polygon_rounded.instance(create_instance());
        // draw_manager.register(polygon_rounded);

        // let mut rect_rounded = RectRounded::new("rect_rounded1")
        //     .paint_type(Fill)
        //     .from(CornerType::Circle { radius: 12.0 }, at(0.0, 0.0))
        //     .to(at(200.0, 100.0))
        //     .vertex1(CornerType::None {},                 at(0.0, 0.0))
            // .vertex2(CornerType::Bevel { radius: 20.0 },  at(100.0, 0.0))
            // .vertex3(CornerType::Circle { radius: 20.0 }, at(100.0, 100.0))
            // .vertex4(CornerType::Inset { radius_x: 20.0, radius_y: 20.0 }, at(0.0, 100.0))
            // .end();
        // rect_rounded.camera(camera_for(surface_size));
        // rect_rounded.instance(create_instance());
        // draw_manager.register(rect_rounded);

        let mut rng = rand::rng();
        // let mut circle = Circle::new("circle1")
        //     .paint_type(Fill)
        //     .vertex_center(Vertex::new_position_color(400.0,400.0,0.0,1.0,1.0,1.0,1.0))
        //     .radius(200.0)
        //     .segment_count(64)
        //     .start_angle_degrees(90.0)
        //     .end_angle_degrees(270.0)
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .add_outer_color((rng.random(), rng.random(), rng.random(), 1.0))
        //     .end_circle_option()
        //     .end();
        // circle.camera(camera_for(surface_size));
        // circle.instance(create_instance());
        // draw_manager.register(circle);

        let mut curve = Lines::new("curve")
            .paint_type(Stroke {
                line_width: 10.0,
                joint_type: JointType::Round,
                strip: true,
                // dash: Some(Dash::new(8.0,4.0)),
                dash: Some(Dash::dots(10.0)),
                // dash: Some(Dash::pattern(&[4.0,2.0,1.0,4.0,2.0,1.0]))
            })
            .curve_type(CurveRepresentationType::CatmullRomSpline)
            .segment_count(128)
            .point(Vertex::new_position_color(0.0, 0.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(30.0, 60.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(100.0, 200.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(200.0, 300.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(300.0, 600.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(400.0, 500.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(500.0, 600.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .point(Vertex::new_position_color(400.0, 200.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .last_point(Vertex::new_position_color(200.0, 400.0,0.0,rng.random(), rng.random(), rng.random(), 1.0))
            .end();
        curve.camera(camera_for(surface_size));
        curve.instance(create_instance());
        draw_manager.register(curve);

        log::info!("scene: 1 object, 1 instance, draw calls: 1");

        Ok(Self {
            draw_manager,
        })
    }

    /// 窓の大きさが変わっても画面いっぱいのままにする。
    ///
    /// カメラだけ貼り直すと、四角形は作ったときの大きさのまま残る
    /// （1 単位 = 1 画素なので）。**拡大も合わせて更新する。**
    fn resize(&mut self, surface_size: SurfaceSize) {
        if let Some(object) = self.draw_manager.object_mut("Square") {
            object.camera(camera_for(surface_size));
            object.scale(
                surface_size.width as f32,
                surface_size.height as f32,
                1.0,
            );
        }

        if let Some(object) = self.draw_manager.object_mut("rect1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("triangle1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("points1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("polygon_rounded1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("rect_rounded1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("circle1") {
            object.camera(camera_for(surface_size));
        }

        if let Some(object) = self.draw_manager.object_mut("curve") {
            object.camera(camera_for(surface_size));
        }
    }
}

fn camera_for(surface_size: SurfaceSize) -> Camera {
    let camera = Camera::orthographic_2d(surface_size.width as f32, surface_size.height as f32);
    camera.with_scale_mode(ScaleMode::Fixed)
}
