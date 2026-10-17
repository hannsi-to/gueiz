use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::draw_manager::DrawManager;
use crate::font::Font;
use crate::gui::window_frame::{Gap, ThemeColor};
use crate::gui::window_item::{built_point, replace_outline, within, CursorShape, Pointer, WindowItem, WindowItemArguments1, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::objects::circle::Circle;
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::PaintType;
use crate::resource::{FontHandle, Resources};
use crate::text::{TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

/// 箱の縁の太さ。
const OUTLINE_WIDTH: f32 = 1.5;

/// 箱と字のあいだ。字の大きさに対する割合。
const LABEL_SPACING_RATIO: f32 = 0.4;

/// 丸めた四角の角の半径。箱の短い辺に対する割合。
const ROUNDED_RADIUS_RATIO: f32 = 0.25;

/// 並べる向き。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Axis {
    /// 横に並べる。
    Row,
    /// 縦に並べる。
    Column,
}

/// 箱の形。
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CheckBoxShape {
    Circle,
    Rect,
    RectRounded,
}

pub struct CheckBoxFont {
    pub base_font: FontHandle,
    pub base_font_size: f32,
}

pub struct CheckBoxTheme {
    /// 箱の縁。
    pub outline_color: ThemeColor,
    /// 入っているときの中身。
    pub enable_color: ThemeColor,
    /// 入っていないときの中身。
    pub disable_color: ThemeColor,
    /// 横に添える字。
    pub text_color: ThemeColor,
}

/// [`CheckBox::on_click`] に渡す、ひとつの箱のようす。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckBoxState {
    pub name: String,
    pub checked: bool,
    /// いま押されて切り替わった箱か。
    pub changed: bool,
}

type OnClick = Box<dyn FnMut(&[CheckBoxState], &String, &[String], &[String])>;

/// 押すたびに入る・切れるが替わる箱を、[`Axis`] の向きに並べたもの。
///
/// 箱は [`CheckBox::add_check_box_item`] で足します。箱の名前は横に添える字にもなります。
///
/// 箱の上で押して、**指を上に置いたまま放すと**切り替わり、[`CheckBox::on_click`] の関数が
/// `(すべての箱, 入っている箱の名前, 切れている箱の名前)` で呼ばれます。
///
/// [`CheckBox::new`] の [`Gap`] は並び全体の左上に、[`CheckBox::add_check_box_item`] の [`Gap`] は
/// 箱ひとつずつの前（左と上）に空けます。
pub struct CheckBox {
    register_name: String,
    axis: Axis,
    gap: Gap,
    check_box_font: CheckBoxFont,
    items: Vec<CheckBoxItem>,
    on_click: Option<OnClick>,
    /// 押されて、まだ放されていない箱。
    armed: Option<usize>,
}

struct CheckBoxItem {
    name: String,
    width: f32,
    height: f32,
    gap: Gap,
    padding: f32,
    shape: CheckBoxShape,
    checked: bool,
    theme: CheckBoxTheme,
    register_name: CheckBoxItemRegisterName,
    /// 箱を組んだときの左上。色を変えるときに、同じ場所で組み直す。
    built_at: [f32; 2],
    /// 箱から字の右端までの幅と、箱ひとつぶんの高さ。押せる範囲に使う。
    hit_size: [f32; 2],
}

/// 図形を新しく登録するときに貼るカメラ。
struct CameraArguments {
    surface_size: SurfaceSize,
    design: [f32; 2],
    scale_mode: ScaleMode,
}

#[derive(Default)]
struct CheckBoxItemRegisterName {
    /// 箱と字をまとめて覆う、見えない四角。字のすき間を押しても当たるように。
    hit_area: Option<String>,
    outline: Option<String>,
    mark: Option<String>,
    text: Option<String>,
}

impl CheckBox {
    pub fn new(register_name_string: String, axis: Axis, gap: Gap, check_box_font: CheckBoxFont) -> Self {
        Self {
            register_name: register_name_string,
            axis,
            gap,
            check_box_font,
            items: Vec::new(),
            on_click: None,
            armed: None,
        }
    }

    /// 箱が切り替わったときに呼ぶ関数。`(すべての箱, 入っている箱の名前, 切れている箱の名前)` が渡ります。
    pub fn on_click(mut self, on_click: impl FnMut(&[CheckBoxState], &String, &[String], &[String]) + 'static) -> Self {
        self.on_click = Some(Box::new(on_click));
        self
    }

    /// 箱を足す。窓に入れる前に呼んでください。
    ///
    /// `gap` はこの箱の前（左と上）に空ける間、`padding` は縁から中身までの間です。
    pub fn add_check_box_item(&mut self, name: String, width: f32, height: f32, gap: Gap, padding: f32, shape: CheckBoxShape, default_state: bool, theme: CheckBoxTheme) -> &mut Self {
        self.items.push(CheckBoxItem {
            name,
            width,
            height,
            gap,
            padding,
            shape,
            checked: default_state,
            theme,
            register_name: CheckBoxItemRegisterName::default(),
            built_at: [0.0; 2],
            hit_size: [0.0; 2],
        });
        self
    }

    /// その名前の箱が入っているか。無ければ `None`。
    pub fn is_checked(&self, name: &str) -> Option<bool> {
        self.items.iter().find(|item| item.name == name).map(|item| item.checked)
    }

    /// 入っている箱の名前。
    pub fn enables(&self) -> Vec<String> {
        self.items.iter().filter(|item| item.checked).map(|item| item.name.clone()).collect()
    }

    /// 切れている箱の名前。
    pub fn disables(&self) -> Vec<String> {
        self.items.iter().filter(|item| !item.checked).map(|item| item.name.clone()).collect()
    }

    fn label_spacing(&self) -> f32 {
        self.check_box_font.base_font_size * LABEL_SPACING_RATIO
    }

    /// 箱ひとつぶんの高さ（箱と字の高いほう）。
    fn row_height(&self, item: &CheckBoxItem) -> f32 {
        item.height.max(self.check_box_font.base_font_size)
    }

    /// 箱の左上。`cursor` は前の箱の終わり（並べる向きの長さ）。
    fn item_origin(&self, item: &CheckBoxItem, item_x: f32, item_y: f32, cursor: f32) -> [f32; 2] {
        let left = item_x + self.gap.x + item.gap.x;
        let top = item_y + self.gap.y + item.gap.y;

        match self.axis {
            Axis::Row => [left + cursor, top],
            Axis::Column => [left, top + cursor],
        }
    }

    /// 字を箱の右に、箱と縦のまん中をそろえて置く。
    fn text_layout_data(&self, item: &CheckBoxItem, item_x: f32, available_width: f32) -> TextLayoutData {
        let [x, y] = item.built_at;
        let text_x = x + item.width + self.label_spacing();

        TextLayoutData {
            text: item.name.clone(),
            style: TextStyle::new(self.check_box_font.base_font_size),
            text_location: TextLocation::Area {
                area: TextArea::new(
                    text_x,
                    y,
                    (available_width - (text_x - item_x)).max(0.0),
                    self.row_height(item),
                ).vertical(TextAlign::Center),
            },
        }
    }

    /// 字を登録して、字の幅を返す。
    fn register_text(text_renderer: &mut TextRenderer, draw_manager: &mut DrawManager, font: &Font) -> f32 {
        match text_renderer.register_draw_manager(draw_manager, font) {
            Ok(text_layout) => text_layout.width,
            Err(error) => {
                log::warn!("failed to lay out the check box text: {error}");
                0.0
            }
        }
    }

    /// 箱を並べ直して、全体の `(幅, 高さ)` を返す。
    ///
    /// `create` が `Some` なら図形を新しく登録し、`None` なら登録済みの図形の中で組み直す。
    fn layout(&mut self, draw_manager: &mut DrawManager, resources: &Resources, item_x: f32, item_y: f32, available_width: f32, create: Option<&CameraArguments>) -> (f32, f32) {
        let font = resources.font(self.check_box_font.base_font);
        if font.is_none() {
            log::warn!("the check box font is not in the resources; the check box text is not drawn");
        }

        let mut cursor = 0.0_f32;
        let mut cross = 0.0_f32;

        for index in 0..self.items.len() {
            let origin = self.item_origin(&self.items[index], item_x, item_y, cursor);
            self.items[index].built_at = origin;

            let item = &self.items[index];
            let layout_data = self.text_layout_data(item, item_x, available_width);
            let text_color = item.theme.text_color;

            let text_width = match create {
                Some(camera) => {
                    let base = format!("{}-{}", self.register_name, item.name);

                    let mut outline = item.build_outline(&format!("{base}-outline"));
                    let mut mark = item.build_mark(&format!("{base}-mark"));
                    let text_name = format!("{base}-text");
                    let mut text_renderer = TextRenderer::new(&text_name);

                    for object in [&mut outline, &mut mark] {
                        object.camera_for(camera.surface_size, camera.design, camera.scale_mode);
                        object.instance(create_instance());
                    }
                    text_renderer.camera_for(camera.surface_size, camera.design, camera.scale_mode);

                    // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
                    let outline_name = draw_manager.register(outline);
                    let mark_name = draw_manager.register(mark);

                    text_renderer.color(text_color.r, text_color.g, text_color.b, text_color.a);
                    text_renderer.text_layout_data(layout_data);
                    let width = font
                        .as_ref()
                        .map_or(0.0, |font| Self::register_text(&mut text_renderer, draw_manager, font));

                    if let Some(replaced) = draw_manager.add_text_renderer(text_renderer) {
                        log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
                    }

                    let register_name = &mut self.items[index].register_name;
                    register_name.outline = Some(outline_name);
                    register_name.mark = Some(mark_name);
                    register_name.text = Some(text_name);

                    width
                }
                None => {
                    item.refresh(draw_manager);

                    match (item.register_name.text.as_deref(), font.as_ref()) {
                        (Some(text), Some(font)) => draw_manager
                            .with_text_renderer(text, |text_renderer, draw_manager| {
                                text_renderer.clear(draw_manager);
                                text_renderer.text_layout_data(layout_data);

                                Self::register_text(text_renderer, draw_manager, font)
                            })
                            .unwrap_or(0.0),
                        _ => 0.0,
                    }
                }
            };

            let item_width = self.items[index].width + self.label_spacing() + text_width;
            let item_height = self.row_height(&self.items[index]);
            self.items[index].hit_size = [item_width, item_height];

            let item = &self.items[index];
            match create {
                Some(camera) => {
                    let mut hit_area = item.build_hit_area(&format!("{}-{}-hit-area", self.register_name, item.name));
                    hit_area.camera_for(camera.surface_size, camera.design, camera.scale_mode);
                    hit_area.instance(create_instance());

                    let hit_area_name = draw_manager.register(hit_area);
                    self.items[index].register_name.hit_area = Some(hit_area_name);
                }
                None => item.refresh_hit_area(draw_manager),
            }

            let item = &self.items[index];

            match self.axis {
                Axis::Row => {
                    cursor += item.gap.x + item_width;
                    cross = cross.max(item.gap.y + item_height);
                }
                Axis::Column => {
                    cursor += item.gap.y + item_height;
                    cross = cross.max(item.gap.x + item_width);
                }
            }
        }

        match self.axis {
            Axis::Row => (self.gap.x + cursor, self.gap.y + cross),
            Axis::Column => (self.gap.x + cross, self.gap.y + cursor),
        }
    }

    /// 指の下で一番手前にある図形が、どの箱のものか。
    ///
    /// 位置で比べず [`DrawManager::pick`] に聞くのは、窓のずらしやカメラを
    /// 通した当たり判定をそのまま使えるのと、別の窓がかぶさっているときに
    /// 反応しないようにするため。
    ///
    /// ただし `pick` は全図形をなめるので重い。指が動くたびに呼ばれるので、
    /// どの箱の当たり（箱と字を囲む四角に、縁の太さを足したもの）にも入っていなければ、座標だけで断る。
    fn hit(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<usize> {
        let first = self.items.first()?.register_name.hit_area.as_deref();
        let point = built_point(draw_manager, first, pointer.x, pointer.y)?;
        let inside = self.items.iter().any(|item| {
            let [x, y] = item.built_at;
            let [width, height] = item.hit_size;

            within([x, y, x + width, y + height], OUTLINE_WIDTH, point)
        });
        if !inside {
            return None;
        }

        let pick = pointer.pick(draw_manager)?;

        self.items.iter().position(|item| {
            let names = &item.register_name;

            names.hit_area.as_deref() == Some(pick.name.as_str())
                || names.outline.as_deref() == Some(pick.name.as_str())
                || names.mark.as_deref() == Some(pick.name.as_str())
                || names
                    .text
                    .as_deref()
                    .is_some_and(|text| draw_manager.group_names(text).any(|name| name == pick.name))
        })
    }

    fn notify(&mut self, changed: usize, changed_name: String) {
        let Some(on_click) = self.on_click.as_mut() else {
            return;
        };

        let states: Vec<CheckBoxState> = self
            .items
            .iter()
            .enumerate()
            .map(|(index, item)| CheckBoxState {
                name: item.name.clone(),
                checked: item.checked,
                changed: index == changed,
            })
            .collect();
        let enables: Vec<String> = states.iter().filter(|state| state.checked).map(|state| state.name.clone()).collect();
        let disables: Vec<String> = states.iter().filter(|state| !state.checked).map(|state| state.name.clone()).collect();

        on_click(&states, &changed_name, &enables, &disables);
    }
}

impl CheckBoxItem {
    /// 中身の色。
    fn mark_color(&self) -> &ThemeColor {
        if self.checked {
            &self.theme.enable_color
        } else {
            &self.theme.disable_color
        }
    }

    /// `left, top, right, bottom` の枠に、箱の形を組む。
    fn build_shape(&self, name: &str, paint_type: PaintType, color: &ThemeColor, [left, top, right, bottom]: [f32; 4]) -> Object {
        let vertex = |x: f32, y: f32| Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a);

        match self.shape {
            CheckBoxShape::Rect => {
                Rect::new(name)
                    .paint_type(paint_type)
                    .from(vertex(left, top))
                    .to(vertex(right, bottom))
                    .end()
            }
            CheckBoxShape::RectRounded => {
                let radius = (right - left).min(bottom - top) * ROUNDED_RADIUS_RATIO;

                RectRounded::new(name)
                    .paint_type(paint_type)
                    .from(CornerType::Circle { radius }, vertex(left, top))
                    .to(vertex(right, bottom))
                    .end()
            }
            CheckBoxShape::Circle => {
                Circle::new(name)
                    .paint_type(paint_type)
                    .vertex_center(vertex((left + right) * 0.5, (top + bottom) * 0.5))
                    .radius_x((right - left) * 0.5)
                    .radius_y((bottom - top) * 0.5)
                    .segment_count(32)
                    .end_circle_option()
                    .end()
            }
        }
    }

    /// 箱の縁。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_outline(&self, name: &str) -> Object {
        let [x, y] = self.built_at;

        self.build_shape(
            name,
            PaintType::stroke(OUTLINE_WIDTH),
            &self.theme.outline_color,
            [x, y, x + self.width, y + self.height],
        )
    }

    /// 縁から `padding` だけ内側の中身。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_mark(&self, name: &str) -> Object {
        let [x, y] = self.built_at;
        let padding = self.padding.min(self.width * 0.5).min(self.height * 0.5).max(0.0);

        self.build_shape(
            name,
            PaintType::Fill,
            self.mark_color(),
            [x + padding, y + padding, x + self.width - padding, y + self.height - padding],
        )
    }

    /// 箱と字をまとめて覆う、見えない四角。組み直しで頂点だけが欲しいときは、名前に `""` を渡します。
    fn build_hit_area(&self, name: &str) -> Object {
        let [x, y] = self.built_at;
        let [width, height] = self.hit_size;
        let vertex = |x: f32, y: f32| Vertex::new_position_color(x, y, 0.0, 0.0, 0.0, 0.0, 0.0);

        Rect::new(name)
            .paint_type(PaintType::Fill)
            .from(vertex(x, y))
            .to(vertex(x + width, y + height))
            .end()
    }

    fn refresh_hit_area(&self, draw_manager: &mut DrawManager) {
        if let Some(object) = self.register_name.hit_area.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_hit_area(""));
        }
    }

    /// いまの色と場所で、縁と中身を登録済みの図形の中に組み直す。
    fn refresh(&self, draw_manager: &mut DrawManager) {
        if let Some(object) = self.register_name.outline.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_outline(""));
        }

        self.refresh_mark(draw_manager);
    }

    fn refresh_mark(&self, draw_manager: &mut DrawManager) {
        if let Some(object) = self.register_name.mark.as_deref().and_then(|name| draw_manager.object_mut(name)) {
            replace_outline(object, &self.build_mark(""));
        }
    }
}

impl WindowItem for CheckBox {
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32) {
        let register_name = window_item_arguments.register_name;
        let counter = window_item_arguments.counter;
        // 同じ名前の部品を 2 つ置いても重ならないよう、窓の名前と窓の中の番号も足す。
        self.register_name = format!("{register_name}-{}-{counter}", self.register_name);

        let camera = CameraArguments {
            surface_size: window_item_arguments.surface_size,
            design: window_item_arguments.design,
            scale_mode: window_item_arguments.scale_mode,
        };

        self.layout(
            window_item_arguments.draw_manager,
            window_item_arguments.resources,
            window_item_arguments.item_x,
            window_item_arguments.item_y,
            window_item_arguments.width,
            Some(&camera),
        )
    }

    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32) {
        self.layout(
            window_item_arguments.draw_manager,
            window_item_arguments.resources,
            window_item_arguments.item_x,
            window_item_arguments.item_y,
            window_item_arguments.width,
            None,
        )
    }

    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String> {
        let mut names = Vec::new();

        for item in &self.items {
            let register_name = &item.register_name;
            names.extend(register_name.hit_area.iter().cloned());
            names.extend(register_name.outline.iter().cloned());
            names.extend(register_name.mark.iter().cloned());

            if let Some(text) = &register_name.text {
                names.extend(draw_manager.group_names(text).map(String::from));
            }
        }

        names
    }

    fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some(index) = self.hit(draw_manager, &Pointer::new(x, y)) else {
            return false;
        };

        self.armed = Some(index);
        true
    }

    fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let Some(armed) = self.armed.take() else {
            return false;
        };

        if self.hit(draw_manager, &Pointer::new(x, y)) != Some(armed) {
            return false;
        }

        let item = &mut self.items[armed];
        item.checked = !item.checked;
        item.refresh_mark(draw_manager);

        let item_name = item.name.clone();
        self.notify(armed, item_name);
        true
    }

    fn cursor_shape(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<CursorShape> {
        self.hit(draw_manager, pointer).map(|_| CursorShape::Pointer)
    }
}
