//! 描画に要る最小限の行列演算。
//!
//! 保持は**列優先**。WGSL の `mat4x4<f32>(c0, c1, c2, c3)` が列を取るので、
//! [`Mat4::to_columns`] の結果をそのまま頂点属性に流せる。

/// 4x4 行列。列優先（`columns[列][行]`）。
#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub struct Mat4 {
    columns: [[f32; 4]; 4],
}

impl Mat4 {
    pub const IDENTITY: Self = Self {
        columns: [
            [1.0, 0.0, 0.0, 0.0],
            [0.0, 1.0, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ],
    };

    pub const fn from_columns(columns: [[f32; 4]; 4]) -> Self {
        Self { columns }
    }

    pub const fn to_columns(self) -> [[f32; 4]; 4] {
        self.columns
    }

    pub const fn from_translation(x: f32, y: f32, z: f32) -> Self {
        Self {
            columns: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [x, y, z, 1.0],
            ],
        }
    }

    pub const fn from_scale(x: f32, y: f32, z: f32) -> Self {
        Self {
            columns: [
                [x, 0.0, 0.0, 0.0],
                [0.0, y, 0.0, 0.0],
                [0.0, 0.0, z, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn from_rotation_x(radians: f32) -> Self {
        let (sine, cosine) = radians.sin_cos();

        Self {
            columns: [
                [1.0, 0.0, 0.0, 0.0],
                [0.0, cosine, sine, 0.0],
                [0.0, -sine, cosine, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn from_rotation_y(radians: f32) -> Self {
        let (sine, cosine) = radians.sin_cos();

        Self {
            columns: [
                [cosine, 0.0, -sine, 0.0],
                [0.0, 1.0, 0.0, 0.0],
                [sine, 0.0, cosine, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    pub fn from_rotation_z(radians: f32) -> Self {
        let (sine, cosine) = radians.sin_cos();

        Self {
            columns: [
                [cosine, sine, 0.0, 0.0],
                [-sine, cosine, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [0.0, 0.0, 0.0, 1.0],
            ],
        }
    }

    /// オイラー角から回転を作る。適用順は Z → Y → X。
    ///
    /// 2D で使うぶんには `z` だけ指定すれば平面内の回転になる。
    pub fn from_rotation(x: f32, y: f32, z: f32) -> Self {
        Self::from_rotation_x(x)
            .multiply(Self::from_rotation_y(y))
            .multiply(Self::from_rotation_z(z))
    }

    /// 左上を原点としたピクセル座標をクリップ空間に写す正射影。
    ///
    /// `(0, 0)` が左上、`(width, height)` が右下になる。
    pub fn orthographic_2d(width: f32, height: f32) -> Self {
        Self {
            columns: [
                [2.0 / width, 0.0, 0.0, 0.0],
                // クリップ空間は y が上向きなので反転する。
                [0.0, -2.0 / height, 0.0, 0.0],
                // z は捨てる。2D では奥行きではなく**重なり順**にしか使わず、
                // 深度バッファも無い。そのまま通すと、クリップ範囲（0..1）を
                // 外れた z の図形が丸ごと切り落とされてしまう。
                [0.0, 0.0, 0.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
        }
    }

    /// 透視投影。`fov_y` はラジアン、奥行きは `near`..`far`。
    ///
    /// クリップ空間の z は wgpu に合わせて 0..1。OpenGL の -1..1 とは違う。
    pub fn perspective(fov_y: f32, aspect: f32, near: f32, far: f32) -> Self {
        let focal = 1.0 / (fov_y * 0.5).tan();
        let range = 1.0 / (near - far);

        Self {
            columns: [
                [focal / aspect, 0.0, 0.0, 0.0],
                [0.0, focal, 0.0, 0.0],
                [0.0, 0.0, far * range, -1.0],
                [0.0, 0.0, near * far * range, 0.0],
            ],
        }
    }

    /// `eye` から `target` を見る視点行列。
    pub fn look_at(eye: [f32; 3], target: [f32; 3], up: [f32; 3]) -> Self {
        // 右手系。前方は -z。
        let forward = normalize(subtract(target, eye));
        let right = normalize(cross(forward, up));
        let actual_up = cross(right, forward);

        Self {
            columns: [
                [right[0], actual_up[0], -forward[0], 0.0],
                [right[1], actual_up[1], -forward[1], 0.0],
                [right[2], actual_up[2], -forward[2], 0.0],
                [
                    -dot(right, eye),
                    -dot(actual_up, eye),
                    dot(forward, eye),
                    1.0,
                ],
            ],
        }
    }

    /// 視錐台の 6 面。`[a, b, c, d]` で `a*x + b*y + c*z + d >= 0` が内側。
    ///
    /// 並びは 左・右・下・上・手前・奥。`self` は `projection * view` を渡す。
    ///
    /// 行を足し引きして取り出す定番の方法。クリップ空間の各面
    /// （`-w <= x <= w` など）を世界空間に引き戻したものになる。
    /// 手前の面だけは wgpu の z 範囲が 0..1 なので `w` を足さない。
    pub fn frustum_planes(self) -> [[f32; 4]; 6] {
        // 行を取り出す。`columns[列][行]` なので転置して読む。
        let row = |index: usize| {
            [
                self.columns[0][index],
                self.columns[1][index],
                self.columns[2][index],
                self.columns[3][index],
            ]
        };

        let x = row(0);
        let y = row(1);
        let z = row(2);
        let w = row(3);

        [
            normalize_plane(add4(w, x)),      // 左
            normalize_plane(subtract4(w, x)), // 右
            normalize_plane(add4(w, y)),      // 下
            normalize_plane(subtract4(w, y)), // 上
            normalize_plane(z),               // 手前（z >= 0）
            normalize_plane(subtract4(w, z)), // 奥
        ]
    }

    /// `self * other`。`other` が先に適用される。
    pub fn multiply(self, other: Self) -> Self {
        let mut columns = [[0.0; 4]; 4];

        for column in 0..4 {
            for row in 0..4 {
                let mut sum = 0.0;
                for index in 0..4 {
                    sum += self.columns[index][row] * other.columns[column][index];
                }
                columns[column][row] = sum;
            }
        }

        Self { columns }
    }

    /// 点（w = 1）を変換する。
    /// 点を写して、`w` で割る。透視投影ではこちらでないとクリップ空間に出ない。
    ///
    /// [`Mat4::transform_point`] は `w` で割らない。正射影では `w` が 1 のままなので
    /// 同じ結果になるが、透視投影では違う。
    pub fn project_point(self, x: f32, y: f32, z: f32) -> [f32; 3] {
        let mut result = [0.0; 4];

        for row in 0..4 {
            result[row] = self.columns[0][row] * x
                + self.columns[1][row] * y
                + self.columns[2][row] * z
                + self.columns[3][row];
        }

        if result[3].abs() < f32::EPSILON {
            return [result[0], result[1], result[2]];
        }

        [
            result[0] / result[3],
            result[1] / result[3],
            result[2] / result[3],
        ]
    }

    pub fn transform_point(self, x: f32, y: f32, z: f32) -> [f32; 3] {
        let mut result = [0.0; 3];

        for row in 0..3 {
            result[row] = self.columns[0][row] * x
                + self.columns[1][row] * y
                + self.columns[2][row] * z
                + self.columns[3][row];
        }

        result
    }
}

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}


/// 3 要素ベクトルの引き算。
fn subtract(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = dot(v, v).sqrt();

    if length < f32::EPSILON {
        return [0.0, 0.0, 0.0];
    }

    [v[0] / length, v[1] / length, v[2] / length]
}

fn add4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2], a[3] + b[3]]
}

fn subtract4(a: [f32; 4], b: [f32; 4]) -> [f32; 4] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2], a[3] - b[3]]
}

/// 面の法線の長さを 1 にする。そうしないと点との距離が測れない。
fn normalize_plane(plane: [f32; 4]) -> [f32; 4] {
    let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();

    if length < f32::EPSILON {
        return plane;
    }

    [
        plane[0] / length,
        plane[1] / length,
        plane[2] / length,
        plane[3] / length,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_neutral() {
        let translation = Mat4::from_translation(3.0, 4.0, 5.0);

        assert_eq!(translation.multiply(Mat4::IDENTITY), translation);
        assert_eq!(Mat4::IDENTITY.multiply(translation), translation);
    }

    #[test]
    fn translates_a_point() {
        let matrix = Mat4::from_translation(10.0, 20.0, 0.0);

        assert_eq!(matrix.transform_point(1.0, 2.0, 0.0), [11.0, 22.0, 0.0]);
    }

    #[test]
    fn applies_the_right_hand_side_first() {
        // 「原点まわりに 2 倍してから (10, 0) へ動かす」の順になる。
        let matrix = Mat4::from_translation(10.0, 0.0, 0.0).multiply(Mat4::from_scale(2.0, 2.0, 1.0));

        assert_eq!(matrix.transform_point(1.0, 0.0, 0.0), [12.0, 0.0, 0.0]);
    }

    #[test]
    fn rotates_a_quarter_turn_around_z() {
        let matrix = Mat4::from_rotation_z(std::f32::consts::FRAC_PI_2);
        let [x, y, _] = matrix.transform_point(1.0, 0.0, 0.0);

        assert!((x - 0.0).abs() < 1e-6, "x was {x}");
        assert!((y - 1.0).abs() < 1e-6, "y was {y}");
    }

    #[test]
    fn maps_pixel_corners_to_clip_space() {
        let matrix = Mat4::orthographic_2d(800.0, 600.0);

        // 左上 -> (-1, 1)、右下 -> (1, -1)
        assert_eq!(matrix.transform_point(0.0, 0.0, 0.0), [-1.0, 1.0, 0.0]);
        assert_eq!(matrix.transform_point(800.0, 600.0, 0.0), [1.0, -1.0, 0.0]);
        assert_eq!(matrix.transform_point(400.0, 300.0, 0.0), [0.0, 0.0, 0.0]);
    }

    /// z をどれだけ離しても、クリップ空間では 0 に潰れる。
    ///
    /// 潰さないと、重なり順のために z を大きくした図形がクリップ範囲を外れて
    /// 消える。2D では z は順番でしかない。
    #[test]
    fn the_projection_drops_z() {
        let matrix = Mat4::orthographic_2d(800.0, 600.0);

        for z in [-100.0, -1.0, 0.0, 1.0, 100.0] {
            let [x, y, projected] = matrix.transform_point(400.0, 300.0, z);

            assert_eq!(projected, 0.0, "z = {z} が潰れていない");
            assert_eq!([x, y], [0.0, 0.0], "z が x と y に漏れている");
        }
    }

    /// 透視投影は、手前の面を 0、奥の面を 1 に写す（wgpu の z 範囲）。
    #[test]
    fn perspective_maps_depth_to_zero_one() {
        let matrix = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);

        // 視点の前方は -z。
        let near = matrix.project_point(0.0, 0.0, -1.0);
        let far = matrix.project_point(0.0, 0.0, -100.0);

        assert!(near[2].abs() < 1e-4, "手前は 0 のはず: {}", near[2]);
        assert!((far[2] - 1.0).abs() < 1e-4, "奥は 1 のはず: {}", far[2]);
    }

    /// 90 度の画角なら、奥行きと同じだけ横にずれた点がちょうど端に来る。
    #[test]
    fn perspective_puts_the_field_of_view_at_the_edge() {
        let matrix = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);

        let edge = matrix.project_point(10.0, 0.0, -10.0);
        assert!((edge[0] - 1.0).abs() < 1e-4, "右端は 1 のはず: {}", edge[0]);
    }

    #[test]
    fn look_at_puts_the_target_in_the_middle() {
        let view = Mat4::look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        // 視点から見て、注視点は前方 10 のところ（前方は -z）。
        let target = view.transform_point(0.0, 0.0, 0.0);
        assert!((target[0]).abs() < 1e-5);
        assert!((target[1]).abs() < 1e-5);
        assert!((target[2] + 10.0).abs() < 1e-5, "{}", target[2]);
    }

    /// 視錐台の面は、内側の点で正・外側の点で負になる。
    ///
    /// ここを間違えると、見えているものが消えるか、見えないものを描き続ける。
    #[test]
    fn frustum_planes_separate_inside_from_outside() {
        let projection = Mat4::perspective(std::f32::consts::FRAC_PI_2, 1.0, 1.0, 100.0);
        let view = Mat4::look_at([0.0, 0.0, 10.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);
        let planes = projection.multiply(view).frustum_planes();

        let distance = |plane: [f32; 4], point: [f32; 3]| {
            plane[0] * point[0] + plane[1] * point[1] + plane[2] * point[2] + plane[3]
        };

        // 原点は視錐台の中。どの面から見ても正。
        for (index, plane) in planes.iter().enumerate() {
            assert!(
                distance(*plane, [0.0, 0.0, 0.0]) > 0.0,
                "面 {index} が原点を外と判定した",
            );
        }

        // 真横に大きく外れた点は、どれか 1 つの面で負になる。
        for outside in [
            [1000.0, 0.0, 0.0],
            [-1000.0, 0.0, 0.0],
            [0.0, 1000.0, 0.0],
            [0.0, 0.0, 1000.0],
            [0.0, 0.0, -1000.0],
        ] {
            assert!(
                planes.iter().any(|plane| distance(*plane, outside) < 0.0),
                "{outside:?} が外と判定されない",
            );
        }
    }

    /// 面の法線は長さ 1。そうでないと、半径との比較が狂う。
    #[test]
    fn frustum_planes_are_normalised() {
        let projection = Mat4::perspective(1.2, 1.777, 0.1, 500.0);
        let view = Mat4::look_at([3.0, 4.0, 5.0], [0.0, 0.0, 0.0], [0.0, 1.0, 0.0]);

        for plane in projection.multiply(view).frustum_planes() {
            let length = (plane[0] * plane[0] + plane[1] * plane[1] + plane[2] * plane[2]).sqrt();
            assert!((length - 1.0).abs() < 1e-5, "{length}");
        }
    }
}
