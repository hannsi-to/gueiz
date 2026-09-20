//! 頂点座標をクリップ空間へ写す投影。

use crate::math::Mat4;

/// 窓の大きさが変わったとき、図形をどうするか。
///
/// # 絶対と相対
///
/// **絶対**（[`ScaleMode::Fixed`]）は、1 単位をいつも 1 画素に保ちます。
/// 窓を広げると**見える範囲が広がる**だけで、図形の大きさは変わりません。
/// 道具の画面や地図など、「実寸が意味を持つ」ものはこちらです。
///
/// **相対**（残り 4 つ）は、決めておいた**基準の大きさ**を窓に合わせて
/// 伸び縮みさせます。窓を広げると**図形も一緒に大きくなり**、
/// 見える範囲は変わりません。ゲームの画面や紙芝居のような、
/// 「絵の収まりが決まっている」ものはこちらです。
///
/// 相対が 4 つに分かれているのは、**縦横比が窓と合わないとき**に
/// 何を諦めるかが違うためです。
///
/// | | 縦横比 | はみ出す | 余る |
/// |---|---|---|---|
/// | [`ScaleMode::Stretch`] | 崩れる | しない | しない |
/// | [`ScaleMode::Fit`] | 保つ | しない | **する**（帯が出る） |
/// | [`ScaleMode::Fill`] | 保つ | **する** | しない |
/// | [`ScaleMode::Match`] | 保つ | 指定しだい | 指定しだい |
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug, Default)]
pub enum ScaleMode {
    /// **絶対。** 1 単位 = 1 画素のまま。窓を広げると見える範囲が広がる。
    #[default]
    Fixed,

    /// **相対。** 基準を窓いっぱいに引き伸ばす。
    ///
    /// 縦横比が合わないと**図形が歪みます**。丸が楕円になってよければ
    /// いちばん単純で、余りもはみ出しも出ません。
    Stretch,

    /// **相対。** 縦横比を保ったまま、基準が**全部入る**大きさにする。
    ///
    /// 合わないぶんは余白（帯）になります。切れては困るときに。
    Fit,

    /// **相対。** 縦横比を保ったまま、窓を**埋める**大きさにする。
    ///
    /// 合わないぶんは基準の外へはみ出して切れます。余白を出したくないときに。
    Fill,

    /// **相対。** 幅と高さのどちらに合わせるかを `0.0`〜`1.0` で選ぶ。
    ///
    /// `0.0` が幅ぴったり、`1.0` が高さぴったり、`0.5` がその中間。
    /// Unity の CanvasScaler の Match と同じ考え方です。
    /// 横に広がる窓では高さ寄りに、縦に伸びる窓では幅寄りにする、
    /// といった調整ができます。
    Match(f32),
}

impl ScaleMode {
    /// 幅に合わせる倍率と高さに合わせる倍率から、実際に使う倍率を選ぶ。
    ///
    /// [`ScaleMode::Fixed`] と [`ScaleMode::Stretch`] は 1 つの倍率で
    /// 表せないので、ここには来ない。
    fn uniform_scale(self, by_width: f32, by_height: f32) -> f32 {
        match self {
            Self::Fit => by_width.min(by_height),
            Self::Fill => by_width.max(by_height),

            Self::Match(amount) => {
                let amount = amount.clamp(0.0, 1.0);

                // 対数で混ぜる。そのまま線形に混ぜると、倍率が
                // 2 倍と 8 倍のときの「中間」が 5 倍になって、
                // 見た目の中間（4 倍）からずれる。
                by_width.powf(1.0 - amount) * by_height.powf(amount)
            }

            Self::Fixed | Self::Stretch => 1.0,
        }
    }

    /// 窓に合わせて図形の大きさが変わるか。
    pub fn is_relative(self) -> bool {
        !matches!(self, Self::Fixed)
    }
}

/// 2D 用のカメラ。
///
/// 既定は「左上原点・ピクセル単位」の正射影なので、`(0, 0)` から
/// `(width, height)` の座標で図形を組める。
///
/// ```no_run
/// # use gueiz_gpu::camera::Camera;
/// let camera = Camera::orthographic_2d(1280.0, 720.0);
/// ```
///
/// # 窓の大きさが変わったとき
///
/// [`Camera::resize`] を呼ぶと投影を貼り替えます。**そのとき図形を
/// どうするかは [`ScaleMode`] が決めます。** 既定は絶対
/// （[`ScaleMode::Fixed`]）で、1 単位 = 1 画素のままです。
///
/// ```no_run
/// # use gueiz_gpu::camera::{Camera, ScaleMode};
/// // 1280x720 で絵を組んで、窓に合わせて丸ごと拡大する。
/// let mut camera = Camera::orthographic_2d(1280.0, 720.0).with_scale_mode(ScaleMode::Fit);
///
/// // 窓が変わったらこれだけ。基準の座標系は 1280x720 のまま使える。
/// camera.resize(1920.0, 1080.0);
/// ```
///
/// # 図形への貼り直し
///
/// カメラは [`crate::camera::Camera`] を**図形ごとに持ちます**。
/// 貼り替えたら、図形に置き直す必要があります。
/// [`ScaleMode::Stretch`] のように投影が窓に依存しない使い方なら、
/// そもそも貼り直しが要りません。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Camera {
    projection: Mat4,
    view: Mat4,
    /// 絵を組むときの基準の大きさ。相対のときだけ意味を持つ。
    design: [f32; 2],
    /// いまの窓の大きさ。画素。
    viewport: [f32; 2],
    scale_mode: ScaleMode,
}

impl Camera {
    /// 左上を原点としたピクセル座標系のカメラ。
    ///
    /// 渡した大きさが**基準**になります。[`ScaleMode`] を相対にすると、
    /// この大きさが窓に合わせて伸び縮みします。
    ///
    /// **z は捨てられます。** 2D では奥行きではなく重なり順にしか使わないので、
    /// どんな値を入れてもクリップ空間では 0 に潰れます。重なり順は
    /// `Object::z` と描く順で決まります。
    pub fn orthographic_2d(width: f32, height: f32) -> Self {
        Self {
            projection: Mat4::orthographic_2d(width, height),
            view: Mat4::IDENTITY,
            design: [width, height],
            viewport: [width, height],
            scale_mode: ScaleMode::Fixed,
        }
    }

    /// 投影を直接指定する。
    ///
    /// 基準の大きさを持たないので、[`Camera::resize`] を呼ぶと
    /// 指定した投影は捨てられ、ピクセル単位の正射影に貼り替わります。
    pub fn from_projection(projection: Mat4) -> Self {
        Self {
            projection,
            view: Mat4::IDENTITY,
            design: [0.0; 2],
            viewport: [0.0; 2],
            scale_mode: ScaleMode::Fixed,
        }
    }

    /// 窓に合わせ方を決めて返す。
    pub fn with_scale_mode(mut self, scale_mode: ScaleMode) -> Self {
        self.set_scale_mode(scale_mode);
        self
    }

    /// 窓に合わせ方を変える。投影はその場で貼り替わる。
    pub fn set_scale_mode(&mut self, scale_mode: ScaleMode) -> &mut Self {
        self.scale_mode = scale_mode;
        self.rebuild();
        self
    }

    pub fn scale_mode(&self) -> ScaleMode {
        self.scale_mode
    }

    /// 絵を組むときの基準の大きさを変える。
    pub fn set_design_size(&mut self, width: f32, height: f32) -> &mut Self {
        self.design = [width, height];
        self.rebuild();
        self
    }

    pub fn design_size(&self) -> [f32; 2] {
        self.design
    }

    pub fn viewport(&self) -> [f32; 2] {
        self.viewport
    }

    /// ウィンドウサイズが変わったときに投影を貼り替える。
    ///
    /// 何が起きるかは [`ScaleMode`] しだいです。
    pub fn resize(&mut self, width: f32, height: f32) {
        self.viewport = [width, height];
        self.rebuild();
    }

    /// 基準 1 単位が画面の何画素になるか。
    ///
    /// 絶対ならいつも 1.0。相対なら窓の大きさで変わります。
    /// [`ScaleMode::Stretch`] は縦横で違うので、2 つ返します。
    pub fn scale_factor(&self) -> [f32; 2] {
        let Some((design, viewport)) = self.sizes() else {
            return [1.0; 2];
        };

        match self.scale_mode {
            ScaleMode::Fixed => [1.0; 2],
            ScaleMode::Stretch => [viewport[0] / design[0], viewport[1] / design[1]],

            other => {
                let scale = other.uniform_scale(viewport[0] / design[0], viewport[1] / design[1]);
                [scale, scale]
            }
        }
    }

    /// 視点を動かす。図形はこれの逆向きに動く。
    pub fn look_at(&mut self, x: f32, y: f32) {
        self.view = Mat4::from_translation(-x, -y, 0.0);
    }

    /// 拡大する。1.0 が等倍。
    pub fn zoom(&mut self, factor: f32) {
        self.view = Mat4::from_scale(factor, factor, 1.0).multiply(self.view);
    }

    /// 頂点に掛ける行列。`projection * view`。
    pub fn view_projection(&self) -> Mat4 {
        self.projection.multiply(self.view)
    }

    /// 画面の画素位置を、図形を組んだ座標へ戻す。
    ///
    /// # なぜ要るか
    ///
    /// 相対にすると、**マウスの画素位置と図形の座標が一致しなくなります。**
    /// 当たり判定やつまみ操作は、ここを通してから比べます。
    /// 絶対（[`ScaleMode::Fixed`]）ならそのまま素通りします。
    ///
    /// 投影が潰れている（窓の大きさが 0、遠近投影など）と `None`。
    ///
    /// ```
    /// # use gueiz_gpu::camera::{Camera, ScaleMode};
    /// let mut camera = Camera::orthographic_2d(800.0, 600.0)
    ///     .with_scale_mode(ScaleMode::Stretch);
    /// camera.resize(1600.0, 1200.0);
    ///
    /// // 窓の真ん中を押したら、基準の真ん中。
    /// let [x, y] = camera.screen_to_world(800.0, 600.0).expect("戻せる");
    ///
    /// assert!((x - 400.0).abs() < 1e-3 && (y - 300.0).abs() < 1e-3);
    /// ```
    pub fn screen_to_world(&self, x: f32, y: f32) -> Option<[f32; 2]> {
        let [width, height] = self.viewport;

        if width <= 0.0 || height <= 0.0 {
            return None;
        }

        // 画素 → クリップ空間。y は上向きなので反転する。
        let clip_x = x / width * 2.0 - 1.0;
        let clip_y = 1.0 - y / height * 2.0;

        let columns = self.view_projection().to_columns();

        // 2D では x と y にしか効かないので、左上の 2x2 だけ戻せばよい。
        let (a, b) = (columns[0][0], columns[0][1]);
        let (c, d) = (columns[1][0], columns[1][1]);
        let (tx, ty) = (columns[3][0], columns[3][1]);

        let determinant = a * d - b * c;

        if determinant.abs() < f32::EPSILON {
            return None;
        }

        let (dx, dy) = (clip_x - tx, clip_y - ty);

        Some([
            (d * dx - c * dy) / determinant,
            (a * dy - b * dx) / determinant,
        ])
    }

    /// 図形の座標を、画面の画素位置へ写す。[`Camera::screen_to_world`] の逆。
    pub fn world_to_screen(&self, x: f32, y: f32) -> [f32; 2] {
        let [clip_x, clip_y, _] = self.view_projection().project_point(x, y, 0.0);
        let [width, height] = self.viewport;

        [(clip_x + 1.0) / 2.0 * width, (1.0 - clip_y) / 2.0 * height]
    }

    /// 基準と窓の大きさ。どちらも使える値のときだけ返す。
    ///
    /// 基準を持たないカメラ（[`Camera::from_projection`] など）は、
    /// 窓の大きさをそのまま基準として扱う。そうすれば絶対と同じになる。
    fn sizes(&self) -> Option<([f32; 2], [f32; 2])> {
        let viewport = self.viewport;

        if viewport[0] <= 0.0 || viewport[1] <= 0.0 {
            return None;
        }

        let design = if self.design[0] > 0.0 && self.design[1] > 0.0 {
            self.design
        } else {
            viewport
        };

        Some((design, viewport))
    }

    fn rebuild(&mut self) {
        // 大きさが分からないうちは、いま入っている投影のままにしておく。
        let Some((design, viewport)) = self.sizes() else {
            return;
        };

        self.projection = match self.scale_mode {
            // 1 単位 = 1 画素。窓が広いほど見える範囲が広い。
            ScaleMode::Fixed => Mat4::orthographic_2d(viewport[0], viewport[1]),

            // 基準をそのまま窓の四隅へ。縦横別々に伸びるので歪む。
            ScaleMode::Stretch => Mat4::orthographic_2d(design[0], design[1]),

            // 縦横同じ倍率。合わないぶんは余るか、はみ出す。
            other => {
                let scale =
                    other.uniform_scale(viewport[0] / design[0], viewport[1] / design[1]);

                // この倍率で窓に入る範囲を、基準の単位で測る。
                let visible = [viewport[0] / scale, viewport[1] / scale];

                // 基準を真ん中に置く。余りは左右（上下）に等分される。
                let offset = [
                    (visible[0] - design[0]) / 2.0,
                    (visible[1] - design[1]) / 2.0,
                ];

                Mat4::orthographic_2d(visible[0], visible[1])
                    .multiply(Mat4::from_translation(offset[0], offset[1], 0.0))
            }
        };
    }
}

impl Default for Camera {
    /// クリップ空間をそのまま使う（`-1..1`）。
    fn default() -> Self {
        Self {
            projection: Mat4::IDENTITY,
            view: Mat4::IDENTITY,
            design: [0.0; 2],
            viewport: [0.0; 2],
            scale_mode: ScaleMode::Fixed,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 図形の見た目の大きさを、画面の画素で測る。
    ///
    /// 基準の `size` 単位ぶんが何画素になるか。
    fn painted_pixels(camera: &Camera, size: f32) -> [f32; 2] {
        let start = camera.world_to_screen(0.0, 0.0);
        let end = camera.world_to_screen(size, size);

        [end[0] - start[0], end[1] - start[1]]
    }

    /// 画素の大きさを比べる。行列を通すので、ぴったり一致はしない。
    #[track_caller]
    fn assert_pixels(actual: [f32; 2], expected: [f32; 2]) {
        assert!(
            (actual[0] - expected[0]).abs() < 1e-3 && (actual[1] - expected[1]).abs() < 1e-3,
            "{actual:?} != {expected:?}",
        );
    }

    #[test]
    fn maps_pixels_to_clip_space() {
        let camera = Camera::orthographic_2d(800.0, 600.0);
        let matrix = camera.view_projection();

        assert_eq!(matrix.transform_point(0.0, 0.0, 0.0), [-1.0, 1.0, 0.0]);
        assert_eq!(matrix.transform_point(800.0, 600.0, 0.0), [1.0, -1.0, 0.0]);
    }

    #[test]
    fn look_at_moves_the_world_the_other_way() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0);
        camera.look_at(400.0, 300.0);

        // 視点を画面中央へ動かすと、中央にあった点が原点に来る。
        let [x, y, _] = camera.view_projection().transform_point(400.0, 300.0, 0.0);

        assert!((x - -1.0).abs() < 1e-6, "x was {x}");
        assert!((y - 1.0).abs() < 1e-6, "y was {y}");
    }

    #[test]
    fn the_default_camera_is_clip_space() {
        let matrix = Camera::default().view_projection();

        assert_eq!(matrix.transform_point(0.5, -0.5, 0.0), [0.5, -0.5, 0.0]);
    }

    /// 既定は絶対。今までの使い方が変わっていないこと。
    #[test]
    fn the_default_mode_is_absolute() {
        let camera = Camera::orthographic_2d(800.0, 600.0);

        assert_eq!(camera.scale_mode(), ScaleMode::Fixed);
        assert!(!camera.scale_mode().is_relative());
    }

    // --- 絶対 ---

    /// 絶対なら、窓を広げても図形の画素の大きさは変わらない。
    #[test]
    fn absolute_keeps_the_painted_size() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0);
        let before = painted_pixels(&camera, 100.0);

        camera.resize(1600.0, 1200.0);

        assert_pixels(painted_pixels(&camera, 100.0), before);
        assert_pixels(painted_pixels(&camera, 100.0), [100.0, 100.0]);
        assert_eq!(camera.scale_factor(), [1.0, 1.0]);
    }

    /// 代わりに、見える範囲が広がる。
    #[test]
    fn absolute_shows_more_of_the_world() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0);
        camera.resize(1600.0, 1200.0);

        // 前は右下の隅だった点が、いまは真ん中。
        let [x, y, _] = camera.view_projection().transform_point(800.0, 600.0, 0.0);

        assert!(x.abs() < 1e-6 && y.abs() < 1e-6, "{x} {y}");
    }

    // --- 相対 ---

    /// 相対なら、窓を広げたぶんだけ図形も大きくなる。
    #[test]
    fn relative_grows_the_painted_size() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Stretch);
        camera.resize(1600.0, 1200.0);

        assert_pixels(painted_pixels(&camera, 100.0), [200.0, 200.0]);
        assert_eq!(camera.scale_factor(), [2.0, 2.0]);
        assert!(camera.scale_mode().is_relative());
    }

    /// 見える範囲は変わらない。基準の四隅がいつも窓の四隅。
    #[test]
    fn relative_shows_the_same_world() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Stretch);
        camera.resize(1920.0, 480.0);

        let matrix = camera.view_projection();

        assert_eq!(matrix.transform_point(0.0, 0.0, 0.0), [-1.0, 1.0, 0.0]);
        assert_eq!(matrix.transform_point(800.0, 600.0, 0.0), [1.0, -1.0, 0.0]);
    }

    /// 引き伸ばしは縦横比が崩れる。そこが他の相対との違い。
    #[test]
    fn stretch_distorts_when_the_shape_of_the_window_changes() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Stretch);
        // 横に 2 倍だけ広げる。
        camera.resize(1600.0, 600.0);

        // 横だけ 2 倍、縦は等倍。
        assert_pixels(painted_pixels(&camera, 100.0), [200.0, 100.0]);
    }

    /// 収める方は縦横比を保つ。合わないぶんは余白になる。
    #[test]
    fn fit_keeps_the_shape_and_leaves_a_gap() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Fit);
        camera.resize(1600.0, 600.0);

        // 高さに合わせるので倍率は 1。縦横とも同じ。
        assert_pixels(painted_pixels(&camera, 100.0), [100.0, 100.0]);

        // 基準は真ん中に来て、左右に 400 画素ずつ余る。
        let top_left = camera.world_to_screen(0.0, 0.0);
        let bottom_right = camera.world_to_screen(800.0, 600.0);

        assert!((top_left[0] - 400.0).abs() < 1e-3, "{:?}", top_left);
        assert!((bottom_right[0] - 1200.0).abs() < 1e-3, "{:?}", bottom_right);
        // 高さは使い切る。
        assert!(top_left[1].abs() < 1e-3 && (bottom_right[1] - 600.0).abs() < 1e-3);
    }

    /// 埋める方も縦横比を保つが、余白を出さずにはみ出す。
    #[test]
    fn fill_keeps_the_shape_and_overflows() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Fill);
        camera.resize(1600.0, 600.0);

        // 幅に合わせるので倍率は 2。
        assert_pixels(painted_pixels(&camera, 100.0), [200.0, 200.0]);

        // 幅は使い切り、高さは上下にはみ出す。
        let top_left = camera.world_to_screen(0.0, 0.0);
        let bottom_right = camera.world_to_screen(800.0, 600.0);

        assert!(top_left[0].abs() < 1e-3, "{:?}", top_left);
        assert!((bottom_right[0] - 1600.0).abs() < 1e-3, "{:?}", bottom_right);
        assert!(top_left[1] < 0.0, "上にはみ出すはず: {:?}", top_left);
        assert!(bottom_right[1] > 600.0, "下にはみ出すはず: {:?}", bottom_right);
    }

    /// 合わせ先を選べる。0 が幅、1 が高さ。
    #[test]
    fn match_picks_which_side_to_follow() {
        let resized = |mode| {
            let mut camera = Camera::orthographic_2d(800.0, 600.0).with_scale_mode(mode);
            camera.resize(1600.0, 600.0);
            camera.scale_factor()[0]
        };

        // 幅に合わせれば 2 倍、高さに合わせれば 1 倍。
        assert!((resized(ScaleMode::Match(0.0)) - 2.0).abs() < 1e-5);
        assert!((resized(ScaleMode::Match(1.0)) - 1.0).abs() < 1e-5);

        // 端に寄せたときは、埋める・収めると同じになる。
        assert!((resized(ScaleMode::Match(0.0)) - resized(ScaleMode::Fill)).abs() < 1e-5);
        assert!((resized(ScaleMode::Match(1.0)) - resized(ScaleMode::Fit)).abs() < 1e-5);
    }

    /// 中間は 2 つの倍率のあいだに入る。対数で混ぜているので、
    /// 半分なら幾何平均。
    #[test]
    fn match_lands_between_the_two() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Match(0.5));
        camera.resize(1600.0, 600.0);

        let scale = camera.scale_factor()[0];

        assert!((scale - 2.0_f32.sqrt()).abs() < 1e-5, "{scale}");
        assert!(scale > 1.0 && scale < 2.0);
    }

    /// 外れた値でも落ちない。端に寄せて扱う。
    #[test]
    fn match_clamps_out_of_range_values() {
        let resized = |amount| {
            let mut camera =
                Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Match(amount));
            camera.resize(1600.0, 600.0);
            camera.scale_factor()[0]
        };

        assert_eq!(resized(-5.0), resized(0.0));
        assert_eq!(resized(9.0), resized(1.0));
    }

    /// 合わせ方を後から変えても、その場で効くこと。
    #[test]
    fn the_mode_can_be_switched_after_the_fact() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0);
        camera.resize(1600.0, 1200.0);

        assert_pixels(painted_pixels(&camera, 100.0), [100.0, 100.0]);

        camera.set_scale_mode(ScaleMode::Stretch);

        assert_pixels(painted_pixels(&camera, 100.0), [200.0, 200.0]);
    }

    /// 基準の大きさも後から変えられる。
    #[test]
    fn the_design_size_can_be_changed() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Stretch);
        camera.resize(1600.0, 1200.0);
        camera.set_design_size(1600.0, 1200.0);

        assert_eq!(camera.design_size(), [1600.0, 1200.0]);
        // 基準と窓が同じなら等倍。
        assert_eq!(camera.scale_factor(), [1.0, 1.0]);
    }

    // --- 画面から座標へ ---

    /// 絶対なら画素位置がそのまま座標。
    #[test]
    fn absolute_needs_no_conversion() {
        let camera = Camera::orthographic_2d(800.0, 600.0);
        let [x, y] = camera.screen_to_world(123.0, 456.0).expect("戻せる");

        assert!((x - 123.0).abs() < 1e-3 && (y - 456.0).abs() < 1e-3, "{x} {y}");
    }

    /// 相対では一致しない。ここを通さないと当たり判定がずれる。
    #[test]
    fn relative_needs_the_conversion() {
        let mut camera =
            Camera::orthographic_2d(800.0, 600.0).with_scale_mode(ScaleMode::Stretch);
        camera.resize(1600.0, 1200.0);

        let [x, y] = camera.screen_to_world(800.0, 600.0).expect("戻せる");

        assert!((x - 400.0).abs() < 1e-3 && (y - 300.0).abs() < 1e-3, "{x} {y}");
    }

    /// 行って戻ると元に戻ること。どの合わせ方でも。
    #[test]
    fn screen_and_world_round_trip() {
        for mode in [
            ScaleMode::Fixed,
            ScaleMode::Stretch,
            ScaleMode::Fit,
            ScaleMode::Fill,
            ScaleMode::Match(0.3),
        ] {
            let mut camera = Camera::orthographic_2d(800.0, 600.0).with_scale_mode(mode);
            camera.resize(1920.0, 720.0);

            let [x, y] = camera.screen_to_world(321.0, 654.0).expect("戻せる");
            let [back_x, back_y] = camera.world_to_screen(x, y);

            assert!(
                (back_x - 321.0).abs() < 1e-2 && (back_y - 654.0).abs() < 1e-2,
                "{mode:?}: {back_x} {back_y}",
            );
        }
    }

    /// 視点を動かしたぶんも戻せること。
    #[test]
    fn the_conversion_follows_the_view() {
        let mut camera = Camera::orthographic_2d(800.0, 600.0);
        camera.look_at(100.0, 50.0);

        let [x, y] = camera.screen_to_world(0.0, 0.0).expect("戻せる");

        assert!((x - 100.0).abs() < 1e-3 && (y - 50.0).abs() < 1e-3, "{x} {y}");
    }

    /// 大きさが決まっていなければ戻せない。黙って 0 を返すより分かりやすい。
    #[test]
    fn a_sizeless_camera_cannot_convert() {
        assert_eq!(Camera::default().screen_to_world(10.0, 10.0), None);
    }

    /// 基準を持たないカメラに窓の大きさを教えたら、絶対として振る舞う。
    #[test]
    fn a_camera_without_a_design_size_falls_back_to_absolute() {
        let mut camera = Camera::from_projection(Mat4::IDENTITY);
        camera.resize(800.0, 600.0);

        let matrix = camera.view_projection();

        assert_eq!(matrix.transform_point(0.0, 0.0, 0.0), [-1.0, 1.0, 0.0]);
        assert_eq!(matrix.transform_point(800.0, 600.0, 0.0), [1.0, -1.0, 0.0]);
    }
}
