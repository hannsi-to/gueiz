use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;
use crate::format::Token::Text;
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::{CreateObjectArguments, WindowItem};
use crate::instance::create_instance;
use crate::object::Object;
use crate::text::{TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

pub struct Label {
    label: String,
    label_size: f32,
    gap: Gap,
    label_color: ThemeColor,
    text_label: Option<TextRenderer>,
    pub(crate) label_register_name: LabelRegisterName,
}

pub struct LabelRegisterName {
    label: Option<String>,
}

pub struct LabelTheme {
    pub label_color : ThemeColor,
}

impl Label {
    pub fn new(label: String, label_size: f32, gap: Gap, label_color: ThemeColor, label_register_name: LabelRegisterName) -> Self {
        Self {
            label,
            label_size,
            gap,
            label_color,
            text_label: None,
            label_register_name,
        }
    }
}

impl Label {
    fn build_shapes(&self) -> Vec<Object> {
        vec![]
    }

    fn text_layout_data(&self, item_x: f32, item_y: f32) -> TextLayoutData {
        // TextLayoutData {
        //     text: self.label.clone(),
        //     style: TextStyle::new(self.label_size),
        //     text_location: TextLocation::Area {
        //
        //     }
        // }
        todo!()
    }
}

impl WindowItem for Label {
    fn create_object(&mut self, create_object_arguments: CreateObjectArguments) -> (f32, f32) {
        let register_name = create_object_arguments.register_name;
        let counter = create_object_arguments.counter;
        self.label_register_name.label = Some(format!("{register_name}-label-{counter}"));

        let mut objects = self.build_shapes();
        for object in &mut objects {
            object.camera_for(create_object_arguments.surface_size, create_object_arguments.design, create_object_arguments.scale_mode);
            object.instance(create_instance());
        }

        (1.0, 1.0)
    }

    fn register_draw_manager(&mut self, draw_manager: &mut DrawManager) {
        todo!()
    }

    fn rebuild(&mut self, draw_manager: &mut DrawManager, item_x: f32, item_y: f32, width: f32) -> (f32, f32) {
        todo!()
    }

    fn object_names(&self) -> Vec<String> {
        self.text_label
            .iter()
            .flat_map(|text| text.shape_ids().map(String::from))
            .collect()
    }
}