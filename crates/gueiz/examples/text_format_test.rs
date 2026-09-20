//! 文字列に埋めた書式が、**描いた画素まで届いている**ことを確かめる。
//! 既定ではウィンドウを開かない。`--window` を付けると、描いた結果を窓に出す。
//!
//! 並べ方そのものは `gueiz-2d` の単体テストで見ているので、ここで見るのは
//! 「書式 → 並べ方 → GPU」がつながっているかどうか。
//!
//! ```sh
//! cargo run -p gueiz --example text_format_test
//! cargo run -p gueiz --example text_format_test -- --window
//! ```

mod common;

use std::error::Error;

use common::preview::Preview;

use gueiz_2d::camera::Camera;
use gueiz_2d::font::Font;
use gueiz_2d::format::Formatted;
use gueiz_2d::object::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::text::{TextLayout, TextRenderer, TextStyle, measure_formatted};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::wgpu;

const SIZE: u32 = 256;
const FORMAT: TextureFormat = TextureFormat::Bgra8Unorm;

/// これより明るければ「塗られている」とみなす。
const LIT: u8 = 16;

/// 手元にあるフォント。無ければ分かるように落とす。
const FONT_PATH: &str = "C:/Windows/Fonts/arial.ttf";

fn main() -> Result<(), Box<dyn Error>> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();

    let data = std::fs::read(FONT_PATH)
        .map_err(|error| format!("{FONT_PATH} を読めませんでした: {error}"))?;
    let font = Font::from_bytes(&data)?;

    let instance_handle =
        wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
    let adapter = pollster::block_on(
        instance_handle.request_adapter(&wgpu::RequestAdapterOptions::default()),
    )?;
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("text format test"),
        required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
        ..Default::default()
    }))?;

    println!("font   : {FONT_PATH}");
    println!("adapter: {}\n", adapter.get_info().name);

    let target = Target::new(&device);
    let mut preview = Preview::new("text_format_test");
    let draw = |text: &str, size: f32, x: f32, y: f32| {
        render(&device, &queue, &target, &font, text, size, x, y)
    };

    // 1. 書式を読むかどうかは呼ぶ側が決める。
    //    `write` はコードを**字として**出し、`write_formatted` は読んで色にする。
    let literal = render_plain(&device, &queue, &target, &font, "§[red]A", 100.0)?;
    let (formatted, _) = draw("§[red]A", 100.0, 20.0, 20.0)?;
    preview.capture("1. 読まない / 読む", SIZE, SIZE, &literal.pixels);
    println!(
        "1. 書式を読む/読まない      字のまま {} 画素、読んだら {} 画素",
        lit_count(&literal.pixels),
        lit_count(&formatted.pixels),
    );
    assert!(
        lit_count(&literal.pixels) > lit_count(&formatted.pixels),
        "読まなければ `§[red]` の 6 文字ぶん多く出るはず",
    );
    assert!(is_red(mean_lit(&formatted.pixels)), "読んだら赤いはず");

    // 2. 色を上書きして、また呼ぶ側の色に戻す。
    //    左半分が赤、右半分が白になっていること。
    let (both, two_glyphs) = draw("§[red]A§[defaultcolor]A", 100.0, 10.0, 20.0)?;
    preview.capture("2. 色と既定の色", SIZE, SIZE, &both.pixels);
    // 半分で割ると 2 文字目が左にはみ出す。並べた結果から境目をもらう。
    let split = (10.0 + two_glyphs.glyphs[1].x) as u32;
    let left = mean_lit_in(&both.pixels, 0..split);
    let right = mean_lit_in(&both.pixels, split..SIZE);
    println!("2. 色 / 既定の色            左 {left:?}、右 {right:?}");
    assert!(is_red(left), "1 文字目は赤");
    assert!(is_white(right), "2 文字目は呼ぶ側の白に戻る");

    // 3. 大きさ。同じ字でも `size` で伸びること。
    let (small, _) = draw("A", 60.0, 20.0, 20.0)?;
    let (large, _) = draw("§[size 120]A", 60.0, 20.0, 20.0)?;
    let small_box = bounds(&small.pixels).expect("塗られている");
    let large_box = bounds(&large.pixels).expect("塗られている");
    println!(
        "3. 大きさ                   60px は {} 画素高、120px は {} 画素高",
        small_box.height(),
        large_box.height(),
    );
    assert!(large_box.height() > small_box.height() * 3 / 2);

    // 4. 太字。別の書体は持っていないので、ずらした複製で太らせている。
    let (plain, _) = draw("A", 100.0, 20.0, 20.0)?;
    let (bold, _) = draw("§[bold]A", 100.0, 20.0, 20.0)?;
    preview.capture("4. 太字", SIZE, SIZE, &bold.pixels);
    println!(
        "4. 太字                     素 {} 画素 → 太字 {} 画素",
        lit_count(&plain.pixels),
        lit_count(&bold.pixels),
    );
    assert!(lit_count(&bold.pixels) > lit_count(&plain.pixels));

    // 5. 斜体。上のほうが右に寄ること。
    //
    //    見本を差し替えても切れないように、**測ってから**大きさと位置を決める。
    //    決め打ちで置くと、字を増やしたとたんに画面からはみ出して
    //    「斜体が切れている」ように見える。
    let italic_sample = "§[italic 0.4]IOT";
    let parsed_italic = Formatted::parse(italic_sample)?;
    let margin = 8.0;
    let room = SIZE as f32 - margin * 2.0;

    // 一度測って、収まる大きさまで落とす。塗られる範囲は大きさに比例する。
    let probe = measure_formatted(&font, &parsed_italic, &TextStyle::new(140.0));
    let italic_points =
        140.0 * (room / probe.ink_width()).min(room / probe.ink_height()).min(1.0);

    // その大きさで測り直して、真ん中に来る左上を出す。
    let fitted = measure_formatted(&font, &parsed_italic, &TextStyle::new(italic_points));
    let (italic, _) = draw(
        italic_sample,
        italic_points,
        (SIZE as f32 - fitted.ink_width()) / 2.0 - fitted.ink.left,
        (SIZE as f32 - fitted.ink_height()) / 2.0 - fitted.ink.top,
    )?;
    preview.capture("5. 斜体", SIZE, SIZE, &italic.pixels);
    let slanted = bounds(&italic.pixels).expect("塗られている");

    println!(
        "5. 斜体                     140px だと塗る幅 {:.1}（画面は {SIZE}）→ {:.0}px に落とす",
        probe.ink_width(),
        italic_points,
    );
    // 画面のどの縁にも触っていないこと。触っていたら切れている。
    assert!(slanted.left > 0 && slanted.right < SIZE - 1, "左右が切れている");
    assert!(slanted.top > 0 && slanted.bottom < SIZE - 1, "上下が切れている");
    let top = centroid_x(&italic.pixels, slanted.top..slanted.top + slanted.height() / 4);
    let bottom =
        centroid_x(&italic.pixels, slanted.bottom - slanted.height() / 4..slanted.bottom);
    println!("                            上の重心 {top:.1}、下の重心 {bottom:.1}");
    assert!(top > bottom + 5.0, "上が右に寄っているはず");

    // 6. 透かす。同じ白でも暗くなること。
    let (ghost, _) = draw("§[ghost 0.6]A", 100.0, 20.0, 20.0)?;
    let solid_brightness = mean_lit(&plain.pixels)[0] as u32;
    let ghost_brightness = mean_lit(&ghost.pixels)[0] as u32;
    println!(
        "6. 透かす                   素 {solid_brightness} → 0.6 透かして {ghost_brightness}",
    );
    assert!(ghost_brightness < solid_brightness * 2 / 3);

    // 7. 縁取り。字の周りに別の色が回ること。
    let (outlined, _) = draw("§[outline #ff0000]A", 100.0, 20.0, 20.0)?;
    preview.capture("7. 縁取り", SIZE, SIZE, &outlined.pixels);
    let outline_box = bounds(&outlined.pixels).expect("塗られている");
    let plain_box = bounds(&plain.pixels).expect("塗られている");
    println!(
        "7. 縁取り                   赤 {} 画素、外周が {} 画素ぶん広がる",
        count_where(&outlined.pixels, is_red),
        outline_box.height() - plain_box.height(),
    );
    assert!(count_where(&outlined.pixels, is_red) > 100, "赤い縁が出ていない");
    assert!(outline_box.height() > plain_box.height(), "縁のぶん広がるはず");
    assert!(count_where(&outlined.pixels, is_white) > 100, "字は白のまま");

    // 8. 影。右下にずれた暗い複製が出ること。
    let (shadow, _) = draw("§[shadow 0.2 45]A", 100.0, 20.0, 20.0)?;
    preview.capture("8. 影", SIZE, SIZE, &shadow.pixels);
    let shadow_box = bounds(&shadow.pixels).expect("塗られている");
    println!(
        "8. 影                       右下に {} 画素、下に {} 画素はみ出す",
        shadow_box.right - plain_box.right,
        shadow_box.bottom - plain_box.bottom,
    );
    assert!(shadow_box.right > plain_box.right, "右にずれた複製が要る");
    assert!(shadow_box.bottom > plain_box.bottom, "下にずれた複製が要る");
    // 影は字の色を落としたもの。まっ白でもまっ暗でもない。
    assert!(
        count_where(&shadow.pixels, |color| color[0] > 30 && color[0] < 120) > 50,
        "影の明るさの画素が無い",
    );

    // 9. 下線。字の下に線が出て、本数ぶん重なること。
    let (bare, _) = draw("nn", 100.0, 20.0, 20.0)?;
    let (single, _) = draw("§[underline]nn", 100.0, 20.0, 20.0)?;
    let (double, _) = draw("§[underline 0.05 miter 2]nn", 100.0, 20.0, 20.0)?;
    preview.capture("9. 下線（2 本）", SIZE, SIZE, &double.pixels);
    let bare_bottom = bounds(&bare.pixels).expect("塗られている").bottom;
    // 字より下だけを見れば、線の本数がそのまま区間の数になる。
    let column = bounds(&single.pixels).expect("塗られている").center_x();
    let one = runs_below(&single.pixels, column, bare_bottom);
    let two = runs_below(&double.pixels, column, bare_bottom);
    println!("9. 下線                     1 本指定 → {one} 区間、2 本指定 → {two} 区間");
    assert_eq!(one, 1);
    assert_eq!(two, 2);

    // 10. 打消し線・上線・下線は、それぞれ違う高さに出ること。
    let (over, _) = draw("§[overline]nn", 100.0, 20.0, 20.0)?;
    let (strike, _) = draw("§[strike]nn", 100.0, 20.0, 20.0)?;
    preview.capture("10. 上線 / 打消し線", SIZE, SIZE, &strike.pixels);
    let over_top = bounds(&over.pixels).expect("塗られている").top;
    let bare_top = bounds(&bare.pixels).expect("塗られている").top;
    let strike_rows = widest_row(&strike.pixels);
    println!(
        "10. 上線 / 打消し線         上線は {} 画素上、打消し線は {} 行目",
        bare_top - over_top,
        strike_rows,
    );
    assert!(over_top < bare_top, "上線は字より上");
    assert!(
        strike_rows > bare_top && strike_rows < bare_bottom,
        "打消し線は字の中ほど",
    );

    // 11. 隠し箱。字は描かれず、塗りつぶした四角だけが出ること。
    let (hidden, _) = draw("§[hidebox]nnn", 80.0, 20.0, 40.0)?;
    preview.capture("11. 隠し箱", SIZE, SIZE, &hidden.pixels);
    let box_bounds = bounds(&hidden.pixels).expect("塗られている");
    let filled = lit_count(&hidden.pixels) as f32 / box_bounds.area() as f32;
    println!(
        "11. 隠し箱                  外枠 {}x{}、埋まり具合 {:.3}",
        box_bounds.width(),
        box_bounds.height(),
        filled,
    );
    assert!(filled > 0.99, "隙間があると字が透けて読める");
    assert_eq!(
        runs_in_row(&hidden.pixels, box_bounds.center_y()),
        1,
        "1 続きの四角になっているはず",
    );

    // 12. 出鱈目な字。並びは動かさずに、出る字だけ変わること。
    let mut style = TextStyle::new(80.0);
    let scrambled = Formatted::parse("§[obfuscated]gueiz")?;
    let plain_layout = layout_of(&font, &Formatted::plain("gueiz"), &style);

    style.obfuscation_seed = 1;
    let first = layout_of(&font, &scrambled, &style);
    style.obfuscation_seed = 2;
    let second = layout_of(&font, &scrambled, &style);

    let glyphs = |layout: &TextLayout| -> Vec<u16> {
        layout.glyphs.iter().map(|glyph| glyph.glyph.0).collect()
    };
    println!(
        "12. 出鱈目な字              幅 {:.1} px のまま、字は {:?} → {:?}",
        first.width,
        &glyphs(&first)[..3],
        &glyphs(&second)[..3],
    );
    assert_eq!(first.width, plain_layout.width, "並びは動かない");
    assert_ne!(glyphs(&first), glyphs(&plain_layout), "字は変わる");
    assert_ne!(glyphs(&first), glyphs(&second), "種が違えば変わる");

    // 13. 積み上げた書式を退避して戻す。赤 → 白 → 赤。
    let (skipped, three_glyphs) = draw("§[red]A§[skippush]A§[skippop]A", 70.0, 10.0, 30.0)?;
    preview.capture("13. 退避と復帰", SIZE, SIZE, &skipped.pixels);
    let cut = |index: usize| (10.0 + three_glyphs.glyphs[index].x) as u32;
    let thirds = [
        mean_lit_in(&skipped.pixels, 0..cut(1)),
        mean_lit_in(&skipped.pixels, cut(1)..cut(2)),
        mean_lit_in(&skipped.pixels, cut(2)..SIZE),
    ];
    println!("13. 退避と復帰              {thirds:?}");
    assert!(is_red(thirds[0]), "1 文字目は赤");
    assert!(is_white(thirds[1]), "退避中は素の白");
    assert!(is_red(thirds[2]), "戻したら赤");

    // 14. 書き直し。`clear` を挟めば、2 回描いても 1 回ぶんになること。
    let twice = render_twice(&device, &queue, &target, &font, "§[red]A", 100.0)?;
    println!(
        "14. 書き直し                1 回 {} 画素、消して 2 回 {} 画素",
        lit_count(&formatted.pixels),
        lit_count(&twice),
    );
    assert_eq!(lit_count(&twice), lit_count(&formatted.pixels));

    // 15. 混ぜて使う。全部同時に効くこと。
    let (mixed, layout) = draw(
        "§[gold]§[bold]WIN§[/]§[ln]§[size 40]§[underline 0.05 round_start_end 1]§[cyan]next",
        64.0,
        16.0,
        16.0,
    )?;
    preview.capture("15. 混ぜて使う", SIZE, SIZE, &mixed.pixels);
    println!(
        "15. 混ぜて使う              {} 行、{} 文字、線 {} 本、金 {} 画素、水 {} 画素",
        layout.rows.len(),
        layout.glyphs.len(),
        layout.decorations.len(),
        count_where(&mixed.pixels, is_gold),
        count_where(&mixed.pixels, is_cyan),
    );
    assert_eq!(layout.rows.len(), 2);
    assert_eq!(layout.glyphs.len(), 7);
    assert_eq!(layout.decorations.len(), 1);
    assert!(count_where(&mixed.pixels, is_gold) > 100, "1 行目が金でない");
    assert!(count_where(&mixed.pixels, is_cyan) > 50, "2 行目が水色でない");
    // 2 行目は小さいので、1 行目より下にしか出ない。
    assert!(
        layout.rows[1].top > layout.rows[0].height,
        "2 行目が 1 行目に重なっている",
    );

    // 16. 塗られる範囲。斜体・影・縁取り・丸い端は**送り幅からはみ出す**ので、
    //     `width` だけを見て場所を決めると切れる。`ink` が拾っていること。
    const AT: f32 = 40.0;

    println!("16. 塗られる範囲");

    // 右の欄は「送り幅の箱では足りないか」。`ink` が要る理由がここ。
    // 縁取りと太字が `false` なのは、アセンダの余白に収まってしまうため。
    // 字と大きさが変われば `true` になる。`ink` はそれを見越して広く取る。
    for (text, escapes_advance_box) in [
        ("Hnb", false),
        ("§[italic 0.4]Hnb", true),
        ("§[italic -0.3]Hnb", true),
        ("§[shadow 0.3 0]Hnb", true),
        ("§[outline #f00]Hnb", false),
        ("§[bold]Hnb", false),
        ("§[underline 0.1 round_start_end 2]Hnb", true),
    ] {
        let (frame, layout) = draw(text, 60.0, AT, AT)?;
        let painted = bounds(&frame.pixels).expect("塗られている");
        let ink = layout.ink;

        // 送り幅の箱。はみ出すものを拾えない。
        let advance_left = AT;
        let advance_right = AT + layout.width;
        let escapes = (painted.left as f32) < advance_left - 0.5
            || (painted.right as f32) > advance_right + 0.5;

        println!(
            "    {text:38} 墨 {}..{} / ink {:.1}..{:.1} / 送り幅 {:.1}..{:.1}",
            painted.left,
            painted.right,
            AT + ink.left,
            AT + ink.right,
            advance_left,
            advance_right,
        );

        // 塗られた画素がひとつ残らず `ink` の中に入っていること。
        assert!(
            AT + ink.left <= painted.left as f32 + 0.5,
            "{text}: 左が ink からはみ出した",
        );
        assert!(
            AT + ink.right >= painted.right as f32 - 0.5,
            "{text}: 右が ink からはみ出した",
        );
        assert!(
            AT + ink.top <= painted.top as f32 + 0.5,
            "{text}: 上が ink からはみ出した",
        );
        assert!(
            AT + ink.bottom >= painted.bottom as f32 - 0.5,
            "{text}: 下が ink からはみ出した",
        );

        // そのうえで、送り幅の箱では足りていないことも押さえる。
        // ここが `false` に戻ったら、はみ出しを測る意味が消えている。
        assert_eq!(
            escapes, escapes_advance_box,
            "{text}: 送り幅の箱に収まるかどうかが変わった",
        );
    }

    // 17. 測ってから置く。右下いっぱいに寄せる。
    //     送り幅で寄せると斜体の上が画面の外に出て、**そのぶん消える**。
    //     塗られる範囲で寄せれば消えない。測る意味はここに出る。
    let leaning = "§[italic 0.4]Wg";
    let measured = measure_formatted(&font, &Formatted::parse(leaning)?, &TextStyle::new(70.0));

    let edge = SIZE as f32;
    let (snug, _) = draw(
        leaning,
        70.0,
        edge - measured.ink.right,
        edge - measured.ink.bottom,
    )?;
    let (clipped, _) = draw(leaning, 70.0, edge - measured.width, edge - measured.height)?;
    preview.capture("17. 塗られる範囲で寄せる", SIZE, SIZE, &snug.pixels);
    preview.capture("17. 送り幅で寄せると欠ける", SIZE, SIZE, &clipped.pixels);

    // ぶつからない場所に置いたものが、欠けていないときの画素数。
    let (whole, _) = draw(leaning, 70.0, 40.0, 40.0)?;

    println!(
        "17. 測ってから寄せる        送り幅 {:.1}x{:.1} / 塗る幅 {:.1}x{:.1}",
        measured.width,
        measured.height,
        measured.ink_width(),
        measured.ink_height(),
    );
    println!(
        "    欠けていないとき        {} 画素",
        lit_count(&whole.pixels),
    );
    println!(
        "    塗られる範囲で寄せる    {} 画素（右端 {} 画素目）",
        lit_count(&snug.pixels),
        bounds(&snug.pixels).expect("塗られている").right,
    );
    println!(
        "    送り幅で寄せる          {} 画素（右端 {} 画素目）",
        lit_count(&clipped.pixels),
        bounds(&clipped.pixels).expect("塗られている").right,
    );

    assert!(measured.ink_width() > measured.width, "斜体ははみ出すはず");
    // 塗られる範囲で寄せれば欠けない。小数点以下の位置が違うと縁の
    // 当たり方が 1 画素ぶん変わるので、そのぶんだけ許す。
    assert!(
        lit_count(&snug.pixels) + 4 >= lit_count(&whole.pixels),
        "測って寄せたのに欠けた: {} < {}",
        lit_count(&snug.pixels),
        lit_count(&whole.pixels),
    );
    // 送り幅で寄せると、同じ文字列なのに画素が減る = 切れている。
    assert!(
        lit_count(&clipped.pixels) < lit_count(&whole.pixels),
        "送り幅で寄せても切れないなら、測る意味が無い",
    );

    println!("\nすべて通りました。");

    if common::preview::window_requested() {
        preview.show()?;
    } else {
        println!("`-- --window` を付けると描いた結果を窓に出します。");
    }

    Ok(())
}

// --- 描く ---

struct Frame {
    pixels: Vec<u8>,
}

/// 書式を読んで描く。
#[allow(clippy::too_many_arguments)]
fn render(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    font: &Font,
    text: &str,
    size: f32,
    x: f32,
    y: f32,
) -> Result<(Frame, TextLayout), Box<dyn Error>> {
    let formatted = Formatted::parse(text)?;
    let mut draw_manager = new_draw_manager(device, queue)?;
    let mut renderer = new_renderer();

    let layout = renderer.write_formatted(
        &mut draw_manager,
        font,
        &formatted,
        &TextStyle::new(size),
        x,
        y,
    )?;

    let pixels = present(device, queue, target, &mut draw_manager)?;

    Ok((Frame { pixels }, layout))
}

/// 書式を読まずに、コードごと字として描く。
fn render_plain(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    font: &Font,
    text: &str,
    size: f32,
) -> Result<Frame, Box<dyn Error>> {
    let mut draw_manager = new_draw_manager(device, queue)?;
    let mut renderer = new_renderer();

    renderer.write(&mut draw_manager, font, text, &TextStyle::new(size), 20.0, 20.0)?;

    Ok(Frame {
        pixels: present(device, queue, target, &mut draw_manager)?,
    })
}

/// 同じ文字列を 2 回書く。あいだに [`TextRenderer::clear`] を挟む。
fn render_twice(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    font: &Font,
    text: &str,
    size: f32,
) -> Result<Vec<u8>, Box<dyn Error>> {
    let formatted = Formatted::parse(text)?;
    let style = TextStyle::new(size);
    let mut draw_manager = new_draw_manager(device, queue)?;
    let mut renderer = new_renderer();

    renderer.write_formatted(&mut draw_manager, font, &formatted, &style, 20.0, 20.0)?;
    renderer.clear(&mut draw_manager);
    renderer.write_formatted(&mut draw_manager, font, &formatted, &style, 20.0, 20.0)?;

    present(device, queue, target, &mut draw_manager)
}

fn new_draw_manager(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
) -> Result<DrawManager, Box<dyn Error>> {
    Ok(DrawManager::new(
        device,
        queue,
        FORMAT,
        &DrawManagerDescriptor::default(),
    )?)
}

fn new_renderer() -> TextRenderer {
    let mut renderer = TextRenderer::new();
    renderer
        .camera(Camera::orthographic_2d(SIZE as f32, SIZE as f32))
        .color(1.0, 1.0, 1.0, 1.0);

    renderer
}

fn present(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    target: &Target,
    draw_manager: &mut DrawManager,
) -> Result<Vec<u8>, Box<dyn Error>> {
    draw_manager.prepare(device, queue)?;

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("text format test"),
    });

    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("text"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &target.view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                    store: wgpu::StoreOp::Store,
                },
            })],
            ..Default::default()
        });

        draw_manager.draw(&mut render_pass);
    }

    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &target.texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &target.readback,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 4),
                rows_per_image: Some(SIZE),
            },
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );

    queue.submit(Some(encoder.finish()));

    target.readback.map_async(wgpu::MapMode::Read, .., |_| {});
    device.poll(wgpu::PollType::wait_indefinitely())?;

    let view = target.readback.get_mapped_range(..)?;
    let pixels = view.to_vec();
    drop(view);
    target.readback.unmap();

    Ok(pixels)
}

fn layout_of(font: &Font, formatted: &Formatted, style: &TextStyle) -> TextLayout {
    gueiz_2d::text::layout_formatted(font, formatted, style)
}

struct Target {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
    readback: wgpu::Buffer,
}

impl Target {
    fn new(device: &wgpu::Device) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("target"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: FORMAT.into(),
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });

        Self {
            view: texture.create_view(&wgpu::TextureViewDescriptor::default()),
            texture,
            readback: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("readback"),
                size: (SIZE * SIZE * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            }),
        }
    }
}

// --- 画素を見る ---

/// BGRA で並んでいるので、赤緑青の順に並べ替えて返す。
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let at = ((y * SIZE + x) * 4) as usize;

    [pixels[at + 2], pixels[at + 1], pixels[at]]
}

fn is_lit(color: [u8; 3]) -> bool {
    color.iter().copied().max().unwrap_or(0) > LIT
}

fn is_red(color: [u8; 3]) -> bool {
    color[0] > 100 && color[1] < 60 && color[2] < 60
}

/// `#ffd700` あたり。赤と緑が高く、青がほぼ無い。
fn is_gold(color: [u8; 3]) -> bool {
    color[0] > 200 && color[1] > 150 && color[2] < 60
}

/// `#00ffff` あたり。緑と青が高く、赤がほぼ無い。
fn is_cyan(color: [u8; 3]) -> bool {
    color[0] < 60 && color[1] > 150 && color[2] > 150
}

fn is_white(color: [u8; 3]) -> bool {
    color.iter().all(|channel| *channel > 100)
        && (color[0] as i32 - color[2] as i32).abs() < 40
}

fn lit_count(pixels: &[u8]) -> usize {
    count_where(pixels, is_lit)
}

fn count_where(pixels: &[u8], predicate: impl Fn([u8; 3]) -> bool) -> usize {
    (0..SIZE)
        .flat_map(|y| (0..SIZE).map(move |x| (x, y)))
        .filter(|(x, y)| predicate(pixel(pixels, *x, *y)))
        .count()
}

/// 塗られている画素の平均色。
fn mean_lit(pixels: &[u8]) -> [u8; 3] {
    mean_lit_in(pixels, 0..SIZE)
}

/// x が範囲に入っている、塗られた画素の平均色。
fn mean_lit_in(pixels: &[u8], columns: std::ops::Range<u32>) -> [u8; 3] {
    let mut sum = [0_u64; 3];
    let mut count = 0_u64;

    for y in 0..SIZE {
        for x in columns.clone() {
            let color = pixel(pixels, x, y);

            if !is_lit(color) {
                continue;
            }

            for channel in 0..3 {
                sum[channel] += color[channel] as u64;
            }

            count += 1;
        }
    }

    if count == 0 {
        return [0; 3];
    }

    [
        (sum[0] / count) as u8,
        (sum[1] / count) as u8,
        (sum[2] / count) as u8,
    ]
}

#[derive(Clone, Copy)]
struct Bounds {
    left: u32,
    right: u32,
    top: u32,
    bottom: u32,
}

impl Bounds {
    fn width(&self) -> u32 {
        self.right - self.left + 1
    }

    fn height(&self) -> u32 {
        self.bottom - self.top + 1
    }

    fn area(&self) -> u32 {
        self.width() * self.height()
    }

    fn center_x(&self) -> u32 {
        (self.left + self.right) / 2
    }

    fn center_y(&self) -> u32 {
        (self.top + self.bottom) / 2
    }
}

fn bounds(pixels: &[u8]) -> Option<Bounds> {
    let mut found: Option<Bounds> = None;

    for y in 0..SIZE {
        for x in 0..SIZE {
            if !is_lit(pixel(pixels, x, y)) {
                continue;
            }

            found = Some(match found {
                None => Bounds {
                    left: x,
                    right: x,
                    top: y,
                    bottom: y,
                },
                Some(box_) => Bounds {
                    left: box_.left.min(x),
                    right: box_.right.max(x),
                    top: box_.top.min(y),
                    bottom: box_.bottom.max(y),
                },
            });
        }
    }

    found
}

/// その行にある、塗られた区間の数。
fn runs_in_row(pixels: &[u8], y: u32) -> usize {
    let mut runs = 0;
    let mut inside = false;

    for x in 0..SIZE {
        let lit = is_lit(pixel(pixels, x, y));

        if lit && !inside {
            runs += 1;
        }

        inside = lit;
    }

    runs
}

/// ある列を、指定の行より下だけ見たときの区間の数。
fn runs_below(pixels: &[u8], x: u32, from: u32) -> usize {
    let mut runs = 0;
    let mut inside = false;

    for y in from + 1..SIZE {
        let lit = is_lit(pixel(pixels, x, y));

        if lit && !inside {
            runs += 1;
        }

        inside = lit;
    }

    runs
}

/// いちばん多く塗られている行。まっすぐな線を見つけるのに使う。
fn widest_row(pixels: &[u8]) -> u32 {
    (0..SIZE)
        .max_by_key(|y| (0..SIZE).filter(|x| is_lit(pixel(pixels, *x, *y))).count())
        .unwrap_or(0)
}

/// ある行の範囲で、塗られた画素の x の重心。
fn centroid_x(pixels: &[u8], rows: std::ops::Range<u32>) -> f32 {
    let mut sum = 0_u64;
    let mut count = 0_u64;

    for y in rows {
        for x in 0..SIZE {
            if is_lit(pixel(pixels, x, y)) {
                sum += x as u64;
                count += 1;
            }
        }
    }

    if count == 0 {
        return 0.0;
    }

    sum as f32 / count as f32
}
