use std::hash::Hash;
use std::panic::panic_any;
use wgpu::Color;
use wgpu::naga::CooperativeRole::C;
use wgpu::naga::SwizzleComponent::W;
use wgpu::wgc::binding_model::GetBindGroupLayoutError;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::font::Font;
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::lines::{CurveRepresentationType, Lines};
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::text::{TextRenderer, TextStyle};

const CORNER_RADIUS: f32 = 10.0;
const LINE_WIDTH: f32 = 1.0;

pub struct WindowFrame {
    scale_mode: ScaleMode,
    title: String,
    frame_quad: Quad,
    title_bar_height: f32,
    gap: Gap,
    window_font: WindowFont,
    window_theme: WindowTheme,
    frame: Option<Object>,
    title_bar: Option<Object>,
    frame_outline: Option<Object>,
    title_bar_slice_line: Option<Object>,
    text_title: Option<TextRenderer>,
    register_name: RegisterName
}

struct RegisterName {
    pub frame: Option<String>,
    pub title_bar: Option<String>,
    pub frame_outline: Option<String>,
    pub title_bar_slice_line: Option<String>,
}

pub struct WindowTheme {
    pub frame_fill_color: ThemeColor,
    pub frame_outline_color: ThemeColor,
    pub title_bar_fill_color: ThemeColor,
    pub title_bar_slice_line_color: ThemeColor,
    pub text_title_color: ThemeColor,
}

pub struct WindowFont {
    pub base_font_path: String,
}

pub struct ThemeColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl WindowFrame {
    pub fn new(scale_mode: ScaleMode,title: String, frame_quad: Quad, title_bar_height: f32, gap: Gap, window_theme: WindowTheme) -> Self {
        Self {
            scale_mode,
            title,
            frame_quad,
            title_bar_height,
            gap,
            window_font: WindowFont {
                base_font_path: "C:/Windows/Fonts/YuGothM.ttc".to_string(),
            },
            window_theme,
            frame: None,
            title_bar: None,
            frame_outline: None,
            title_bar_slice_line: None,
            text_title: None,
            register_name: RegisterName {
                frame: None,
                title_bar: None,
                frame_outline: None,
                title_bar_slice_line: None,
            }
        }
    }

    pub fn create_object(&mut self, register_name: &str, surface_size: SurfaceSize) {
        let register_name_string = register_name.to_string();

        self.register_name.frame = Some(register_name_string.clone());
        let mut frame = RectRounded::new(register_name)
            .paint_type(PaintType::Fill)
            .vertex1(CornerType::None {}, Vertex::new_position_color(self.frame_quad.x,self.frame_quad.y + self.title_bar_height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex2(CornerType::None {}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width,self.frame_quad.y + self.title_bar_height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex3(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width,self.frame_quad.y + self.frame_quad.height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex4(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x,self.frame_quad.y + self.frame_quad.height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .end();
        frame.camera_for(surface_size, self.scale_mode);
        frame.instance(create_instance());

        let mut register_name_title_bar = register_name_string.clone();
        register_name_title_bar.push_str("-title_bar");
        self.register_name.title_bar = Some(register_name_title_bar);
        let mut title_bar = RectRounded::new(self.register_name.title_bar.clone().unwrap().as_str())
            .paint_type(PaintType::Fill)
            .vertex1(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x, self.frame_quad.y, 0.0, self.window_theme.title_bar_fill_color.r, self.window_theme.title_bar_fill_color.g, self.window_theme.title_bar_fill_color.b, self.window_theme.title_bar_fill_color.a))
            .vertex2(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width, self.frame_quad.y, 0.0, self.window_theme.title_bar_fill_color.r, self.window_theme.title_bar_fill_color.g, self.window_theme.title_bar_fill_color.b, self.window_theme.title_bar_fill_color.a))
            .vertex3(CornerType::None{}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width, self.frame_quad.y + self.title_bar_height, 0.0, self.window_theme.title_bar_fill_color.r, self.window_theme.title_bar_fill_color.g, self.window_theme.title_bar_fill_color.b, self.window_theme.title_bar_fill_color.a))
            .vertex4(CornerType::None{}, Vertex::new_position_color(self.frame_quad.x, self.frame_quad.y + self.title_bar_height, 0.0, self.window_theme.title_bar_fill_color.r, self.window_theme.title_bar_fill_color.g, self.window_theme.title_bar_fill_color.b, self.window_theme.title_bar_fill_color.a))
            .end();
        title_bar.camera_for(surface_size, self.scale_mode);
        title_bar.instance(create_instance());

        let mut register_name_frame_outline = register_name_string.clone();
        register_name_frame_outline.push_str("-frame_outline");
        self.register_name.frame_outline = Some(register_name_frame_outline);
        let mut frame_outline = RectRounded::new(self.register_name.frame_outline.clone().unwrap().as_str())
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::Bevel, strip: false, dash: None})
            .from(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x, self.frame_quad.y,0.0, self.window_theme.frame_outline_color.r,self.window_theme.frame_outline_color.g, self.window_theme.frame_outline_color.b,self.window_theme.frame_outline_color.a))
            .to_wh(Vertex::new_position_color(self.frame_quad.width, self.frame_quad.height,0.0, self.window_theme.frame_outline_color.r,self.window_theme.frame_outline_color.g, self.window_theme.frame_outline_color.b,self.window_theme.frame_outline_color.a))
            .end();
        frame_outline.camera_for(surface_size, self.scale_mode);
        frame_outline.instance(create_instance());

        let mut register_name_title_bar_slice_line = register_name_string.clone();
        register_name_title_bar_slice_line.push_str("-title_bar_slice_line");
        self.register_name.title_bar_slice_line = Some(register_name_title_bar_slice_line);
        let mut title_bar_slice_line = Lines::new(self.register_name.title_bar_slice_line.clone().unwrap().as_str())
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
            .curve_type(CurveRepresentationType::Normal)
            .point(Vertex::new_position_color(self.frame_quad.x + self.gap.x, self.frame_quad.y + self.title_bar_height, 0.0, self.window_theme.title_bar_slice_line_color.r, self.window_theme.title_bar_slice_line_color.g,self.window_theme.title_bar_slice_line_color.b,self.window_theme.title_bar_slice_line_color.a))
            .last_point(Vertex::new_position_color(self.frame_quad.x - self.gap.x + self.frame_quad.width, self.frame_quad.y + self.title_bar_height, 0.0, self.window_theme.title_bar_slice_line_color.r, self.window_theme.title_bar_slice_line_color.g,self.window_theme.title_bar_slice_line_color.b,self.window_theme.title_bar_slice_line_color.a))
            .end();
        title_bar_slice_line.camera_for(surface_size, self.scale_mode);
        title_bar_slice_line.instance(create_instance());

        let mut text_title = TextRenderer::new();
        text_title.camera_for(surface_size, self.scale_mode);
        text_title.color(self.window_theme.text_title_color.r,self.window_theme.text_title_color.g,self.window_theme.text_title_color.b,self.window_theme.text_title_color.a);

        self.frame = Some(frame);
        self.title_bar = Some(title_bar);
        self.frame_outline = Some(frame_outline);
        self.title_bar_slice_line = Some(title_bar_slice_line);
        self.text_title = Some(text_title);
    }

    pub fn register_draw_manager(&mut self, draw_manager: &mut DrawManager) {
        if let Some(frame) = self.frame.take() {
            draw_manager.register(frame);
        }
        if let Some(title_bar) = self.title_bar.take() {
            draw_manager.register(title_bar);
        }
        if let Some(frame_outline) = self.frame_outline.take() {
            draw_manager.register(frame_outline);
        }
        if let Some(title_bar_slice_line) = self.title_bar_slice_line.take() {
            draw_manager.register(title_bar_slice_line);
        }
        if let Some(text_title) = self.text_title.as_mut() {
            let data = std::fs::read(self.window_font.base_font_path.clone());
            let font = Font::from_bytes(data.unwrap().leak()).unwrap();

            text_title
                .write(
                    draw_manager,
                    &font,
                    self.title.as_str(),
                    &TextStyle::new(13.0),
                    self.frame_quad.x,
                    self.frame_quad.y,
                )
                .expect("TODO: panic message");
        }
    }

    pub fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize) {
        if let Some(register_name) = self.register_name.frame.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.frame_outline.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar_slice_line.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, self.scale_mode);
            }
        }
        if let Some(text_title) = self.text_title.as_mut() {
            text_title.camera_for(surface_size, self.scale_mode);
            
            for name in text_title.shape_ids() {
                if let Some(object) = draw_manager.object_mut(name) {
                    object.camera_for(surface_size,self.scale_mode);
                }
            }
        }
    }
}

pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

pub struct Gap {
    pub x: f32,
    pub y: f32,
}