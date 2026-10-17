use std::collections::HashSet;
use std::hash::Hash;
use std::panic::panic_any;
use std::sync::Arc;
use log::info;
use wgpu::Color;
use wgpu::naga::CooperativeRole::C;
use wgpu::naga::SwizzleComponent::W;
use wgpu::wgc::binding_model::GetBindGroupLayoutError;
use wgpu::wgc::impl_parent_device;
use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use gueiz_gpu::vertex::Vertex;
use crate::effect::Block;
use crate::draw_manager::DrawManager;
use std::time::Instant;
use crate::gui::clipboard::Clipboard;
use crate::gui::context_menu::{ContextMenu, ContextMenuEvent, ContextMenuFont, ContextMenuLabels, ContextMenuLayout, ContextMenuStyle, ContextMenuTheme, INPUT_TEXT_CONTEXT_MENU};
use crate::gui::window_item::{replace_outline, CursorShape, EditKey, KeyModifiers, Pointer, WindowItemArguments1, WindowItem, WindowItemArguments2};
use crate::instance::create_instance;
use crate::object::Object;
use crate::resource::{FontHandle, Resources};
use crate::objects::lines::{CurveRepresentationType, Lines};
use crate::objects::polygon_rounded::CornerType;
use crate::objects::rect::Rect;
use crate::objects::rect_rounded::RectRounded;
use crate::paint_type::{JointType, PaintType};
use crate::text::{measure, TextAlign, TextArea, TextLayoutData, TextLocation, TextRenderer, TextStyle};

const LINE_WIDTH: f32 = 1.0;

/// 縮められる幅の下限。題名を測れなかったとき（書体が無い・題名が空）に使います。
///
/// 測れたときは [`WindowFrame::min_width`] が題名の幅から決めます。
pub const MIN_WIDTH: f32 = 120.0;
/// 縁の**外側**で、大きさを変える取っ手になる幅。
const HANDLE_OUTSIDE: f32 = 6.0;
/// 縁の**内側**で、取っ手になる幅。広くすると、帯の端で掴みにくくなる。
const HANDLE_INSIDE: f32 = 3.0;

pub struct WindowFrame {
    scale_mode: ScaleMode,
    title: String,
    frame_quad: Quad,
    title_bar_height: f32,
    gap: Gap,
    window_font: WindowFont,
    window_theme: WindowTheme,
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
    /// 部品の図形に最後に積んだ、窓の外を削る山。積み直すときに、これを探して差し替える。
    ///
    /// 枠は角丸の四角なので、覆いを焼かずに式（[`Block::ClipRect`] と
    /// [`Block::ClipHalfPlane`]）で削ります。大きさや置き場所が変わっても、
    /// 数を差し替えるだけで済みます。
    applied_clip: Option<[Block; 2]>,
    /// この窓の図形の名前。枠の形・題名の字・部品の順。
    ///
    /// 動かすたびに集め直すと、そのたびに名前を複製することになるので覚えておきます。
    /// 字の形が増えるのは [`WindowFrame::create_object`] と組み直しのときだけなので、
    /// そこで [`WindowFrame::refresh_owned_names`] から作り直します。
    owned_names: Vec<String>,
    /// `owned_names` と同じ名前を、[`WindowFrame::owns`] で引けるように。
    owned_set: HashSet<String>,
    /// `owned_names` のうち、部品の図形が始まる位置。覆いを積むのはここから後ろ。
    item_names_start: usize,
    /// 名前を集め直したので、覆いが前と同じでも積み直す必要がある。
    clip_stale: bool,
    /// 指で動かした・大きさを変えたぶんのうち、まだ図形に移していないもの。
    /// [`WindowFrame::update`] でまとめて移す。
    pending: Pending,
    /// 縮められる幅の下限。[`WindowFrame::create_object`] で題名を測って決める。
    min_width: f32,
    /// 題名を最後に置いたときの窓の幅。まだ置いていなければ `None`。
    ///
    /// 題名の置き方で変わるのは幅だけなので、幅が同じなら置き直しを飛ばします
    /// （下の縁だけを引いたときなど）。
    title_layout_width: Option<f32>,
    /// 右クリックのメニュー。部品より手前に出す。
    context_menu: ContextMenu,
    /// 部品が次に動かしてほしい時刻（キャレットの点滅など）。
    next_update: Option<Instant>,
    /// 重なり順の底上げ。この窓の図形すべての z に足す。
    ///
    /// 窓を何枚も重ねるとき、手前の窓ほど大きくします（[`GuiManager`](crate::gui::gui_manager::GuiManager) が決める）。
    /// 部品は z を 0（ポップアップは 500、メニューは 1000）から決めるので、その上に足します。
    depth: f32,
}

/// 図形に移し待ちの変更。
///
/// 指の動きは 1 フレームに何度も届くことがあります。届くたびに組み直すと、
/// 描かれない途中の形まで組むことになるので、印だけ付けておいて
/// [`WindowFrame::update`] で 1 回にまとめます。
#[derive(Default, Clone, Copy)]
struct Pending {
    /// 大きさが変わった。形・題名・部品を組み直す（ずらしも含む）。
    rebuild: bool,
    /// 置き場所だけが変わった。ずらしを掛け直す。
    offset: bool,
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
    /// この取っ手の上で出す指の形。
    pub fn cursor_shape(self) -> CursorShape {
        match self {
            Self::Left | Self::Right => CursorShape::ResizeHorizontal,
            Self::Top | Self::Bottom => CursorShape::ResizeVertical,
            Self::TopLeft | Self::BottomRight => CursorShape::ResizeNwse,
            Self::TopRight | Self::BottomLeft => CursorShape::ResizeNesw,
        }
    }

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
    pub base_font_size: f32,
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct ThemeColor {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl WindowFrame {
    pub fn new(register_name_string: String,scale_mode: ScaleMode,title: String, frame_quad: Quad, title_bar_height: f32, gap: Gap, window_font: WindowFont, window_theme: WindowTheme) -> Self {
        Self {
            scale_mode,
            title,
            frame_quad,
            title_bar_height,
            gap,
            window_font,
            window_theme,
            register_name: RegisterName {
                frame: Some(register_name_string),
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
            applied_clip: None,
            owned_names: Vec::new(),
            owned_set: HashSet::new(),
            item_names_start: 0,
            clip_stale: true,
            pending: Pending::default(),
            min_width: MIN_WIDTH,
            title_layout_width: None,
            context_menu: ContextMenu::new(),
            next_update: None,
            depth: 0.0,
        }
    }

    pub fn add_window_item(&mut self, window_item: Box<dyn WindowItem>) {
        self.window_items.push(window_item);
    }

    pub fn create_object(&mut self, draw_manager: &mut DrawManager, resources: &Resources, surface_size: SurfaceSize, design: [f32; 2]) {
        // 題名が収まる幅より狭くはしない。形を組む前に決める。
        self.min_width = self.measure_min_width(resources);
        self.frame_quad.width = self.frame_quad.width.max(self.min_width);

        // ずらす量はここからの差で出す。
        self.built_origin = [self.frame_quad.x, self.frame_quad.y];

        let register_name_frame = self.register_name.frame.as_ref().unwrap();
        self.register_name.title_bar = Some(format!("{register_name_frame}-title_bar"));
        self.register_name.frame_outline = Some(format!("{register_name_frame}-frame_outline"));
        self.register_name.title_bar_slice_line = Some(format!("{register_name_frame}-title_bar_slice_line"));
        self.register_name.text_title = Some(format!("{register_name_frame}-text_title"));

        let scale_mode = self.scale_mode;
        let objects = self.build_shapes(self.shape_names().map(Option::unwrap_or_default));
        // 名前が重なると DrawManager が後ろに数を足すので、返ってきた名前を覚える。
        let [frame, title_bar, frame_outline, title_bar_slice_line] = objects.map(|mut object| {
            object.camera_for(surface_size, design, scale_mode);
            object.instance(create_instance());
            draw_manager.register(object)
        });
        self.register_name.frame = Some(frame);
        self.register_name.title_bar = Some(title_bar);
        self.register_name.frame_outline = Some(frame_outline);
        self.register_name.title_bar_slice_line = Some(title_bar_slice_line);

        let mut text_title = TextRenderer::new(self.register_name.text_title.as_deref().unwrap_or_default());
        text_title.camera_for(surface_size, design, self.scale_mode);
        text_title.color(self.window_theme.text_title_color.r,self.window_theme.text_title_color.g,self.window_theme.text_title_color.b,self.window_theme.text_title_color.a);
        text_title.text_layout_data(self.title_layout_data());
        // 題名が置けなくても、部品は組む。
        match resources.font(self.window_font.base_font) {
            Some(font) => match text_title.register_draw_manager(draw_manager, &font) {
                Ok(_) => self.title_layout_width = Some(self.frame_quad.width),
                Err(error) => log::warn!("failed to lay out the title: {error}"),
            },
            None => log::warn!("the title font is not in the resources; the title is not drawn"),
        }
        // 置けなかったときも預けておく。組み直しで幅が変われば、もう一度置いてみる。
        if let Some(replaced) = draw_manager.add_text_renderer(text_title) {
            log::warn!("the text renderer '{}' was already registered; it is replaced", replaced.name());
        }

        let [item_x, mut item_y] = self.item_origin();
        let width = self.item_width();
        let mut counter = 0;
        for window_item in &mut self.window_items {
            let item_size = window_item.create_object(
                WindowItemArguments1 {
                    draw_manager,
                    resources: &resources,
                    register_name: self.register_name.frame.as_ref().unwrap(),
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

        // 部品の図形は、ここまでで全部登録されている。
        self.refresh_owned_names(draw_manager);
        self.apply_clip(draw_manager);

        // メニューは部品より後に作って、手前に出す。見た目を決めていなければ、窓のテーマから。
        let (theme, font) = (&self.window_theme, &self.window_font);
        self.context_menu.style_or(|| ContextMenuStyle {
            font: ContextMenuFont { base_font: font.base_font, base_font_size: font.base_font_size },
            theme: ContextMenuTheme {
                fill_color: theme.frame_fill_color,
                outline_color: theme.frame_outline_color,
                text_color: theme.text_title_color,
                sub_title_color: ThemeColor { a: theme.text_title_color.a * 0.6, ..theme.text_title_color },
                highlight_color: theme.title_bar_slice_line_color,
                shadow_color: ThemeColor { r: 0.0, g: 0.0, b: 0.0, a: 0.35 },
                disabled_alpha: 0.35,
            },
            layout: ContextMenuLayout::default(),
        });

        // 文字を打ち込む欄のメニューが足されていなければ、いつものものを足す。
        if !self.context_menu.has_menu(INPUT_TEXT_CONTEXT_MENU) {
            self.context_menu.add_labels(ContextMenuLabels::text_input());
        }

        let base_name = self.register_name.frame.clone().unwrap_or_default();
        self.context_menu.create(draw_manager, resources, &base_name, surface_size, design, self.scale_mode);
    }

    /// 右クリックのメニューの見た目（書体・色・形）を決める。[`WindowFrame::create_object`] より前に。
    ///
    /// 決めなければ、窓のテーマと書体から作ります。
    pub fn set_context_menu_style(&mut self, style: ContextMenuStyle) {
        self.context_menu.set_style(style);
    }

    /// 右クリックのメニューを足す。[`WindowFrame::create_object`] より前に。
    ///
    /// 部品は [`ContextMenuLabels::name`] でどのメニューを開くかを決めます。
    /// 同じ名前のメニューがあれば差し替えます。
    pub fn add_context_menu_labels(&mut self, labels: ContextMenuLabels) {
        self.context_menu.add_labels(labels);
    }

    /// メニューで押された項目の関数を呼ぶ。押された項目が無ければ何もせず `false`。
    ///
    /// 関数は窓・[`DrawManager`]・[`Resources`]・クリップボードを使うので、
    /// それらを持っている呼ぶ側（`gueiz::winit_gui` など）が、入力を渡したあとに呼びます。
    pub fn run_context_menu_event(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        let Some((menu, entry)) = self.context_menu.take_pending() else {
            return false;
        };
        // 関数は窓の中にあるので、いったん借り出してから窓ごと渡す。
        let Some(mut handler) = self.context_menu.take_handler(menu, entry) else {
            return false;
        };

        handler(&mut ContextMenuEvent { window_frame: self, draw_manager, resources, clipboard });

        self.context_menu.put_handler(menu, entry, handler);
        true
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
        self.refresh_owned_names(draw_manager);

        // 組み直しで新しく出てきた図形（字の形など）には、まだずらしも覆いも掛かっていない。
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
            style: TextStyle::new(self.window_font.base_font_size),
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
    ///
    /// 前に置いたときと幅が同じなら、置き方も同じなので何もしません。
    fn relayout_title(&mut self, draw_manager: &mut DrawManager, resources: &Resources) {
        if self.title_layout_width == Some(self.frame_quad.width) {
            return;
        }

        let layout_data = self.title_layout_data();

        if let (Some(font), Some(name)) = (resources.font(self.window_font.base_font), self.register_name.text_title.as_ref()) {
            let laid_out = draw_manager.with_text_renderer(name, |text_title, draw_manager| {
                text_title.clear(draw_manager);
                text_title.text_layout_data(layout_data);

                match text_title.register_draw_manager(draw_manager, &font) {
                    Ok(_) => true,
                    Err(error) => {
                        log::warn!("failed to lay out the title again: {error}");
                        false
                    }
                }
            });

            if laid_out == Some(true) {
                self.title_layout_width = Some(self.frame_quad.width);
            }
        }
    }

    /// 枠・帯・縁取り・区切り線の形を組む。
    ///
    /// **組んだときの左上（`built_origin`）に、いまの大きさで組みます。**
    /// 置き場所のずれは [`Object::translate`] で持つので、ここには入れません。
    /// 名前は `names` のものを使います。順番は [`WindowFrame::shape_names`] と同じです。
    /// 組み直しで頂点だけが欲しいときは、名前に `""` を渡せば名前の確保が起きません。
    fn build_shapes(&self, names: [&str; 4]) -> [Object; 4] {
        let [x, y] = self.built_origin;
        let width = self.frame_quad.width;
        let height = self.frame_quad.height;
        let theme = &self.window_theme;
        let [frame_name, title_bar_name, frame_outline_name, title_bar_slice_line_name] = names;

        let vertex = |x: f32, y: f32, color: &ThemeColor| {
            Vertex::new_position_color(x, y, 0.0, color.r, color.g, color.b, color.a)
        };

        let frame = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(frame_name)
                    .paint_type(PaintType::Fill)
                    .vertex1(vertex(x, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex2(vertex(x + width, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex3(vertex(x + width, y + height, &theme.frame_fill_color))
                    .vertex4(vertex(x, y + height, &theme.frame_fill_color))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(frame_name)
                    .paint_type(PaintType::Fill)
                    .vertex1(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex2(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.frame_fill_color))
                    .vertex3(CornerType::Circle {radius}, vertex(x + width, y + height, &theme.frame_fill_color))
                    .vertex4(CornerType::Circle {radius}, vertex(x, y + height, &theme.frame_fill_color))
                    .end()
            }
        };

        let title_bar = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(title_bar_name)
                    .paint_type(PaintType::Fill)
                    .vertex1(vertex(x, y, &theme.title_bar_fill_color))
                    .vertex2(vertex(x + width, y, &theme.title_bar_fill_color))
                    .vertex3(vertex(x + width, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .vertex4(vertex(x, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .end()
            }
            QuadType::Rounded { radius} => {
                RectRounded::new(title_bar_name)
                    .paint_type(PaintType::Fill)
                    .vertex1(CornerType::Circle {radius}, vertex(x, y, &theme.title_bar_fill_color))
                    .vertex2(CornerType::Circle {radius}, vertex(x + width, y, &theme.title_bar_fill_color))
                    .vertex3(CornerType::None {}, vertex(x + width, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .vertex4(CornerType::None {}, vertex(x, y + self.title_bar_height, &theme.title_bar_fill_color))
                    .end()
            }
        };

        let frame_outline = match self.frame_quad.quad_type {
            QuadType::Normal => {
                Rect::new(frame_outline_name)
                    .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
                    .from(vertex(x, y, &theme.frame_outline_color))
                    .to_wh(vertex(width, height, &theme.frame_outline_color))
                    .end()
            }
            QuadType::Rounded { radius } => {
                RectRounded::new(frame_outline_name)
                    .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
                    .from(CornerType::Circle {radius},vertex(x, y, &theme.frame_outline_color))
                    .to_wh(vertex(width, height, &theme.frame_outline_color))
                    .end()
            }
        };

        let title_bar_slice_line = Lines::new(title_bar_slice_line_name)
            .paint_type(PaintType::Stroke {line_width: LINE_WIDTH, joint_type: JointType::None, strip: false, dash: None})
            .curve_type(CurveRepresentationType::Normal)
            .point(vertex(x + self.gap.x, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .last_point(vertex(x - self.gap.x + width, y + self.title_bar_height, &theme.title_bar_slice_line_color))
            .end();

        [frame, title_bar, frame_outline, title_bar_slice_line]
    }

    /// いまの大きさで形を組み直し、**登録済みの図形の中に**入れ直す。
    ///
    /// 登録し直すと [`DrawManager`] に名前が溜まるので、頂点だけを差し替えます。
    /// カメラ・複製・ずらしは図形に残ったままです。
    fn rebuild_shapes(&self, draw_manager: &mut DrawManager) {
        // 頂点しか使わないので、名前は付けない。
        for (name, built) in self.shape_names().into_iter().zip(self.build_shapes([""; 4])) {
            let Some(object) = name.and_then(|name| draw_manager.object_mut(name)) else {
                continue;
            };

            replace_outline(object, &built);
        }
    }

    /// 枠・帯・縁取り・区切り線の名前。[`WindowFrame::build_shapes`] が返す順と同じ。
    fn shape_names(&self) -> [Option<&str>; 4] {
        [
            self.register_name.frame.as_deref(),
            self.register_name.title_bar.as_deref(),
            self.register_name.frame_outline.as_deref(),
            self.register_name.title_bar_slice_line.as_deref(),
        ]
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
        if let Some(register_name) = self.register_name.text_title.as_ref() {
            draw_manager.with_text_renderer(register_name, |text_title, draw_manager| {
                text_title.camera_for(surface_size, design, self.scale_mode);

                for name in text_title.shape_ids() {
                    if let Some(object) = draw_manager.object_mut(name) {
                        object.camera_for(surface_size, design, self.scale_mode);
                    }
                }
            });
        }

        // 部品の図形のカメラも貼り直す。部品の側では要らない。
        for window_item in &mut self.window_items {
            for name in window_item.object_names(draw_manager) {
                if let Some(object) = draw_manager.object_mut(&name) {
                    object.camera_for(surface_size, design, self.scale_mode);
                }
            }

            window_item.resize(draw_manager, surface_size, design);
        }

        self.context_menu.resize(draw_manager, surface_size, design, self.scale_mode);
    }

    // --- フレームごと ---

    /// 指で動かした・大きさを変えたぶんを図形に移す。**描く前に 1 フレーム 1 回呼んでください。**
    ///
    /// [`WindowFrame::mouse_moved`] は窓の場所と大きさを覚えるだけで、図形には触りません。
    /// 指の動きが 1 フレームに何度届いても、組み直しはここでの 1 回で済みます。
    ///
    /// 時間で動くもの（キャレットの点滅など）も、ここで進めます。
    /// 次に動かしてほしい時刻は [`WindowFrame::next_update`] で分かります。
    ///
    /// 何か移したら `true`。
    pub fn update(&mut self, draw_manager: &mut DrawManager, resources: &Resources) -> bool {
        let pending = std::mem::take(&mut self.pending);

        if pending.rebuild {
            // ずらしの掛け直しも含む。
            self.rebuild_all(draw_manager, resources);
        } else if pending.offset {
            self.apply_offset(draw_manager);
        }

        let now = Instant::now();
        let mut relaid_out = false;
        let mut next: Option<Instant> = None;

        for window_item in &mut self.window_items {
            let animation = window_item.animate(draw_manager, resources, now);
            relaid_out |= animation.relaid_out;
            next = match (next, animation.next) {
                (Some(a), Some(b)) => Some(a.min(b)),
                (a, b) => a.or(b),
            };
        }

        // 並べ直しで増えた字の形にも、ずらしと覆いを掛ける。
        if relaid_out && self.refresh_owned_names(draw_manager) {
            self.apply_offset(draw_manager);
        }

        self.next_update = next;

        pending.rebuild || pending.offset || relaid_out
    }

    /// 部品が次に動かしてほしい時刻。その時刻に描き直しを頼んでください。
    /// 動くものが無ければ `None`（入力が来るまで待ってよい）。
    pub fn next_update(&self) -> Option<Instant> {
        self.next_update
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

        self.pending.offset = false;
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

    /// 縮められる幅の下限。題名が左右の余白ごと収まる幅です。
    ///
    /// [`WindowFrame::create_object`] より前は [`MIN_WIDTH`] を返します。
    pub fn min_width(&self) -> f32 {
        self.min_width
    }

    /// 題名を測って、幅の下限を出す。測れなければ [`MIN_WIDTH`]。
    ///
    /// 斜体や影で送り幅より右まで塗られることがあるので、塗られる右端も見ます。
    /// 丸い角が潰れないよう、角 2 つぶんより狭くはしません。
    fn measure_min_width(&self, resources: &Resources) -> f32 {
        if self.title.is_empty() {
            return MIN_WIDTH;
        }
        let Some(font) = resources.font(self.window_font.base_font) else {
            return MIN_WIDTH;
        };

        let size = measure(&font, &self.title, &TextStyle::new(self.window_font.base_font_size));
        let title_width = size.width.max(size.ink.right);

        (title_width + self.gap.x * 2.0).max(self.frame_quad.quad_type.radius() * 2.0)
    }

    /// 縮められる高さの下限。帯と、下の丸い角が収まるぶん。
    pub fn min_height(&self) -> f32 {
        self.title_bar_height + self.frame_quad.quad_type.radius() * 2.0
    }

    /// 大きさを決める。左上は動きません。下限より小さくはなりません。
    ///
    /// 題名と部品も新しい幅で置き直します。
    pub fn set_size(&mut self, draw_manager: &mut DrawManager, resources: &Resources, width: f32, height: f32) {
        self.frame_quad.width = width.max(self.min_width);
        self.frame_quad.height = height.max(self.min_height());

        // 組み直しはずらしも含むので、待っていたぶんはここで済む。
        self.pending = Pending::default();
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
        self.handle_at(draw_manager, &Pointer::new(x, y))
    }

    /// [`WindowFrame::resize_handle_at`] の中身。当てた結果を `pointer` に残すので、
    /// 呼んだ側はそのまま部品に渡せる。
    fn handle_at(&self, draw_manager: &DrawManager, pointer: &Pointer) -> Option<ResizeHandle> {
        let [world_x, world_y] = self.to_world(draw_manager, pointer.x, pointer.y)?;

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

        // 丸い角の上なら、縁は円弧。円の中心からの距離で取っ手の帯に入るかを見る。
        let radius = quad.quad_type.radius().min(quad.width / 2.0).min(quad.height / 2.0);
        if radius > 0.0 {
            let center_x = if world_x < left + radius {
                Some(left + radius)
            } else if world_x > right - radius {
                Some(right - radius)
            } else {
                None
            };
            let center_y = if world_y < top + radius {
                Some(top + radius)
            } else if world_y > bottom - radius {
                Some(bottom - radius)
            } else {
                None
            };

            if let (Some(center_x), Some(center_y)) = (center_x, center_y) {
                let distance = (world_x - center_x).hypot(world_y - center_y) - radius;
                if distance > HANDLE_OUTSIDE || distance < -HANDLE_INSIDE {
                    return None;
                }

                let handle = match (world_x < center_x, world_y < center_y) {
                    (true, true) => ResizeHandle::TopLeft,
                    (false, true) => ResizeHandle::TopRight,
                    (true, false) => ResizeHandle::BottomLeft,
                    (false, false) => ResizeHandle::BottomRight,
                };

                if distance < 0.0 && !self.in_front(draw_manager, pointer) {
                    return None;
                }

                return Some(handle);
            }
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

        if quad.hover(world_x, world_y) && !self.in_front(draw_manager, pointer) {
            return None;
        }

        Some(handle)
    }

    /// 掴んだときの窓から、指が `delta` 動いたときの窓。
    fn resized_quad(&self, resize: &Resize, delta: [f32; 2]) -> Quad {
        let start = &resize.start_quad;
        let handle = resize.handle;
        let min_width = self.min_width;
        let min_height = self.min_height();
        let mut quad = *start;

        // 左と上は、**向かいの縁を止めたまま**動かす。
        if handle.moves_left() {
            let right = start.x + start.width;
            quad.x = (start.x + delta[0]).min(right - min_width);
            quad.width = right - quad.x;
        }
        if handle.moves_right() {
            quad.width = (start.width + delta[0]).max(min_width);
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
            height: self.title_bar_height,
            ..self.frame_quad
        }
    }

    /// 組んだ場所からのずれを、持っている図形すべてに貼る。
    ///
    /// 頂点は組んだ場所に置いたままで、**親の変換だけ**をずらします。
    /// [`Object::translate`] は複製より手前に掛かるので、字のように
    /// 複製をたくさん持つ図形も、1 回で丸ごと動きます。
    fn apply_offset(&mut self, draw_manager: &mut DrawManager) {
        let x = self.frame_quad.x - self.built_origin[0];
        let y = self.frame_quad.y - self.built_origin[1];

        for name in &self.owned_names {
            if let Some(object) = draw_manager.object_mut(name) {
                // 重なり順（z）は部品が決めたものを残す（カラーピッカーのパレットのように、手前へ開くものがある）。
                let z = object.depth();
                object.translate(x, y, z);
            }
        }

        for window_item in &mut self.window_items {
            window_item.offset_changed(draw_manager, [x, y]);
        }

        // 削る範囲はワールドで持つので、枠と一緒には動かない。いまの場所で積み直す。
        self.apply_clip(draw_manager);
    }

    /// 窓の外を削る山。いまの置き場所と大きさで組む。
    ///
    /// 削るのは枠（題名の帯より下）の外です。角丸の四角で窓全体の外を削り、
    /// 半平面で帯から上を削ります。
    ///
    /// 縁取りと区切り線は線の中心が縁に乗っているので、線の太さの半分だけ内側で削ります。
    /// 部品が線の上に描かれないように。
    fn clip_blocks(&self) -> [Block; 2] {
        let inset = LINE_WIDTH;
        let quad = &self.frame_quad;
        let top = quad.y + self.title_bar_height + inset;

        [
            Block::ClipRect {
                min: [quad.x + inset, quad.y + inset],
                max: [quad.x + quad.width - inset, quad.y + quad.height - inset],
                radius: (quad.quad_type.radius() - inset).max(0.0),
                softness: 0.0,
                invert: false,
            },
            Block::ClipHalfPlane {
                normal: [0.0, -1.0],
                distance: -top,
                softness: 0.0,
                invert: false,
            },
        ]
    }

    /// 部品の図形に、窓の外を削る山を積む。何度呼んでも 1 組にしかなりません。
    ///
    /// 前に積んだ山があれば差し替え、無ければ（新しく出てきた字の形など）積みます。
    /// 山が前と同じで、図形も増えていなければ何もしません。
    fn apply_clip(&mut self, draw_manager: &mut DrawManager) {
        let blocks = self.clip_blocks();
        let applied = self.applied_clip;

        if !self.clip_stale && applied == Some(blocks) {
            return;
        }

        for name in &self.owned_names[self.item_names_start..] {
            let Some(object) = draw_manager.object_mut(name) else {
                continue;
            };

            for (index, block) in blocks.into_iter().enumerate() {
                let old = applied.map(|applied| applied[index]);
                object.replace_effect(|effect| Some(*effect) == old, block);
            }
        }

        self.applied_clip = Some(blocks);
        self.clip_stale = false;
    }

    /// 持っている図形の名前を集め直す。図形が増えうるところ（作る・組み直す）の後で呼ぶ。
    ///
    /// 前と同じ名前なら（新しい字の形が増えていなければ）何もせず `false`。
    /// 変わったら `true` で、覆いは積み直しが要る印を付ける。
    fn refresh_owned_names(&mut self, draw_manager: &mut DrawManager) -> bool {
        let mut names: Vec<String> = self.shape_names().into_iter().flatten().map(String::from).collect();

        if let Some(text_title) = &self.register_name.text_title {
            names.extend(draw_manager.group_names(text_title).map(String::from));
        }

        self.item_names_start = names.len();

        for window_item in &self.window_items {
            names.extend(window_item.object_names(draw_manager));
        }

        if names == self.owned_names {
            return false;
        }

        // 新しく出てきた図形（打って増えた字の形など）は、部品が決めた z のまま登録されている。
        // 窓の底上げをここで足す。前からある図形には足してあるので触らない。
        if self.depth != 0.0 {
            for name in names.iter().filter(|name| !self.owned_set.contains(name.as_str())) {
                if let Some(object) = draw_manager.object_mut(name) {
                    let z = object.depth();
                    object.z(z + self.depth);
                }
            }
        }

        self.owned_set = names.iter().cloned().collect();
        self.owned_names = names;
        self.clip_stale = true;
        true
    }

    /// この窓の図形か。字の形と部品の図形も入ります。
    ///
    /// 指が動くたびに引くので、名前の並び（`owned_names`）をなめずに [`HashSet`] で引く。
    fn owns(&self, name: &str) -> bool {
        self.owned_set.contains(name)
    }

    /// 指の下で一番手前にあるのが、この窓の図形か。当てるのは `pointer` につき 1 回だけ。
    fn in_front(&self, draw_manager: &DrawManager, pointer: &Pointer) -> bool {
        pointer.pick(draw_manager).is_some_and(|pick| self.owns(&pick.name))
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
    pub fn mouse_left_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        // メニューが開いていれば、押したのは項目か、閉じるための外か。どちらもここで取る。
        if self.context_menu.is_open() {
            if !self.context_menu.press(draw_manager, x, y) {
                self.context_menu.close(draw_manager);
            }
            return true;
        }

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };
        // 当てるのは、取っ手・窓・部品を通して 1 回だけ。
        let pointer = Pointer::new(x, y);

        if let Some(handle) = self.handle_at(draw_manager, &pointer) {
            self.unfocus_items(draw_manager, None);
            self.resize = Some(Resize { handle, start_quad: self.frame_quad, start: world });
            return true;
        }

        // 窓の外なら取らない。押されたのはよそなので、フォーカスは手放す。
        if !self.frame_quad.hover(world[0], world[1]) {
            self.unfocus_items(draw_manager, None);
            return false;
        }
        // 手前のものが自分の図形でなければ、別の窓がかぶさっている（A3）。
        if !self.in_front(draw_manager, &pointer) {
            self.unfocus_items(draw_manager, None);
            return false;
        }

        // 部品が先（A4）。取った部品があればそこで止める。
        let taken = self
            .window_items
            .iter_mut()
            .position(|window_item| window_item.mouse_left_pressed(draw_manager, &pointer));

        // 取った部品のほかは、フォーカスを手放す。
        self.unfocus_items(draw_manager, taken);

        if taken.is_some() {
            return true;
        }

        // 帯の上なら掴む。題名の字の上でも掴めるよう、図形の名前ではなく範囲で見る。
        if self.title_bar_quad().hover(world[0], world[1]) {
            self.drag = Some(Drag { grab: [world[0] - self.frame_quad.x, world[1] - self.frame_quad.y] });
        }

        // 窓の中身の上は、何も無くても後ろへは渡さない。
        true
    }

    /// その画素の位置で出したい指の形。
    ///
    /// - 縁を引いているあいだは、引いている取っ手の形のまま（指が縁から外れても）。
    /// - 掴んで動かしているあいだは [`CursorShape::Move`]。
    /// - 縁の上なら、その取っ手の形。
    /// - 窓の中で手前がこの窓なら、指の下の部品に聞く。
    pub fn cursor_shape(&self, draw_manager: &DrawManager, x: f32, y: f32) -> CursorShape {
        self.cursor_shape_at(draw_manager, &Pointer::new(x, y))
    }

    /// [`WindowFrame::cursor_shape`] の中身。窓を何枚も持つ側が、当てた結果を窓のあいだで使い回せるように。
    pub(crate) fn cursor_shape_at(&self, draw_manager: &DrawManager, pointer: &Pointer) -> CursorShape {
        let (x, y) = (pointer.x, pointer.y);
        if self.context_menu.contains(draw_manager, x, y) {
            return CursorShape::Default;
        }
        if let Some(resize) = &self.resize {
            return resize.handle.cursor_shape();
        }
        if self.drag.is_some() {
            return CursorShape::Move;
        }
        // 当てるのは、取っ手・窓・部品を通して 1 回だけ。
        if let Some(handle) = self.handle_at(draw_manager, pointer) {
            return handle.cursor_shape();
        }

        // 窓の外なら当てるまでもない。pick は全図形をなめるので、指が動くたびには避けたい。
        if !self.to_world(draw_manager, x, y).is_some_and(|world| self.frame_quad.hover(world[0], world[1])) {
            return CursorShape::Default;
        }
        if !self.in_front(draw_manager, pointer) {
            return CursorShape::Default;
        }

        self.window_items
            .iter()
            .find_map(|window_item| window_item.cursor_shape(draw_manager, pointer))
            .unwrap_or_default()
    }

    /// 指がこの窓の上か（開いているメニュー・縁の取っ手を含む）。上に別の窓が重なっていれば `false`。
    ///
    /// 掴んでいる・縁を引いているあいだは、指がどこにあっても `true`。
    pub fn contains(&self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        self.contains_at(draw_manager, &Pointer::new(x, y))
    }

    /// [`WindowFrame::contains`] の中身。
    pub(crate) fn contains_at(&self, draw_manager: &DrawManager, pointer: &Pointer) -> bool {
        if self.drag.is_some() || self.resize.is_some() {
            return true;
        }
        if self.context_menu.contains(draw_manager, pointer.x, pointer.y) {
            return true;
        }
        if self.handle_at(draw_manager, pointer).is_some() {
            return true;
        }

        self.to_world(draw_manager, pointer.x, pointer.y).is_some_and(|world| self.frame_quad.hover(world[0], world[1]))
            && self.in_front(draw_manager, pointer)
    }

    // --- 重なり ---

    /// 重なり順の底上げ。
    pub fn depth(&self) -> f32 {
        self.depth
    }

    /// 重なり順の底上げを変える。この窓の図形すべて（開くメニューも）の z を、差のぶんだけずらす。
    ///
    /// 窓を何枚も重ねるときに、手前の窓ほど大きくします。手前に出すたびに図形をすべてなめるので、
    /// 指が動くたびには呼ばないでください。
    pub fn set_depth(&mut self, draw_manager: &mut DrawManager, depth: f32) {
        let delta = depth - self.depth;
        if delta == 0.0 {
            return;
        }

        for name in &self.owned_names {
            if let Some(object) = draw_manager.object_mut(name) {
                let z = object.depth();
                object.z(z + delta);
            }
        }

        self.depth = depth;
        self.context_menu.set_depth(depth);
    }

    /// 部品のフォーカスを手放し、開いているメニューを閉じる。ほかの窓が押されたときに。
    /// 見た目が変わったら `true`。
    pub fn unfocus(&mut self, draw_manager: &mut DrawManager) -> bool {
        let menu_open = self.context_menu.is_open();
        if menu_open {
            self.context_menu.close(draw_manager);
        }

        self.unfocus_items(draw_manager, None) || menu_open
    }

    /// 右クリックのメニューが開いているか。
    pub fn is_context_menu_open(&self) -> bool {
        self.context_menu.is_open()
    }

    /// `keep` の部品のほかは、フォーカスを手放させる。見た目が変わったら `true`。
    fn unfocus_items(&mut self, draw_manager: &mut DrawManager, keep: Option<usize>) -> bool {
        let mut changed = false;

        for (index, window_item) in self.window_items.iter_mut().enumerate() {
            if Some(index) != keep {
                changed |= window_item.unfocus(draw_manager);
            }
        }

        changed
    }

    // --- キーボード ---

    /// キーと文字を受け取りたい部品があるか。
    pub fn has_focus(&self) -> bool {
        self.window_items.iter().any(|window_item| window_item.has_focus())
    }

    /// IME を使いたい部品があるか。呼ぶ側はこれを見て IME を点けたり消したりする。
    pub fn wants_ime(&self) -> bool {
        self.window_items.iter().any(|window_item| window_item.wants_ime())
    }

    /// フォーカスを持っている部品に渡す。部品が図形を書き換えたら、
    /// 名前を集め直して、新しい字の形にもずらしと覆いを掛ける。
    fn with_focused_item(
        &mut self,
        draw_manager: &mut DrawManager,
        f: impl FnOnce(&mut dyn WindowItem, &mut DrawManager) -> bool,
    ) -> bool {
        let Some(window_item) = self.window_items.iter_mut().find(|window_item| window_item.has_focus()) else {
            return false;
        };

        if !f(window_item.as_mut(), draw_manager) {
            return false;
        }

        // 字の形が増えたときだけ、新しい形にずらしと覆いを掛ける。
        // キャレットを動かしただけなら、どの図形も前のままでよい。
        if self.refresh_owned_names(draw_manager) {
            self.apply_offset(draw_manager);
        }

        true
    }

    /// 文字が打たれた（IME で確定した文字も）。受け取った部品があれば `true`。
    pub fn text_input(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.with_focused_item(draw_manager, |window_item, draw_manager| {
            window_item.text_input(draw_manager, resources, text)
        })
    }

    /// IME で変換中の文字が変わった。
    pub fn ime_preedit(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str, cursor: Option<usize>) -> bool {
        self.with_focused_item(draw_manager, |window_item, draw_manager| {
            window_item.ime_preedit(draw_manager, resources, text, cursor)
        })
    }

    /// 文字を書き換えるキーが押された。
    pub fn key_pressed(&mut self, draw_manager: &mut DrawManager, resources: &Resources, key: EditKey) -> bool {
        // キーを押したらメニューは閉じる。Esc は閉じるだけ。
        if self.context_menu.is_open() {
            self.context_menu.close(draw_manager);
            if key == EditKey::Escape {
                return true;
            }
        }

        self.with_focused_item(draw_manager, |window_item, draw_manager| {
            window_item.key_pressed(draw_manager, resources, key)
        })
    }

    /// Shift や Ctrl の押し具合が変わった。すべての部品に知らせる。
    pub fn modifiers_changed(&mut self, modifiers: KeyModifiers) {
        for window_item in &mut self.window_items {
            window_item.modifiers_changed(modifiers);
        }
    }

    /// フォーカスを持っている部品で選んでいる文字。コピーに使う。
    pub fn selected_text(&self) -> Option<String> {
        self.window_items
            .iter()
            .find(|window_item| window_item.has_focus())?
            .selected_text()
    }

    /// 選んでいる文字をクリップボードへ入れる。何も選んでいなければ何もせず `false`。
    pub fn copy_to_clipboard(&self, clipboard: &mut dyn Clipboard) -> bool {
        self.selected_text().is_some_and(|text| clipboard.set_text(&text))
    }

    /// 選んでいる文字をクリップボードへ入れて消す。何も選んでいなければ何もせず `false`。
    pub fn cut_to_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        let Some(text) = self.selected_text() else {
            return false;
        };
        if !clipboard.set_text(&text) {
            return false;
        }

        self.key_pressed(draw_manager, resources, EditKey::Delete)
    }

    /// クリップボードの文字を貼り付ける。
    pub fn paste_from_clipboard(&mut self, draw_manager: &mut DrawManager, resources: &Resources, clipboard: &mut dyn Clipboard) -> bool {
        let Some(text) = clipboard.get_text() else {
            return false;
        };

        self.paste(draw_manager, resources, &text)
    }

    /// 文字を、フォーカスを持っている部品に貼り付ける（打った文字と違い、1 回で元に戻せる）。
    pub fn paste(&mut self, draw_manager: &mut DrawManager, resources: &Resources, text: &str) -> bool {
        self.with_focused_item(draw_manager, |window_item, draw_manager| {
            window_item.paste(draw_manager, resources, text)
        })
    }

    /// 字を打つ位置を、**画面の画素**で（`[x, y, 幅, 高さ]`）。IME の候補を出す場所に使う。
    ///
    /// フォーカスを持っている部品が無ければ `None`。
    pub fn ime_cursor_area(&self, draw_manager: &DrawManager) -> Option<[f32; 4]> {
        let [x, y, width, height] = self
            .window_items
            .iter()
            .find(|window_item| window_item.has_focus())?
            .caret_area()?;

        // 部品は組んだときの座標で答えるので、窓のずれを足してワールドにする。
        let offset = [self.frame_quad.x - self.built_origin[0], self.frame_quad.y - self.built_origin[1]];
        let camera = draw_manager.object(self.register_name.title_bar.as_deref()?)?.view_camera();

        let [left, top] = camera.world_to_screen(x + offset[0], y + offset[1]);
        let [right, bottom] = camera.world_to_screen(x + width + offset[0], y + height + offset[1]);

        Some([left.min(right), top.min(bottom), (right - left).abs(), (bottom - top).abs()])
    }

    /// 左を放した。掴んでいたか、縁を引いていたら `true`。
    pub fn mouse_left_released(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        let dragged = self.drag.take().is_some();
        let resized = self.resize.take().is_some();

        // 押したまま外へ出た部品も戻せるよう、どこで放しても全部に渡す。
        // 当てるのは、要った部品があったときに 1 回だけ。
        let pointer = Pointer::new(x, y);
        let mut item_changed = false;
        for window_item in &mut self.window_items {
            item_changed |= window_item.mouse_left_released(draw_manager, &pointer);
        }

        dragged || resized || item_changed
    }

    /// 右を押した。部品がメニューの項目を返したら、指の位置にメニューを開く。**この窓が取ったら `true`。**
    pub fn mouse_right_pressed(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        if self.context_menu.is_open() {
            self.context_menu.close(draw_manager);
        }

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };
        if !self.frame_quad.hover(world[0], world[1]) {
            return false;
        }
        // 当てるのはここで 1 回だけ。結果は部品にも使い回させる。
        let pointer = Pointer::new(x, y);
        if !self.in_front(draw_manager, &pointer) {
            return false;
        }

        let opened = self
            .window_items
            .iter_mut()
            .enumerate()
            .find_map(|(index, window_item)| window_item.context_menu(draw_manager, &pointer).map(|request| (index, request)));

        if let Some((index, request)) = opened {
            self.unfocus_items(draw_manager, Some(index));
            self.context_menu.open(draw_manager, &request.menu, world, request.state);
        }

        // 窓の中身の上は、メニューが無くても後ろへは渡さない。
        true
    }

    /// ホイールが回った。指の下の部品に渡す。`lines` は回した行数 `[横, 縦]` で、正なら左・上へ送る。
    /// **この窓が取ったら `true`。**
    pub fn mouse_wheel(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32, lines: [f32; 2]) -> bool {
        if self.context_menu.is_open() {
            self.context_menu.close(draw_manager);
        }

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };
        if !self.frame_quad.hover(world[0], world[1]) {
            return false;
        }
        // 当てるのはここで 1 回だけ。結果は部品にも使い回させる。
        let pointer = Pointer::new(x, y);
        if !self.in_front(draw_manager, &pointer) {
            return false;
        }

        self.window_items
            .iter_mut()
            .any(|window_item| window_item.mouse_wheel(draw_manager, &pointer, lines))
    }

    pub fn mouse_right_released(&mut self) -> bool {
        false
    }

    /// 指が動いた。掴んでいれば窓が付いてきて `true`。
    ///
    /// **枠の外へ出ても離しません。** 放すまでは動かし続けます。
    /// 離すと、少し外れた瞬間に窓が置き去りになります。
    ///
    /// 窓の場所と大きさを覚えるだけで、図形はまだ動きません。
    /// [`WindowFrame::update`] で移してください。
    ///
    /// 掴んでも引いてもいなければ、部品に渡します（指が上に来たら色を変えるなど）。
    /// 部品の見た目が変わったときも `true`。
    pub fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        self.mouse_moved_at(draw_manager, &Pointer::new(x, y))
    }

    /// [`WindowFrame::mouse_moved`] の中身。窓を何枚も持つ側が、当てた結果を窓のあいだで使い回せるように。
    pub(crate) fn mouse_moved_at(&mut self, draw_manager: &mut DrawManager, pointer: &Pointer) -> bool {
        let (x, y) = (pointer.x, pointer.y);
        // メニューが開いているあいだは、指の下の項目を光らせるだけ。
        if self.context_menu.is_open() {
            return self.context_menu.hover(draw_manager, x, y);
        }

        if self.resize.is_some() {
            return self.resize_moved(draw_manager, x, y);
        }

        let Some(drag) = self.drag.as_ref() else {
            // 当てるのは、どれかの部品が要ったときに 1 回だけ。
            let mut item_changed = false;
            for window_item in &mut self.window_items {
                item_changed |= window_item.mouse_moved(draw_manager, pointer);
            }

            return item_changed;
        };

        let grab = drag.grab;

        let Some(world) = self.to_world(draw_manager, x, y) else {
            return false;
        };

        // 掴んだ場所が指の下に来るように置く。動いた量を足していくのではないので、
        // 取りこぼしてもずれが溜まらない。
        self.frame_quad.x = world[0] - grab[0];
        self.frame_quad.y = world[1] - grab[1];

        self.pending.offset = true;

        true
    }

    /// 縁を引いている指が動いた。[`WindowFrame::mouse_moved`] からだけ呼ぶ。
    ///
    /// 動かすときと同じく、枠の外へ出ても放すまでは付いてきます。
    fn resize_moved(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
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
            self.pending.rebuild = true;
        } else if moved {
            // 左や上の縁は左上も動く。形は組んだ左上で組み直すので、ずらしで合わせる。
            self.pending.offset = true;
        }

        true
    }
}

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DeviceId(pub i64);

#[derive(Debug, Copy, Clone, PartialEq)]
pub enum QuadType {
    Normal,
    Rounded{
        radius: f32
    }
}

impl QuadType {
    /// 角の半径。角の無い四角は `0.0`。
    pub fn radius(self) -> f32 {
        match self {
            Self::Normal => 0.0,
            Self::Rounded { radius } => radius,
        }
    }
}

#[derive(Debug, Copy, Clone, PartialEq)]
pub struct Quad {
    pub quad_type: QuadType,
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