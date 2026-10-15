//! 窓に置いた [`InputTextarea`] に、押す・打つ・動く・送るを流して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test input_textarea_gpu -- --nocapture
//! ```

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Duration;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::input_text::{InputTextFont, InputTextTheme};
use gueiz_2d::gui::input_textarea::InputTextarea;
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{EditKey, KeyModifiers};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

/// 欄は (105, 140) から 250 x 120、余白 8。字を置くのは (113, 148) から 234 x 104。
const BOX: [f32; 4] = [105.0, 140.0, 355.0, 260.0];
const INNER_LEFT: f32 = 113.0;
const INNER_TOP: f32 = 148.0;

struct Gpu {
    device: wgpu::Device,
    queue: wgpu::Queue,
}

fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();

    GPU.get_or_init(|| {
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).ok()?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("input textarea gpu test"),
            required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
            ..Default::default()
        }))
        .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

fn color(value: f32) -> ThemeColor {
    ThemeColor { r: value, g: value, b: value, a: 1.0 }
}

fn load_font(resources: &mut Resources) -> Option<FontHandle> {
    [
        "C:/Windows/Fonts/YuGothM.ttc",
        "C:/Windows/Fonts/arial.ttf",
        "/System/Library/Fonts/Avenir Next.ttc",
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
    ]
    .into_iter()
    .find_map(|path| resources.load_font_file("font", path).ok())
}

struct Scene {
    draw_manager: DrawManager,
    resources: Resources,
    window_frame: WindowFrame,
    text: Rc<RefCell<String>>,
    submitted: Rc<RefCell<Vec<String>>>,
}

fn scene() -> Option<Scene> {
    scene_with(|area| area)
}

/// 欄に手を加えて組む（折り返さないなど）。
fn scene_with(customize: impl FnOnce(InputTextarea) -> InputTextarea) -> Option<Scene> {
    let Some(gpu) = gpu() else {
        eprintln!("skipped: no gpu adapter");
        return None;
    };
    let mut resources = Resources::new();
    let Some(font) = load_font(&mut resources) else {
        eprintln!("skipped: no font");
        return None;
    };

    let draw_manager = DrawManager::new(&gpu.device, &gpu.queue, TextureFormat::Bgra8UnormSrgb, &DrawManagerDescriptor::default())
        .expect("DrawManager を作れなかった");

    let text = Rc::new(RefCell::new(String::new()));
    let submitted = Rc::new(RefCell::new(Vec::new()));

    let mut window_frame = WindowFrame::new(
        "Window".to_string(),
        ScaleMode::Stretch,
        "Window".to_string(),
        Quad { quad_type: QuadType::Normal, x: 100.0, y: 100.0, width: 400.0, height: 300.0 },
        30.0,
        Gap { x: 5.0, y: 5.0 },
        WindowFont { base_font: font, base_font_size: 13.0 },
        WindowTheme {
            frame_fill_color: color(0.1),
            frame_outline_color: color(0.5),
            title_bar_fill_color: color(0.1),
            title_bar_slice_line_color: color(0.2),
            text_title_color: color(1.0),
        },
    );

    let changed = text.clone();
    let submit = submitted.clone();
    window_frame.add_window_item(Box::new(customize(
        InputTextarea::new(
            "Area".to_string(),
            "placeholder".to_string(),
            250.0,
            120.0,
            8.0,
            QuadType::Rounded { radius: 4.0 },
            Gap { x: 0.0, y: 5.0 },
            InputTextFont { base_font: font, base_font_size: 14.0 },
            InputTextTheme {
                fill_color: color(0.05),
                outline_color: color(0.3),
                focus_outline_color: color(0.6),
                text_color: color(1.0),
                placeholder_color: color(0.5),
                caret_color: color(1.0),
                selection_color: color(0.3),
            },
        )
        .on_change(move |text| *changed.borrow_mut() = text.to_string())
        .on_submit(move |text| submit.borrow_mut().push(text.to_string())),
    )));

    let mut scene = Scene { draw_manager, resources, window_frame, text, submitted };
    scene.window_frame.create_object(&mut scene.draw_manager, &scene.resources, SurfaceSize::new(WIDTH, HEIGHT), [WIDTH as f32, HEIGHT as f32]);

    Some(scene)
}

impl Scene {
    fn click(&mut self, x: f32, y: f32) {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y);
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
    }

    fn type_text(&mut self, text: &str) {
        self.window_frame.text_input(&mut self.draw_manager, &self.resources, text);
    }

    fn key(&mut self, key: EditKey) -> bool {
        self.window_frame.key_pressed(&mut self.draw_manager, &self.resources, key)
    }

    fn modifiers(&mut self, shift: bool, word: bool) {
        self.window_frame.modifiers_changed(KeyModifiers { shift, word });
    }

    fn update(&mut self) {
        self.window_frame.update(&mut self.draw_manager, &self.resources);
    }

    fn text(&self) -> String {
        self.text.borrow().clone()
    }

    fn wheel(&mut self, lines: [f32; 2]) -> bool {
        let taken = self.window_frame.mouse_wheel(&mut self.draw_manager, 200.0, 200.0, lines);
        self.update();
        taken
    }

    /// 図形が描かれているなら、その外接矩形。透明なら `None`。
    fn drawn(&self, name: &str) -> Option<[f32; 4]> {
        let object = self.draw_manager.object(name).expect("図形がある");
        let vertices = object.vertices();
        vertices.iter().any(|vertex| vertex.a > 0.0).then(|| {
            let xs = vertices.iter().map(|vertex| vertex.x);
            let ys = vertices.iter().map(|vertex| vertex.y);
            [
                xs.clone().fold(f32::INFINITY, f32::min),
                ys.clone().fold(f32::INFINITY, f32::min),
                xs.fold(f32::NEG_INFINITY, f32::max),
                ys.fold(f32::NEG_INFINITY, f32::max),
            ]
        })
    }

    /// キャレットの `[x, y, 幅, 高さ]`（画面の画素。窓は動かしていないので組んだ座標と同じ）。
    fn caret(&self) -> [f32; 4] {
        self.window_frame.ime_cursor_area(&self.draw_manager).expect("フォーカスがある")
    }

    /// 描いているキャレットの外接矩形。消えていれば `None`。
    fn drawn_caret(&self) -> Option<[f32; 4]> {
        let caret = self.draw_manager.object("Window-Area-0-caret").expect("キャレットの図形がある");
        let vertices = caret.vertices();
        vertices.iter().any(|vertex| vertex.a > 0.0).then(|| {
            let xs = vertices.iter().map(|vertex| vertex.x);
            let ys = vertices.iter().map(|vertex| vertex.y);
            [
                xs.clone().fold(f32::INFINITY, f32::min),
                ys.clone().fold(f32::INFINITY, f32::min),
                xs.fold(f32::NEG_INFINITY, f32::max),
                ys.fold(f32::NEG_INFINITY, f32::max),
            ]
        })
    }

    /// 1 行の高さ。2 行打ってキャレットの上下の差から測る。
    fn line_height(&mut self) -> f32 {
        let first = self.caret()[1];
        self.key(EditKey::Enter);
        let second = self.caret()[1];
        self.key(EditKey::Backspace);
        second - first
    }
}

/// 描いているキャレットが、字を削る範囲（余白の半分の内側）にあるか。消えていてもよい。
fn caret_inside(caret: Option<[f32; 4]>) -> bool {
    caret.is_none_or(|[left, top, right, bottom]| left >= BOX[0] + 4.0 && right <= BOX[2] - 4.0 && top >= BOX[1] + 4.0 && bottom <= BOX[3] - 4.0)
}

#[test]
fn enter_makes_new_lines_and_up_down_keep_the_column() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);

    scene.type_text("abc");
    scene.key(EditKey::Enter);
    scene.type_text("defgh");
    assert_eq!(scene.text(), "abc\ndefgh");

    // ↑ で上の行へ。上の行は短いので末尾に止まる。
    scene.key(EditKey::Up);
    scene.type_text("X");
    assert_eq!(scene.text(), "abcX\ndefgh");

    // ↓ で戻ると、↑ の前の横の位置（defgh の末尾）を覚えている。打つと横の位置は忘れる。
    scene.key(EditKey::Up);
    scene.key(EditKey::Down);
    scene.type_text("Y");
    assert_eq!(scene.text(), "abcX\ndefgYh", "打ったので、↑ の前の位置ではなく X の真下へ");

    // 1 行目でさらに ↑ なら先頭、最後の行でさらに ↓ なら末尾。
    scene.key(EditKey::Up);
    scene.key(EditKey::Up);
    scene.type_text("<");
    scene.key(EditKey::Down);
    scene.key(EditKey::Down);
    scene.type_text(">");
    assert_eq!(scene.text(), "<abcX\ndefgYh>");

    // Ctrl+Enter は改行せず、渡す。
    scene.modifiers(false, true);
    scene.key(EditKey::Enter);
    scene.modifiers(false, false);
    assert_eq!(*scene.submitted.borrow(), vec!["<abcX\ndefgYh>".to_string()]);
    assert_eq!(scene.text(), "<abcX\ndefgYh>");
}

#[test]
fn long_lines_wrap_and_home_end_follow_visual_rows() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let line_height = scene.line_height();

    let text = "lorem ipsum dolor sit amet ".repeat(4);
    scene.type_text(&text);

    // 折り返したので、キャレットは 1 行目より下にいる。
    let caret = scene.caret();
    assert!(caret[1] > INNER_TOP + line_height, "折り返して下の行にいる: {caret:?}");
    assert!(caret[0] < BOX[2], "欄の幅を超えない: {caret:?}");

    // Home は見た目の行の先頭。そこは文字の先頭ではない。
    scene.key(EditKey::Home);
    scene.type_text("|");
    assert_ne!(scene.text().find('|'), Some(0), "見た目の行の先頭");

    // Ctrl+Home は全体の先頭。
    scene.modifiers(false, true);
    scene.key(EditKey::Home);
    scene.modifiers(false, false);
    scene.type_text("^");
    assert!(scene.text().starts_with('^'));
}

#[test]
fn clicking_and_triple_clicking_pick_lines() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let line_height = scene.line_height();
    scene.type_text("first line\nsecond line\nthird");

    // 2 行目のまん中あたりを押して打つ。
    let row = |index: f32| INNER_TOP + line_height * (index + 0.5);
    scene.click(INNER_LEFT + 1.0, row(1.0));
    scene.type_text("*");
    assert_eq!(scene.text(), "first line\n*second line\nthird");

    // 3 回続けて押すと、その 1 行を選ぶ。
    sleep(Duration::from_millis(600));
    for _ in 0..3 {
        scene.click(INNER_LEFT + 20.0, row(2.0));
    }
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("third"));
}

#[test]
fn wheel_scrolls_and_typing_follows_the_caret() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let lines: Vec<String> = (0..30).map(|index| format!("line {index}")).collect();
    scene.type_text(&lines.join("\n"));

    // 打ち終わりのキャレットは欄の中に見えている。
    scene.update();
    assert!(scene.drawn_caret().is_some(), "末尾まで送って見えている");
    assert!(caret_inside(scene.drawn_caret()));

    // ホイールで上へ送りきると、一番上の行が見える。そこを押すと 1 行目に入る。
    assert!(scene.wheel([0.0, 100.0]));
    assert!(caret_inside(scene.drawn_caret()), "キャレットが外なら描かない");
    scene.click(INNER_LEFT + 1.0, INNER_TOP + 2.0);
    scene.type_text("@");
    assert!(scene.text().starts_with("@line 0"), "{:?}", &scene.text()[..10]);
}

#[test]
fn dragging_below_scrolls_down_while_selecting() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let lines: Vec<String> = (0..30).map(|index| format!("line {index}")).collect();
    let text = lines.join("\n");
    scene.type_text(&text);

    // 先頭へ戻ってから、押したまま欄の下の外へ。
    scene.modifiers(false, true);
    scene.key(EditKey::Home);
    scene.modifiers(false, false);
    scene.update();
    scene.window_frame.mouse_left_pressed(&mut scene.draw_manager, INNER_LEFT + 1.0, INNER_TOP + 2.0);
    // 行の右側にいるので、最後の行の末尾まで選ぶ。
    scene.window_frame.mouse_moved(&mut scene.draw_manager, BOX[2] - 20.0, BOX[3] + 80.0);

    for _ in 0..240 {
        scene.update();
        assert!(caret_inside(scene.drawn_caret()), "送っているあいだもキャレットは欄の中");
        if scene.window_frame.selected_text().is_some_and(|selected| selected.contains("line 29")) {
            break;
        }
        sleep(Duration::from_millis(16));
    }
    let selected = scene.window_frame.selected_text().unwrap_or_default();
    assert!(selected.starts_with("line 0") && selected.contains("line 29"), "下まで送って選ぶ: {} 字", selected.len());

    scene.window_frame.mouse_left_released(&mut scene.draw_manager, BOX[2] - 20.0, BOX[3] + 80.0);
}

#[test]
#[ignore = "速さを測るだけ。cargo test -p gueiz-2d --test input_textarea_gpu --release measure -- --ignored --nocapture"]
fn measure_long_textarea() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let lines: Vec<String> = (0..300).map(|index| format!("line {index} lorem ipsum dolor sit amet consectetur")).collect();
    scene.type_text(&lines.join("\n"));

    let start = std::time::Instant::now();
    for _ in 0..200 {
        scene.key(EditKey::Up);
        scene.update();
    }
    eprintln!("200 rows up through {} chars: {:?}", scene.text().len(), start.elapsed());

    let start = std::time::Instant::now();
    for _ in 0..200 {
        scene.type_text("x");
        scene.update();
    }
    eprintln!("200 characters typed: {:?}", start.elapsed());

    let start = std::time::Instant::now();
    for _ in 0..200 {
        scene.wheel([0.0, -1.0]);
    }
    eprintln!("200 wheel steps: {:?}", start.elapsed());
}

/// 30 行の文字。
fn thirty_lines() -> String {
    (0..30).map(|index| format!("line {index}")).collect::<Vec<_>>().join("\n")
}

#[test]
fn page_up_and_down_move_by_a_page() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    let line_height = scene.line_height();
    scene.type_text(&thirty_lines());

    // 欄に見えるのは 104 / 行の高さ 行。1 ページはそこから 1 行引いたぶん。
    let page = ((104.0 / line_height).floor() as usize - 1).max(1);

    // 末尾（line 29 の後ろ）から 1 ページ上へ。行の頭へ戻ってから印を打つ。
    scene.key(EditKey::PageUp);
    scene.key(EditKey::Home);
    scene.type_text("@");
    let marked = scene.text().lines().position(|line| line.starts_with('@')).expect("印がある");
    assert_eq!(marked, 29 - page, "1 ページ（{page} 行）上");

    // 1 ページ下へ戻る。
    scene.key(EditKey::PageDown);
    scene.key(EditKey::Home);
    scene.type_text("#");
    let marked = scene.text().lines().position(|line| line.starts_with('#')).expect("印がある");
    assert_eq!(marked, 29);

    // Shift を押しながらなら選ぶ。キャレットは欄の中に見えている。
    scene.modifiers(true, false);
    scene.key(EditKey::PageUp);
    scene.modifiers(false, false);
    assert_eq!(scene.window_frame.selected_text().map(|text| text.lines().count()), Some(page + 1));
    scene.update();
    assert!(caret_inside(scene.drawn_caret()));
}

#[test]
fn tab_inserts_spaces() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    scene.key(EditKey::Tab);
    scene.type_text("x");
    assert_eq!(scene.text(), "    x");

    let Some(mut scene) = scene_with(|area| area.tab_width(2)) else { return };
    scene.click(150.0, 160.0);
    scene.key(EditKey::Tab);
    assert_eq!(scene.text(), "  ");
}

#[test]
fn without_wrapping_long_lines_scroll_sideways() {
    let Some(mut scene) = scene_with(|area| area.wrap(false)) else { return };
    scene.click(150.0, 160.0);
    let text = "lorem ipsum dolor sit amet ".repeat(4);
    scene.type_text(&text);
    scene.update();

    // 折り返さないので 1 行のまま。キャレットは横に送って欄の中に見えている。
    let caret = scene.caret();
    assert!(caret[1] < INNER_TOP + 30.0, "1 行目のまま: {caret:?}");
    assert!(scene.drawn_caret().is_some() && caret_inside(scene.drawn_caret()), "{:?}", scene.drawn_caret());

    // Home で行の頭へ戻ると、横の送りも戻る。
    scene.key(EditKey::Home);
    scene.update();
    assert!((scene.caret()[0] - INNER_LEFT).abs() < 2.0, "{:?}", scene.caret());

    // 横のホイールで右へ送ってから、欄の左端を押すと、行の頭ではないところに入る。
    assert!(scene.wheel([-10.0, 0.0]));
    scene.click(INNER_LEFT + 1.0, INNER_TOP + 5.0);
    scene.type_text("|");
    assert!(scene.text().find('|').is_some_and(|index| index > 0), "横に送ってある");
}

#[test]
fn scrollbar_appears_and_can_be_dragged_or_paged() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 160.0);
    scene.type_text("short");
    scene.update();
    assert_eq!(scene.drawn("Window-Area-0-thumb"), None, "収まっていればスクロールバーは出ない");

    scene.type_text(&format!("\n{}", thirty_lines()));
    scene.update();
    let thumb = scene.drawn("Window-Area-0-thumb").expect("あふれたら出る");
    assert!(thumb[2] <= BOX[2] && thumb[0] >= BOX[2] - 10.0, "右端に: {thumb:?}");
    assert!(thumb[3] > BOX[3] - 10.0, "末尾まで送ってあるので、つまみは下: {thumb:?}");
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, thumb[0] + 3.0, thumb[1] + 3.0), gueiz_2d::gui::window_item::CursorShape::Default);

    // つまみを掴んで一番上まで引くと、1 行目が見える。
    let grab = [thumb[0] + 3.0, (thumb[1] + thumb[3]) / 2.0];
    scene.window_frame.mouse_left_pressed(&mut scene.draw_manager, grab[0], grab[1]);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, grab[0], BOX[1] - 200.0);
    scene.window_frame.mouse_left_released(&mut scene.draw_manager, grab[0], BOX[1] - 200.0);
    scene.update();
    let top_thumb = scene.drawn("Window-Area-0-thumb").expect("出ている");
    assert!(top_thumb[1] < BOX[1] + 5.0, "つまみは上: {top_thumb:?}");
    scene.click(INNER_LEFT + 1.0, INNER_TOP + 2.0);
    scene.type_text("^");
    assert!(scene.text().starts_with("^short"));

    // つまみの下の溝を押すと、1 ページ下へ送る。
    let thumb = scene.drawn("Window-Area-0-thumb").expect("出ている");
    scene.window_frame.mouse_left_pressed(&mut scene.draw_manager, thumb[0] + 3.0, BOX[3] - 4.0);
    scene.window_frame.mouse_left_released(&mut scene.draw_manager, thumb[0] + 3.0, BOX[3] - 4.0);
    scene.update();
    let paged = scene.drawn("Window-Area-0-thumb").expect("出ている");
    assert!(paged[1] > thumb[1], "下へ送った: {thumb:?} -> {paged:?}");
}
