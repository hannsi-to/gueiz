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
use crate::gui::window_item::{WindowItemArguments1, WindowItem, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::resource::{FontHandle, Resources};
use crate::objects::lines::{CurveRepresentationType, Lines};
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::text::{TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

const CORNER_RADIUS: f32 = 10.0;
const LINE_WIDTH: f32 = 1.0;

/// 縮められる幅の下限。
pub const MIN_WIDTH: f32 = 120.0;
/// 縁の**外側**で、大きさを変える取っ手になる幅。
const HANDLE_OUTSIDE: f32 = 6.0;
/// 縁の**内側**で、取っ手になる幅。広くすると、帯の端で掴みにくくなる。
const HANDLE_INSIDE: f32 = 3.0;
/// 題名の字の大きさ。
const TITLE_SIZE: f32 = 13.0;

pub struct WindowFrame {
    scale_mode: ScaleMode,
    title: String,
    frame_quad: Quad,
    title_bar_height: f32,
    gap: Gap,
    window_font: WindowFont,
    window_theme: WindowTheme,
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
    /// 縁を引いているあいだだけ。
    resize: Option<Resize>,
    hidden: bool,
    window_items: Vec<Box<dyn WindowItem>>,
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

/// 縁を引いているあいだの覚え。
struct Resize {
    handle: ResizeHandle,
    /// 掴んだときの窓。**いまの窓からではなく、ここからの差で決める。**
    /// 下限で止めたぶんが溜まらないので、戻せば元の大きさに戻る。
    start_quad: Quad,
    /// 掴んだ場所。
    start: [f32; 2],
}

/// 大きさを変える取っ手。縁と角。
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum ResizeHandle {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl ResizeHandle {
    fn moves_left(self) -> bool {
        matches!(self, Self::Left | Self::TopLeft | Self::BottomLeft)
    }

    fn moves_right(self) -> bool {
        matches!(self, Self::Right | Self::TopRight | Self::BottomRight)
    }

    fn moves_top(self) -> bool {
        matches!(self, Self::Top | Self::TopLeft | Self::TopRight)
    }

    fn moves_bottom(self) -> bool {
        matches!(self, Self::Bottom | Self::BottomLeft | Self::BottomRight)
    }
}

struct RegisterName {
    pub frame: Option<String>,
    pub title_bar: Option<String>,
    pub frame_outline: Option<String>,
    pub title_bar_slice_line: Option<String>,
    pub text_title: Option<String>,
}

pub struct WindowTheme {
    pub frame_fill_color: ThemeColor,
    pub frame_outline_color: ThemeColor,
    pub title_bar_fill_color: ThemeColor,
    pub title_bar_slice_line_color: ThemeColor,
    pub text_title_color: ThemeColor,
}

pub struct WindowFont {
    /// 題名に使う書体。中身は呼ぶ側の [`Resources`] が持つ。
    pub base_font: FontHandle,
}

pub struct ThemeColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl WindowFrame {
    pub fn new(scale_mode: ScaleMode,title: String, frame_quad: Quad, title_bar_height: f32, gap: Gap, window_font: WindowFont, window_theme: WindowTheme) -> Self {
        Self {
            scale_mode,
            title,
            frame_quad,
            title_bar_height,
            gap,
            window_font,
            window_theme,
            text_title: None,
            register_name: RegisterName {
                frame: None,
                title_bar: None,
                frame_outline: None,
                title_bar_slice_line: None,
                text_title: None,
            },
            built_origin: [0.0; 2],
            drag: None,
            resize: None,
            hidden: false,
            window_items: Vec::new(),
        }
    }

    pub fn add_window_item(&mut self, window_item: Box<dyn WindowItem>) {
        self.window_items.push(window_item);
    }

    pub fn create_object(&mut self, draw_manager: &mut DrawManager, resources: &Resources, register_name: &str, surface_size: SurfaceSize, design: [f32; 2]) {
        // ずらす量はここからの差で出す。
        self.built_origin = [self.frame_quad.x, self.frame_quad.y];

        let register_name_string = register_name.to_string();

        self.register_name.frame = Some(register_name_string.clone());
        self.register_name.title_bar = Some(format!("{register_name_string}-title_bar"));
        self.register_name.frame_outline = Some(format!("{register_name_string}-frame_outline"));
        self.register_name.title_bar_slice_line = Some(format!("{register_name_string}-title_bar_slice_line"));

        let mut objects = self.build_shapes();
        for object in &mut objects {
            object.camera_for(surface_size, design, self.scale_mode);
            object.instance(create_instance());
        }
        let [frame, title_bar, frame_outline, title_bar_slice_line]: [Object; 4] = objects
            .try_into()
            .unwrap_or_else(|_| unreachable!("build_shapes は 4 つ組む"));

        let mut register_name_text_title = register_name_string.clone();
        register_name_text_title.push_str("-text_title");
        self.register_name.text_title = Some(register_name_text_title);
        let mut text_title = TextRenderer::new(self.register_name.text_title.clone().unwrap().as_str());
        text_title.camera_for(surface_size, design, self.scale_mode);
        text_title.color(self.window_theme.text_title_color.r,self.window_theme.text_title_color.g,self.window_theme.text_title_color.b,self.window_theme.text_title_color.a);
        text_title.text_layout_data(self.title_layout_data());

        draw_manager.register(frame);
        draw_manager.register(title_bar);
        draw_manager.register(frame_outline);
        draw_manager.register(title_bar_slice_line);

        self.register_title(draw_manager, resources);

        let [item_x, mut item_y] = self.item_origin();
        let width = self.item_width();
        let mut counter = 0;
        for window_item in &mut self.window_items {
            let item_size = window_item.create_object(
                WindowItemArguments1 {
                    draw_manager,
                    resources: &resources,
                    register_name: &register_name_string,
                    counter,
                    item_x,
                    item_y,
                    width,
                    surface_size,
                    scale_mode: self.scale_mode,
                    design
                }
            );
            item_y += item_size.1;
            counter += 1;
        }
    }

    // --- 部品 ---

    /// 1 つ目の部品を置く左上。帯のすぐ下で、余白ぶん内側。
    ///
    /// 形と同じく、**組んだときの左上（`built_origin`）が基準**です。
    /// 置き場所のずれは、部品の図形にも [`Object::translate`] で掛けます。
    fn item_origin(&self) -> [f32; 2] {
        let [x, y] = self.built_origin;

        [x + self.gap.x, y + self.title_bar_height + self.gap.y]
    }

    /// 部品に使える幅。窓の幅から左右の余白を引いたもの。
    fn item_width(&self) -> f32 {
        self.frame_quad.width - self.gap.x * 2.0
    }

    /// いまの幅で部品を組み直す。上から順に、前の部品の高さぶん下へ積む。
    fn rebuild_items(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        let [item_x, mut item_y] = self.item_origin();
        let width = self.item_width();
        let mut counter = 0;
        for window_item in &mut self.window_items {
            let item_size = window_item.rebuild(
                WindowItemArguments2 {
                    draw_manager,
                    resources: &resources,
                    register_name: &self.register_name.frame.as_ref().unwrap(),
                    counter,
                    item_x,
                    item_y,
                    width,
                    scale_mode: self.scale_mode,
                }
            );
            item_y += item_size.1;
            counter += 1;
        }
    }

    /// 窓の大きさが変わった。枠の形・題名・部品をすべて今の大きさで組み直す。
    fn rebuild_all(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        self.rebuild_shapes(draw_manager);
        self.relayout_title(draw_manager, resources);
        self.rebuild_items(draw_manager, resources, );

        // 組み直しで新しく出てきた図形（字の形など）には、まだずらしが掛かっていない。
        self.apply_offset(draw_manager);
    }

    /// 題名をどこにどう置くか。
    ///
    /// 形と同じく、**組んだときの左上（`built_origin`）を基準に、いまの幅で**置きます。
    /// 置き場所のずれは字の形にも [`Object::translate`] で掛かるので、ここには入れません。
    fn title_layout_data(&self) -> TextLayoutData {
        let [x, y] = self.built_origin;

        TextLayoutData {
            text: self.title.clone(),
            style: TextStyle::new(TITLE_SIZE),
            text_location: TextLocation::Area {
                area: TextArea::new(x + self.gap.x, y, self.frame_quad.width - (self.gap.x * 2.0), self.title_bar_height)
                    .vertical(TextAlign::Center),
            },
        }
    }

    /// いまの幅で題名を置き直す。
    ///
    /// 置いた字を消してから書き直すので、字は増えません。字の形は残っているので、
    /// 同じ題名なら形の作り直しも起きません。書体を読めていなければ何もしません。
    ///
    /// 新しく出てきた字の形にずらしを掛けるのは、呼ぶ側（[`WindowFrame::rebuild_all`]）です。
    fn relayout_title(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        let layout_data = self.title_layout_data();

        let (Some(text_title), Some(font)) = (self.text_title.as_mut(), resources.font(self.window_font.base_font)) else {
            return;
        };

        text_title.clear(draw_manager);
        text_title.text_layout_data(layout_data);

        if let Err(error) = text_title.register_draw_manager(draw_manager, &font) {
            log::warn!("failed to lay out the title again: {error}");
        }
    }

    /// 枠・帯・縁取り・区切り線の形を組む。
    ///
    /// **組んだときの左上（`built_origin`）に、いまの大きさで組みます。**
    /// 置き場所のずれは [`Object::translate`] で持つので、ここには入れません。
    /// 名前は [`WindowFrame::create_object`] で決めたものを使います。
    fn build_shapes(&self) -> Vec<Object> {
        let [x, y] = self.built_origin;
        let width = self.frame_quad.width;
        let height = self.frame_quad.height;
        let theme = &self.window_theme;

        let name = |name: &Option<String>| name.clone().unwrap_or_default();
        let vertex = |x: f32, y: f32, color: &ThemeColor| {
            Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
        };

        let frame = RectRounded::new(&name(&self.register_name.frame))
            .paint_type(PaintType::Fill)
            .vertex1(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.frame_fill_color))
            .vertex2(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.frame_fill_color))
            .vertex3(CornerType::Circle {radius: CORNER_RADIUS}, vertex(x + width, y + height, &theme.frame_fill_color))
            .vertex4(CornerType::Circle {radius: CORNER_RADIUS}, vertex(x, y + height, &theme.frame_fill_color))
            .end();

        let title_bar = RectRounded::new(&name(&self.register_name.title_bar))
            .paint_type(PaintType::Fill)
            .vertex1(CornerType::Circle {radius: CORNER_RADIUS}, vertex(x, y, &theme.title_bar_fill_color))
            .vertex2(CornerType::Circle {radius: CORNER_RADIUS}, vertex(x + width, y, &theme.title_bar_fill_color))
            .vertex3(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.title_bar_fill_color))
            .vertex4(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.title_bar_fill_color))
            .end();

        let frame_outline = RectRounded::new(&name(&self.register_name.frame_outline))
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::Bevel, strip: false, dash: None})
            .from(CornerType::Circle {radius: CORNER_RADIUS}, vertex(x, y, &theme.frame_outline_color))
            .to_wh(vertex(width, height, &theme.frame_outline_color))
            .end();

        let title_bar_slice_line = Lines::new(&name(&self.register_name.title_bar_slice_line))
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
            .curve_type(CurveRepresentationType::Normal)
            .point(vertex(x + self.gap.x, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .last_point(vertex(x - self.gap.x + width, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .end();

        vec![frame, title_bar, frame_outline, title_bar_slice_line]
    }

    /// いまの大きさで形を組み直し、**登録済みの図形の中に**入れ直す。
    ///
    /// 登録し直すと [`DrawManager`] に名前が溜まるので、頂点だけを差し替えます。
    /// カメラ・複製・ずらしは図形に残ったままです。
    fn rebuild_shapes(&self, draw_manager: &mut DrawManager) {
        let names = [
            self.register_name.frame.as_ref(),
            self.register_name.title_bar.as_ref(),
            self.register_name.frame_outline.as_ref(),
            self.register_name.title_bar_slice_line.as_ref(),
        ];

        for (name, built) in names.into_iter().zip(self.build_shapes()) {
            let Some(object) = name.and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };

            object.begin(built.paint_type());
            for vertex in built.vertices() {
                object.put_vertex(*vertex);
            }
            object.end();
        }
    }

    /// 題名の字を登録する。書体が読めなければ、題名を出さずに続ける。
    fn register_title(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        // 題名が無ければ書体は読まない。書体の無い環境でも枠だけは出せる。
        if self.title.is_empty() {
            return;
        }
        let Some(font) = resources.font(self.window_font.base_font) else {
            log::warn!("the title font is not in the resources; the title is not drawn");
            return;
        };
        if let Some(text_title) = self.text_title.as_mut() {
            text_title.register_draw_manager(draw_manager, &font).expect("Failed to register draw manager");
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

        // 部品の図形のカメラも貼り直す。部品の側では要らない。
        for window_item in &mut self.window_items {
            for name in window_item.object_names() {
                if let Some(object) = draw_manager.object_mut(&name) {
                    object.camera_for(surface_size, design, self.scale_mode);
                }
            }

            window_item.resize(draw_manager, surface_size, design);
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

    // --- 大きさを変える ---

    /// いまの幅と高さ。
    pub fn size(&self) -> [f32; 2] {
        [self.frame_quad.width, self.frame_quad.height]
    }

    /// 縮められる高さの下限。帯と、下の丸い角が収まるぶん。
    pub fn min_height(&self) -> f32 {
        self.title_bar_height + CORNER_RADIUS * 2.0
    }

    /// 大きさを決める。左上は動きません。下限より小さくはなりません。
    ///
    /// 題名と部品も新しい幅で置き直します。
    pub fn set_size(&mut self, draw_manager: &mut DrawManager, resources: &Resources, width: f32, height: f32) {
        self.frame_quad.width = width.max(MIN_WIDTH);
        self.frame_quad.height = height.max(self.min_height());

        self.rebuild_all(draw_manager, resources);
    }

    /// 縁を引いているか。
    pub fn is_resizing(&self) -> bool {
        self.resize.is_some()
    }

    /// その画素の位置が、どの取っ手の上か。指の形を変えるのに使います。
    ///
    /// 取っ手は縁の外側に [`HANDLE_OUTSIDE`]、内側に [`HANDLE_INSIDE`] の幅です。
    /// 窓の内側の取っ手は、**手前にあるものが自分のものの時だけ**取ります。
    /// 別の窓がかぶさっているなら、そちらの中身です。
    pub fn resize_handle_at(&self, draw_manager: &DrawManager, x: f32, y: f32) -> Option<ResizeHandle> {
        let [world_x, world_y] = self.to_world(draw_manager, x, y)?;

        let quad = &self.frame_quad;
        let (left, top) = (quad.x, quad.y);
        let (right, bottom) = (quad.x + quad.width, quad.y + quad.height);

        // 取っ手まで含めた外枠の外なら、どれでもない。
        if world_x < left - HANDLE_OUTSIDE
            || world_x > right + HANDLE_OUTSIDE
            || world_y < top - HANDLE_OUTSIDE
            || world_y > bottom + HANDLE_OUTSIDE
        {
            return None;
        }

        let near_left = world_x <= left + HANDLE_INSIDE;
        let near_right = world_x >= right - HANDLE_INSIDE;
        let near_top = world_y <= top + HANDLE_INSIDE;
        let near_bottom = world_y >= bottom - HANDLE_INSIDE;

        let handle = match (near_left, near_right, near_top, near_bottom) {
            (true, _, true, _) => ResizeHandle::TopLeft,
            (_, true, true, _) => ResizeHandle::TopRight,
            (true, _, _, true) => ResizeHandle::BottomLeft,
            (_, true, _, true) => ResizeHandle::BottomRight,
            (true, _, _, _) => ResizeHandle::Left,
            (_, true, _, _) => ResizeHandle::Right,
            (_, _, true, _) => ResizeHandle::Top,
            (_, _, _, true) => ResizeHandle::Bottom,
            _ => return None,
        };

        if quad.hover(world_x, world_y) {
            match draw_manager.pick(x, y) {
                Some(pick) if self.owns(&pick.name) => {}
                _ => return None,
            }
        }

        Some(handle)
    }

    /// 掴んだときの窓から、指が `delta` 動いたときの窓。
    fn resized_quad(&self, resize: &Resize, delta: [f32; 2]) -> Quad {
        let start = &resize.start_quad;
        let handle = resize.handle;
        let min_height = self.min_height();

        let mut quad = Quad {
            x: start.x,
            y: start.y,
            width: start.width,
            height: start.height,
        };

        // 左と上は、**向かいの縁を止めたまま**動かす。
        if handle.moves_left() {
            let right = start.x + start.width;
            quad.x = (start.x + delta[0]).min(right - MIN_WIDTH);
            quad.width = right - quad.x;
        }
        if handle.moves_right() {
            quad.width = (start.width + delta[0]).max(MIN_WIDTH);
        }
        if handle.moves_top() {
            let bottom = start.y + start.height;
            quad.y = (start.y + delta[1]).min(bottom - min_height);
            quad.height = bottom - quad.y;
        }
        if handle.moves_bottom() {
            quad.height = (start.height + delta[1]).max(min_height);
        }

        quad
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

        for window_item in &self.window_items {
            names.extend(window_item.object_names());
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

        // 縁の上なら、動かすより先に大きさを変える。帯の端も縁なので、こちらが勝つ。
        if let Some(handle) = self.resize_handle_at(draw_manager, x, y) {
            let quad = &self.frame_quad;

            self.resize = Some(Resize {
                handle,
                start_quad: Quad {
                    x: quad.x,
                    y: quad.y,
                    width: quad.width,
                    height: quad.height,
                },
                start: world,
            });

            return true;
        }

        let picks = draw_manager.pick_all(x, y);
        for pick in picks {
            if Some(pick.name) == self.register_name.title_bar {
                self.drag = Some(Drag {
                    grab: [world[0] - self.frame_quad.x, world[1] - self.frame_quad.y],
                });
            }
        }

        true
    }

    /// 左を放した。掴んでいたか、縁を引いていたら `true`。
    pub fn mouse_left_released(&mut self) -> bool {
        let dragged = self.drag.take().is_some();
        let resized = self.resize.take().is_some();

        dragged || resized
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
    pub fn mouse_moved(&mut self, draw_manager: &mut DrawManager, resources: &Resources, x: f32, y: f32) -> bool {
        if self.resize.is_some() {
            return self.resize_moved(draw_manager, resources, x, y);
        }

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

    /// 縁を引いている指が動いた。[`WindowFrame::mouse_moved`] からだけ呼ぶ。
    ///
    /// 動かすときと同じく、枠の外へ出ても放すまでは付いてきます。
    fn resize_moved(&mut self, draw_manager: &mut DrawManager, resources: &Resources, x: f32, y: f32) -> bool {
        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };
        let Some(resize) = self.resize.as_ref() else {
            return false;
        };

        let delta = [world[0] - resize.start[0], world[1] - resize.start[1]];
        let quad = self.resized_quad(resize, delta);

        let moved = quad.x != self.frame_quad.x || quad.y != self.frame_quad.y;
        let sized = quad.width != self.frame_quad.width || quad.height != self.frame_quad.height;

        self.frame_quad = quad;

        if sized {
            // ずらしの掛け直しも含む。
            self.rebuild_all(draw_manager, resources);
        } else if moved {
            // 左や上の縁は左上も動く。形は組んだ左上で組み直すので、ずらしで合わせる。
            self.apply_offset(draw_manager);
        }

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