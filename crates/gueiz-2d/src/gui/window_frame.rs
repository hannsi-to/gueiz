use std::hash::Hash;
use std::panic::panic_any;
use log::info;
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
use crate::text::{TextAlign, TextArea, TextRenderer, TextStyle};

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
    register_name: RegisterName,
    /// 頂点を組んだときの左上。**動かしても変わりません。**
    ///
    /// 動かすのに頂点を組み直すと、図形を登録し直すことになります
    /// （[`DrawManager`] に消す手立てが無いので名前が溜まります）。
    /// 代わりに [`Object::translate`] でずらすので、「どこから」ずらすのかを
    /// 覚えておく必要があります。
    built_origin: [f32; 2],
    /// 掴んでいるあいだだけ。
    drag: Option<Drag>,
    hidden: bool,
}

/// 掴んでいるあいだの覚え。
struct Drag {
    /// 掴んだ場所の、**窓の左上からのずれ**。
    ///
    /// 動いた量を足していくのではなくこれを覚えるのは、取りこぼしても
    /// ずれが溜まらないからです。指のどこを掴んだかは変わらないので、
    /// 「いまの位置 - このずれ」がいつでも正しい置き場所になります。
    grab: [f32; 2],
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
                // base_font_path: "C:/Windows/Fonts/YuGothM.ttc".to_string(),
                base_font_path: "/System/Library/Fonts/Avenir Next.ttc".to_string(),
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
            },
            built_origin: [0.0; 2],
            drag: None,
            hidden: false,
        }
    }

    pub fn create_object(&mut self, register_name: &str, surface_size: SurfaceSize, design: [f32; 2]) {
        // ずらす量はここからの差で出す。
        self.built_origin = [self.frame_quad.x, self.frame_quad.y];

        let register_name_string = register_name.to_string();

        self.register_name.frame = Some(register_name_string.clone());
        let mut frame = RectRounded::new(register_name)
            .paint_type(PaintType::Fill)
            .vertex1(CornerType::None {}, Vertex::new_position_color(self.frame_quad.x,self.frame_quad.y + self.title_bar_height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex2(CornerType::None {}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width,self.frame_quad.y + self.title_bar_height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex3(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x + self.frame_quad.width,self.frame_quad.y + self.frame_quad.height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .vertex4(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x,self.frame_quad.y + self.frame_quad.height,0.0,self.window_theme.frame_fill_color.r,self.window_theme.frame_fill_color.g,self.window_theme.frame_fill_color.b,self.window_theme.frame_fill_color.a))
            .end();
        frame.camera_for(surface_size, design, self.scale_mode);
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
        title_bar.camera_for(surface_size, design, self.scale_mode);
        title_bar.instance(create_instance());

        let mut register_name_frame_outline = register_name_string.clone();
        register_name_frame_outline.push_str("-frame_outline");
        self.register_name.frame_outline = Some(register_name_frame_outline);
        let mut frame_outline = RectRounded::new(self.register_name.frame_outline.clone().unwrap().as_str())
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::Bevel, strip: false, dash: None})
            .from(CornerType::Circle {radius: CORNER_RADIUS}, Vertex::new_position_color(self.frame_quad.x, self.frame_quad.y,0.0, self.window_theme.frame_outline_color.r,self.window_theme.frame_outline_color.g, self.window_theme.frame_outline_color.b,self.window_theme.frame_outline_color.a))
            .to_wh(Vertex::new_position_color(self.frame_quad.width, self.frame_quad.height,0.0, self.window_theme.frame_outline_color.r,self.window_theme.frame_outline_color.g, self.window_theme.frame_outline_color.b,self.window_theme.frame_outline_color.a))
            .end();
        frame_outline.camera_for(surface_size, design, self.scale_mode);
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
        title_bar_slice_line.camera_for(surface_size, design, self.scale_mode);
        title_bar_slice_line.instance(create_instance());

        let mut text_title = TextRenderer::new();
        text_title.camera_for(surface_size, design, self.scale_mode);
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
                .write_in(
                    draw_manager,
                    &font,
                    &self.title,
                    &TextStyle::new(13.0),
                    TextArea::new(self.frame_quad.x + self.gap.x, self.frame_quad.y, self.frame_quad.width - (self.gap.x * 2.0), self.title_bar_height)
                        .vertical(TextAlign::Center),
                )
                .expect("TODO: panic message");
        }
    }

    pub fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {
        if let Some(register_name) = self.register_name.frame.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.frame_outline.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(register_name) = self.register_name.title_bar_slice_line.as_ref() {
            if let Some(object) = draw_manager.object_mut(register_name.as_str()) {
                object.camera_for(surface_size, design, self.scale_mode);
            }
        }
        if let Some(text_title) = self.text_title.as_mut() {
            text_title.camera_for(surface_size, design, self.scale_mode);
            
            for name in text_title.shape_ids() {
                if let Some(object) = draw_manager.object_mut(name) {
                    object.camera_for(surface_size, design, self.scale_mode);
                }
            }
        }
    }

    // --- 動かす ---

    /// いまの左上。動かすとここが変わります。
    pub fn position(&self) -> [f32; 2] {
        [self.frame_quad.x, self.frame_quad.y]
    }

    /// 置き場所を決める。掴んでいなくても動かせます。
    pub fn set_position(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) {
        self.frame_quad.x = x;
        self.frame_quad.y = y;

        self.apply_offset(draw_manager);
    }

    /// 掴んでいるか。
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// 題名の帯。局所ではなく**いまの置き場所**での矩形。
    pub fn title_bar_quad(&self) -> Quad {
        Quad {
            x: self.frame_quad.x,
            y: self.frame_quad.y,
            width: self.frame_quad.width,
            height: self.title_bar_height,
        }
    }

    /// 組んだ場所からのずれを、持っている図形すべてに貼る。
    ///
    /// 頂点は組んだ場所に置いたままで、**親の変換だけ**をずらします。
    /// [`Object::translate`] は複製より手前に掛かるので、字のように
    /// 複製をたくさん持つ図形も、1 回で丸ごと動きます。
    fn apply_offset(&self, draw_manager: &mut DrawManager) {
        let x = self.frame_quad.x - self.built_origin[0];
        let y = self.frame_quad.y - self.built_origin[1];

        for name in self.object_names() {
            if let Some(object) = draw_manager.object_mut(&name) {
                object.translate(x, y, 0.0);
            }
        }
    }

    /// この窓が持っている図形の名前。字の形も入ります。
    fn object_names(&self) -> Vec<String> {
        let mut names = Vec::new();

        for name in [
            self.register_name.frame.as_ref(),
            self.register_name.title_bar.as_ref(),
            self.register_name.frame_outline.as_ref(),
            self.register_name.title_bar_slice_line.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            names.push(name.clone());
        }

        if let Some(text_title) = self.text_title.as_ref() {
            names.extend(text_title.shape_ids().map(String::from));
        }

        names
    }

    /// この窓の図形か。
    fn owns(&self, name: &str) -> bool {
        self.object_names().iter().any(|owned| owned == name)
    }

    /// 画素の位置を、図形を組んだ座標へ戻す。
    ///
    /// カメラは図形ごとに持っているので、この窓のものを 1 つ借りて通します。
    /// **[`ScaleMode`] が相対だと画素位置と座標が一致しない**ので、
    /// 掴む判定も動かす量も、必ずここを通してから出します。
    fn to_world(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<[f32; 2]> {
        let name = self.register_name.title_bar.as_ref()?;
        let object = draw_manager.object(name.as_str())?;

        object.view_camera().screen_to_world(x, y)
    }

    // --- 指 ---

    /// 左を押した。**この窓が取ったら `true`。**
    ///
    /// 取ったら呼ぶ側は後ろのものへ渡さないでください。窓が重なっているとき、
    /// 下の窓まで一緒に動いてしまいます。
    ///
    /// 掴むのは題名の帯の上だけです。中身の上で掴めると、中に置いたつまみが
    /// 動かせなくなります。
    pub fn mouse_left_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };

        // 帯の上か。**手前にあるものが自分のものか**も見る。
        // 別の窓がかぶさっているなら、そちらが取るべき。
        if !self.title_bar_quad().hover(world[0], world[1]) {
            return false;
        }

        match draw_manager.pick(x, y) {
            Some(pick) if self.owns(&pick.name) => {}
            // 何かがかぶさっている、または当たっていない。
            _ => return false,
        }

        self.drag = Some(Drag {
            grab: [world[0] - self.frame_quad.x, world[1] - self.frame_quad.y],
        });

        true
    }

    /// 左を放した。掴んでいたら `true`。
    pub fn mouse_left_released(&mut self) -> bool {
        self.drag.take().is_some()
    }

    pub fn mouse_right_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        let _ = (draw_manager, x, y);
        false
    }

    pub fn mouse_right_released(&mut self) -> bool {
        false
    }

    /// 指が動いた。掴んでいれば窓が付いてきて `true`。
    ///
    /// **枠の外へ出ても離しません。** 放すまでは動かし続けます。
    /// 離すと、少し外れた瞬間に窓が置き去りになります。
    pub fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some(drag) = self.drag.as_ref() else {
            return false;
        };

        let grab = drag.grab;

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };

        // 掴んだ場所が指の下に来るように置く。動いた量を足していくのではないので、
        // 取りこぼしてもずれが溜まらない。
        self.frame_quad.x = world[0] - grab[0];
        self.frame_quad.y = world[1] - grab[1];

        self.apply_offset(draw_manager);

        true
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub i64);

pub struct Quad {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Quad {
    pub fn hover(&self, x: f32, y: f32) -> bool {
        self.x < x && self.y < y && self.x + self.width > x && self.y + self.height > y
    }
}

pub struct Gap {
    pub x: f32,
    pub y: f32,
}