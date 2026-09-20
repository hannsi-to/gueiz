//! 3D のカメラ。透視投影と、錐台の 6 面。
//!
//! # 2D との違い
//!
//! 2D のカメラは z を捨てます（重なり順にしか使わないため）。3D は z が
//! 奥行きそのものなので捨てられません。そして **視点行列と投影行列を
//! 別々に持つ必要があります** — 世界空間で錐台カリングをするからです。
//!
//! 2D は `camera * object` を CPU で畳んで 1 本の行列にしていますが、
//! 3D で同じことをすると、カリングに使う世界座標が取り出せなくなります。

use gueiz_gpu::math::Mat4;

/// 視点と投影。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Camera3d {
    projection: Mat4,
    view: Mat4,
    eye: [f32; 3],
}

impl Camera3d {
    /// 透視投影のカメラ。`fov_y` はラジアン。
    ///
    /// ```
    /// # use gueiz_3d::camera::Camera3d;
    /// let camera = Camera3d::perspective(
    ///     std::f32::consts::FRAC_PI_3,
    ///     1280.0 / 720.0,
    ///     0.1,
    ///     1000.0,
    /// );
    /// ```
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        Self {
            projection: Mat4::perspective(fov_y, aspect, near, far),
            view: Mat4::IDENTITY,
            eye: [0.0, 0.0, 0.0],
        }
    }

    /// 視点を置いて、注視点を向く。
    pub fn look_at(&mut self, eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> &mut Self {
        self.view = Mat4::look_at(eye, target, up);
        self.eye = eye;
        self
    }

    /// 画面比が変わったら投影を貼り替える。
    pub fn resize(&mut self, fov_y: f32, aspect: f32, near: f32, far: f32) -> &mut Self {
        self.projection = Mat4::perspective(fov_y, aspect, near, far);
        self
    }

    pub fn projection(&self) -> Mat4 {
        self.projection
    }

    pub fn view(&self) -> Mat4 {
        self.view
    }

    /// 視点の位置。ライティングで視線方向が要るときに使う。
    pub fn eye(&self) -> [f32; 3] {
        self.eye
    }

    /// 頂点に掛ける行列。`projection * view`。
    ///
    /// **世界座標の頂点に掛ける**ことに注意。2D のように図形の変換まで
    /// 畳み込んでいない。
    pub fn view_projection(&self) -> Mat4 {
        self.projection.multiply(self.view)
    }

    /// 視錐台の 6 面。世界空間で、内側が正。
    ///
    /// カリングはこれを使って**世界座標のまま**判定する。
    pub fn frustum_planes(&self) -> [[f32; 4]; 6] {
        self.view_projection().frustum_planes()
    }
}

impl Default for Camera3d {
    /// 画角 60 度、比 1:1、0.1 から 1000 まで。
    fn default() -> Self {
        Self::perspective(std::f32::consts::FRAC_PI_3, 1.0, 0.1, 1000.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_at_the_origin_puts_it_in_the_middle() {
        let mut camera = Camera3d::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        camera.look_at([0.0, 0.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let centre = camera.view_projection().project_point(0.0, 0.0, 0.0);

        assert!(centre[0].abs() < 1e-5);
        assert!(centre[1].abs() < 1e-5);
        // 手前より奥なので 0..1 の中。
        assert!(centre[2] > 0.0 && centre[2] < 1.0, "{}", centre[2]);
    }

    /// 遠いほうが小さく写る。透視投影になっている証拠。
    #[test]
    fn distant_things_look_smaller() {
        let mut camera = Camera3d::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        camera.look_at([0.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]);

        let near = camera.view_projection().project_point(1.0, 0.0, -2.0);
        let far = camera.view_projection().project_point(1.0, 0.0, -20.0);

        assert!(near[0] > far[0], "近い {} 遠い {}", near[0], far[0]);
    }

    /// 錐台の面は、視線の先を内・背後を外と判定する。
    #[test]
    fn the_frustum_faces_the_way_the_camera_looks() {
        let mut camera = Camera3d::perspective(std::f32::consts::FRAC_PI_2, 1.0, 0.1, 100.0);
        camera.look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        let planes = camera.frustum_planes();
        let distance = |plane: [f32; 4], point: [f32; 3]| {
            plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] + plane[3]
        };

        // 視線の先
        assert!(planes.iter().all(|plane| distance(*plane, [0.0, 0.0, 0.0]) > 0.0));
        // カメラの背後
        assert!(planes.iter().any(|plane| distance(*plane, [0.0, 0.0, 50.0]) < 0.0));
    }

    #[test]
    fn the_eye_is_remembered() {
        let mut camera = Camera3d::default();
        camera.look_at([1.0, 2.0, 3.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        assert_eq!(camera.eye(), [1.0, 2.0, 3.0]);
    }
}
