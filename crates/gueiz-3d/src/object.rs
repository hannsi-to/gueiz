//! 描くもの 1 つぶん。メッシュと、置き方と、複製。
//!
//! # 2D との違い
//!
//! 2D の `Object` は輪郭を記録する API（`begin` / `put_vertex` / `end`）を
//! 持っていて、閉じたときに三角形へ開きます。3D は三角形が最初からあるので、
//! **メッシュを渡すだけ**です。
//!
//! 一方で、置き方（平行移動・回転・拡大）と複製の持ち方は 2D とほぼ同じです。

use gueiz_gpu::math::Mat4;

/// 同じメッシュを別の場所・別の色で描くための複製。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Instance3d {
    translation: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 3],
    tint: [f32; 4],
}

/// 変換なし・白の複製を作る。
pub fn create_instance() -> Instance3d {
    Instance3d::new()
}

impl Instance3d {
    pub fn new() -> Self {
        Self {
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
            tint: [1.0; 4],
        }
    }

    pub fn translate(mut self, x: f32, y: f32, z: f32) -> Self {
        self.translation = [x, y, z];
        self
    }

    /// ラジアン。`x` 軸 → `y` 軸 → `z` 軸の順に掛かる。
    pub fn rotate(mut self, x: f32, y: f32, z: f32) -> Self {
        self.rotation = [x, y, z];
        self
    }

    pub fn scale(mut self, x: f32, y: f32, z: f32) -> Self {
        self.scale = [x, y, z];
        self
    }

    pub fn color(mut self, red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        self.tint = [red, green, blue, alpha];
        self
    }

    pub fn set_translation(&mut self, x: f32, y: f32, z: f32) {
        self.translation = [x, y, z];
    }

    pub fn set_rotation(&mut self, x: f32, y: f32, z: f32) {
        self.rotation = [x, y, z];
    }

    pub fn set_scale(&mut self, x: f32, y: f32, z: f32) {
        self.scale = [x, y, z];
    }

    pub fn set_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.tint = [red, green, blue, alpha];
    }

    pub fn translation(&self) -> [f32; 3] {
        self.translation
    }

    pub fn rotation_angles(&self) -> [f32; 3] {
        self.rotation
    }

    pub fn scale_factors(&self) -> [f32; 3] {
        self.scale
    }

    pub fn tint(&self) -> [f32; 4] {
        self.tint
    }

    /// 平行移動 → 回転 → 拡大。コンピュートシェーダ側と同じ順。
    pub fn transform(&self) -> Mat4 {
        let [tx, ty, tz] = self.translation;
        let [rx, ry, rz] = self.rotation;
        let [sx, sy, sz] = self.scale;

        Mat4::from_translation(tx, ty, tz)
            .multiply(Mat4::from_rotation(rx, ry, rz))
            .multiply(Mat4::from_scale(sx, sy, sz))
    }
}

impl Default for Instance3d {
    fn default() -> Self {
        Self::new()
    }
}

/// メッシュ 1 つと、その置き方と、複製。
pub struct Object3d {
    name: String,
    mesh: crate::mesh::Mesh,
    /// メッシュが差し替わったので積み直しが要る。
    mesh_dirty: bool,

    translation: [f32; 3],
    rotation: [f32; 3],
    scale: [f32; 3],

    instances: Vec<Instance3d>,
    /// 複製が書き換わったので送り直しが要る。
    instances_dirty: bool,
}

/// メッシュに名前を付けて、描くものにする。
pub fn create_object(name: &str, mesh: crate::mesh::Mesh) -> Object3d {
    Object3d::new(name, mesh)
}

impl Object3d {
    pub fn new(name: &str, mesh: crate::mesh::Mesh) -> Self {
        Self {
            name: String::from(name),
            mesh,
            mesh_dirty: true,
            translation: [0.0; 3],
            rotation: [0.0; 3],
            scale: [1.0; 3],
            instances: Vec::new(),
            instances_dirty: true,
        }
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    /// 名前を付け替える。**[`crate::draw_manager::DrawManager3d`] だけが呼ぶ。**
    ///
    /// 名前は登録先でひとつに保たれているので、外から勝手に変えられると
    /// 索引と食い違います。だから公開していません。
    pub(crate) fn rename(&mut self, name: String) {
        self.name = name;
    }

    pub fn mesh(&self) -> &crate::mesh::Mesh {
        &self.mesh
    }

    /// メッシュを差し替える。頂点と索引が積み直される。
    pub fn set_mesh(&mut self, mesh: crate::mesh::Mesh) -> &mut Self {
        self.mesh = mesh;
        self.mesh_dirty = true;
        self
    }

    pub fn translate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.translation = [x, y, z];
        self
    }

    pub fn rotate(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.rotation = [x, y, z];
        self
    }

    pub fn scale(&mut self, x: f32, y: f32, z: f32) -> &mut Self {
        self.scale = [x, y, z];
        self
    }

    /// 複製を 1 つ足す。1 つも足さなければ、変換なしの 1 つがあるものとして描く。
    pub fn instance(&mut self, instance: Instance3d) -> usize {
        self.instances.push(instance);
        self.instances_dirty = true;
        self.instances.len() - 1
    }

    /// 全複製を書き換える。呼ぶだけで送り直しの印が付く。
    pub fn instances_mut(&mut self) -> &mut [Instance3d] {
        self.instances_dirty = true;
        &mut self.instances
    }

    pub fn instances(&self) -> &[Instance3d] {
        &self.instances
    }

    pub fn clear_instances(&mut self) -> &mut Self {
        self.instances.clear();
        self.instances_dirty = true;
        self
    }

    /// 世界空間へ写す行列。**カメラは掛けない。**
    ///
    /// 2D は `camera * object` を畳んでいるが、3D は錐台カリングを世界空間で
    /// やるので、カメラと分けておく必要がある。
    pub fn world_transform(&self) -> Mat4 {
        let [tx, ty, tz] = self.translation;
        let [rx, ry, rz] = self.rotation;
        let [sx, sy, sz] = self.scale;

        Mat4::from_translation(tx, ty, tz)
            .multiply(Mat4::from_rotation(rx, ry, rz))
            .multiply(Mat4::from_scale(sx, sy, sz))
    }

    /// この図形の一番大きな拡大率。境界球の半径に掛ける。
    pub fn max_scale(&self) -> f32 {
        self.scale[0].abs().max(self.scale[1].abs()).max(self.scale[2].abs())
    }

    pub(crate) fn is_mesh_dirty(&self) -> bool {
        self.mesh_dirty
    }

    pub(crate) fn clear_mesh_dirty(&mut self) {
        self.mesh_dirty = false;
    }

    pub(crate) fn is_instances_dirty(&self) -> bool {
        self.instances_dirty
    }

    pub(crate) fn clear_instances_dirty(&mut self) {
        self.instances_dirty = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::Mesh;

    fn cube() -> Object3d {
        create_object("Cube", Mesh::cube(2.0, [1.0; 4]))
    }

    #[test]
    fn a_fresh_object_needs_uploading() {
        let object = cube();

        assert!(object.is_mesh_dirty());
        assert!(object.is_instances_dirty());
    }

    #[test]
    fn reading_instances_leaves_it_clean() {
        let mut object = cube();
        object.instance(create_instance());
        object.clear_instances_dirty();

        let _ = object.instances().len();

        assert!(!object.is_instances_dirty());
    }

    #[test]
    fn writing_instances_marks_it_dirty() {
        let mut object = cube();
        object.instance(create_instance());
        object.clear_instances_dirty();

        object.instances_mut()[0].set_translation(1.0, 0.0, 0.0);

        assert!(object.is_instances_dirty());
    }

    #[test]
    fn replacing_the_mesh_marks_it_dirty() {
        let mut object = cube();
        object.clear_mesh_dirty();

        object.set_mesh(Mesh::sphere(1.0, 8, 6, [1.0; 4]));

        assert!(object.is_mesh_dirty());
        assert_eq!(object.mesh().triangle_count(), 8 * 6 * 2);
    }

    /// 世界へ写す行列にカメラは入らない。入れるとカリングが世界空間でできない。
    #[test]
    fn the_world_transform_places_the_object() {
        let mut object = cube();
        object.translate(10.0, 0.0, -5.0);

        assert_eq!(
            object.world_transform().transform_point(0.0, 0.0, 0.0),
            [10.0, 0.0, -5.0],
        );
    }

    /// 境界球に掛ける拡大率は、いちばん大きい軸で見積もる。
    /// 小さいほうで見ると、はみ出した部分が消える。
    #[test]
    fn the_scale_for_culling_takes_the_largest_axis() {
        let mut object = cube();
        object.scale(1.0, 5.0, 2.0);

        assert_eq!(object.max_scale(), 5.0);

        object.scale(-7.0, 1.0, 1.0);
        assert_eq!(object.max_scale(), 7.0, "負の拡大も大きさとして見る");
    }

    #[test]
    fn an_instance_applies_translation_then_rotation_then_scale() {
        let instance = create_instance().scale(2.0, 2.0, 2.0).translate(10.0, 0.0, 0.0);

        // 原点まわりに 2 倍してから動かす。逆順なら (20, 0, 0)。
        assert_eq!(instance.transform().transform_point(1.0, 0.0, 0.0), [12.0, 0.0, 0.0]);
    }
}
