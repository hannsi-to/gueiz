//! 例で使う書体。**日本語の字を持つもの**を探す。
//!
//! 名札や説明に日本語を使うので、Arial では字が出ません（漢字も仮名も持たない）。
//! 機械ごとに入っている書体が違うので、候補を順に当たります。
//!
//! 英字も持っているので、`o` の穴や `i` の点を見る検証もそのまま通ります。

use gueiz_2d::font::Font;

/// 当たる順。Windows の標準書体を先に置く。
pub const CANDIDATES: &[&str] = &[
    "C:/Windows/Fonts/YuGothM.ttc",
    "C:/Windows/Fonts/meiryo.ttc",
    "C:/Windows/Fonts/msgothic.ttc",
    "C:/Windows/Fonts/NotoSansJP-VF.ttf",
    "/System/Library/Fonts/ヒラギノ角ゴシック W3.ttc",
    "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
];

/// 日本語の書体を読む。見つかった場所とバイト列を返す。
///
/// 読めても仮名を持たないものは飛ばす。どれも無ければ、探した場所を並べて落とす。
#[allow(dead_code)] // 書体を使わない例もこのモジュールを読み込む。
pub fn read_japanese_font() -> Result<(&'static str, Vec<u8>), String> {
    for path in CANDIDATES {
        let Ok(data) = std::fs::read(path) else {
            continue;
        };

        let has_kana = Font::from_bytes(&data).is_ok_and(|font| font.glyph('あ').is_some());

        if has_kana {
            return Ok((path, data));
        }
    }

    Err(format!(
        "日本語の書体が見つかりませんでした。探した場所:\n  {}",
        CANDIDATES.join("\n  "),
    ))
}

/// 日本語の書体がある場所。中身は読まずに返すので、置き場に預けるときに使う。
#[allow(dead_code)] // 書体を使わない例もこのモジュールを読み込む。
pub fn japanese_font_path() -> Result<&'static str, String> {
    read_japanese_font().map(|(path, _)| path)
}
