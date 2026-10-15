use log::info;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::font::Font;
use crate::gui::window_frame::{Gap, QuadType, ThemeColor};
use crate::gui::window_item::{replace_outline, CursorShape, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::PaintType;
use crate::resource::FontHandle;
use crate::text::{TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// 押せる四角。
///
/// 上で押して、**指を上に置いたまま放すと** [`Button::on_click`] の関数が呼ばれます。
/// 押したまま外へ出て放したときは呼ばれません（押し間違いを取り消せるように）。
///
/// 指が上にある・押しているあいだは、[`ButtonTheme`] の色に塗り替えます。
pub struct Button {
    text: String,
    text_size: f32,
    width: f32,
    height: f32,
    quad_type: QuadType,
    gap: Gap,
    button_font: ButtonFont,
    button_theme: ButtonTheme,
    on_click: Option<Box<dyn FnMut()>>,
    pub(crate) button_register_name: ButtonRegisterName,
    /// 組んだときの左上（余白を足す前）。色を変えるときに、同じ場所で組み直す。
    built_at: [f32; 2],
    /// 指が上にあるか。
    hovered: bool,
    /// このボタンの上で押されて、まだ放されていないか。
    armed: bool,
}

pub struct ButtonRegisterName {
    background: Option<String>,
    text: Option<String>,
}

pub struct ButtonFont {
    pub base_font: FontHandle,
}

pub struct ButtonTheme {
    pub fill_color: ThemeColor,
    /// 指が上にあるとき。
    pub hover_fill_color: ThemeColor,
    /// 押しているあいだ。
    pub pressed_fill_color: ThemeColor,
    pub text_color: ThemeColor,
}

impl Button {
    pub fn new(register_name_string: String, text: String, text_size: f32, width: f32, height: f32, quad_type: QuadType, gap: Gap, button_font: ButtonFont, button_theme: ButtonTheme) -> Self {
        Self {
            text,
            text_size,
            width,
            height,
            quad_type,
            gap,
            button_font,
            button_theme,
            on_click: None,
            button_register_name: ButtonRegisterName {
                background: Some(register_name_string),
                text: None,
            },
            built_at: [0.0; 2],
            hovered: false,
            armed: false,
        }
    }

    /// 押されたときに呼ぶ関数。
    pub fn on_click(mut self, on_click: impl FnMut() + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }

    /// いま押しているか（このボタンの上で押して、指がまだ上にある）。
    pub fn is_pressed(&self) -> bool {
        self.armed && self.hovered
    }

    /// いまの見た目に合う塗りの色。
    fn fill_color(&self) -> &ThemeColor {
        let theme = &self.button_theme;

        if self.is_pressed() {
            &theme.pressed_fill_color
        } else if self.hovered {
            &theme.hover_fill_color
        } else {
            &theme.fill_color
        }
    }

    /// 地の四角を組む。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_background(&self, name: &str) -> Object {
        let [x, y] = self.built_at;
        let (left, top) = (x + self.gap.x, y + self.gap.y);
        let (right, bottom) = (left + self.width, top + self.height);

        let color = self.fill_color();
        let vertex = |x: f32, y: f32| Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a);

        match self.quad_type {
            QuadType::Normal => {
                Rect::new(name)
                    .paint_type(PaintType::Fill)
                    .from(vertex(left, top))
                    .to(vertex(right, bottom))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(name)
                    .paint_type(PaintType::Fill)
                    .from(CornerType::Circle { radius }, vertex(left, top))
                    .to(vertex(right, bottom))
                    .end()
            }
        }
    }

    /// 字をボタンのまん中に置く。
    fn text_layout_data(&self) -> TextLayoutData {
        let [x, y] = self.built_at;

        TextLayoutData {
            text: self.text.clone(),
            style: TextStyle::new(self.text_size),
            text_location: TextLocation::Area {
                area: TextArea::new(x + self.gap.x, y + self.gap.y, self.width, self.height).centered(),
            },
        }
    }

    /// いまの色と場所で、地の四角を登録済みの図形の中に組み直す。
    fn refresh_background(&self, draw_manager: &mut DrawManager) {
        let Some(object) = self
            .button_register_name
            .background
            .as_deref()
            .and_then(|name| draw_manager.object_mut(name))
        else {
            return;
        };

        replace_outline(object, &self.build_background(""));
    }

    /// 指の下で一番手前にあるのが、このボタンの図形か。
    ///
    /// 位置で比べず [`DrawManager::pick`] に聞くのは、窓のずらしやカメラを
    /// 通した当たり判定をそのまま使えるのと、別の窓がかぶさっているときに
    /// 反応しないようにするため。
    fn hit(&self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        let Some(pick) = draw_manager.pick(x, y) else {
            return false;
        };

        if self.button_register_name.background.as_deref() == Some(pick.name.as_str()) {
            return true;
        }

        self.button_register_name
            .text
            .as_deref()
            .is_some_and(|text| draw_manager.group_names(text).any(|name| name == pick.name))
    }

    /// 指が上にあるか・押しているかを変えて、見た目が変わったら塗り直す。変わったら `true`。
    fn set_state(&mut self, draw_manager: &mut DrawManager, hovered: bool, armed: bool) -> bool {
        if (self.hovered, self.armed) == (hovered, armed) {
            return false;
        }

        self.hovered = hovered;
        self.armed = armed;
        self.refresh_background(draw_manager);

        true
    }

    fn register_text(text_renderer: &mut TextRenderer, draw_manager: &mut DrawManager, font: &Font) {
        if let Err(error) = text_renderer.register_draw_manager(draw_manager, font) {
            log::warn!("failed to lay out the button text: {error}");
        }
    }
}

impl WindowItem for Button {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        let Some(local_name) = self.button_register_name.background.as_deref() else {
            return (0.0, 0.0);
        };
        let name = format!("{register_name}-{local_name}-{counter}");

        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];

        let mut background = self.build_background(&name);
        background.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
        background.instance(create_instance());
        // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
        let name = window_item_arguments.draw_manager.register(background);

        let text_name = format!("{name}-text");
        let mut text_renderer = TextRenderer::new(&text_name);
        let color = &self.button_theme.text_color;
        text_renderer.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
        text_renderer.color(color.r, color.g, color.b, color.a);
        text_renderer.text_layout_data(self.text_layout_data());

        match window_item_arguments.resources.font(self.button_font.base_font) {
            Some(font) => Self::register_text(&mut text_renderer, window_item_arguments.draw_manager, &font),
            None => log::warn!("the button font is not in the resources; the button text is not drawn"),
        }

        if let Some(replaced) = window_item_arguments.draw_manager.add_text_renderer(text_renderer) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }

        self.button_register_name.background = Some(name);
        self.button_register_name.text = Some(text_name);

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.built_at = [window_item_arguments.item_x, window_item_arguments.item_y];
        self.refresh_background(window_item_arguments.draw_manager);

        let layout_data = self.text_layout_data();
        if let (Some(text), Some(font)) = (
            self.button_register_name.text.as_deref(),
            window_item_arguments.resources.font(self.button_font.base_font),
        ) {
            window_item_arguments.draw_manager.with_text_renderer(text, |text_renderer, draw_manager| {
                text_renderer.clear(draw_manager);
                text_renderer.text_layout_data(layout_data);

                Self::register_text(text_renderer, draw_manager, &font);
            });
        }

        (self.gap.x + self.width, self.gap.y + self.height)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut names: Vec<String> = self.button_register_name.background.iter().cloned().collect();

        if let Some(text) = &self.button_register_name.text {
            names.extend(draw_manager.group_names(text).map(String::from));
        }

        names
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.hit(draw_manager, x, y) {
            return false;
        }

        self.set_state(draw_manager, true, true);
        true
    }

    fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if !self.armed {
            return false;
        }

        let hovered = self.hit(draw_manager, x, y);
        self.set_state(draw_manager, hovered, false);

        if hovered {
            if let Some(on_click) = self.on_click.as_mut() {
                on_click();
            }
        }

        true
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<CursorShape> {
        self.hit(draw_manager, x, y).then_some(CursorShape::Pointer)
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let hovered = self.hit(draw_manager, x, y);
        let armed = self.armed;

        self.set_state(draw_manager, hovered, armed)
    }
}
