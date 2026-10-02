//! **日本語の書体で、字形がそのとおりに塗れているか**を確かめる。
//!
//! 日本語の書体は画（かく）ごとの輪郭を重ねて字を作ります。重なりを
//! 包含の偶奇で塗ると抜けてしまうので、字形は輪郭の向き（非ゼロ巻き数）で
//! 塗ります。ここでは、出てきた三角形を**フォントの輪郭から数えた巻き数**と
//! 格子で突き合わせます。
//!
//! 書体はこの機械に入っているものを使います。無ければ飛ばし、
//! **飛ばしたことは出力に残します。**
//!
//! ```sh
//! cargo test -p gueiz-2d --test glyph_fill -- --nocapture
//! ```

use std::sync::OnceLock;

use gueiz_2d::draw_manager::{DrawManager, DrawManagerDescriptor};
use gueiz_2d::font::{DEFAULT_TOLERANCE, Font, GlyphOutline};
use gueiz_2d::paint_type::PaintType;
use gueiz_2d::tessellate::tessellate;
use gueiz_2d::text::{TextRenderer, TextStyle};
use gueiz_2d::texture::TextureFormat;
use gueiz_2d::vertex::Vertex;
use gueiz_2d::wgpu;

/// 画を重ねて字を作る書体。**偶奇で塗ると崩れるのはこれ。**
const OVERLAPPING: &str = "C:/Windows/Fonts/NotoSansJP-VF.ttf";

/// 調べる書体。重ねない書体でも、塗り方を変えて崩れていないことを見る。
const FONTS: &[&str] = &[
    OVERLAPPING,
    "C:/Windows/Fonts/YuGothM.ttc",
    "C:/Windows/Fonts/msgothic.ttc",
    "C:/Windows/Fonts/meiryo.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
];

/// よく使う字と、偶奇で塗ると Noto Sans JP で大きく崩れていた字。
const SAMPLE: &str = "漢字描画日本語東京書道愛永十田買鴞鵅鶆鶓黿麗鼙麤";

/// 格子の細かさ。em の 1.2 倍の枠を、この数で割る。
const GRID: usize = 48;

fn read(path: &str) -> Option<&'static [u8]> {
    match std::fs::read(path) {
        Ok(data) => Some(data.leak()),
        Err(_) => {
            eprintln!("{path} が無いので飛ばします");
            None
        }
    }
}

/// 答え。フォントの輪郭から、向きつきで辺を数える。
fn winding(outline: &GlyphOutline, x: f32, y: f32) -> i32 {
    let mut total = 0;

    for (index, &start) in outline.contour_starts.iter().enumerate() {
        let end = outline
            .contour_starts
            .get(index + 1)
            .copied()
            .unwrap_or(outline.points.len());
        let contour = &outline.points[start..end];

        for step in 0..contour.len() {
            let a = contour[step];
            let b = contour[(step + 1) % contour.len()];

            if (a[1] <= y) != (b[1] <= y) {
                let crossing = a[0] + (y - a[1]) / (b[1] - a[1]) * (b[0] - a[0]);

                if crossing > x {
                    total += if b[1] > a[1] { 1 } else { -1 };
                }
            }
        }
    }

    total
}

/// 点を内部に含む三角形の数。辺の上は数えない。
fn coverage(triangles: &[Vertex], x: f32, y: f32) -> usize {
    let side = |o: &Vertex, a: &Vertex| (a.x - o.x) * (y - o.y) - (a.y - o.y) * (x - o.x);

    triangles
        .chunks_exact(3)
        .filter(|corner| {
            let sides = [
                side(&corner[0], &corner[1]),
                side(&corner[1], &corner[2]),
                side(&corner[2], &corner[0]),
            ];

            sides.iter().all(|value| *value > 0.0) || sides.iter().all(|value| *value < 0.0)
        })
        .count()
}

/// 格子で突き合わせて、合わなかった点の数を返す。
///
/// 塗るべきところは**ちょうど 1 枚**で覆うこと。0 枚は欠け、2 枚は重ね塗り
/// （半透明の字で濃く出る）。格子は辺に乗らないよう半端にずらす。
fn mismatches(outline: &GlyphOutline, triangles: &[Vertex]) -> usize {
    let mut wrong = 0;

    for row in 0..GRID {
        for column in 0..GRID {
            let x = (column as f32 + 0.5) / GRID as f32 * 1.2 - 0.1 + 0.000137;
            let y = (row as f32 + 0.5) / GRID as f32 * 1.2 - 1.0 + 0.000291;

            let expected = usize::from(winding(outline, x, y) != 0);

            if coverage(triangles, x, y) != expected {
                wrong += 1;
            }
        }
    }

    wrong
}

fn painted(outline: &GlyphOutline, paint_type: PaintType) -> Vec<Vertex> {
    let vertices: Vec<Vertex> = outline
        .points
        .iter()
        .map(|point| Vertex::new_position_color(point[0], point[1], 0.0, 1.0, 1.0, 1.0, 1.0))
        .collect();

    let mut triangles = Vec::new();
    tessellate(&vertices, &outline.contour_starts, paint_type, &mut triangles);
    triangles
}

/// 書体ごとに、見本の字をすべて突き合わせる。
#[test]
fn glyphs_are_filled_by_their_winding() {
    let mut checked = 0;

    for path in FONTS {
        let Some(data) = read(path) else {
            continue;
        };
        let font = Font::from_bytes(data).expect("書体を読めない");

        for character in SAMPLE.chars() {
            let Some(glyph) = font.glyph(character) else {
                continue;
            };
            let Some(outline) = font.outline(glyph, DEFAULT_TOLERANCE) else {
                continue;
            };

            let triangles = painted(&outline, PaintType::FillNonZero);
            let wrong = mismatches(&outline, &triangles);

            assert_eq!(wrong, 0, "{path} の `{character}` が {wrong} 点ずれている");
            checked += 1;
        }
    }

    eprintln!("{checked} 字を確かめました");
}

/// **偶奇で塗ると崩れる**ことを確かめておく。
///
/// これが崩れなくなったら、上のテストは重なりを見ていないことになる。
#[test]
fn the_even_odd_fill_breaks_an_overlapping_font() {
    let Some(data) = read(OVERLAPPING) else {
        return;
    };
    let font = Font::from_bytes(data).expect("書体を読めない");

    let broken = SAMPLE
        .chars()
        .filter_map(|character| font.outline(font.glyph(character)?, DEFAULT_TOLERANCE))
        .filter(|outline| mismatches(outline, &painted(outline, PaintType::Fill)) > 0)
        .count();

    assert!(broken > 0, "偶奇でも崩れない。見本に重なった画が無い");
}

// ---- 文字を描く側が、この塗り方を使っていること ----

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

        let (device, queue) =
            pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
                label: Some("glyph fill test"),
                required_features: wgpu::Features::INDIRECT_FIRST_INSTANCE & adapter.features(),
                ..Default::default()
            }))
            .ok()?;

        Some(Gpu { device, queue })
    })
    .as_ref()
}

/// [`TextRenderer`] が登録した字形の三角形を、そのまま突き合わせる。
///
/// 塗り方を選ぶのは文字を描く側なので、テッセレータだけ見ていても
/// 「描く側が偶奇のまま」には気づけない。
#[test]
fn the_text_renderer_fills_glyphs_by_their_winding() {
    let Some(gpu) = gpu() else {
        eprintln!("GPU アダプタが取れないので飛ばします");
        return;
    };
    let Some(data) = read(OVERLAPPING) else {
        return;
    };
    let font = Font::from_bytes(data).expect("書体を読めない");

    let mut manager =
        DrawManager::new(&gpu.device, &gpu.queue, FORMAT, &DrawManagerDescriptor::default())
            .expect("DrawManager を作れなかった");

    let mut text = TextRenderer::new();
    text.write(&mut manager, &font, SAMPLE, &TextStyle::new(48.0), 0.0, 0.0)
        .expect("書けなかった");

    let mut checked = 0;

    for character in SAMPLE.chars() {
        let glyph = font.glyph(character).expect("見本の字は入っている");
        let outline = font.outline(glyph, DEFAULT_TOLERANCE).expect("形がある");

        // 形の名前は「Glyph <番号> ...」。番号で引き当てる。
        let prefix = format!("Glyph {} ", glyph.0);
        let name = text
            .shape_ids()
            .find(|name| name.starts_with(&prefix))
            .unwrap_or_else(|| panic!("`{character}` の形が登録されていない"));
        let object = manager.object(name).expect("登録した形がある");

        let wrong = mismatches(&outline, object.triangles());

        assert_eq!(wrong, 0, "`{character}` が {wrong} 点ずれている");
        checked += 1;
    }

    assert_eq!(checked, SAMPLE.chars().count());
}
