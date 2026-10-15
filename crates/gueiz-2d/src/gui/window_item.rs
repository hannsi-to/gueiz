use gueiz_gpu::camera::ScaleMode;
use gueiz_gpu::renderer::SurfaceSize;
use crate::draw_manager::DrawManager;
use crate::object::Object;
use crate::resource::Resources;

/// 組み直した形の頂点を、登録済みの図形に入れ直す。
///
/// 図形を登録し直すと [`DrawManager`] に名前が溜まるので、頂点だけを差し替えます。
/// カメラ・複製・ずらし・効果は `object` に残ったままです。
pub(crate) fn replace_outline(object: &mut Object, built: &Object) {
    object.begin(built.paint_type());
    for vertex in built.vertices() {
        object.put_vertex(*vertex);
    }
    object.end();
}

pub struct WindowItemArguments1<'a> {
    pub draw_manager: &'a mut DrawManager,
    pub resources: &'a Resources,
    pub register_name: &'a String,
    pub counter: i32,
    pub item_x: f32,
    pub item_y: f32,
    pub width: f32,
    pub surface_size: SurfaceSize,
    pub scale_mode: ScaleMode,
    pub design: [f32; 2],
}


pub struct WindowItemArguments2<'a> {
    pub draw_manager: &'a mut DrawManager,
    pub resources: &'a Resources,
    pub register_name: &'a String,
    pub counter: i32,
    pub item_x: f32,
    pub item_y: f32,
    pub width: f32,
    pub scale_mode: ScaleMode,
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
    fn create_object(&mut self, window_item_arguments: WindowItemArguments1) -> (f32, f32);

    /// 窓の大きさが変わった。**登録済みの図形の中で**組み直す。返すのは新しい `(幅, 高さ)`。
    ///
    /// `width` と座標の決め方は [`WindowItem::create_object`] と同じです。
    ///
    /// 図形を登録し直さないでください。[`DrawManager`] には消す手立てが無いので、
    /// 名前が溜まります。頂点を入れ直すなら
    /// [`Object::begin`](crate::object::Object::begin) から
    /// [`Object::end`](crate::object::Object::end) で、
    /// 字なら [`TextRenderer::clear`](crate::text::TextRenderer::clear) してから書き直します。
    fn rebuild(&mut self, window_item_arguments: WindowItemArguments2) -> (f32, f32);

    /// この部品が登録した図形の名前。字の形
    /// （[`TextRenderer::shape_ids`](crate::text::TextRenderer::shape_ids)）も入れてください。
    ///
    /// ここに無い図形は、窓を動かしても付いてきません。
    fn object_names(&self, draw_manager: &mut DrawManager) -> Vec<String>;

    /// 描き先の大きさが変わった。カメラは窓が貼り直すので、それ以外にすることがあれば。
    fn resize(&mut self, draw_manager: &mut DrawManager, surface_size: SurfaceSize, design: [f32; 2]) {

    }

    /// 左を押した。窓の中で、手前がこの窓のときだけ届きます。**取ったら `true`。**
    ///
    /// 取ったら、後ろの部品にも窓の帯にも渡りません。
    /// 見た目を変えるなら、ここで図形を書き換えて構いません。
    fn mouse_left_pressed(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    fn mouse_right_pressed(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    /// 左を放した。**どこで放しても、すべての部品に届きます**（押したまま外へ出たのを戻せるように）。
    /// 何か変わったら `true`。
    fn mouse_left_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    fn mouse_right_released(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }

    /// 指が動いた。窓を掴んでいる・縁を引いているあいだは届きません。
    /// 見た目が変わったら `true`（呼ぶ側が描き直します）。
    fn mouse_moved(&mut self, _draw_manager: &mut DrawManager, _x: f32, _y: f32) -> bool {
        false
    }
}
