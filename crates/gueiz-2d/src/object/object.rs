//! 図形 1 つぶんの定義。頂点の記録・変換・インスタンスを持つ。

use std::ops::{Deref, DerefMut, Range};

use crate::camera::Camera;
use crate::effect::{Block, EffectStack, EffectStage};
use crate::math::Mat4;
use crate::object::instance::Instance;
use crate::paint_type::{JointType, PaintType};
use crate::atlas::TextureRegion;
use crate::sprite::WHOLE_LAYER;
use crate::tessellate;
use crate::vertex::Vertex;

/// 名前を付けて図形を作る。
///
/// ```no_run
/// # use gueiz_2d::object;
/// # use gueiz_2d::paint_type::PaintType;
/// # use gueiz_2d::vertex::Vertex;
/// let mut triangle = object::create_object("Triangle");
/// triangle.begin(PaintType::Fill);
/// triangle.put_vertex(Vertex::new_position_color(0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
/// triangle.put_vertex(Vertex::new_position_color(100.0, 0.0, 0.0, 1.0, 0.0, 0.0, 1.0));
/// triangle.put_vertex(Vertex::new_position_color(100.0, 100.0, 0.0, 1.0, 0.0, 0.0, 1.0));
/// triangle.end();
/// ```
pub fn create_object(name: &str) -> Object {
    Object::new(name)
}

/// 描く図形。
///
/// [`Object::begin`] から [`Object::end`] までに積んだ頂点が形になり、
/// [`Object::translate`] などの変換と [`Object::instance`] で足した複製のぶんだけ描かれる。
pub struct Object {
    name: String,
    paint_type: PaintType,

    /// `begin` / `end` で受け取った生の頂点。輪郭の順に並ぶ。
    outline: Vec<Vertex>,
    /// 各輪郭が `outline` のどこから始まるか。先頭は外周、以降は穴。
    contour_starts: Vec<usize>,
    /// 三角形に開いた結果。[`crate::object::draw_manager::DrawManager`] が読む。
    triangles: Vec<Vertex>,
    recording: bool,
    /// 形が変わったので積み直しが要る。
    geometry_dirty: bool,
    /// インスタンスが書き換わったので送り直しが要る。
    instances_dirty: bool,
    /// エフェクトが書き換わったので積み直しが要る。
    effects_dirty: bool,
    /// [`Object::edit`] で囲まれている深さ。0 なら書き換えのたびに開き直す。
    edit_depth: u32,
    /// 囲まれている間に輪郭が変わった。閉じるときにまとめて開き直す。
    outline_changed: bool,

    camera: Camera,
    translation: [f32; 3],
    scale: [f32; 3],
    rotation: [f32; 3],

    instances: Vec<Instance>,
    effects: EffectStack,

    /// 読むスプライトシートの層。`None` なら絵を貼らない。
    sprite_layer: Option<u32>,
    /// 層のどこを切り出すか。`[u, v, 幅, 高さ]`、いずれも 0..1。
    uv_rect: [f32; 4],
}

impl Object {
    pub fn new(name: &str) -> Self {
        Self {
            name: String::from(name),
            paint_type: PaintType::default(),
            outline: Vec::new(),
            contour_starts: vec![0],
            triangles: Vec::new(),
            recording: false,
            geometry_dirty: true,
            instances_dirty: true,
            effects_dirty: true,
            edit_depth: 0,
            outline_changed: false,
            camera: Camera::default(),
            translation: [0.0; 3],
            scale: [1.0; 3],
            rotation: [0.0; 3],
            instances: Vec::new(),
            effects: EffectStack::new(),
            sprite_layer: None,
            uv_rect: WHOLE_LAYER,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 名前を付け替える。**[`crate::object::DrawManager`] だけが呼ぶ。**
    ///
    /// 名前は登録先でひとつに保たれているので、外から勝手に変えられると
    /// 索引と食い違います。だから公開していません。
    pub(crate) fn rename(&mut self, name: String) {
        self.name = name;
    }

    /// 頂点の記録を始める。すでに入っていた形は捨てる。
    pub fn begin(&mut self, paint_type: PaintType) -> &mut Self {
        self.paint_type = paint_type;
        self.outline.clear();
        self.contour_starts.clear();
        self.contour_starts.push(0);
        self.recording = true;
        self
    }

    /// ここから先の頂点を**穴**として記録する。何個でも開けられる。
    ///
    /// [`PaintType::Fill`] なら外周から抜かれ、[`PaintType::Stroke`] なら
    /// 穴の縁にも同じ太さの線が引かれる。
    ///
    /// 巻き方向は気にしなくてよい。外周と逆向きになるよう自動で揃えられる。
    ///
    /// ```no_run
    /// # use gueiz_2d::object;
    /// # use gueiz_2d::paint_type::PaintType;
    /// # use gueiz_2d::vertex::Vertex;
    /// # let point = |x, y| Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0);
    /// let mut plate = object::create_object("Plate");
    /// plate.begin(PaintType::Fill);
    /// for [x, y] in [[0.0, 0.0], [90.0, 0.0], [90.0, 90.0], [0.0, 90.0]] {
    ///     plate.put_vertex(point(x, y));
    /// }
    /// plate.begin_hole();
    /// for [x, y] in [[30.0, 30.0], [60.0, 30.0], [60.0, 60.0], [30.0, 60.0]] {
    ///     plate.put_vertex(point(x, y));
    /// }
    /// plate.end();
    /// ```
    pub fn begin_hole(&mut self) -> &mut Self {
        if !self.recording {
            log::warn!("begin_hole on '{}' outside begin/end; ignored", self.name);
            return self;
        }

        // 直前の輪郭が空なら区切りを増やさない。空の輪郭は後で落とされるが、
        // 続けて呼ばれたときに区切りが積み上がるのを避ける。
        if self.contour_starts.last() != Some(&self.outline.len()) {
            self.contour_starts.push(self.outline.len());
        }

        self
    }

    /// 輪郭に頂点を 1 つ足す。[`Object::begin`] の後でだけ効く。
    pub fn put_vertex(&mut self, vertex: Vertex) -> &mut Self {
        if !self.recording {
            log::warn!("put_vertex on '{}' outside begin/end; ignored", self.name);
            return self;
        }

        self.outline.push(vertex);
        self
    }

    /// 記録を終えて三角形に開く。
    pub fn end(&mut self) -> &mut Self {
        if !self.recording {
            log::warn!("end without begin on '{}'; ignored", self.name);
            return self;
        }

        self.recording = false;
        self.mark_outline_changed();
        self
    }

    /// この図形を写すカメラ。指定しなければクリップ空間（`-1..1`）をそのまま使う。
    pub fn camera(&mut self, camera: Camera) -> &mut Self {
        self.camera = camera;
        self
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.translation = [x, y, z];
        self
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.scale = [x, y, z];
        self
    }

    /// ラジアン。2D なら `z` だけ指定すれば平面内で回る。
    pub fn rotate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.rotation = [x, y, z];
        self
    }

    // --- 頂点の編集 ---
    //
    // `begin` / `end` で録り直さずに、置いた後の頂点を触るための一式。
    // 位置は「どの輪郭の何番目か」で指定する。輪郭をまたぐ通し番号だと、
    // 穴を持つ図形で「足したつもりが隣の輪郭に入る」事故が起きる。
    //
    // 書き換えるたびに三角形に開き直すので、`triangles()` は常に正しい。
    // たくさん触るときは [`Object::edit`] で囲めば開き直しは 1 回で済む。

    /// 輪郭の数。外周だけなら 1、穴が 2 つあれば 3。
    pub fn contour_count(&self) -> usize {
        self.contour_starts.len()
    }

    /// その輪郭が `outline` のどこからどこまでか。
    pub fn contour_range(&self, contour: usize) -> Option<Range<usize>> {
        let start = *self.contour_starts.get(contour)?;
        let end = self
            .contour_starts
            .get(contour + 1)
            .copied()
            .unwrap_or(self.outline.len());

        Some(start..end)
    }

    /// その輪郭の頂点。`contour` が無ければ空。
    pub fn contour(&self, contour: usize) -> &[Vertex] {
        match self.contour_range(contour) {
            Some(range) => &self.outline[range],
            None => &[],
        }
    }

    /// 全輪郭をつないだ頂点列。外周が先、以降が穴。
    pub fn vertices(&self) -> &[Vertex] {
        &self.outline
    }

    /// 頂点を 1 つ書き換える。番号が無ければ何もせず `false`。
    pub fn set_vertex(&mut self, contour: usize, position: usize, vertex: Vertex) -> bool {
        let Some(range) = self.contour_range(contour) else {
            return false;
        };

        if position >= range.len() {
            return false;
        }

        self.outline[range.start + position] = vertex;
        self.mark_outline_changed();
        true
    }

    /// 頂点を差し込む。`position` がその輪郭の頂点数以上なら末尾に足す。
    ///
    /// 後ろの輪郭の区切りは自動でずれる。
    pub fn insert_vertex(&mut self, contour: usize, position: usize, vertex: Vertex) -> bool {
        let Some(range) = self.contour_range(contour) else {
            return false;
        };

        let position = position.min(range.len());
        self.outline.insert(range.start + position, vertex);

        for start in &mut self.contour_starts[contour + 1..] {
            *start += 1;
        }

        self.mark_outline_changed();
        true
    }

    /// その輪郭の末尾に頂点を足す。
    pub fn push_vertex(&mut self, contour: usize, vertex: Vertex) -> bool {
        self.insert_vertex(contour, usize::MAX, vertex)
    }

    /// 頂点を 1 つ消す。消した頂点を返す。
    pub fn remove_vertex(&mut self, contour: usize, position: usize) -> Option<Vertex> {
        let range = self.contour_range(contour)?;

        if position >= range.len() {
            return None;
        }

        let removed = self.outline.remove(range.start + position);

        for start in &mut self.contour_starts[contour + 1..] {
            *start -= 1;
        }

        self.mark_outline_changed();
        Some(removed)
    }

    /// 空の穴を足して、その輪郭番号を返す。頂点は [`Object::push_vertex`] で入れる。
    ///
    /// [`Object::begin_hole`] の実行時版。`begin` / `end` の外でも使える。
    pub fn add_hole(&mut self) -> usize {
        self.contour_starts.push(self.outline.len());
        self.contour_starts.len() - 1
    }

    /// 輪郭をまるごと消す。穴を塞ぐときに使う。
    ///
    /// 外周（`0`）を消すと、次の輪郭が外周に繰り上がる。
    /// 最後の 1 本を消したときは、空の外周が残る。
    pub fn remove_contour(&mut self, contour: usize) -> bool {
        let Some(range) = self.contour_range(contour) else {
            return false;
        };

        let removed = range.len();
        self.outline.drain(range);
        self.contour_starts.remove(contour);

        for start in &mut self.contour_starts[contour..] {
            *start -= removed;
        }

        // 区切りは必ず 1 本以上あり、先頭は必ず 0。
        if self.contour_starts.is_empty() {
            self.contour_starts.push(0);
        }

        self.mark_outline_changed();
        true
    }

    /// まとめて編集する。閉じたときに 1 回だけ三角形に開き直す。
    ///
    /// 頂点を 1 つ触るたびに図形全体を開き直すので、たくさん触るときはこれで囲む。
    /// スライスごと書き換える [`ObjectEdit::vertices_mut`] もここからだけ使える。
    ///
    /// ```no_run
    /// # use gueiz_2d::object;
    /// # use gueiz_2d::paint_type::PaintType;
    /// # use gueiz_2d::vertex::Vertex;
    /// # let mut shape = object::create_object("Shape");
    /// # shape.begin(PaintType::Fill);
    /// # for i in 0..4 { shape.put_vertex(Vertex::new_position_color(i as f32, 0.0, 0.0, 1.0, 1.0, 1.0, 1.0)); }
    /// # shape.end();
    /// {
    ///     let mut edit = shape.edit();
    ///
    ///     for vertex in edit.vertices_mut() {
    ///         vertex.x += 1.0;
    ///     }
    ///
    ///     edit.remove_vertex(0, 0);
    /// } // ここで 1 回だけ開き直す
    /// ```
    pub fn edit(&mut self) -> ObjectEdit<'_> {
        self.edit_depth += 1;
        ObjectEdit { object: self }
    }

    /// 輪郭が変わった。囲まれていなければその場で開き直す。
    fn mark_outline_changed(&mut self) {
        if self.edit_depth > 0 {
            self.outline_changed = true;
            return;
        }

        self.rebuild_triangles();
    }

    // --- スプライト ---

    /// スプライトシートの層を貼る。層をまるごと使う。
    ///
    /// 貼らなければ頂点色がそのまま出る。貼ると、絵の色に頂点色が掛かる。
    ///
    /// UV は**図形の外接矩形**を 0..1 にしたもの。四角に貼れば素直に収まり、
    /// それ以外の形でも外接矩形に合わせて伸ばされる。
    ///
    /// ```no_run
    /// # use gueiz_2d::object;
    /// # let mut sprite = object::create_object("Sprite");
    /// sprite.sprite(2);          // シートの 3 枚目
    /// ```
    pub fn sprite(&mut self, layer: u32) -> &mut Self {
        self.sprite_layer = Some(layer);
        self.uv_rect = WHOLE_LAYER;
        self
    }

    /// 層の一部だけを貼る。`region` は `[u, v, 幅, 高さ]`、いずれも 0..1。
    ///
    /// 画素で指定したいときは [`crate::sprite::SpriteSheet::region`] で直す。
    /// 1 枚のシートに何コマも並べておいて、ここを変えるだけでコマ送りできる。
    /// 形は変わらないので、三角形の積み直しは起きない。
    pub fn sprite_region(&mut self, layer: u32, region: [f32; 4]) -> &mut Self {
        self.sprite_layer = Some(layer);
        self.uv_rect = region;
        self
    }

    /// アトラスに詰め込んだ絵を貼る。
    ///
    /// ページ番号と切り出し範囲をまとめて受け取るだけで、
    /// [`Object::sprite_region`] と同じことをする。
    ///
    /// ```no_run
    /// # use gueiz_2d::object::Object;
    /// # use gueiz_2d::resource::{Resources, TextureHandle};
    /// # fn run(resources: &Resources, sprite: &mut Object, grass: TextureHandle) {
    /// if let Some(region) = resources.texture(grass) {
    ///     sprite.sprite_texture(region);
    /// }
    /// # }
    /// ```
    pub fn sprite_texture(&mut self, region: TextureRegion) -> &mut Self {
        self.sprite_region(region.page, region.uv_rect)
    }

    /// 絵を剥がす。頂点色だけになる。
    pub fn no_sprite(&mut self) -> &mut Self {
        self.sprite_layer = None;
        self
    }

    pub fn sprite_layer(&self) -> Option<u32> {
        self.sprite_layer
    }

    pub fn sprite_rect(&self) -> [f32; 4] {
        self.uv_rect
    }

    /// 重なり順。**大きいほど手前**に描かれる。
    ///
    /// [`Object::translate`] の z と同じものなので、どちらで設定してもよい。
    /// 深度バッファではなく描く順で解決しているので、半透明も正しく混ざる。
    pub fn z(&mut self, z: f32) -> &mut Self {
        self.translation[2] = z;
        self
    }

    /// いまの重なり順。
    pub fn depth(&self) -> f32 {
        self.translation[2]
    }

    // --- エフェクト ---

    /// この図形に掛かるエフェクト。
    pub fn effects(&self) -> &EffectStack {
        &self.effects
    }

    /// エフェクトを積む。段は [`Block::stage`] が決めるので指定は要らない。
    ///
    /// ```no_run
    /// # use gueiz_2d::object;
    /// # use gueiz_2d::effect::Block;
    /// # let mut shape = object::create_object("Shape");
    /// shape.effect(Block::Gradient {
    ///     from: [1.0, 0.4, 0.2, 1.0],
    ///     to: [0.2, 0.4, 1.0, 1.0],
    ///     angle: 0.0,
    /// });
    /// shape.effect(Block::Spin { speed: 1.5 });
    /// ```
    pub fn effect(&mut self, block: Block) -> &mut Self {
        let stage = block.stage();
        self.effects.push(block);
        self.after_effects_changed(stage);
        self
    }

    /// エフェクトをまるごと差し替える。
    pub fn set_effects(&mut self, effects: EffectStack) -> &mut Self {
        self.effects = effects;
        // 形の段が変わったかもしれないので、開き直す。
        self.mark_outline_changed();
        self.effects_dirty = true;
        self
    }

    /// エフェクトを書き換える。閉じたときに必要なぶんだけ作り直す。
    ///
    /// 順番の入れ替え（[`EffectStack::move_block`]）や削除はここから。
    pub fn edit_effects(&mut self, edit: impl FnOnce(&mut EffectStack)) -> &mut Self {
        edit(&mut self.effects);
        self.mark_outline_changed();
        self.effects_dirty = true;
        self
    }

    pub(crate) fn is_effects_dirty(&self) -> bool {
        self.effects_dirty
    }

    pub(crate) fn clear_effects_dirty(&mut self) {
        self.effects_dirty = false;
    }

    /// 段によって作り直すものが違う。形の段だけが三角形に効く。
    fn after_effects_changed(&mut self, stage: EffectStage) {
        self.effects_dirty = true;

        if stage == EffectStage::Shape {
            self.mark_outline_changed();
        }
    }

    /// 複製を 1 つ足す。戻り値は [`Object::instance_mut`] で引くための番号。
    ///
    /// 1 つも足さなければ、変換なしの複製が 1 つあるものとして描かれる。
    pub fn instance(&mut self, instance: Instance) -> usize {
        self.instances.push(instance);
        self.instances_dirty = true;
        self.instances.len() - 1
    }

    /// 1 つだけ引いて書き換える。呼んだ時点で送り直しの印が付く。
    pub fn instance_mut(&mut self, index: usize) -> Option<&mut Instance> {
        // 実際に書き換えたかは分からないので、引かれた時点で dirty にする。
        if index < self.instances.len() {
            self.instances_dirty = true;
        }

        self.instances.get_mut(index)
    }

    /// 全インスタンスを書き換える。毎フレームの更新用。
    ///
    /// 呼ぶだけでこの図形のインスタンスが**全部**送り直しになる。
    /// 動かさないフレームでは呼ばないこと。読むだけなら [`Object::instances`] を使う。
    pub fn instances_mut(&mut self) -> &mut [Instance] {
        self.instances_dirty = true;
        &mut self.instances
    }

    pub fn instances(&self) -> &[Instance] {
        &self.instances
    }

    pub fn clear_instances(&mut self) -> &mut Self {
        self.instances.clear();
        self.instances_dirty = true;
        self
    }

    /// 平行移動 → 回転 → 拡大の順に適用する行列。
    pub fn transform(&self) -> Mat4 {
        let [tx, ty, tz] = self.translation;
        let [rx, ry, rz] = self.rotation;
        let [sx, sy, sz] = self.scale;

        Mat4::from_translation(tx, ty, tz)
            .multiply(Mat4::from_rotation(rx, ry, rz))
            .multiply(Mat4::from_scale(sx, sy, sz))
    }

    /// カメラと図形の変換まで。インスタンスのぶんはこの後に掛かる。
    ///
    /// GPU-driven の経路では、ここまでを CPU が畳んで送り、
    /// インスタンスの変換はコンピュートシェーダが組み立てる。
    pub fn view_transform(&self) -> Mat4 {
        self.camera.view_projection().multiply(self.transform())
    }

    /// 頂点に最終的に掛かる行列。`camera * object * instance`。
    ///
    /// オブジェクトの変換は**親ノード**として働く。つまりインスタンスの座標も
    /// オブジェクトの変換を通るので、[`Object::scale`] を掛けると図形だけでなく
    /// **インスタンスの配置間隔ごと縮む**。平行移動は足し算なので配置に影響しない。
    ///
    /// 図形そのものの大きさだけを変えたいときは、頂点を最終的な寸法で組むか、
    /// [`Instance::scale`] を使う。
    pub fn instance_transform(&self, instance: &Instance) -> Mat4 {
        self.camera
            .view_projection()
            .multiply(self.transform())
            .multiply(instance.transform())
    }

    /// 三角形に開いた頂点列。
    pub fn triangles(&self) -> &[Vertex] {
        &self.triangles
    }

    pub(crate) fn is_geometry_dirty(&self) -> bool {
        self.geometry_dirty
    }

    pub(crate) fn clear_geometry_dirty(&mut self) {
        self.geometry_dirty = false;
    }

    pub(crate) fn is_instances_dirty(&self) -> bool {
        self.instances_dirty
    }

    pub(crate) fn clear_instances_dirty(&mut self) {
        self.instances_dirty = false;
    }

    /// 輪郭を三角形リストに開く。`end` と、頂点やエフェクトの編集から呼ばれる。
    ///
    /// 本体を開いたあと、Shape 段のエフェクトが足す形を後ろに積む。
    /// 後ろに積むので、輪郭線は塗りの上に乗る。
    fn rebuild_triangles(&mut self) {
        self.triangles.clear();
        tessellate::tessellate(
            &self.outline,
            &self.contour_starts,
            self.paint_type,
            &mut self.triangles,
        );

        // `self` を丸ごと借りるので、山だけ先に写す。数は多くても数個。
        let shape_blocks: Vec<Block> = self.effects.blocks(EffectStage::Shape).to_vec();

        for block in shape_blocks {
            self.apply_shape_block(block);
        }

        self.geometry_dirty = true;
    }

    /// Shape 段の 1 山ぶんの形を三角形に足す。
    fn apply_shape_block(&mut self, block: Block) {
        match block {
            Block::Outline { width, color } => {
                if width <= 0.0 {
                    return;
                }

                // 輪郭ごとに線を引く。穴の縁にも付く。
                let outline: Vec<Vertex> = self
                    .outline
                    .iter()
                    .map(|vertex| {
                        let mut tinted = *vertex;
                        [tinted.r, tinted.g, tinted.b, tinted.a] = color;
                        tinted
                    })
                    .collect();

                tessellate::tessellate(
                    &outline,
                    &self.contour_starts,
                    PaintType::Stroke {
                        line_width: width,
                        joint_type: JointType::Miter,
                        strip: false,
                    },
                    &mut self.triangles,
                );
            }

            // 他の段の山はここでは何もしない。
            _ => {}
        }
    }
}


/// [`Object::edit`] が返す取っ手。閉じたときに 1 回だけ三角形に開き直す。
///
/// [`Object`] のメソッドはそのまま使える。違うのは、囲まれている間は
/// 書き換えが溜まるだけで、開き直しが起きないこと。
pub struct ObjectEdit<'a> {
    object: &'a mut Object,
}

impl ObjectEdit<'_> {
    /// 全頂点をスライスで書き換える。位置や色は変えられるが、数は変えられない。
    ///
    /// 呼ぶだけで開き直しの予約が入る。読むだけなら [`Object::vertices`] を使う。
    pub fn vertices_mut(&mut self) -> &mut [Vertex] {
        self.object.outline_changed = true;
        &mut self.object.outline
    }

    /// その輪郭の頂点をスライスで書き換える。無ければ空。
    pub fn contour_mut(&mut self, contour: usize) -> &mut [Vertex] {
        let Some(range) = self.object.contour_range(contour) else {
            return &mut [];
        };

        self.object.outline_changed = true;
        &mut self.object.outline[range]
    }
}

impl Deref for ObjectEdit<'_> {
    type Target = Object;

    fn deref(&self) -> &Object {
        self.object
    }
}

impl DerefMut for ObjectEdit<'_> {
    fn deref_mut(&mut self) -> &mut Object {
        self.object
    }
}

impl Drop for ObjectEdit<'_> {
    fn drop(&mut self) {
        self.object.edit_depth -= 1;

        // 入れ子になっているときは、いちばん外側が閉じるまで待つ。
        if self.object.edit_depth > 0 {
            return;
        }

        if std::mem::take(&mut self.object.outline_changed) {
            self.object.rebuild_triangles();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paint_type::JointType;

    fn point(x: f32, y: f32) -> Vertex {
        Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0)
    }

    #[test]
    fn a_triangle_becomes_one_triangle() {
        let mut object = create_object("Triangle");
        object.begin(PaintType::Fill);
        object.put_vertex(point(0.0, 0.0));
        object.put_vertex(point(100.0, 0.0));
        object.put_vertex(point(100.0, 100.0));
        object.end();

        assert_eq!(object.triangles().len(), 3);
    }

    #[test]
    fn a_quad_becomes_two_triangles() {
        let mut object = create_object("Quad");
        object.begin(PaintType::Fill);
        for [x, y] in [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]] {
            object.put_vertex(point(x, y));
        }
        object.end();

        assert_eq!(object.triangles().len(), 6);
    }

    /// 線の形そのものは `tessellate` 側で検証している。ここでは
    /// `Object` が `PaintType` の設定をそのまま渡していることだけを見る。
    #[test]
    fn a_stroke_goes_through_the_tessellator() {
        let outline = [point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)];

        let paint_type = PaintType::Stroke {
            line_width: 4.0,
            joint_type: JointType::Bevel,
            strip: false,
        };

        let mut object = create_object("Outline");
        object.begin(paint_type);
        for vertex in outline {
            object.put_vertex(vertex);
        }
        object.end();

        let mut expected = Vec::new();
        tessellate::tessellate(&outline, &[0], paint_type, &mut expected);

        assert_eq!(object.triangles().len(), expected.len());
        assert!(!expected.is_empty());
    }

    /// 塗りと線で頂点数が変わる = `PaintType` が効いている。
    #[test]
    fn the_paint_type_changes_the_geometry() {
        let outline = [point(0.0, 0.0), point(10.0, 0.0), point(10.0, 10.0)];

        let mut filled = create_object("Filled");
        filled.begin(PaintType::Fill);
        for vertex in outline {
            filled.put_vertex(vertex);
        }
        filled.end();

        let mut stroked = create_object("Stroked");
        stroked.begin(PaintType::stroke(4.0));
        for vertex in outline {
            stroked.put_vertex(vertex);
        }
        stroked.end();

        assert_eq!(filled.triangles().len(), 3);
        assert!(stroked.triangles().len() > filled.triangles().len());
    }

    #[test]
    fn too_few_vertices_produce_nothing() {
        let mut object = create_object("Degenerate");
        object.begin(PaintType::Fill);
        object.put_vertex(point(0.0, 0.0));
        object.put_vertex(point(1.0, 0.0));
        object.end();

        assert!(object.triangles().is_empty());
    }

    #[test]
    fn put_vertex_outside_begin_is_ignored() {
        let mut object = create_object("Stray");
        object.put_vertex(point(0.0, 0.0));

        assert!(object.triangles().is_empty());
    }

    #[test]
    fn begin_discards_the_previous_shape() {
        let mut object = create_object("Replaced");
        object.begin(PaintType::Fill);
        for [x, y] in [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]] {
            object.put_vertex(point(x, y));
        }
        object.end();
        assert_eq!(object.triangles().len(), 6);

        object.begin(PaintType::Fill);
        object.put_vertex(point(0.0, 0.0));
        object.put_vertex(point(10.0, 0.0));
        object.put_vertex(point(10.0, 10.0));
        object.end();
        assert_eq!(object.triangles().len(), 3);
    }

    #[test]
    fn the_object_transform_is_a_parent_node() {
        let mut object = create_object("Parent");
        object.scale(0.5, 0.5, 1.0);

        // インスタンスを (100, 0) に置いても、親が半分ならそこも半分になる。
        let instance = Instance::new().translate(100.0, 0.0, 0.0);
        let [x, _, _] = object
            .instance_transform(&instance)
            .transform_point(0.0, 0.0, 0.0);

        assert_eq!(x, 50.0);
    }

    #[test]
    fn a_parent_translation_does_not_scale_the_layout() {
        let mut object = create_object("Parent");
        object.translate(-16.0, 0.0, 0.0);

        // 平行移動は足し算なので、どのインスタンスも同じだけずれる。
        let near = Instance::new().translate(100.0, 0.0, 0.0);
        let far = Instance::new().translate(900.0, 0.0, 0.0);

        let [near_x, _, _] = object.instance_transform(&near).transform_point(0.0, 0.0, 0.0);
        let [far_x, _, _] = object.instance_transform(&far).transform_point(0.0, 0.0, 0.0);

        assert_eq!(near_x, 84.0);
        assert_eq!(far_x, 884.0);
    }

    /// 100x100 の外周に 30x30 の穴を 1 つ空けた図形。
    fn plate() -> Object {
        let mut object = create_object("Plate");
        object.begin(PaintType::Fill);
        for [x, y] in [[0.0, 0.0], [100.0, 0.0], [100.0, 100.0], [0.0, 100.0]] {
            object.put_vertex(point(x, y));
        }
        object.begin_hole();
        for [x, y] in [[30.0, 30.0], [60.0, 30.0], [60.0, 60.0], [30.0, 60.0]] {
            object.put_vertex(point(x, y));
        }
        object.end();
        object
    }

    /// 三角形リストの総面積。編集が形に効いたかを測る。
    fn total_area(triangles: &[Vertex]) -> f32 {
        triangles
            .chunks_exact(3)
            .map(|triangle| {
                let [a, b, c] = [triangle[0], triangle[1], triangle[2]];
                ((b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)).abs() * 0.5
            })
            .sum()
    }

    #[test]
    fn a_shape_reports_its_contours() {
        let object = plate();

        assert_eq!(object.contour_count(), 2);
        assert_eq!(object.contour(0).len(), 4);
        assert_eq!(object.contour(1).len(), 4);
        assert_eq!(object.vertices().len(), 8);
        // 無い輪郭を引いても落ちない。
        assert!(object.contour(2).is_empty());
        assert_eq!(object.contour_range(2), None);
    }

    #[test]
    fn editing_a_vertex_changes_the_shape() {
        let mut object = plate();
        let before = total_area(object.triangles());

        // 外周を横に縮める。穴（x は 30..60）がはみ出さない幅まで。
        assert!(object.set_vertex(0, 1, point(80.0, 0.0)));
        assert!(object.set_vertex(0, 2, point(80.0, 100.0)));

        let after = total_area(object.triangles());
        assert!((before - (100.0 * 100.0 - 30.0 * 30.0)).abs() < 1e-2);
        assert!((after - (80.0 * 100.0 - 30.0 * 30.0)).abs() < 1e-2);
    }

    /// 穴のある図形で外周に頂点を足しても、穴の区切りが一緒にずれること。
    /// ここがずれると、足したはずの頂点が穴の側に紛れ込む。
    #[test]
    fn inserting_into_the_outline_keeps_the_hole_intact() {
        let mut object = plate();

        assert!(object.insert_vertex(0, 1, point(50.0, -20.0)));

        assert_eq!(object.contour_count(), 2);
        assert_eq!(object.contour(0).len(), 5);
        assert_eq!(object.contour(1).len(), 4);
        // 穴はそのままの位置に残っている。
        assert_eq!(object.contour(1)[0].x, 30.0);
        assert_eq!(object.contour(1)[0].y, 30.0);

        // 外周が上に出っ張ったぶんだけ広くなる。
        let expected = 100.0 * 100.0 + 0.5 * 100.0 * 20.0 - 30.0 * 30.0;
        assert!((total_area(object.triangles()) - expected).abs() < 1e-2);
    }

    #[test]
    fn removing_a_vertex_keeps_the_hole_intact() {
        let mut object = plate();

        let removed = object.remove_vertex(0, 3).expect("外周の 4 番目");
        assert_eq!(removed.x, 0.0);
        assert_eq!(removed.y, 100.0);

        assert_eq!(object.contour(0).len(), 3);
        assert_eq!(object.contour(1).len(), 4);
        assert_eq!(object.contour(1)[2].x, 60.0);
    }

    #[test]
    fn pushing_appends_to_that_contour() {
        let mut object = plate();

        assert!(object.push_vertex(0, point(-20.0, 50.0)));

        assert_eq!(object.contour(0).len(), 5);
        assert_eq!(object.contour(0)[4].x, -20.0);
        assert_eq!(object.contour(1).len(), 4);
    }

    #[test]
    fn a_hole_can_be_added_after_the_fact() {
        let mut object = plate();

        let hole = object.add_hole();
        assert_eq!(hole, 2);

        for [x, y] in [[70.0, 70.0], [90.0, 70.0], [90.0, 90.0], [70.0, 90.0]] {
            assert!(object.push_vertex(hole, point(x, y)));
        }

        assert_eq!(object.contour_count(), 3);

        let expected = 100.0 * 100.0 - 30.0 * 30.0 - 20.0 * 20.0;
        assert!((total_area(object.triangles()) - expected).abs() < 1e-2);
    }

    #[test]
    fn removing_a_contour_fills_the_hole_back_in() {
        let mut object = plate();

        assert!(object.remove_contour(1));

        assert_eq!(object.contour_count(), 1);
        assert_eq!(object.vertices().len(), 4);
        assert!((total_area(object.triangles()) - 100.0 * 100.0).abs() < 1e-2);
    }

    /// 外周を消すと穴が繰り上がる。区切りの先頭は必ず 0 でなければならない。
    #[test]
    fn removing_the_outline_promotes_the_hole() {
        let mut object = plate();

        assert!(object.remove_contour(0));

        assert_eq!(object.contour_count(), 1);
        assert_eq!(object.contour_range(0), Some(0..4));
        assert_eq!(object.contour(0)[0].x, 30.0);
        assert!((total_area(object.triangles()) - 30.0 * 30.0).abs() < 1e-2);
    }

    /// 最後の 1 本まで消しても、空の外周が残って壊れない。
    #[test]
    fn removing_every_contour_leaves_an_empty_shape() {
        let mut object = plate();

        assert!(object.remove_contour(1));
        assert!(object.remove_contour(0));

        assert_eq!(object.contour_count(), 1);
        assert_eq!(object.contour_range(0), Some(0..0));
        assert!(object.vertices().is_empty());
        assert!(object.triangles().is_empty());
    }

    #[test]
    fn out_of_range_edits_do_nothing() {
        let mut object = plate();
        let before = object.triangles().len();

        assert!(!object.set_vertex(5, 0, point(0.0, 0.0)));
        assert!(!object.set_vertex(0, 9, point(0.0, 0.0)));
        assert!(!object.insert_vertex(5, 0, point(0.0, 0.0)));
        assert_eq!(object.remove_vertex(0, 9), None);
        assert_eq!(object.remove_vertex(5, 0), None);
        assert!(!object.remove_contour(5));

        assert_eq!(object.triangles().len(), before);
        assert_eq!(object.contour_count(), 2);
    }

    /// 囲んでいる間は開き直さず、閉じたときに 1 回だけ開き直す。
    #[test]
    fn a_batch_edit_rebuilds_once_at_the_end() {
        let mut object = plate();
        let before = object.triangles().to_vec();

        {
            let mut edit = object.edit();

            for vertex in edit.vertices_mut() {
                vertex.x += 10.0;
            }

            // 囲まれている間は前の形のまま。
            assert_eq!(edit.triangles(), before.as_slice());

            edit.set_vertex(0, 0, point(10.0, 0.0));
            assert_eq!(edit.triangles(), before.as_slice());
        }

        // 閉じた時点で反映されている。面積は平行移動しただけなので変わらない。
        assert_ne!(object.triangles(), before.as_slice());
        let expected = 100.0 * 100.0 - 30.0 * 30.0;
        assert!((total_area(object.triangles()) - expected).abs() < 1e-2);
    }

    /// 入れ子にしても、いちばん外側が閉じるまで開き直さない。
    #[test]
    fn nested_batch_edits_rebuild_once() {
        let mut object = plate();
        let before = object.triangles().to_vec();

        {
            let mut outer = object.edit();
            outer.set_vertex(0, 1, point(80.0, 0.0));

            {
                let mut inner = outer.edit();
                inner.set_vertex(0, 2, point(80.0, 100.0));
                assert_eq!(inner.triangles(), before.as_slice());
            }

            // 内側が閉じても、まだ外側の中。
            assert_eq!(outer.triangles(), before.as_slice());
        }

        let expected = 80.0 * 100.0 - 30.0 * 30.0;
        assert!((total_area(object.triangles()) - expected).abs() < 1e-2);
    }

    /// 編集したら積み直しの印が立つこと。立たないと GPU 側が古い形のまま。
    #[test]
    fn editing_marks_the_geometry_dirty() {
        let mut object = plate();
        object.clear_geometry_dirty();

        object.set_vertex(0, 0, point(1.0, 1.0));
        assert!(object.is_geometry_dirty());

        object.clear_geometry_dirty();
        {
            let mut edit = object.edit();
            edit.vertices_mut()[0].x = 2.0;
            // 閉じるまでは立たない。
            assert!(!edit.is_geometry_dirty());
        }
        assert!(object.is_geometry_dirty());
    }

    /// Shape 段の山は CPU 側で三角形になる。塗りの後ろに積むので上に乗る。
    #[test]
    fn an_outline_block_adds_geometry() {
        let mut object = plate();
        let filled = object.triangles().len();

        object.effect(Block::Outline {
            width: 4.0,
            color: [1.0, 0.0, 0.0, 1.0],
        });

        assert!(
            object.triangles().len() > filled,
            "輪郭線のぶん三角形が増えるはず",
        );

        // 後ろに積まれている = 塗りの上に描かれる。
        assert_eq!(&object.triangles()[..filled], &plate().triangles()[..filled]);
    }

    /// 幅 0 の輪郭線は何も足さない。
    #[test]
    fn a_zero_width_outline_adds_nothing() {
        let mut object = plate();
        let filled = object.triangles().len();

        object.effect(Block::Outline {
            width: 0.0,
            color: [1.0; 4],
        });

        assert_eq!(object.triangles().len(), filled);
    }

    /// 段によって作り直すものが違う。形の段だけが三角形に効く。
    #[test]
    fn only_the_shape_stage_rebuilds_the_geometry() {
        let mut object = plate();
        let before = object.triangles().len();
        object.clear_geometry_dirty();
        object.clear_effects_dirty();

        // 色の段は三角形を変えない。積み直しの印だけ立つ。
        object.effect(Block::Tint { color: [1.0; 4] });
        assert_eq!(object.triangles().len(), before);
        assert!(object.is_effects_dirty());
        assert!(!object.is_geometry_dirty());

        object.clear_effects_dirty();

        // 形の段は三角形を変える。
        object.effect(Block::Outline {
            width: 2.0,
            color: [1.0; 4],
        });
        assert!(object.triangles().len() > before);
        assert!(object.is_effects_dirty());
        assert!(object.is_geometry_dirty());
    }

    /// 並べ替えても三角形は変わらないが、送り直しの印は立つ。
    #[test]
    fn reordering_effects_marks_them_dirty() {
        let mut object = plate();
        object.effect(Block::Tint { color: [1.0; 4] });
        object.effect(Block::Flicker { amount: 0.5, speed: 1.0 });
        object.clear_effects_dirty();

        object.edit_effects(|effects| {
            effects.move_block(EffectStage::Color, 1, 0);
        });

        assert!(object.is_effects_dirty());
        assert!(matches!(
            object.effects().blocks(EffectStage::Color)[0],
            Block::Flicker { .. },
        ));
    }

    #[test]
    fn a_shape_has_no_sprite_by_default() {
        let object = plate();

        assert_eq!(object.sprite_layer(), None);
        assert_eq!(object.sprite_rect(), WHOLE_LAYER);
    }

    #[test]
    fn a_sprite_layer_can_be_set_and_cleared() {
        let mut object = plate();

        object.sprite(2);
        assert_eq!(object.sprite_layer(), Some(2));
        assert_eq!(object.sprite_rect(), WHOLE_LAYER);

        object.sprite_region(1, [0.0, 0.0, 0.25, 0.5]);
        assert_eq!(object.sprite_layer(), Some(1));
        assert_eq!(object.sprite_rect(), [0.0, 0.0, 0.25, 0.5]);

        object.no_sprite();
        assert_eq!(object.sprite_layer(), None);
        // 剥がしても切り出し範囲は残る。貼り直したときに効く。
        assert_eq!(object.sprite_rect(), [0.0, 0.0, 0.25, 0.5]);
    }

    /// コマ送りで形が積み直されると、毎フレーム三角形を組み直すことになる。
    #[test]
    fn changing_the_region_does_not_rebuild_the_geometry() {
        let mut object = plate();
        let before = object.triangles().to_vec();
        object.clear_geometry_dirty();

        object.sprite_region(0, [0.25, 0.0, 0.25, 1.0]);

        assert_eq!(object.triangles(), before.as_slice());
        assert!(!object.is_geometry_dirty());
    }

    /// z は translate の z と同じもの。二重に持つと必ずずれる。
    #[test]
    fn z_is_the_same_as_the_translation_z() {
        let mut object = plate();

        object.z(3.0);
        assert_eq!(object.depth(), 3.0);

        object.translate(10.0, 20.0, -1.5);
        assert_eq!(object.depth(), -1.5);

        object.z(7.0);
        assert_eq!(object.depth(), 7.0);
        // x と y は触らない。
        let [x, y, _] = [10.0, 20.0, 0.0];
        assert_eq!(
            object.transform().transform_point(0.0, 0.0, 0.0),
            [x, y, 7.0],
        );
    }

    /// dirty フラグは「送り直しが要るか」の判断に使う。立て忘れると
    /// 書き換えが画面に出ず、立てすぎると速度の意味が無くなる。
    #[test]
    fn adding_an_instance_marks_it_dirty() {
        let mut object = create_object("Marked");
        object.clear_instances_dirty();

        object.instance(Instance::new());

        assert!(object.is_instances_dirty());
    }

    #[test]
    fn reading_instances_leaves_it_clean() {
        let mut object = create_object("Read");
        object.instance(Instance::new());
        object.clear_instances_dirty();

        // 読むだけなら送り直しは要らない。
        let _ = object.instances();
        let _ = object.instances().len();

        assert!(!object.is_instances_dirty());
    }

    #[test]
    fn taking_instances_mutably_marks_it_dirty() {
        let mut object = create_object("Written");
        object.instance(Instance::new());
        object.clear_instances_dirty();

        object.instances_mut()[0].set_translation(1.0, 2.0, 0.0);

        assert!(object.is_instances_dirty());
    }

    #[test]
    fn taking_one_instance_mutably_marks_it_dirty() {
        let mut object = create_object("Written");
        object.instance(Instance::new());
        object.clear_instances_dirty();

        object.instance_mut(0).unwrap().set_scale(2.0, 2.0, 1.0);

        assert!(object.is_instances_dirty());
    }

    /// 範囲外を引いただけなら何も変わっていないので、印は立てない。
    #[test]
    fn a_missing_instance_leaves_it_clean() {
        let mut object = create_object("Absent");
        object.instance(Instance::new());
        object.clear_instances_dirty();

        assert!(object.instance_mut(7).is_none());
        assert!(!object.is_instances_dirty());
    }

    #[test]
    fn clearing_instances_marks_it_dirty() {
        let mut object = create_object("Emptied");
        object.instance(Instance::new());
        object.clear_instances_dirty();

        object.clear_instances();

        assert!(object.is_instances_dirty());
    }

    /// 作った直後は GPU に何も送っていないので、最初から dirty でなければならない。
    #[test]
    fn a_fresh_object_starts_dirty() {
        let object = create_object("New");

        assert!(object.is_instances_dirty());
    }

    #[test]
    fn composes_camera_object_and_instance() {
        let mut object = create_object("Placed");
        object.camera(Camera::orthographic_2d(800.0, 600.0));
        object.translate(100.0, 100.0, 0.0);

        let instance = Instance::new().translate(300.0, 200.0, 0.0);
        let transform = object.instance_transform(&instance);

        // ローカル原点は (400, 300) ピクセル = 画面中央 = クリップ原点。
        let [x, y, _] = transform.transform_point(0.0, 0.0, 0.0);
        assert!(x.abs() < 1e-6, "x was {x}");
        assert!(y.abs() < 1e-6, "y was {y}");
    }
}
