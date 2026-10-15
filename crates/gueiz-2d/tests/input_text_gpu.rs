//! 窓に置いた [`InputText`] に、押す・打つ・消す・変換するを流して確かめる。
//!
//! 当たり判定（[`DrawManager::pick`]）を通すので GPU が要ります。
//! GPU か書体が無ければ何もせずに終わります。**飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test input_text_gpu -- --nocapture
//! ```

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::OnceLock;
use std::thread::sleep;
use std::time::Duration;

use gueiz_2d::camera::ScaleMode;
use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::gui::clipboard::MemoryClipboard;
use gueiz_2d::gui::context_menu::{ContextMenuDisabled, ContextMenuFont, ContextMenuLabel, ContextMenuLabels, ContextMenuLayout, ContextMenuSlicer, ContextMenuStyle, ContextMenuTheme, INPUT_TEXT_CONTEXT_MENU};
use gueiz_2d::gui::input_text::{InputText, InputTextFont, InputTextTheme};
use gueiz_2d::gui::window_frame::{Gap, Quad, QuadType, ThemeColor, WindowFont, WindowFrame, WindowTheme};
use gueiz_2d::gui::window_item::{CursorShape, EditKey, KeyModifiers};
use gueiz_2d::renderer::SurfaceSize;
use gueiz_2d::resource::{FontHandle, Resources};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const WIDTH: u32 = 640;
const HEIGHT: u32 = 480;

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
            label: Some("input text gpu test"),
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

/// 窓 1 つに欄を 1 つ置いたもの。
struct Scene {
    draw_manager: DrawManager,
    resources: Resources,
    window_frame: WindowFrame,
    /// 欄の文字。`on_change` で写す。
    text: Rc<RefCell<String>>,
    submitted: Rc<RefCell<Vec<String>>>,
}

/// 書体を探す。テストを動かす機械にあるものを使う。
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

fn scene() -> Option<Scene> {
    scene_with(|input| input)
}

/// 欄に手を加えて組む（パスワード欄にするなど）。
fn scene_with(customize: impl FnOnce(InputText) -> InputText) -> Option<Scene> {
    scene_with_frame(customize, |_, _| {})
}

/// 欄と窓の両方に手を加えて組む（メニューの見た目を変えるなど）。窓には書体も渡す。
fn scene_with_frame(
    customize: impl FnOnce(InputText) -> InputText,
    customize_frame: impl FnOnce(&mut WindowFrame, FontHandle),
) -> Option<Scene> {
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

    // 窓は (100, 100) から。帯 30・余白 5 なので、部品は (105, 135) から積まれる。
    // 欄はさらに 5 下げるので、(105, 140) から 200 x 28。
    let mut window_frame = WindowFrame::new(
        "Window".to_string(),
        ScaleMode::Stretch,
        "Window".to_string(),
        Quad { quad_type: QuadType::Normal, x: 100.0, y: 100.0, width: 400.0, height: 200.0 },
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
        InputText::new(
            "Input".to_string(),
            "placeholder".to_string(),
            200.0,
            28.0,
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

    customize_frame(&mut window_frame, font);

    let mut scene = Scene { draw_manager, resources, window_frame, text, submitted };
    let surface_size = SurfaceSize::new(WIDTH, HEIGHT);
    scene.window_frame.create_object(&mut scene.draw_manager, &scene.resources, surface_size, [WIDTH as f32, HEIGHT as f32]);

    Some(scene)
}

impl Scene {
    fn click(&mut self, x: f32, y: f32) -> bool {
        let taken = self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y);
        self.window_frame.mouse_left_released(&mut self.draw_manager, x, y);
        taken
    }

    fn type_text(&mut self, text: &str) -> bool {
        self.window_frame.text_input(&mut self.draw_manager, &self.resources, text)
    }

    fn key(&mut self, key: EditKey) -> bool {
        self.window_frame.key_pressed(&mut self.draw_manager, &self.resources, key)
    }

    /// メニューで押された項目の関数を走らせる。走ったら `true`。
    fn run_menu(&mut self, clipboard: &mut MemoryClipboard) -> bool {
        self.window_frame.run_context_menu_event(&mut self.draw_manager, &self.resources, clipboard)
    }

    fn right_click(&mut self, x: f32, y: f32) -> bool {
        self.window_frame.mouse_right_pressed(&mut self.draw_manager, x, y)
    }

    fn press(&mut self, x: f32, y: f32) -> bool {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, x, y)
    }

    fn modifiers(&mut self, shift: bool, word: bool) {
        self.window_frame.modifiers_changed(KeyModifiers { shift, word });
    }

    /// 欄の中の、先頭からの字の位置 `x` を押す（余白 8 の内側）。
    fn press_at(&mut self, x: f32) {
        self.window_frame.mouse_left_pressed(&mut self.draw_manager, 105.0 + 8.0 + x, 154.0);
    }

    fn release_at(&mut self, x: f32) {
        self.window_frame.mouse_left_released(&mut self.draw_manager, 105.0 + 8.0 + x, 154.0);
    }

    fn text(&self) -> String {
        self.text.borrow().clone()
    }

    /// 描く前の 1 回ぶん。時間で動くもの（点滅・自動スクロール）が進む。
    fn update(&mut self) {
        self.window_frame.update(&mut self.draw_manager, &self.resources);
    }

    /// 描いているキャレットの左右（組んだときの座標）。消えていれば `None`。
    fn drawn_caret(&self) -> Option<[f32; 2]> {
        let caret = self.draw_manager.object("Window-Input-0-caret").expect("キャレットの図形がある");
        let vertices = caret.vertices();
        vertices.iter().any(|vertex| vertex.a > 0.0).then(|| {
            let left = vertices.iter().map(|vertex| vertex.x).fold(f32::INFINITY, f32::min);
            let right = vertices.iter().map(|vertex| vertex.x).fold(f32::NEG_INFINITY, f32::max);
            [left, right]
        })
    }

    /// キャレットを点けているか。キャレットの図形の色で見る。
    fn caret_shown(&self) -> bool {
        let caret = self.draw_manager.object("Window-Input-0-caret").expect("キャレットの図形がある");
        caret.vertices().iter().any(|vertex| vertex.a > 0.0)
    }
}

#[test]
fn typing_needs_focus_and_click_outside_releases_it() {
    let Some(mut scene) = scene() else { return };

    // フォーカスが無ければ届かない。
    assert!(!scene.type_text("x"));
    assert_eq!(scene.text(), "");

    assert!(scene.click(150.0, 154.0), "欄を押したら窓が取る");
    assert!(scene.window_frame.has_focus());

    assert!(scene.type_text("abc"));
    assert_eq!(scene.text(), "abc");

    // 窓の外を押すと手放す。
    assert!(!scene.click(20.0, 20.0));
    assert!(!scene.window_frame.has_focus());
    assert!(!scene.type_text("d"));
    assert_eq!(scene.text(), "abc");
}

#[test]
fn editing_keys_move_and_delete_by_character() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);

    scene.type_text("日本語");
    scene.key(EditKey::Left);
    scene.key(EditKey::Backspace);
    assert_eq!(scene.text(), "日語", "1 字ずつ消える（バイトではなく）");

    scene.key(EditKey::Home);
    scene.key(EditKey::Delete);
    assert_eq!(scene.text(), "語");

    scene.key(EditKey::End);
    scene.type_text("x\ty\n");
    assert_eq!(scene.text(), "語xy", "制御文字は入らない");

    scene.key(EditKey::Enter);
    assert_eq!(*scene.submitted.borrow(), vec!["語xy".to_string()]);

    // 端では何も変わらない。
    scene.key(EditKey::End);
    assert!(!scene.key(EditKey::Right));
    assert!(!scene.key(EditKey::Delete));
}

#[test]
fn preedit_is_shown_but_not_committed_until_commit() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("ab");
    scene.key(EditKey::Left);

    assert!(scene.window_frame.ime_preedit(&mut scene.draw_manager, &scene.resources, "にほん", Some(9)));
    assert_eq!(scene.text(), "ab", "変換中は文字に入らない");

    // 変換中のキーは IME のもの。
    assert!(!scene.key(EditKey::Backspace));

    scene.window_frame.ime_preedit(&mut scene.draw_manager, &scene.resources, "", None);
    scene.type_text("日本");
    assert_eq!(scene.text(), "a日本b", "確定した文字は字を打つ位置に入る");
}

#[test]
fn clicking_places_the_cursor_near_the_pointer() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("abcdef");

    // 欄の左端の内側（余白 8 の少し右）を押すと先頭に。
    scene.click(114.0, 154.0);
    scene.type_text("X");
    assert_eq!(scene.text(), "Xabcdef");
}

#[test]
fn long_text_scrolls_and_keeps_the_caret_inside() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text(&"long text ".repeat(10));

    let [x, _, width, _] = scene.window_frame.ime_cursor_area(&scene.draw_manager).expect("フォーカスがある");
    assert!(x >= 105.0 && x + width <= 305.0, "字を打つ位置は欄の中: {x}");
}

#[test]
fn cursor_shape_follows_what_is_under_the_pointer() {
    let Some(mut scene) = scene() else { return };
    let shape = |scene: &Scene, x: f32, y: f32| scene.window_frame.cursor_shape(&scene.draw_manager, x, y);

    assert_eq!(shape(&scene, 150.0, 154.0), CursorShape::Text, "欄の上");
    assert_eq!(shape(&scene, 499.0, 200.0), CursorShape::ResizeHorizontal, "右の縁");
    assert_eq!(shape(&scene, 499.0, 299.0), CursorShape::ResizeNwse, "右下の角");
    assert_eq!(shape(&scene, 200.0, 115.0), CursorShape::Default, "帯の上");
    assert_eq!(shape(&scene, 20.0, 20.0), CursorShape::Default, "窓の外");

    // 掴んでいるあいだは、指がどこにあっても動かす形。
    scene.window_frame.mouse_left_pressed(&mut scene.draw_manager, 200.0, 115.0);
    assert_eq!(shape(&scene, 20.0, 20.0), CursorShape::Move);
    scene.window_frame.mouse_left_released(&mut scene.draw_manager, 200.0, 115.0);
    assert_eq!(shape(&scene, 20.0, 20.0), CursorShape::Default);
}

#[test]
fn shift_and_word_modifiers_reach_the_input() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("hello brave world");

    // Ctrl+Backspace で 1 単語。
    scene.modifiers(false, true);
    scene.key(EditKey::Backspace);
    assert_eq!(scene.text(), "hello brave ");

    // Shift+Ctrl+← で 1 単語選んで、打てば置き換わる。
    scene.modifiers(true, true);
    scene.key(EditKey::Left);
    scene.modifiers(false, false);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("brave "));
    scene.type_text("new ");
    assert_eq!(scene.text(), "hello new ");

    scene.key(EditKey::Undo);
    assert_eq!(scene.text(), "hello brave ");
    scene.key(EditKey::Redo);
    assert_eq!(scene.text(), "hello new ");
}

#[test]
fn cut_and_paste_go_through_the_window() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("copy me");

    assert_eq!(scene.window_frame.selected_text(), None, "選んでいなければコピーするものは無い");

    scene.key(EditKey::SelectAll);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("copy me"));

    let mut clipboard = MemoryClipboard::new();
    assert!(scene.window_frame.cut_to_clipboard(&mut scene.draw_manager, &scene.resources, &mut clipboard));
    assert_eq!(clipboard.text(), Some("copy me"));
    assert_eq!(scene.text(), "");

    assert!(scene.window_frame.paste_from_clipboard(&mut scene.draw_manager, &scene.resources, &mut clipboard));
    assert_eq!(scene.text(), "copy me");

    assert!(scene.window_frame.paste(&mut scene.draw_manager, &scene.resources, "pasted\ntext"));
    assert_eq!(scene.text(), "copy mepastedtext", "改行は入らない");
}

#[test]
fn dragging_and_multi_clicks_select_with_the_mouse() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("alpha beta");

    // 末尾の右から、欄の左の外まで引くと、すべて選ぶ。
    scene.press_at(150.0);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 50.0, 154.0);
    scene.release_at(150.0);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("alpha beta"));

    // 2 回続けて押すと単語。
    scene.press_at(2.0);
    scene.release_at(2.0);
    scene.press_at(2.0);
    scene.release_at(2.0);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("alpha"));

    // 3 回目ですべて。
    scene.press_at(2.0);
    scene.release_at(2.0);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("alpha beta"));
}

#[test]
fn shift_click_extends_from_the_caret() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text("abcdef");
    scene.key(EditKey::Home);

    // 文字の右の空いたところ（欄の中）。前に押した場所から離れているので、続けて押したとはみなされない。
    scene.modifiers(true, false);
    scene.press_at(150.0);
    scene.release_at(150.0);
    scene.modifiers(false, false);

    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("abcdef"));
}

#[test]
fn caret_blinks_only_while_focused() {
    let Some(mut scene) = scene() else { return };

    scene.update();
    assert_eq!(scene.window_frame.next_update(), None, "フォーカスが無ければ起こさなくてよい");

    scene.click(150.0, 154.0);
    scene.update();
    assert!(scene.caret_shown(), "押した直後は点いている");
    assert!(scene.window_frame.next_update().is_some(), "点滅のために起こしてほしい");

    sleep(Duration::from_millis(600));
    scene.update();
    assert!(!scene.caret_shown(), "しばらくすると消える");

    // 打つと点けた状態からやり直す。
    scene.type_text("a");
    scene.update();
    assert!(scene.caret_shown());

    scene.click(20.0, 20.0);
    scene.update();
    assert!(!scene.caret_shown());
    assert_eq!(scene.window_frame.next_update(), None);
}

#[test]
fn dragging_outside_scrolls_until_the_end_is_selected() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    let text = "long text ".repeat(10);
    scene.type_text(&text);
    scene.key(EditKey::Home);

    // 先頭を押して、欄の右の外へ引いたまま待つ。
    scene.press_at(0.0);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 400.0, 154.0);

    for _ in 0..120 {
        scene.update();
        if scene.window_frame.selected_text().as_deref() == Some(text.as_str()) {
            break;
        }
        sleep(Duration::from_millis(16));
    }
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some(text.as_str()), "末尾まで送って選ぶ");

    scene.release_at(0.0);
    scene.update();
    assert!(scene.window_frame.next_update().is_some(), "放したあとは点滅だけ");
}

#[test]
fn password_hides_text_from_copy_and_ime() {
    let Some(mut scene) = scene_with(|input| input.password()) else { return };
    scene.click(150.0, 154.0);
    scene.type_text("secret");
    assert_eq!(scene.text(), "secret", "中身はそのまま");

    assert!(scene.window_frame.has_focus());
    assert!(!scene.window_frame.wants_ime(), "パスワード欄では IME を使わない");

    scene.key(EditKey::SelectAll);
    assert_eq!(scene.window_frame.selected_text(), None, "コピーさせない");
    let mut clipboard = MemoryClipboard::new();
    assert!(!scene.window_frame.copy_to_clipboard(&mut clipboard));
    assert!(!scene.window_frame.cut_to_clipboard(&mut scene.draw_manager, &scene.resources, &mut clipboard), "切り取らせない");
    assert_eq!(clipboard.text(), None);
    assert_eq!(scene.text(), "secret");

    // 伏せていても、押した位置と 1 字ずつの消し方は字の数どおり。
    scene.click(114.0, 154.0);
    scene.key(EditKey::Delete);
    assert_eq!(scene.text(), "ecret");
}

#[test]
fn right_click_menu_runs_commands() {
    let Some(mut scene) = scene() else { return };
    let mut clipboard = MemoryClipboard::new();
    scene.click(150.0, 154.0);
    scene.type_text("hello");

    // いつものメニュー（Cut・Copy・Paste・Select All）。押した位置から、上の余白 4、1 項目 25。
    let row = |index: f32| 154.0 + 4.0 + 25.0 * index + 12.0;

    // 何も選んでいなければ、切り取りは選べない（押しても何も起きず、閉じない）。
    assert!(scene.right_click(150.0, 154.0));
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, 170.0, row(0.0)), CursorShape::Default);
    scene.press(170.0, row(0.0));
    assert!(!scene.run_menu(&mut clipboard));

    // すべて選択。押した項目の関数が走る。
    scene.press(170.0, row(3.0));
    assert!(scene.run_menu(&mut clipboard));
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("hello"));

    // 選んだ範囲の上で右クリックしても範囲は残り、コピーできる。
    scene.right_click(150.0, 154.0);
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("hello"));
    scene.press(170.0, row(1.0));
    assert!(scene.run_menu(&mut clipboard));
    assert_eq!(clipboard.text(), Some("hello"));

    // 切り取って、貼り付けて戻す。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, row(0.0));
    scene.run_menu(&mut clipboard);
    assert_eq!(scene.text(), "");
    scene.right_click(150.0, 154.0);
    scene.press(170.0, row(2.0));
    scene.run_menu(&mut clipboard);
    assert_eq!(scene.text(), "hello");

    // 外を押すと閉じるだけ。Esc でも閉じる。
    scene.right_click(150.0, 154.0);
    assert!(scene.press(20.0, 20.0));
    assert!(!scene.run_menu(&mut clipboard));
    assert!(scene.window_frame.has_focus(), "閉じるための押しでは、フォーカスは外れない");

    scene.right_click(150.0, 154.0);
    assert!(scene.key(EditKey::Escape));
    scene.press(170.0, row(3.0));
    assert!(!scene.run_menu(&mut clipboard), "閉じたあとは項目が無い");
}

#[test]
fn context_menu_style_changes_its_size() {
    let Some(mut scene) = scene_with_frame(|input| input, |window_frame, font| {
        window_frame.set_context_menu_style(ContextMenuStyle {
            font: ContextMenuFont { base_font: font, base_font_size: 13.0 },
            theme: menu_theme(),
            layout: ContextMenuLayout {
                quad_type: QuadType::Rounded { radius: 6.0 },
                padding: [10.0, 10.0],
                item_height: Some(40.0),
                min_width: 220.0,
                shadow_offset: Some([0.0, 4.0]),
                ..ContextMenuLayout::default()
            },
        });
    }) else { return };
    let mut clipboard = MemoryClipboard::new();

    scene.click(150.0, 154.0);
    scene.type_text("hello");
    scene.right_click(150.0, 154.0);

    // 上の余白 10、1 項目 40。4 つ目（すべて選択）の真ん中。
    let row = |index: f32| 154.0 + 10.0 + 40.0 * index + 20.0;
    // 幅は 220 まで広がるので、押した位置から 200 右もメニューの中。
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, 350.0, row(3.0)), CursorShape::Default);
    scene.press(350.0, row(3.0));
    assert!(scene.run_menu(&mut clipboard));
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("hello"));

    // 4 項目ぶん + 上下の余白の下は、もうメニューの外。外を押すと閉じるだけ。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 10.0 + 40.0 * 4.0 + 15.0);
    assert!(!scene.run_menu(&mut clipboard));
}

fn menu_theme() -> ContextMenuTheme {
    ContextMenuTheme {
        fill_color: color(0.2),
        outline_color: color(0.4),
        text_color: color(1.0),
        sub_title_color: color(0.7),
        highlight_color: color(0.3),
        shadow_color: color(0.0),
        disabled_alpha: 0.5,
    }
}

#[test]
fn custom_menu_with_slicer_and_sub_menu() {
    let marked = Rc::new(Cell::new(false));
    let mark = marked.clone();

    let Some(mut scene) = scene_with_frame(|input| input, move |window_frame, font| {
        // 幅を決めておくと、右に開くメニューの位置が分かる（字が短いので 200 のまま）。
        window_frame.set_context_menu_style(ContextMenuStyle {
            font: ContextMenuFont { base_font: font, base_font_size: 13.0 },
            theme: menu_theme(),
            layout: ContextMenuLayout { min_width: 200.0, ..ContextMenuLayout::default() },
        });

        let mut case = ContextMenuLabels::new("Case");
        case.add_context_menu_item(
            ContextMenuLabel::new("Upper")
                .enabled(|state| state.has_selection && state.editable)
                .on_event(|event| {
                    if let Some(text) = event.selected_text() {
                        event.replace_selection(&text.to_uppercase());
                    }
                }),
        );

        let mut menu = ContextMenuLabels::new(INPUT_TEXT_CONTEXT_MENU);
        menu.add_context_menu_item(ContextMenuLabel {
            label: "Mark".to_string(),
            sub_title: Some("Ctrl+M".to_string()),
            on_event: Some(Box::new(move |_| mark.set(true))),
            ..Default::default()
        })
        .add_context_menu_item(ContextMenuSlicer { color: color(0.5) })
        .add_context_menu_item(ContextMenuLabel::new("Case").sub_menu(case));

        window_frame.add_context_menu_labels(menu);
    }) else { return };
    let mut clipboard = MemoryClipboard::new();

    scene.click(150.0, 154.0);
    scene.type_text("hello");
    scene.key(EditKey::SelectAll);

    // 上から、Mark（4..29）・区切り線（29..38）・Case（38..63）。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 4.0 + 12.0);
    assert!(scene.run_menu(&mut clipboard));
    assert!(marked.get(), "自分で書いた関数が走る");

    // 区切り線は押しても何も起きない。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 29.0 + 4.0);
    assert!(!scene.run_menu(&mut clipboard));

    // Case に指を乗せると、右（150 + 200 = 350）に開く。項目の上端に、中身の上端をそろえる。
    let case_row = 154.0 + 38.0 + 12.0;
    assert!(scene.window_frame.mouse_moved(&mut scene.draw_manager, 170.0, case_row));
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, 400.0, case_row), CursorShape::Default, "右に開いた");

    // 右に開いたメニューへ指を移しても閉じず、Upper を押せる。
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 400.0, case_row);
    scene.press(400.0, case_row);
    assert!(scene.run_menu(&mut clipboard));
    assert_eq!(scene.text(), "HELLO");

    // 開いた親の項目から別の項目へ移ると、右のメニューは閉じる。
    scene.right_click(150.0, 154.0);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 170.0, case_row);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 170.0, 154.0 + 4.0 + 12.0);
    // 右のメニューがあったところは、もうメニューの外。押すと閉じるだけで、何も選ばれない。
    scene.press(400.0, case_row);
    assert!(!scene.run_menu(&mut clipboard), "右のメニューは閉じている");
    assert_eq!(scene.text(), "HELLO");
}

#[test]
fn disabled_items_can_be_hidden_and_the_rest_close_up() {
    let Some(mut scene) = scene_with_frame(|input| input, |window_frame, font| {
        // 既定を「消す」にして、Paste だけは「薄くする」に戻す。
        window_frame.set_context_menu_style(ContextMenuStyle {
            font: ContextMenuFont { base_font: font, base_font_size: 13.0 },
            theme: menu_theme(),
            layout: ContextMenuLayout { when_disabled: ContextMenuDisabled::Hide, ..ContextMenuLayout::default() },
        });

        let mut menu = ContextMenuLabels::new(INPUT_TEXT_CONTEXT_MENU);
        menu.add_context_menu_item(ContextMenuLabel::cut("Cut"))
            .add_context_menu_item(ContextMenuLabel::copy("Copy"))
            .add_context_menu_item(ContextMenuSlicer { color: color(0.5) })
            .add_context_menu_item(ContextMenuLabel::paste("Paste").when_disabled(ContextMenuDisabled::Dim))
            .add_context_menu_item(ContextMenuSlicer { color: color(0.5) })
            .add_context_menu_item(ContextMenuLabel::select_all("Select All"));
        window_frame.add_context_menu_labels(menu);
    }) else { return };
    let mut clipboard = MemoryClipboard::new();
    clipboard_set(&mut clipboard, "!");

    scene.click(150.0, 154.0);
    scene.type_text("hello");

    // 選んでいないので Cut と Copy は消え、先頭に来た区切り線も消える。
    // 並びは Paste（4..29）・区切り線（29..38）・Select All（38..63）、高さは 67。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 4.0 + 12.0);
    assert!(scene.run_menu(&mut clipboard), "詰めた先頭は Paste");
    assert_eq!(scene.text(), "hello!");

    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 38.0 + 12.0);
    assert!(scene.run_menu(&mut clipboard), "その下は Select All");
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some("hello!"));

    // 選んでいれば、すべて見える。並びは元どおり（Cut・Copy・区切り線・Paste…）。
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 4.0 + 25.0 + 12.0);
    assert!(scene.run_menu(&mut clipboard), "2 つ目は Copy");
    assert_eq!(clipboard.text(), Some("hello!"));

    // 縮めた高さ（67）より下は、もうメニューの外。
    scene.key(EditKey::End);
    scene.right_click(150.0, 154.0);
    scene.press(170.0, 154.0 + 100.0);
    assert!(!scene.run_menu(&mut clipboard));
}

#[test]
fn a_menu_with_nothing_to_show_does_not_open() {
    let Some(mut scene) = scene_with_frame(|input| input, |window_frame, _| {
        let mut menu = ContextMenuLabels::new(INPUT_TEXT_CONTEXT_MENU);
        menu.add_context_menu_item(ContextMenuLabel::copy("Copy").hide_when_disabled())
            .add_context_menu_item(ContextMenuSlicer { color: color(0.5) });
        window_frame.add_context_menu_labels(menu);
    }) else { return };

    scene.click(150.0, 154.0);
    scene.type_text("hello");

    // 選んでいないので Copy は消え、区切り線だけでは開かない。
    scene.right_click(150.0, 154.0);
    assert_eq!(scene.window_frame.cursor_shape(&scene.draw_manager, 170.0, 158.0), CursorShape::Text, "メニューは出ていない（出ていれば欄を覆う）");
}

fn clipboard_set(clipboard: &mut MemoryClipboard, text: &str) {
    use gueiz_2d::gui::clipboard::Clipboard;
    clipboard.set_text(text);
}

#[test]
#[ignore = "速さを測るだけ。cargo test -p gueiz-2d --test input_text_gpu --release measure -- --ignored --nocapture"]
fn measure_moving_through_long_text() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text(&"long text ".repeat(300));

    let start = std::time::Instant::now();
    for _ in 0..200 {
        scene.key(EditKey::Left);
    }
    eprintln!("200 moves through {} chars: {:?}", scene.text().chars().count(), start.elapsed());

    let start = std::time::Instant::now();
    for _ in 0..200 {
        scene.type_text("x");
    }
    eprintln!("200 characters typed into {} chars: {:?}", scene.text().chars().count(), start.elapsed());
}

#[test]
#[ignore = "速さを測るだけ。cargo test -p gueiz-2d --test input_text_gpu --release measure -- --ignored --nocapture"]
fn measure_mouse_selection_through_long_text() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    scene.type_text(&"long text ".repeat(300));
    scene.key(EditKey::Home);
    scene.update();

    // 押したまま欄の中を動かす。指が動くたびに、指の形も聞かれ、描く前の update も走る。
    scene.press_at(0.0);
    let start = std::time::Instant::now();
    for step in 0..200 {
        let x = 113.0 + (step % 180) as f32;
        scene.window_frame.mouse_moved(&mut scene.draw_manager, x, 154.0);
        let _ = scene.window_frame.cursor_shape(&scene.draw_manager, x, 154.0);
        scene.update();
    }
    eprintln!("200 drag moves inside: {:?}", start.elapsed());

    // 欄の右の外へ出したまま。update のたびに横へ送る。
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 400.0, 154.0);
    let start = std::time::Instant::now();
    for _ in 0..200 {
        let _ = scene.window_frame.cursor_shape(&scene.draw_manager, 400.0, 154.0);
        scene.update();
    }
    eprintln!("200 auto-scroll frames: {:?}", start.elapsed());
    scene.release_at(0.0);
}

#[test]
fn fast_drag_past_the_edge_keeps_the_caret_inside() {
    let Some(mut scene) = scene() else { return };
    scene.click(150.0, 154.0);
    let text = "a".repeat(200);
    scene.type_text(&text);
    scene.key(EditKey::Home);
    scene.update();

    // 欄は 105..305。字を削るのは余白の半分（4）の内側。
    let inside = |caret: Option<[f32; 2]>| caret.is_none_or(|[left, right]| left >= 109.0 && right <= 301.0);

    // 先頭を押して、欄の右の外まで一気に動かす。
    scene.press_at(0.0);
    scene.window_frame.mouse_moved(&mut scene.draw_manager, 450.0, 154.0);
    assert!(inside(scene.drawn_caret()), "動かした直後: {:?}", scene.drawn_caret());

    let selected = scene.window_frame.selected_text().map_or(0, |text| text.len());
    assert!(selected < text.len(), "一気には選ばず、欄の端まで: {selected}");

    // 横へ送りながら広げるあいだも、キャレットは欄の中。
    for _ in 0..120 {
        scene.update();
        assert!(inside(scene.drawn_caret()), "送っているあいだ: {:?}", scene.drawn_caret());
        if scene.window_frame.selected_text().as_deref() == Some(text.as_str()) {
            break;
        }
        sleep(Duration::from_millis(16));
    }
    assert_eq!(scene.window_frame.selected_text().as_deref(), Some(text.as_str()));
    scene.release_at(0.0);
}
