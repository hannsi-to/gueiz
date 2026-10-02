//! 色。
//!
//! # sRGB で持つ
//!
//! 中身は **sRGB の 0.0〜1.0**、不透明度は掛けていない素のままです。
//! [`crate::draw_manager`] が受け取るのと同じ約束なので、
//! [`Color::to_array`] で渡すだけで済みます。
//!
//! `#808080` と書けば画面でも `#808080` が出ます。線形への変換は
//! 頂点シェーダの入口で起きるので、ここでは考えません。
//!
//! # 混ぜるときだけ注意
//!
//! [`Color::lerp`] は **sRGB のまま**混ぜます。厳密には光の量で混ぜるべきで、
//! 黒と白の中間はやや暗く出ます。UI の色味の補間（押した状態へ寄せる、など）は
//! 見た目を揃えるほうが大事なので、ここでは sRGB のままにしています。
//!
//! 光として正しく混ぜたいなら [`Color::lerp_linear`] を使ってください。

/// sRGB の色。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Color {
    pub red: f32,
    pub green: f32,
    pub blue: f32,
    pub alpha: f32,
}

impl Color {
    pub const TRANSPARENT: Self = Self::rgba(0.0, 0.0, 0.0, 0.0);
    pub const BLACK: Self = Self::rgb(0.0, 0.0, 0.0);
    pub const WHITE: Self = Self::rgb(1.0, 1.0, 1.0);

    pub const fn rgba(red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        Self {
            red,
            green,
            blue,
            alpha,
        }
    }

    pub const fn rgb(red: f32, green: f32, blue: f32) -> Self {
        Self::rgba(red, green, blue, 1.0)
    }

    /// `0xRRGGBB`。不透明。
    ///
    /// ```
    /// # use gueiz_2d::gui::color::Color;
    /// let grey = Color::hex(0x808080);
    ///
    /// assert_eq!(grey.to_bytes(), [0x80, 0x80, 0x80, 0xff]);
    /// ```
    pub fn hex(value: u32) -> Self {
        Self::rgb(
            byte_at(value, 2),
            byte_at(value, 1),
            byte_at(value, 0),
        )
    }

    /// `0xRRGGBBAA`。
    pub fn hex_alpha(value: u32) -> Self {
        Self::rgba(
            byte_at(value, 3),
            byte_at(value, 2),
            byte_at(value, 1),
            byte_at(value, 0),
        )
    }

    pub fn gray(level: f32) -> Self {
        Self::rgb(level, level, level)
    }

    /// 不透明度だけ差し替える。
    pub fn with_alpha(self, alpha: f32) -> Self {
        Self { alpha, ..self }
    }

    /// 不透明度に掛ける。影や無効状態を薄くするとき。
    pub fn scale_alpha(self, factor: f32) -> Self {
        self.with_alpha((self.alpha * factor).clamp(0.0, 1.0))
    }

    /// まったく見えないか。描かなくてよいかの判断に使う。
    pub fn is_invisible(self) -> bool {
        self.alpha <= 0.0
    }

    /// sRGB のまま混ぜる。`t` が 0 で `self`、1 で `other`。
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| a + (b - a) * t;

        Self::rgba(
            mix(self.red, other.red),
            mix(self.green, other.green),
            mix(self.blue, other.blue),
            mix(self.alpha, other.alpha),
        )
    }

    /// 光の量に直してから混ぜ、sRGB へ戻す。
    ///
    /// 黒と白の中間が「ちょうど半分の明るさ」になります。
    /// 見た目の中間よりは明るく出ます。
    pub fn lerp_linear(self, other: Self, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        let mix = |a: f32, b: f32| {
            let linear = to_linear(a) + (to_linear(b) - to_linear(a)) * t;
            to_srgb(linear)
        };

        Self::rgba(
            mix(self.red, other.red),
            mix(self.green, other.green),
            mix(self.blue, other.blue),
            // 不透明度は光の量ではないので、そのまま混ぜる。
            self.alpha + (other.alpha - self.alpha) * t,
        )
    }

    /// `[r, g, b, a]`。[`crate::vertex::Vertex`] や
    /// [`crate::instance::Instance`] に渡す形。
    pub fn to_array(self) -> [f32; 4] {
        [self.red, self.green, self.blue, self.alpha]
    }

    /// `[R, G, B, A]` の 0〜255。
    pub fn to_bytes(self) -> [u8; 4] {
        [
            to_byte(self.red),
            to_byte(self.green),
            to_byte(self.blue),
            to_byte(self.alpha),
        ]
    }
}

impl From<[f32; 4]> for Color {
    fn from(value: [f32; 4]) -> Self {
        Self::rgba(value[0], value[1], value[2], value[3])
    }
}

impl From<Color> for [f32; 4] {
    fn from(value: Color) -> Self {
        value.to_array()
    }
}

fn byte_at(value: u32, index: u32) -> f32 {
    ((value >> (index * 8)) & 0xff) as f32 / 255.0
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// sRGB → 線形。
fn to_linear(value: f32) -> f32 {
    if value <= 0.040_45 {
        value / 12.92
    } else {
        ((value + 0.055) / 1.055).powf(2.4)
    }
}

/// 線形 → sRGB。
fn to_srgb(value: f32) -> f32 {
    if value <= 0.003_130_8 {
        value * 12.92
    } else {
        1.055 * value.powf(1.0 / 2.4) - 0.055
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_round_trips() {
        assert_eq!(Color::hex(0xff0000).to_bytes(), [255, 0, 0, 255]);
        assert_eq!(Color::hex_alpha(0x11223344).to_bytes(), [0x11, 0x22, 0x33, 0x44]);
    }

    #[test]
    fn srgb_lerp_is_the_plain_average() {
        let middle = Color::BLACK.lerp(Color::WHITE, 0.5);

        assert!((middle.red - 0.5).abs() < 1e-6);
    }

    #[test]
    fn linear_lerp_is_brighter_than_srgb_lerp() {
        let plain = Color::BLACK.lerp(Color::WHITE, 0.5);
        let light = Color::BLACK.lerp_linear(Color::WHITE, 0.5);

        // 光の量で半分は、sRGB では 0.5 より明るい（約 0.74）。
        assert!(light.red > plain.red + 0.2, "{} vs {}", light.red, plain.red);
    }

    #[test]
    fn alpha_scaling_clamps() {
        assert_eq!(Color::WHITE.scale_alpha(0.5).alpha, 0.5);
        assert_eq!(Color::WHITE.scale_alpha(5.0).alpha, 1.0);
        assert!(Color::TRANSPARENT.is_invisible());
    }
}
