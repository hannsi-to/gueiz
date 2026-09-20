//! 同じ図形を別の位置・色で描くための複製。

use crate::math::Mat4;

/// 図形 1 つぶんの複製。オブジェクト自身の変換に、さらにこれが掛かる。
///
/// ```no_run
/// # use gueiz_2d::object::instance;
/// let instance = instance::create_instance()
///     .translate(100.0, 50.0, 0.0)
///     .color(1.0, 0.0, 0.0, 1.0);
/// ```
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Instance {
    translation: [f32; 3],
    scale: [f32; 3],
    rotation: [f32; 3],
    color: [f32; 4],
}

/// 変換なし・白のインスタンスを作る。
pub fn create_instance() -> Instance {
    Instance::new()
}

impl Instance {
    pub fn new() -> Self {
        Self {
            translation: [0.0; 3],
            scale: [1.0; 3],
            rotation: [0.0; 3],
            color: [1.0; 4],
        }
    }

    pub fn translate(mut self, x: f32, y: f32, z: f32) -> Self {
        self.translation = [x, y, z];
        self
    }

    pub fn scale(mut self, x: f32, y: f32, z: f32) -> Self {
        self.scale = [x, y, z];
        self
    }

    /// ラジアン。2D なら `z` だけ指定すれば平面内で回る。
    pub fn rotate(mut self, x: f32, y: f32, z: f32) -> Self {
        self.rotation = [x, y, z];
        self
    }

    /// 頂点色に掛かる色。白なら頂点色がそのまま出る。
    pub fn color(mut self, red: f32, green: f32, blue: f32, alpha: f32) -> Self {
        self.color = [red, green, blue, alpha];
        self
    }

    /// 毎フレーム書き換える用。[`Instance::translate`] と違い所有権を取らない。
    pub fn set_translation(&mut self, x: f32, y: f32, z: f32) {
        self.translation = [x, y, z];
    }

    pub fn set_scale(&mut self, x: f32, y: f32, z: f32) {
        self.scale = [x, y, z];
    }

    pub fn set_rotation(&mut self, x: f32, y: f32, z: f32) {
        self.rotation = [x, y, z];
    }

    pub fn set_color(&mut self, red: f32, green: f32, blue: f32, alpha: f32) {
        self.color = [red, green, blue, alpha];
    }

    pub fn translation(&self) -> [f32; 3] {
        self.translation
    }

    pub fn scale_factors(&self) -> [f32; 3] {
        self.scale
    }

    pub fn rotation_angles(&self) -> [f32; 3] {
        self.rotation
    }

    pub fn tint(&self) -> [f32; 4] {
        self.color
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
}

impl Default for Instance {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_fresh_instance_changes_nothing() {
        let transform = create_instance().transform();

        assert_eq!(transform.transform_point(3.0, 4.0, 5.0), [3.0, 4.0, 5.0]);
        assert_eq!(create_instance().tint(), [1.0; 4]);
    }

    #[test]
    fn scales_before_translating() {
        let transform = create_instance()
            .scale(2.0, 2.0, 1.0)
            .translate(10.0, 0.0, 0.0)
            .transform();

        // 原点まわりに 2 倍してから動かす。逆順なら (22, 0) になる。
        assert_eq!(transform.transform_point(1.0, 0.0, 0.0), [12.0, 0.0, 0.0]);
    }
}
