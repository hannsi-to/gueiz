use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;

pub struct CreateObjectArguments<'a> {
    pub register_name: &'a String,
    pub counter: i32,
    pub item_x: f32,
    pub item_y: f32,
    pub width: f32,
    pub surface_size: SurfaceSize,
    pub scale_mode: ScaleMode,
    pub design: [f32; 2],
}

/// 窓の中に置く部品。
///
/// # 座標
///
/// `item_x` / `item_y` は**窓を組んだときの左上を基準にした**座標です。窓を動かしても
/// 変わりません。窓を動かしたぶんは、窓が [`WindowItem::object_names`] の図形すべてに
/// [`Object::translate`](crate::object::Object::translate) で掛けます。部品の側で
/// 置き場所をずらす必要はありません。
///
/// # 窓が部品の図形に代わりにすること
///
/// [`WindowItem::object_names`] で返した図形には、窓が次のことをします。
///
/// - 窓を動かしたら、同じだけずらす
/// - 描き先の大きさが変わったら、カメラを貼り直す
/// - 窓の当たり判定で、窓自身の図形として扱う（縁の取っ手を、別の窓とまちがえない）
///
/// 窓の大きさが変わったときの組み直しだけは、部品ごとに形が違うので
/// [`WindowItem::rebuild`] で部品にしてもらいます。
pub trait WindowItem {
    /// 図形を組む。返すのは部品の `(幅, 高さ)`。次の部品はこの高さぶん下に置かれる。
    ///
    /// `width` は部品に使える幅（窓の幅から左右の余白を引いたもの）です。
    /// [`WindowItem::rebuild`] に渡す幅と同じ決め方なので、寄せや折り返しを最初から合わせられます。
    fn create_object(&mut self, create_object_arguments: CreateObjectArguments) -> (f32, f32);

    fn register_draw_manager(&mut self, draw_manager: &mut DrawManager);

    /// 窓の大きさが変わった。**登録済みの図形の中で**組み直す。返すのは新しい `(幅, 高さ)`。
    ///
    /// `width` と座標の決め方は [`WindowItem::create_object`] と同じです。
    ///
    /// 図形を登録し直さないでください。[`DrawManager`] には消す手立てが無いので、
    /// 名前が溜まります。頂点を入れ直すなら
    /// [`Object::begin`](crate::object::Object::begin) から
    /// [`Object::end`](crate::object::Object::end) で、
    /// 字なら [`TextRenderer::clear`](crate::text::TextRenderer::clear) してから書き直します。
    fn rebuild(&mut self, draw_manager: &mut DrawManager, item_x: f32, item_y: f32, width: f32) -> (f32, f32);

    /// この部品が登録した図形の名前。字の形
    /// （[`TextRenderer::shape_ids`](crate::text::TextRenderer::shape_ids)）も入れてください。
    ///
    /// ここに無い図形は、窓を動かしても付いてきません。
    fn object_names(&self) -> Vec<String>;

    /// 描き先の大きさが変わった。カメラは窓が貼り直すので、それ以外にすることがあれば。
    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {

    }

    fn mouse_left_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_right_pressed(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_left_released(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_right_released(&mut self, draw_manager: &DrawManager, x: f32, y: f32) -> bool {
        false
    }

    fn mouse_moved(&mut self, draw_manager: &mut DrawManager, x: f32, y: f32) -> bool {
        false
    }
}
