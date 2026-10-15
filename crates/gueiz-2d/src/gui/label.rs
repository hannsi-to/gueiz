use log::error;
use ttf_parser::Width;
use gueiz_gpu::error::Gueiz2DError;
use gueiz_gpu::renderer::SurfaceSize;
use crate::clip::ClipMaskKind;
use crate::draw_manager::DrawManager;
use crate::font::Font;
use crate::format::Token::Text;
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::{WindowItemArguments1, WindowItem, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::resource::FontHandle;
use crate::text::{TextAlign, TextArea, TextLayout, TextLayoutData, TextLocation, TextRenderer, TextStyle};

pub struct Label {
    label: String,
    label_size: f32,
    gap: Gap,
    label_font: LabelFont,
    label_theme: LabelTheme,
    pub(crate) label_register_name: LabelRegisterName,
}

pub struct LabelRegisterName {
    text_label: Option<String>,
}

pub struct LabelTheme {
    pub label_color : ThemeColor,
}

pub struct LabelFont {
    pub base_font: FontHandle,
}

impl Label {
    pub fn new(register_name_string: String, label: String, label_size: f32, gap: Gap,label_font: LabelFont, label_theme: LabelTheme) -> Self {
        Self {
            label,
            label_size,
            gap,
            label_font,
            label_theme,
            label_register_name: LabelRegisterName {
                text_label: Some(register_name_string),
            },
        }
    }
    
    fn build_shapes(&self) -> Vec<Object> {
        vec![]
    }

    fn text_layout_data(&self, window_item_arguments: &WindowItemArguments2) -> TextLayoutData {
        TextLayoutData {
            text: self.label.clone(),
            style: TextStyle::new(self.label_size),
            text_location: TextLocation::Area {
                area: TextArea::new(
                    window_item_arguments.item_x + self.gap.x,
                    window_item_arguments.item_y + self.gap.y,
                    window_item_arguments.width,
                    self.label_size
                ).vertical(TextAlign::Center)
            }
        }
    }

    fn relayout_data(&self, window_item_arguments: &mut WindowItemArguments2<'_>, ) -> (f32, f32) {
        let layout_data = self.text_layout_data(window_item_arguments);

        let (Some(name), Some(font)) = (
            self.label_register_name.text_label.as_deref(),
            window_item_arguments.resources.font(self.label_font.base_font),
        ) else {
            return (0.0, 0.0);
        };

        let gap = &self.gap;
        window_item_arguments.draw_manager.with_text_renderer(name, |text_label, draw_manager| {
            text_label.clear(draw_manager);
            text_label.text_layout_data(layout_data);

            Self::register_draw_manager(gap, text_label, draw_manager, &font)
        }).unwrap_or((0.0, 0.0))
    }

    /// 字を登録して、余白込みの `(幅, 高さ)` を返す。
    ///
    /// `self` を取らないのは、[`DrawManager::with_text_renderer`] の中から
    /// 名前を借りたまま呼べるようにするため。
    fn register_draw_manager(gap: &Gap, text_label: &mut TextRenderer, draw_manager: &mut DrawManager, font: &Font) -> (f32, f32) {
        match text_label.register_draw_manager(draw_manager, font) {
            Ok(text_layout) => {
                (gap.x + text_layout.width, gap.y + text_layout.height)
            }
            Err(error) => {
                log::warn!("failed to lay out the label: {error}");
                (0.0, 0.0)
            }
        }
    }
}

impl WindowItem for Label {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        // 同じ名前の部品を 2 つ置いても重ならないよう、窓の中の番号も足す。
        if let Some(name) = self.label_register_name.text_label.as_mut() {
            *name = format!("{register_name}-{name}-{counter}");
        }

        let mut objects = self.build_shapes();
        for object in &mut objects {
            object.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
            object.instance(create_instance());
        }

        let mut text_label = TextRenderer::new(self.label_register_name.text_label.as_deref().unwrap_or_default());
        text_label.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
        text_label.color(self.label_theme.label_color.r,self.label_theme.label_color.g,self.label_theme.label_color.b,self.label_theme.label_color.a);
        text_label.text_layout_data(self.text_layout_data(&WindowItemArguments2 {
            draw_manager: window_item_arguments.draw_manager,
            resources: window_item_arguments.resources,
            register_name,
            counter: window_item_arguments.counter,
            item_x: window_item_arguments.item_x,
            item_y: window_item_arguments.item_y,
            width: window_item_arguments.width,
            scale_mode: window_item_arguments.scale_mode,
        }));

        // 窓の外を削る覆いは、窓が object_names() の図形すべてに積みます。
        // 字の形はこの下の register_draw_manager で初めて登録されるので、
        // ここで shape_ids() を回しても何もありません。
        let Some(font) = window_item_arguments.resources.font(self.label_font.base_font) else {
            log::warn!("the label font is not in the resources; the label is not drawn");
            return (0.0, 0.0);
        };
        let size = Self::register_draw_manager(&self.gap, &mut text_label, window_item_arguments.draw_manager, &font);

        // 組み直しで with_text_renderer から引けるよう、DrawManager に預ける。
        if let Some(replaced) = window_item_arguments.draw_manager.add_text_renderer(text_label) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }

        size
    }

    fn rebuild(&mut self, mut window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.relayout_data(&mut window_item_arguments)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut names = Vec::new();
        if let Some(text_label) = &self.label_register_name.text_label {
            names.extend(draw_manager.group_names(text_label).map(String::from));
        }
        names
    }
}