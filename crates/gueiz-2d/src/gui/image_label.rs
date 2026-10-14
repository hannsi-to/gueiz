use gueiz_gpu::vertex::Vertex;
use crate::gui::test_window_frame::{Gap, Quad, ThemeColor};
use crate::gui::window_item::{WindowItem, WindowItemArguments1, WindowItemArguments2};
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
    pub fn new(width: f32, height: f32, uv: Quad, gap: Gap, image_label_theme: ImageLabelTheme, image_label_texture: ImageLabelTexture) -> Self {
        Self {
            width,
            height,
            uv,
            gap,
            image_label_theme,
            image_label_texture,
            image_label_register_name: ImageLabelRegisterName {
                image_label: None,
            }
        }
    }

    fn build_shapes(
        &mut self,
        window_item_arguments2: &WindowItemArguments2<'_>,
    ) -> Vec<Object> {
        let register_name = window_item_arguments2.register_name;
        let counter = window_item_arguments2.counter;

        self.image_label_register_name.image_label =
            Some(format!("{register_name}-image_label-{counter}"));

        let mut image_label = Rect::new(
            self.image_label_register_name.image_label.clone().unwrap().as_str(),
        )
            .paint_type(PaintType::Fill)
            .from(Vertex::new_position_color_uv(
                window_item_arguments2.item_x + self.gap.x,
                window_item_arguments2.item_y + self.gap.y,
                0.0,
                self.image_label_theme.texture_color.r,
                self.image_label_theme.texture_color.g,
                self.image_label_theme.texture_color.b,
                self.image_label_theme.texture_color.a,
                self.uv.x,
                self.uv.y,
            ))
            .to(Vertex::new_position_color_uv(
                window_item_arguments2.item_x + self.gap.x + self.width,
                window_item_arguments2.item_y + self.gap.y + self.height,
                0.0,
                self.image_label_theme.texture_color.r,
                self.image_label_theme.texture_color.g,
                self.image_label_theme.texture_color.b,
                self.image_label_theme.texture_color.a,
                self.uv.width,
                self.uv.height,
            ))
            .end();

        image_label.sprite_texture(
            window_item_arguments2
                .resources
                .texture(self.image_label_texture.texture_handle)
                .ok_or("the texture was not committed")
                .unwrap(),
        );

        vec![image_label]
    }

    fn relayout_data(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        let built_shapes = self.build_shapes(&window_item_arguments);

        let names = [
            self.image_label_register_name.image_label.clone(),
        ];

        for (name, built) in names.into_iter().zip(built_shapes) {
            let Some(object) = name
                .as_ref()
                .and_then(|name| window_item_arguments.draw_manager.object_mut(name))
            else {
                continue;
            };

            object.begin(built.paint_type());

            for vertex in built.vertices() {
                object.put_vertex(*vertex);
            }

            object.end();
        }

        (self.width + self.gap.x, self.height + self.gap.y)
    }
}

impl WindowItem for ImageLabel {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let objects = self.build_shapes(
            &WindowItemArguments2 {
                draw_manager: window_item_arguments.draw_manager,
                resources: window_item_arguments.resources,
                register_name: window_item_arguments.register_name,
                counter: window_item_arguments.counter,
                item_x: window_item_arguments.item_x,
                item_y: window_item_arguments.item_y,
                width: window_item_arguments.width,
                scale_mode: window_item_arguments.scale_mode,
            }
        );

        for mut object in objects {
            object.camera_for(window_item_arguments.surface_size, window_item_arguments.design, window_item_arguments.scale_mode);
            object.instance(create_instance());
            window_item_arguments.draw_manager.register(object);
        }

        (self.width + self.gap.x, self.height + self.gap.y)
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.relayout_data(window_item_arguments)
    }

    fn object_names(&self) -> Vec<String> {
        vec![self.image_label_register_name.image_label.clone().unwrap()]
    }
}