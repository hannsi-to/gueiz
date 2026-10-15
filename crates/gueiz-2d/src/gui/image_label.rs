use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::gui::window_frame::{Gap, Quad, ThemeColor};
use crate::gui::window_item::{replace_outline, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::rect::Rect;
use crate::paint_type::PaintType;
use crate::resource::TextureHandle;
use crate::text::TextLayoutData;

pub struct ImageLabel {
    width: f32,
    height: f32,
    uv: Quad,
    gap: Gap,
    image_label_theme: ImageLabelTheme,
    image_label_texture: ImageLabelTexture,
    pub(crate) image_label_register_name: ImageLabelRegisterName,
}

pub struct ImageLabelRegisterName {
    image_label: Option<String>,
}

pub struct ImageLabelTheme {
    pub texture_color: ThemeColor,
}

pub struct ImageLabelTexture {
    pub texture_handle: TextureHandle,
}

impl ImageLabel {
    pub fn new(register_name_string: String, width: f32, height: f32, uv: Quad, gap: Gap, image_label_theme: ImageLabelTheme, image_label_texture: ImageLabelTexture) -> Self {
        Self {
            width,
            height,
            uv,
            gap,
            image_label_theme,
            image_label_texture,
            image_label_register_name: ImageLabelRegisterName {
                image_label: Some(register_name_string),
            }
        }
    }

    /// 絵の四角を組む。テクスチャは貼りません（組み直しでは頂点しか使わないので）。
    ///
    /// 組み直しで頂点だけが欲しいときは、名前に `""` を渡せば名前の確保が起きません。
    fn build_rect(&self, name: &str, item_x: f32, item_y: f32) -> Object {
        let color = &self.image_label_theme.texture_color;
        let left = item_x + self.gap.x;
        let top = item_y + self.gap.y;

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(Vertex::new_position_color_uv(
                left, top, 0.0,
                color.r, color.g, color.b, color.a,
                self.uv.x, self.uv.y,
            ))
            .to(Vertex::new_position_color_uv(
                left + self.width, top + self.height, 0.0,
                color.r, color.g, color.b, color.a,
                self.uv.width, self.uv.height,
            ))
            .end()
    }

    fn relayout_data(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        let object = self
            .image_label_register_name
            .image_label
            .as_deref()
            .and_then(|name| window_item_arguments.draw_manager.object_mut(name));

        if let Some(object) = object {
            let built = self.build_rect("", window_item_arguments.item_x, window_item_arguments.item_y);
            replace_outline(object, &built);
        }

        (self.width + self.gap.x, self.height + self.gap.y)
    }
}

impl WindowItem for ImageLabel {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let Some(local_name) = self.image_label_register_name.image_label.as_deref() else {
            return (0.0, 0.0);
        };
        let name = format!("{register_name}-{local_name}");

        let mut object = self.build_rect(&name, window_item_arguments.item_x, window_item_arguments.item_y);

        match window_item_arguments.resources.texture(self.image_label_texture.texture_handle) {
            Some(texture) => {
                object.sprite_texture(texture);
            }
            None => log::warn!("the texture of '{}' is not committed; it is drawn without one", object.name()),
        }

        object.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
        object.instance(create_instance());
        // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
        self.image_label_register_name.image_label = Some(window_item_arguments.draw_manager.register(object));

        (self.width + self.gap.x, self.height + self.gap.y)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.relayout_data(window_item_arguments)
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        self.image_label_register_name.image_label.iter().cloned().collect()
    }
}