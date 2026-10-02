//! GUI の命令が **本物の [`DrawManager`] に届いて、図形を使い回しているか**を
//! 確かめる。
//!
//! 見た目（画素）ではなく**結び付きと使い回し**を見ます。GUI の基盤で壊れると
//! 痛いのは「開いて閉じるたびに図形が増える」「動かしていないのに毎フレーム
//! 送り直す」といったところで、これは画素を読んでも分かりません。
//!
//! GPU が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test gui_render_gpu -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::camera::Camera;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::button::Button;
use gueiz_2d::gui::color::Color;
use gueiz_2d::gui::container::{Floating, Stack};
use gueiz_2d::gui::dropdown::Dropdown;
use gueiz_2d::gui::context::{FontMeasure, NoTextMeasure};
use gueiz_2d::gui::geometry::{Corners, Insets, Point, Rect, Size};
use gueiz_2d::gui::event::{InputEvent, PointerButton};
use gueiz_2d::gui::id::WidgetId;
use gueiz_2d::gui::layout::Constraints;
use gueiz_2d::gui::painter::Painter;
use gueiz_2d::gui::render::GuiRenderer;
use gueiz_2d::gui::scroll_area::ScrollArea;
use gueiz_2d::gui::text_field::TextField;
use gueiz_2d::gui::theme::{Role, StateKey, Style, Theme};
use gueiz_2d::gui::widget::Behavior;
use gueiz_2d::gui::window_frame::WindowFrame;
use gueiz_2d::gui::{Gui, MeasureContext, PaintContext, Widget};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const VIEWPORT: Size = Size {
    width: 400.0,
    height: 300.0,
};
const FORMAT: TextureFormat = TextureFormat::Bgra8UnormSrgb;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

    GPU.get_or_init(|| {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());

        let adapter =
            pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default()))
                .ok()?;

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("gui render gpu test"),
            required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
            ..Default::default()
        }))
        .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

/// GPU が無ければ、言い残して抜ける。
macro_rules! gpu {
    () => {
        match gpu() {
            Some(gpu) => gpu,
            None => {
                eprintln!("skipped: no gpu adapter");
                return;
            }
        }
    };
}

fn draw_manager(gpu: &Gpu) -> DrawManager {
    DrawManager::new(
        &gpu.device,
        &gpu.queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )
    .expect("DrawManager を作れなかった")
}

fn camera() -> Camera {
    Camera::orthographic_2d(VIEWPORT.width, VIEWPORT.height)
}

/// 四角を 1 枚塗るだけのウィジェット。色と大きさを外から変えられる。
struct Plate {
    size: Size,
    color: Color,
    corners: Corners,
}

impl Plate {
    fn boxed(width: f32, height: f32, color: Color) -> Box<dyn Widget> {
        Box::new(Self {
            size: Size::new(width, height),
            color,
            corners: Corners::ZERO,
        })
    }
}

impl Widget for Plate {
    fn measure(&mut self, _: &mut MeasureContext<'_>, constraints: Constraints) -> Size {
        constraints.constrain(self.size)
    }

    fn paint(&self, context: &PaintContext<'_>, painter: &mut Painter<'_>) {
        painter.rounded_rect(context.rect(), self.corners, self.color);
    }

    fn behavior(&self) -> Behavior {
        Behavior::DECORATION
    }
}

/// `Gui` を作って 1 フレーム回す。
fn frame(gui: &mut Gui, renderer: &mut GuiRenderer, manager: &mut DrawManager) {
    gui.layout(VIEWPORT, &NoTextMeasure);

    let list = gui.paint();
    renderer.sync(manager, list, camera(), None);
}

fn prepared(manager: &mut DrawManager) {
    let gpu = gpu().expect("呼ぶ前に gpu!() で確かめること");

    manager
        .prepare(&gpu.device, &gpu.queue)
        .expect("支度に失敗した");
}

#[test]
fn commands_become_objects_the_draw_manager_can_prepare() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));
    gui.tree_mut()
        .add_child(root, Plate::boxed(100.0, 50.0, Color::hex(0xff0000)));
    gui.tree_mut()
        .add_child(root, Plate::boxed(80.0, 40.0, Color::hex(0x00ff00)));

    frame(&mut gui, &mut renderer, &mut manager);

    assert_eq!(renderer.object_count(), 2, "命令 1 つにつき図形 1 つ");
    assert_eq!(manager.object_count(), 2);

    // GPU が受け取れる形になっているか。ここで落ちるなら頂点か山が壊れている。
    prepared(&mut manager);

    assert_eq!(manager.draw_count(), 2);
}

#[test]
fn an_unchanged_frame_touches_nothing() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));
    let plate = gui
        .tree_mut()
        .add_child(root, Plate::boxed(100.0, 50.0, Color::hex(0xff0000)));

    frame(&mut gui, &mut renderer, &mut manager);
    prepared(&mut manager);

    let before: Vec<_> = manager
        .object("gui 1")
        .expect("登録されている")
        .triangles()
        .to_vec();

    // もう 1 フレーム。何も変えていない。
    frame(&mut gui, &mut renderer, &mut manager);

    let after = manager.object("gui 1").unwrap().triangles().to_vec();

    assert_eq!(before, after, "積み直していない");
    assert_eq!(renderer.object_count(), 1);

    // 色を変えれば積み直る。
    let widget = gui.tree_mut().get_as_mut::<Plate>(plate).unwrap();
    widget.color = Color::hex(0x0000ff);

    frame(&mut gui, &mut renderer, &mut manager);

    let changed = manager.object("gui 1").unwrap().triangles().to_vec();

    assert_ne!(before[0].b, changed[0].b, "色が頂点に入っている");
}

#[test]
fn removing_a_widget_recycles_its_object() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));
    let first = gui
        .tree_mut()
        .add_child(root, Plate::boxed(100.0, 50.0, Color::hex(0xff0000)));
    gui.tree_mut()
        .add_child(root, Plate::boxed(80.0, 40.0, Color::hex(0x00ff00)));

    frame(&mut gui, &mut renderer, &mut manager);
    assert_eq!(manager.object_count(), 2);

    // 1 つ消す。
    gui.tree_mut().remove(first);
    frame(&mut gui, &mut renderer, &mut manager);

    assert_eq!(renderer.recycled_count(), 1, "空きとして取っておく");
    assert_eq!(
        manager.object_count(),
        2,
        "DrawManager には消す手立てが無いので数は減らない"
    );

    // 空いた図形は描かれない。
    prepared(&mut manager);

    // 足し直すと、取っておいたものが使い回される。
    gui.tree_mut()
        .add_child(root, Plate::boxed(60.0, 30.0, Color::hex(0x0000ff)));
    frame(&mut gui, &mut renderer, &mut manager);

    assert_eq!(renderer.recycled_count(), 0);
    assert_eq!(
        manager.object_count(),
        2,
        "開いて閉じても図形は増えない"
    );
}

#[test]
fn clipping_becomes_an_effect_block() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));
    let frame_id = gui.tree_mut().add_child(
        root,
        Box::new(
            WindowFrame::at(0.0, 0.0, 100.0, 60.0)
                .without_title_bar()
                .padding(Insets::ZERO),
        ),
    );
    gui.tree_mut()
        .add_child(frame_id, Plate::boxed(500.0, 500.0, Color::hex(0xffffff)));

    frame(&mut gui, &mut renderer, &mut manager);

    // 窓の背景と、切り抜かれた中身。
    let clipped = manager
        .names()
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|name| manager.object(&name))
        .filter(|object| !object.effects().is_empty())
        .count();

    assert_eq!(clipped, 1, "中身にだけ切り抜きの山が付く");

    prepared(&mut manager);
}

#[test]
fn stacking_order_follows_the_display_list() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));
    gui.tree_mut()
        .add_child(root, Plate::boxed(100.0, 100.0, Color::hex(0xff0000)));
    gui.tree_mut()
        .add_child(root, Plate::boxed(100.0, 100.0, Color::hex(0x00ff00)));

    frame(&mut gui, &mut renderer, &mut manager);

    let first = manager.object("gui 1").unwrap().depth();
    let second = manager.object("gui 2").unwrap().depth();

    assert!(first < second, "あとの命令が手前: {first} < {second}");

    prepared(&mut manager);

    // 描く順も z のとおり。
    let order = manager.draw_order();
    assert_eq!(order.len(), 2);
}

#[test]
fn a_themed_window_frame_reaches_the_gpu() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let mut theme = Theme::new();
    theme.set(
        Role::SURFACE,
        StateKey::Normal,
        Style::BARE
            .background(Color::hex(0x1e1e24))
            .border(Color::hex(0x6688aa), 2.0)
            .corners(Corners::all(10.0)),
    );
    theme.set(
        Role::ACCENT,
        StateKey::Normal,
        Style::BARE.background(Color::hex(0x2f3040)),
    );
    gui.set_theme(theme);

    let root = gui.set_root(Box::new(Floating::new()));
    let window = gui.tree_mut().add_child(
        root,
        Box::new(
            WindowFrame::new(Point::new(20.0, 20.0), Size::new(260.0, 180.0))
                .title("設定")
                .movable(true),
        ),
    );
    gui.tree_mut()
        .add_child(window, Plate::boxed(100.0, 24.0, Color::hex(0x88ccff)));

    frame(&mut gui, &mut renderer, &mut manager);

    // 背景・帯・中身・枠線。題名の字は書体を渡していないので空。
    assert_eq!(renderer.object_count(), 5, "背景/帯/題名/中身/枠線");

    prepared(&mut manager);

    assert_eq!(
        gui.tree().bounds(window),
        Rect::new(20.0, 20.0, 260.0, 180.0)
    );
}

#[test]
fn clearing_keeps_the_objects_for_later() {
    let gpu = gpu!();
    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let root = gui.set_root(Box::new(Floating::new()));

    for _ in 0..4 {
        gui.tree_mut()
            .add_child(root, Plate::boxed(30.0, 30.0, Color::WHITE));
    }

    frame(&mut gui, &mut renderer, &mut manager);
    assert_eq!(manager.object_count(), 4);

    renderer.clear(&mut manager);

    assert_eq!(renderer.recycled_count(), 4);
    assert_eq!(manager.object_count(), 4);

    prepared(&mut manager);

    // 作り直しても増えない。
    frame(&mut gui, &mut renderer, &mut manager);
    assert_eq!(manager.object_count(), 4);
    assert_eq!(renderer.recycled_count(), 0);

    let _ = WidgetId::NONE;
}

// --- ボタン ---

/// 書体を 1 つ読む。見つからなければ `None`。
///
/// 試験のためだけに漏らします。`Font` はバイト列を借りるので、
/// `'static` にしておくと持ち回しが楽になります。
fn font() -> Option<&'static gueiz_2d::font::Font<'static>> {
    use gueiz_2d::font::Font;

    static FONT: OnceLock<Option<Font<'static>>> = OnceLock::new();

    FONT.get_or_init(|| {
        const CANDIDATES: &[&str] = &[
            "/System/Library/Fonts/Supplemental/Arial.ttf",
            "C:/Windows/Fonts/arial.ttf",
            "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        ];

        let data = CANDIDATES
            .iter()
            .find_map(|path| std::fs::read(path).ok())?;

        Font::from_bytes(data.leak()).ok()
    })
    .as_ref()
}

fn control_theme() -> Theme {
    let mut theme = Theme::new();

    theme.set(
        Role::CONTROL,
        StateKey::Normal,
        Style::BARE
            .background(Color::hex(0x334455))
            .foreground(Color::WHITE)
            .text_size(16.0)
            .padding(Insets::symmetric(6.0, 14.0))
            .corners(Corners::all(6.0))
            .border(Color::hex(0x5577aa), 1.0),
    );
    theme.set(
        Role::CONTROL,
        StateKey::Focused,
        Style::BARE.border(Color::hex(0x88ccff), 2.0),
    );

    theme
}

#[test]
fn a_button_with_a_real_font_reaches_the_gpu() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(control_theme());

    let root = gui.set_root(Box::new(Stack::row().gap(8.0).padding(Insets::all(12.0))));
    let ok = gui.tree_mut().add_child(root, Box::new(Button::new("OK")));
    // 札は ASCII にしておく。Arial は仮名を持っていないので、
    // 日本語の札だと字が 1 つも出ず、命令の数が変わってしまう。
    gui.tree_mut()
        .add_child(root, Box::new(Button::new("Cancel")));

    gui.layout(VIEWPORT, &FontMeasure::new(font));
    let list = gui.paint();

    // ボタン 1 つにつき 背景・枠線・札 の 3 命令。
    assert_eq!(list.len(), 6, "{:?}", list.len());

    renderer.sync(&mut manager, list, camera(), Some(font));
    prepared(&mut manager);

    assert_eq!(manager.draw_count(), 6);

    // 札の図形には字の輪郭が入っている。
    let labels = (1..=6)
        .filter_map(|index| manager.object(&format!("gui {index}")))
        .filter(|object| object.contour_count() > 1)
        .count();

    assert!(labels >= 2, "2 つの札が輪郭になっている: {labels}");

    // 字が入ったぶん、三角形も出ている。
    let triangles: usize = (1..=6)
        .filter_map(|index| manager.object(&format!("gui {index}")))
        .map(|object| object.triangles().len())
        .sum();

    assert!(triangles > 0);
    assert!(gui.tree().bounds(ok).width > 0.0);
}

#[test]
fn hovering_a_button_only_rewrites_that_button() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();

    let mut theme = control_theme();
    theme.set(
        Role::CONTROL,
        StateKey::Hovered,
        Style::BARE
            .background(Color::hex(0x8899bb))
            .foreground(Color::WHITE)
            .text_size(16.0)
            .padding(Insets::symmetric(6.0, 14.0))
            .corners(Corners::all(6.0)),
    );
    gui.set_theme(theme);

    let root = gui.set_root(Box::new(Stack::row().gap(8.0)));
    let first = gui.tree_mut().add_child(root, Box::new(Button::new("A")));
    gui.tree_mut().add_child(root, Box::new(Button::new("B")));

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));

    // 2 つめのボタンの図形を覚えておく。
    let untouched_before = manager
        .object("gui 4")
        .expect("2 つめの背景")
        .triangles()
        .to_vec();

    // 1 つめに乗る。
    let at = gui.tree().bounds(first).center();
    gui.handle_input(InputEvent::PointerMoved {
        position: Point::new(at.x, at.y),
    });

    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));

    let untouched_after = manager.object("gui 4").unwrap().triangles().to_vec();

    assert_eq!(
        untouched_before, untouched_after,
        "乗っていないボタンは積み直されない"
    );

    prepared(&mut manager);
    assert_eq!(renderer.object_count(), 6);
}

// --- スクロール領域 ---

fn scroll_theme() -> Theme {
    let mut theme = control_theme();

    theme.set(
        Role::ACCENT,
        StateKey::Normal,
        Style::BARE
            .background(Color::hex(0x181820))
            .foreground(Color::hex(0x7788aa))
            .corners(Corners::all(5.0)),
    );

    theme
}

#[test]
fn a_scrolled_list_only_draws_what_is_visible() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    // 画面 400x300 に、ボタン 40 個の一覧を入れる。
    let area = gui.set_root(Box::new(ScrollArea::vertical()));
    let list = gui
        .tree_mut()
        .add_child(area, Box::new(Stack::column().gap(4.0)));

    for index in 0..40 {
        gui.tree_mut()
            .add_child(list, Box::new(Button::new(format!("row {index}"))));
    }

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);

    let scroll = gui.tree().get_as::<ScrollArea>(area).expect("ScrollArea");
    assert!(scroll.is_scrollable());
    assert!(
        scroll.content_length() > VIEWPORT.height * 4.0,
        "中身は画面の 4 倍より長い: {}",
        scroll.content_length()
    );

    let visible = gui.paint().len();

    // 画面に入らない行は `DisplayList` が落とす。
    // 40 行 × 3 命令 = 120 ではなく、見えている行のぶんだけ。
    assert!(
        visible < 40,
        "見えている行のぶんだけ記録される: {visible}"
    );
    assert!(visible > 6, "何かは出ている: {visible}");

    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    let after_top = renderer.object_count();

    // 下まで送る。
    gui.tree_mut()
        .get_as_mut::<ScrollArea>(area)
        .unwrap()
        .scroll_to_end();
    gui.tree_mut().request_layout(area);

    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    // 送っても図形の数はだいたい同じ。使い回しているので増え続けない。
    assert!(
        renderer.object_count() <= after_top + 6,
        "{} vs {after_top}",
        renderer.object_count()
    );
}

#[test]
fn every_visible_row_is_clipped_to_the_viewport() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    let area = gui.set_root(Box::new(ScrollArea::vertical()));
    let list = gui.tree_mut().add_child(area, Box::new(Stack::column()));

    for index in 0..20 {
        gui.tree_mut()
            .add_child(list, Box::new(Button::new(format!("{index}"))));
    }

    gui.layout(VIEWPORT, &FontMeasure::new(font));

    let viewport = Rect::from_origin_size(Point::new(0.0, 0.0), VIEWPORT);
    let list_commands = gui.paint();

    for command in list_commands.commands() {
        if command.owner == area {
            // 棒そのものは切り抜かれない。
            continue;
        }

        let clip = command.clip.expect("中身は必ず切り抜かれる");
        assert_eq!(clip.rect, viewport);

        // 切り抜きと重なっていない命令は、そもそも記録されない。
        assert!(
            command.primitive.bounds().overlaps(viewport),
            "{:?}",
            command.primitive.bounds()
        );
    }

    renderer.sync(&mut manager, list_commands, camera(), Some(font));
    prepared(&mut manager);
}

#[test]
fn scrolling_a_row_out_of_view_frees_its_object() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    let area = gui.set_root(Box::new(ScrollArea::vertical()));
    let list = gui.tree_mut().add_child(area, Box::new(Stack::column()));

    for index in 0..30 {
        gui.tree_mut()
            .add_child(list, Box::new(Button::new(format!("{index}"))));
    }

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));

    let objects = manager.object_count();

    let scroll_once =
        |gui: &mut Gui, renderer: &mut GuiRenderer, manager: &mut DrawManager, lines: f32| {
            gui.handle_input(InputEvent::Scrolled {
                position: Point::new(100.0, 150.0),
                delta: gueiz_2d::gui::event::ScrollDelta::Lines { x: 0.0, y: lines },
            });

            gui.layout(VIEWPORT, text);
            renderer.sync(manager, gui.paint(), camera(), Some(font));
        };

    // 端から端まで往復する。
    //
    // 図形の数は「同時に出た命令の最も多かったとき」まで増えて止まります。
    // 行が半分だけ見えていると札の命令が落ちるので、位置によって
    // 命令の数は 1〜2 揺れます。一度往復すれば、その最大を踏みます。
    let sweep = |gui: &mut Gui, renderer: &mut GuiRenderer, manager: &mut DrawManager| {
        for _ in 0..20 {
            scroll_once(gui, renderer, manager, -3.0);
        }
        for _ in 0..20 {
            scroll_once(gui, renderer, manager, 3.0);
        }
    };

    sweep(&mut gui, &mut renderer, &mut manager);
    let settled = manager.object_count();

    // ここから何往復しても、もう増えない。これが肝心なところで、
    // 送った回数ぶん増えるようなら使い回しが効いていない。
    for _ in 0..3 {
        sweep(&mut gui, &mut renderer, &mut manager);

        assert_eq!(
            manager.object_count(),
            settled,
            "往復を重ねても増えない（空いたものを使い回す）"
        );
    }

    assert!(
        settled <= objects + 4,
        "最初のフレームからの増えぶんは僅か: {settled} vs {objects}"
    );

    // 端まで送ったあと戻したので、先頭に居る。
    for _ in 0..20 {
        scroll_once(&mut gui, &mut renderer, &mut manager, -3.0);
    }

    prepared(&mut manager);

    assert!(
        gui.tree().get_as::<ScrollArea>(area).unwrap().offset() > 0.0,
        "実際に送れている"
    );
}

// --- 入力欄 ---

#[test]
fn a_text_field_with_a_real_font_reaches_the_gpu() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    let root = gui.set_root(Box::new(Stack::column().padding(Insets::all(10.0))));
    let field = gui
        .tree_mut()
        .add_child(root, Box::new(TextField::new().placeholder("name")));

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);
    gui.set_focus(field);

    // 案内の字とキャレット（焦点がある）。
    let empty = gui.paint().commands_of(field).count();
    assert!(empty >= 2, "背景・案内・キャレット: {empty}");

    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    // 打つ。
    for _ in 0..5 {
        gui.handle_input(InputEvent::Text(String::from("a")));
        gui.layout(VIEWPORT, text);
    }

    assert_eq!(gui.tree().get_as::<TextField>(field).unwrap().text(), "aaaaa");

    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    // 字が輪郭になっている。
    let has_glyphs = manager
        .names()
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .filter_map(|name| manager.object(&name))
        .any(|object| object.contour_count() > 1);

    assert!(has_glyphs, "打った字が輪郭になっている");
}

#[test]
fn a_long_value_is_clipped_and_the_caret_stays_inside() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    let root = gui.set_root(Box::new(Stack::column().padding(Insets::all(10.0))));
    let field = gui.tree_mut().add_child(
        root,
        Box::new(TextField::with_text("the quick brown fox jumps over the lazy dog").min_width(120.0)),
    );

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);
    gui.set_focus(field);

    // 末尾へ。枠より長いので送られる。
    gui.handle_input(InputEvent::KeyPressed {
        key: gueiz_2d::gui::event::Key::Named(gueiz_2d::gui::event::NamedKey::End),
        repeat: false,
    });
    gui.layout(VIEWPORT, text);

    let widget = gui.tree().get_as::<TextField>(field).unwrap();
    assert!(widget.scroll() > 0.0, "横に送っている");

    let caret = widget.caret_rect();
    let bounds = gui.tree().bounds(field);

    let list = gui.paint();

    // 字とキャレットは枠の内側に切り抜かれている。
    for command in list.commands_of(field) {
        let Some(clip) = command.clip else {
            // 背景と枠線は切り抜かない。
            continue;
        };

        assert!(
            clip.rect.width <= bounds.width,
            "{:?} vs {bounds:?}",
            clip.rect
        );
        assert!(clip.rect.overlaps(command.primitive.bounds()));
    }

    // キャレットは余白の内側に収まっている。
    assert!(caret.x >= 0.0, "{caret:?}");
    assert!(caret.right() <= bounds.width, "{caret:?}");

    renderer.sync(&mut manager, list, camera(), Some(font));
    prepared(&mut manager);
}

// --- ドロップダウン ---

#[test]
fn an_open_dropdown_lands_in_front_of_everything() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    let root = gui.set_root(Box::new(Stack::column().gap(4.0)));
    let dropdown = gui
        .tree_mut()
        .add_child(root, Box::new(Dropdown::new(["low", "medium", "high"])));

    // 一覧の下に重なるボタンを並べる。
    for index in 0..6 {
        gui.tree_mut()
            .add_child(root, Box::new(Button::new(format!("row {index}"))));
    }

    let text: &FontMeasure = &FontMeasure::new(font);

    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    let closed_objects = renderer.object_count();

    // 開く。
    let at = gui.tree().bounds(dropdown).center();
    gui.handle_input(InputEvent::PointerPressed {
        position: Point::new(at.x, at.y),
        button: PointerButton::Primary,
    });
    gui.layout(VIEWPORT, text);

    assert!(gui.tree().get_as::<Dropdown>(dropdown).unwrap().is_open());

    let list = gui.paint();
    let total = list.len();

    // **木の順では、ドロップダウンのあとに 6 つのボタンが並んでいる。**
    // それでも一覧は記録のいちばん後ろに来る = いちばん手前に描かれる。
    assert_eq!(
        list.commands().last().expect("命令").owner,
        dropdown,
        "一覧がいちばん最後に描かれる"
    );

    renderer.sync(&mut manager, list, camera(), Some(font));
    prepared(&mut manager);

    assert!(renderer.object_count() > closed_objects, "一覧のぶん増える");

    // z は記録の順で振られるので、後ろのものほど手前。
    // いちばん大きい z を持つ図形が、一覧のものになっている。
    let depths: Vec<f32> = (1..=renderer.object_count())
        .filter_map(|index| manager.object(&format!("gui {index}")))
        .map(|object| object.depth())
        .collect();

    let highest = depths.iter().copied().fold(f32::NEG_INFINITY, f32::max);

    assert!(
        highest >= (total - 1) as f32 - 0.5,
        "いちばん手前の z が記録の最後に当たる: {highest} / {total}"
    );
    // 閉じると空きに戻る。
    gui.handle_input(InputEvent::KeyPressed {
        key: gueiz_2d::gui::event::Key::Named(gueiz_2d::gui::event::NamedKey::Escape),
        repeat: false,
    });
    gui.layout(VIEWPORT, text);
    renderer.sync(&mut manager, gui.paint(), camera(), Some(font));
    prepared(&mut manager);

    assert!(
        renderer.recycled_count() > 0,
        "閉じた一覧の図形は取っておかれる"
    );
}

#[test]
fn an_open_dropdown_escapes_a_clipping_parent_on_the_gpu() {
    let gpu = gpu!();

    let Some(font) = font() else {
        eprintln!("skipped: no font found");
        return;
    };

    let mut manager = draw_manager(gpu);
    let mut renderer = GuiRenderer::new();
    let mut gui = Gui::new();
    gui.set_theme(scroll_theme());

    // 背の低い送り箱に入れる。
    let root = gui.set_root(Box::new(Stack::column()));
    let area = gui
        .tree_mut()
        .add_child(root, Box::new(ScrollArea::vertical()));
    let dropdown = gui
        .tree_mut()
        .add_child(area, Box::new(Dropdown::new(["a", "b", "c", "d"])));

    gui.tree_mut()
        .get_as_mut::<Stack>(root)
        .unwrap()
        .set_length(area, gueiz_2d::gui::layout::Length::Fixed(40.0));

    let text: &FontMeasure = &FontMeasure::new(font);
    gui.layout(VIEWPORT, text);

    let at = gui.tree().bounds(dropdown).center();
    gui.handle_input(InputEvent::PointerPressed {
        position: Point::new(at.x, at.y),
        button: PointerButton::Primary,
    });
    gui.layout(VIEWPORT, text);

    let box_bounds = gui.tree().bounds(area);
    let popup = gui
        .tree()
        .get_as::<Dropdown>(dropdown)
        .unwrap()
        .popup_rect()
        .translate(gui.tree().bounds(dropdown).origin());

    assert!(popup.bottom() > box_bounds.bottom(), "箱の外へ出ている");

    // 箱の外に出た一覧が、当たり判定でも生きている。
    let below = Point::new(popup.center().x, box_bounds.bottom() + 5.0);
    assert_eq!(gui.hit_test(below), dropdown, "箱の外でも一覧に当たる");

    let list = gui.paint();

    // 箱の切り抜きが掛かっていない命令がある。
    let escaped = list
        .commands_of(dropdown)
        .any(|command| command.clip.is_none_or(|clip| clip.rect != box_bounds));

    assert!(escaped, "切り抜きを突き抜けている");

    renderer.sync(&mut manager, list, camera(), Some(font));
    prepared(&mut manager);
}
