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
    text_label: Option<TextRenderer>,
    pub(crate) label_register_name: LabelRegisterName,
}

pub struct LabelRegisterName {
    label: Option<String>,
}

pub struct LabelTheme {
    pub label_color : ThemeColor,
}

pub struct LabelFont {
    pub base_font: FontHandle,
}

impl Label {
    pub fn new(label: String, label_size: f32, gap: Gap,label_font: LabelFont, label_theme: LabelTheme) -> Self {
        Self {
            label,
            label_size,
            gap,
            label_font,
            label_theme,
            text_label: None,
            label_register_name: LabelRegisterName {
                label: None,
            },
        }
    }
}

impl Label {
    fn build_shapes(&self) -> Vec<Object> {
        vec![]
    }

    fn text_layout_data(&self, create_object_arguments: &WindowItemArguments2) -> TextLayoutData {
        TextLayoutData {
            text: self.label.clone(),
            style: TextStyle::new(self.label_size),
            text_location: TextLocation::Area {
                area: TextArea::new(
                    create_object_arguments.item_x + self.gap.x,
                    create_object_arguments.item_y + self.gap.y,
                    create_object_arguments.width,
                    self.label_size
                ).vertical(TextAlign::Center)
            }
        }
    }

    fn relayout_data(&mut self, create_object_arguments: &mut WindowItemArguments2) -> (f32, f32) {
        let layout_data = self.text_layout_data(create_object_arguments);

        let (Some(text_label), Some(font)) = (self.text_label.as_mut(), create_object_arguments.resources.font(self.label_font.base_font)) else {
            return (0.0, 0.0);
        };

        text_label.clear(&mut create_object_arguments.draw_manager);
        text_label.text_layout_data(layout_data);

        match text_label.register_draw_manager(create_object_arguments.draw_manager, &font) {
            Ok(text_layout) => {
                (self.gap.x + text_layout.width, self.gap.y + text_layout.height)
            }
            Err(error) => {
                log::warn!("failed to lay out the title again: {error}");
                (0.0, 0.0)
            }
        }
    }
}

impl WindowItem for Label {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        self.label_register_name.label = Some(format!("{register_name}-label-{counter}"));

        let mut objects = self.build_shapes();
        for object in &mut objects {
            object.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
            object.instance(create_instance());
        }

        let mut text_label = TextRenderer::new(self.label_register_name.label.clone().unwrap().as_str());
        text_label.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
        text_label.color(self.label_theme.label_color.r,self.label_theme.label_color.g,self.label_theme.label_color.b,self.label_theme.label_color.a);
        text_label.text_layout_data(self.text_layout_data(&WindowItemArguments2 {
            draw_manager: window_item_arguments.draw_manager,
            resources: window_item_arguments.resources,
            register_name,
            counter,
            item_x: window_item_arguments.item_x,
            item_y: window_item_arguments.item_y,
            width: window_item_arguments.width,
            scale_mode: window_item_arguments.scale_mode,
        }));
        for name in text_label.shape_ids() {
            if let Some(object) = window_item_arguments.draw_manager.object_mut(name) {
                object.effect(window_item_arguments.clip_mask.block());
            }
        }
        
        let Some(font) = window_item_arguments.resources.font(self.label_font.base_font) else {
            log::warn!("the title font is not in the resources; the title is not drawn");
            return (0.0, 0.0);
        };
        let text_layout = text_label.register_draw_manager(window_item_arguments.draw_manager, &font).expect("Failed to register draw manager");

        self.text_label = Some(text_label);

        (self.gap.x + text_layout.width, self.gap.y + text_layout.height)
    }

    fn rebuild(&mut self, mut window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.relayout_data(&mut window_item_arguments)
    }

    fn object_names(&self) -> Vec<String> {
        self.text_label
            .iter()
            .flat_map(|text| text.shape_ids().map(String::from))
            .collect()
    }
}