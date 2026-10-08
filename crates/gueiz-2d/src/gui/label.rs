use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::WindowItem;
use crate::text::TextRenderer;

pub struct Label {
    label: String,
    gap: Gap,
    label_color: ThemeColor,
    text_label: Option<TextRenderer>,
    pub(crate) label_register_name: LabelRegisterName,
}

pub struct LabelRegisterName {

}

pub struct LabelTheme {
    pub label_color : ThemeColor,
}

impl Label {
    pub fn new(label: String, gap: Gap, label_color: ThemeColor, label_register_name: LabelRegisterName) -> Self {
        Self {
            label,
            gap,
            label_color,
            text_label: None,
            label_register_name,
        }
    }
}

impl WindowItem for Label {
    fn create_object(&mut self, item_x: f32, item_y: f32, surface_size: SurfaceSize, design: [f32; 2]) {
        todo!()
    }

    fn register_draw_manager(&mut self, draw_manager: &mut DrawManager) {
        todo!()
    }
}