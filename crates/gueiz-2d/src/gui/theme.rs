//! 見た目の値を引くところ。
//!
//! # ここにあるのは入れ物だけ
//!
//! 「どの色か」「どれだけ丸いか」は**デザインが決めること**なので、
//! このモジュールは決めません。決めるのは**引き方**だけです。
//!
//! ```text
//! (役 Role, 状態 StateKey) ──▶ Style
//! ```
//!
//! ウィジェットは「自分は押せるものだ」（[`Role`]）とだけ言い、
//! 色は [`Theme`] に聞きます。こうしておくと、**ウィジェットを書き換えずに
//! 色だけ差し替えられます**。明るい配色と暗い配色を切り替えるのも、
//! [`Theme`] を 1 つ入れ替えるだけになります。
//!
//! # 引けなかったときは落ちていく
//!
//! ```text
//! (役, その状態) ──無ければ──▶ (役, Normal) ──無ければ──▶ 既定の Style
//! ```
//!
//! 全部の状態を埋める必要はありません。「押したときだけ色を変える」なら
//! `(CONTROL, Normal)` と `(CONTROL, Pressed)` の 2 つで済みます。
//!
//! # 役は好きに増やせる
//!
//! [`Role`] は番号の付いた札です。組み込みは
//! [`Role::SURFACE`] / [`Role::CONTROL`] / [`Role::TEXT`] / [`Role::ACCENT`] の
//! 4 つだけで、足りなければ [`Role::custom`] で増やしてください。
//! 列挙にしていないのは、**この箱を書き換えずに役を増やせる**ようにするためです。

use fxhash::FxHashMap;

use crate::gui::color::Color;
use crate::gui::geometry::{Corners, Insets};
use crate::gui::widget::WidgetState;

/// 何の役をするウィジェットか。
///
/// 番号そのものに意味はありません。[`Theme`] の鍵としてだけ使います。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub struct Role(pub u32);

impl Role {
    /// 背景・パネル・窓。
    pub const SURFACE: Self = Self(0);
    /// 押せるもの。
    pub const CONTROL: Self = Self(1);
    /// 文字。
    pub const TEXT: Self = Self(2);
    /// 目立たせるもの。
    pub const ACCENT: Self = Self(3);

    /// 組み込みとぶつからない番号から始まる自前の役。
    ///
    /// ```
    /// # use gueiz_2d::gui::theme::Role;
    /// const TOOLTIP: Role = Role::custom(0);
    /// const BADGE: Role = Role::custom(1);
    ///
    /// assert_ne!(TOOLTIP, Role::CONTROL);
    /// assert_ne!(TOOLTIP, BADGE);
    /// ```
    pub const fn custom(index: u32) -> Self {
        Self(CUSTOM_ROLE_BASE + index)
    }
}

/// [`Role::custom`] がここから始まる。
pub const CUSTOM_ROLE_BASE: u32 = 1 << 16;

/// [`Theme`] を引くときの状態。
///
/// [`WidgetState`] は 4 つの真偽値の組み合わせなので 16 通りありますが、
/// 色を変えたいのは**いちばん強い 1 つ**です。優先順は
///
/// ```text
/// Disabled ▶ Pressed ▶ Hovered ▶ Focused ▶ Normal
/// ```
///
/// 無効がいちばん強いのは、押せないものが押されたように見えてはいけないからです。
/// 焦点がいちばん弱いのは、焦点は**乗っているあいだも持ち続ける**ので、
/// これを優先すると乗った見た目が出なくなるからです。
#[derive(Clone, Copy)]
#[derive(Eq, PartialEq)]
#[derive(Hash)]
#[derive(Debug, Default)]
pub enum StateKey {
    #[default]
    Normal,
    Hovered,
    Pressed,
    Focused,
    Disabled,
}

impl From<WidgetState> for StateKey {
    fn from(value: WidgetState) -> Self {
        if value.disabled {
            Self::Disabled
        } else if value.pressed {
            Self::Pressed
        } else if value.hovered {
            Self::Hovered
        } else if value.focused {
            Self::Focused
        } else {
            Self::Normal
        }
    }
}

/// ある役・ある状態での見た目。
///
/// **足りなければ増やしてください。** ここに並んでいるのは
/// 「どのウィジェットでも要りそうなもの」だけです。
/// 特定のウィジェットしか使わない値は、そのウィジェットに持たせたほうが
/// 引き回しが短くなります。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub struct Style {
    /// 塗り。
    pub background: Color,
    /// 枠線の色。
    pub border: Color,
    /// 枠線の太さ。0 なら引かない。
    pub border_width: f32,
    pub corners: Corners,
    /// 文字と図形の色。
    pub foreground: Color,
    /// 文字の大きさ。ピクセル。
    pub text_size: f32,
    /// 内側の余白。
    pub padding: Insets,
}

impl Style {
    /// 何も描かない、文字だけの型。
    pub const BARE: Self = Self {
        background: Color::TRANSPARENT,
        border: Color::TRANSPARENT,
        border_width: 0.0,
        corners: Corners::ZERO,
        foreground: Color::WHITE,
        text_size: 16.0,
        padding: Insets::ZERO,
    };

    pub fn background(mut self, background: Color) -> Self {
        self.background = background;
        self
    }

    pub fn border(mut self, border: Color, width: f32) -> Self {
        self.border = border;
        self.border_width = width;
        self
    }

    pub fn corners(mut self, corners: Corners) -> Self {
        self.corners = corners;
        self
    }

    pub fn foreground(mut self, foreground: Color) -> Self {
        self.foreground = foreground;
        self
    }

    pub fn text_size(mut self, text_size: f32) -> Self {
        self.text_size = text_size;
        self
    }

    pub fn padding(mut self, padding: Insets) -> Self {
        self.padding = padding;
        self
    }

    /// 2 つのあいだを混ぜる。乗ったときの色へ滑らかに寄せるときなど。
    ///
    /// 丸みと余白も混ぜます。**太さ 0 の枠線は色も混ぜません**
    /// （透明から色が湧いて見えるのを避けるため）。
    pub fn lerp(self, other: Self, t: f32) -> Self {
        let mix = |a: f32, b: f32| a + (b - a) * t.clamp(0.0, 1.0);

        Self {
            background: self.background.lerp(other.background, t),
            border: self.border.lerp(other.border, t),
            border_width: mix(self.border_width, other.border_width),
            corners: Corners::new(
                mix(self.corners.top_left, other.corners.top_left),
                mix(self.corners.top_right, other.corners.top_right),
                mix(self.corners.bottom_right, other.corners.bottom_right),
                mix(self.corners.bottom_left, other.corners.bottom_left),
            ),
            foreground: self.foreground.lerp(other.foreground, t),
            text_size: mix(self.text_size, other.text_size),
            padding: Insets::new(
                mix(self.padding.top, other.padding.top),
                mix(self.padding.right, other.padding.right),
                mix(self.padding.bottom, other.padding.bottom),
                mix(self.padding.left, other.padding.left),
            ),
        }
    }
}

/// 寸法の目安。
///
/// 色と違って状態で変わらないものを置きます。レイアウトが参照するので、
/// ここを変えると**測り直しが要ります**
/// （[`Gui::set_theme`](crate::gui::Gui::set_theme) が印を立てます）。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Metrics {
    /// 余白の基本の 1 つぶん。`spacing * 2` のように倍で使う。
    pub spacing: f32,
    /// 文字の既定の大きさ。
    pub text_size: f32,
    /// 押せるものの最小の高さ。指で押せる大きさの下限。
    pub control_height: f32,
    /// 行送り。文字の大きさに対する倍率。
    pub line_height_factor: f32,
}

impl Default for Metrics {
    fn default() -> Self {
        Self {
            spacing: 8.0,
            text_size: 16.0,
            control_height: 32.0,
            line_height_factor: 1.4,
        }
    }
}

impl Metrics {
    /// `steps` 個ぶんの余白。
    pub fn space(&self, steps: f32) -> f32 {
        self.spacing * steps
    }

    /// その大きさの文字の行送り。
    pub fn line_height(&self, text_size: f32) -> f32 {
        text_size * self.line_height_factor
    }
}

/// 見た目の表。
///
/// ```
/// # use gueiz_2d::gui::color::Color;
/// # use gueiz_2d::gui::theme::{Role, StateKey, Style, Theme};
/// # use gueiz_2d::gui::widget::WidgetState;
/// let mut theme = Theme::new();
///
/// theme.set(Role::CONTROL, StateKey::Normal, Style::BARE.background(Color::hex(0x334455)));
/// theme.set(Role::CONTROL, StateKey::Hovered, Style::BARE.background(Color::hex(0x445566)));
///
/// let hovered = WidgetState { hovered: true, ..Default::default() };
/// assert_eq!(theme.style(Role::CONTROL, hovered).background, Color::hex(0x445566));
///
/// // 押したときは登録していないので Normal に落ちる。
/// let pressed = WidgetState { pressed: true, ..Default::default() };
/// assert_eq!(theme.style(Role::CONTROL, pressed).background, Color::hex(0x334455));
/// ```
#[derive(Clone)]
#[derive(Debug, Default)]
pub struct Theme {
    styles: FxHashMap<(Role, StateKey), Style>,
    fallback: Style,
    metrics: Metrics,
}

impl Theme {
    /// 空の表。引いても [`Theme::fallback`] が返る。
    pub fn new() -> Self {
        Self {
            styles: FxHashMap::default(),
            fallback: Style::BARE,
            metrics: Metrics::default(),
        }
    }

    pub fn metrics(&self) -> &Metrics {
        &self.metrics
    }

    pub fn set_metrics(&mut self, metrics: Metrics) -> &mut Self {
        self.metrics = metrics;
        self
    }

    /// どれにも当たらなかったときの型。
    pub fn fallback(&self) -> Style {
        self.fallback
    }

    pub fn set_fallback(&mut self, fallback: Style) -> &mut Self {
        self.fallback = fallback;
        self
    }

    /// 1 つ登録する。
    pub fn set(&mut self, role: Role, state: StateKey, style: Style) -> &mut Self {
        self.styles.insert((role, state), style);
        self
    }

    /// その役の全部の状態に同じものを敷く。差分だけ後から上書きする使い方。
    pub fn set_all(&mut self, role: Role, style: Style) -> &mut Self {
        for state in [
            StateKey::Normal,
            StateKey::Hovered,
            StateKey::Pressed,
            StateKey::Focused,
            StateKey::Disabled,
        ] {
            self.styles.insert((role, state), style);
        }

        self
    }

    /// 引く。無ければ `Normal`、それも無ければ [`Theme::fallback`]。
    pub fn style(&self, role: Role, state: impl Into<StateKey>) -> Style {
        let state = state.into();

        self.styles
            .get(&(role, state))
            .or_else(|| self.styles.get(&(role, StateKey::Normal)))
            .copied()
            .unwrap_or(self.fallback)
    }

    /// 登録されているものだけを引く。落ちていかない。
    pub fn style_exact(&self, role: Role, state: StateKey) -> Option<Style> {
        self.styles.get(&(role, state)).copied()
    }

    /// 登録されている数。
    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_wins_over_everything() {
        let state = WidgetState {
            hovered: true,
            pressed: true,
            focused: true,
            disabled: true,
        };

        assert_eq!(StateKey::from(state), StateKey::Disabled);
    }

    #[test]
    fn focus_is_the_weakest() {
        // 焦点は乗っているあいだも続くので、乗った見た目を潰してはいけない。
        let hovered_and_focused = WidgetState {
            hovered: true,
            focused: true,
            ..Default::default()
        };

        assert_eq!(StateKey::from(hovered_and_focused), StateKey::Hovered);

        let only_focused = WidgetState {
            focused: true,
            ..Default::default()
        };
        assert_eq!(StateKey::from(only_focused), StateKey::Focused);
    }

    #[test]
    fn lookup_falls_back_to_normal_then_to_the_default() {
        let mut theme = Theme::new();
        let normal = Style::BARE.background(Color::WHITE);

        theme.set(Role::CONTROL, StateKey::Normal, normal);

        assert_eq!(theme.style(Role::CONTROL, StateKey::Pressed), normal);
        // 役ごと登録が無ければ既定。
        assert_eq!(theme.style(Role::ACCENT, StateKey::Normal), theme.fallback());
        assert_eq!(theme.style_exact(Role::CONTROL, StateKey::Pressed), None);
    }

    #[test]
    fn set_all_then_override_one() {
        let mut theme = Theme::new();

        theme.set_all(Role::CONTROL, Style::BARE.background(Color::BLACK));
        theme.set(
            Role::CONTROL,
            StateKey::Pressed,
            Style::BARE.background(Color::WHITE),
        );

        assert_eq!(theme.style(Role::CONTROL, StateKey::Hovered).background, Color::BLACK);
        assert_eq!(theme.style(Role::CONTROL, StateKey::Pressed).background, Color::WHITE);
        assert_eq!(theme.len(), 5);
    }

    #[test]
    fn custom_roles_never_collide_with_built_ins() {
        assert!(Role::custom(0).0 > Role::ACCENT.0);
        assert_eq!(Role::custom(5).0, CUSTOM_ROLE_BASE + 5);
    }

    #[test]
    fn style_lerp_moves_every_field() {
        let from = Style::BARE.background(Color::BLACK).text_size(10.0);
        let to = Style::BARE.background(Color::WHITE).text_size(20.0);

        let middle = from.lerp(to, 0.5);

        assert!((middle.text_size - 15.0).abs() < 1e-6);
        assert!((middle.background.red - 0.5).abs() < 1e-6);
    }

    #[test]
    fn metrics_scale_spacing_and_line_height() {
        let metrics = Metrics::default();

        assert_eq!(metrics.space(2.0), metrics.spacing * 2.0);
        assert!((metrics.line_height(10.0) - 14.0).abs() < 1e-6);
    }
}
