use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::gui::window_frame::{Gap, Quad, QuadType, ThemeColor};
use crate::gui::window_item::{replace_outline, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::PaintType;
use crate::resource::TextureHandle;
use crate::text::TextLayoutData;

/// 絵を貼った四角。
///
/// `uv` は貼る範囲と形です。
///
/// - `x` / `y` / `width` / `height`: 絵のどこを貼るか（`0..1`、左上と幅・高さ）。
///   `0, 0, 1, 1` で絵の全体。
/// - `quad_type`: 四角の形。[`QuadType::Rounded`] なら角を丸めます（半径は描く大きさの単位）。
///   絵は四角いっぱいに貼られ、丸めた角の外だけが欠けます。
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
    ///
    /// 頂点に UV は入れません。シェーダが図形の外接矩形から UV を作るので、
    /// 角を丸めても絵は四角いっぱいに貼られます。どこを貼るかは
    /// [`ImageLabel::texture_region`] で決めます。
    fn build_rect(&self, name: &str, item_x: f32, item_y: f32) -> Object {
        let color = &self.image_label_theme.texture_color;
        let left = item_x + self.gap.x;
        let top = item_y + self.gap.y;
        let vertex = |x: f32, y: f32| Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a);

        match self.uv.quad_type {
            QuadType::Normal => {
                Rect::new(name)
                    .paint_type(PaintType::Fill)
                    .from(vertex(left, top))
                    .to(vertex(left + self.width, top + self.height))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(name)
                    .paint_type(PaintType::Fill)
                    .from(CornerType::Circle { radius }, vertex(left, top))
                    .to(vertex(left + self.width, top + self.height))
                    .end()
            }
        }
    }

    /// アトラスの中の、貼る範囲（`[u, v, 幅, 高さ]`）。
    ///
    /// `uv` は絵 1 枚を `0..1` とした範囲なので、アトラスの中のその絵の範囲に写します。
    fn texture_region(&self, whole: [f32; 4]) -> [f32; 4] {
        let [u, v, width, height] = whole;

        [
            u + self.uv.x * width,
            v + self.uv.y * height,
            self.uv.width * width,
            self.uv.height * height,
        ]
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
                object.sprite_region(texture.page, self.texture_region(texture.uv_rect));
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