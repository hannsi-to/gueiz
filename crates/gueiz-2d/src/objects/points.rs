//! 点を印として並べる。散布図、頂点のつまみ、粒など。
//!
//! # 点ごとに別の輪郭
//!
//! 1 点ずつ**独立した輪郭**として起こします。つなげてしまうと、
//! 隣の点と 1 本の折れ線になって別の形になります。
//!
//! 輪郭は塗り方ごとに独立して扱われるので、
//!
//! - [`PaintType::Fill`] なら、塗りつぶされた印
//! - [`PaintType::Stroke`] なら、**点ごとに縁取りだけ**の印
//!
//! ```
//! # use gueiz_2d::objects::points::{Points, PointType};
//! # use gueiz_2d::paint_type::PaintType;
//! # use gueiz_2d::vertex::Vertex;
//! # let at = |x, y| Vertex::new_position_color(x, y, 0.0, 1.0, 1.0, 1.0, 1.0);
//! let dots = Points::new("Dots")
//!     .paint_type(PaintType::Fill)
//!     .point(PointType::Circle { radius: 4.0 }, at(10.0, 10.0))
//!     .point(PointType::Circle { radius: 4.0 }, at(50.0, 10.0))
//!     .last_point(PointType::Rect { width: 8.0, height: 8.0 }, at(90.0, 10.0))
//!     .end();
//!
//! assert_eq!(dots.contour_count(), 3);
//! assert!(dots.contains(10.0, 10.0), "1 つめの真ん中");
//! assert!(!dots.contains(30.0, 10.0), "あいだは空いている");
//! ```
//!
//! # 型で順番を縛る
//!
//! ```text
//! new ──▶ paint_type ──▶ add_point（何度でも）──▶ last_point ──▶ end
//! ```
//!
//! 点を 1 つも置かずに終えることはできません。

use std::f32::consts::{PI, TAU};
use std::marker::PhantomData;

use gueiz_gpu::vertex::Vertex;
use crate::object::{create_object, Object};
use crate::paint_type::PaintType;

pub struct Points<State> {
    object: Object,
    points: Vec<PointData>,
    _state: PhantomData<State>,
}

struct PointData {
    point: Vertex,
    point_type: PointType,
}

#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum PointType {
    Circle { radius: f32 },
    Rect { width: f32, height: f32 },
}

pub struct Init;
pub struct PaintTypeSet;
pub struct VertexSet;
pub struct End;

impl<State> Points<State> {
    fn advance<Next>(self) -> Points<Next> {
        Points {
            object: self.object,
            points: self.points,
            _state: PhantomData,
        }
    }
}

impl Points<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            points: Vec::new(),
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> Points<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl Points<PaintTypeSet> {
    pub fn point(mut self, point_type: PointType, vertex: Vertex) -> Points<PaintTypeSet> {
        self.points.push(PointData{
            point: vertex,
            point_type,
        });
        self.advance()
    }

    pub fn last_point(mut self, point_type: PointType, vertex: Vertex) -> Points<End> {
        self.points.push(PointData{
            point: vertex,
            point_type,
        });
        self.advance()
    }
}

impl Points<End> {
    pub fn end(mut self) -> Object {
        let points = std::mem::take(&mut self.points);
        let mut written = 0;

        for PointData { point, point_type } in points {
            let corners = point_type.outline(point);

            if corners.is_empty() {
                continue;
            }

            if written > 0 {
                self.object.begin_hole();
            }

            for corner in corners {
                self.object.put_vertex(corner);
            }

            written += 1;
        }

        self.object.end();
        self.object
    }
}

impl PointType {
    fn outline(self, centre: Vertex) -> Vec<Vertex> {
        let at = |x: f32, y: f32| {
            let mut corner = centre;
            corner.x = x;
            corner.y = y;
            corner
        };

        match self {
            Self::Circle { radius } => {
                if radius <= 0.0 {
                    return Vec::new();
                }

                let steps = circle_steps(radius);

                (0..steps)
                    .map(|step| {
                        let angle = TAU * step as f32 / steps as f32;
                        at(
                            centre.x + radius * angle.cos(),
                            centre.y + radius * angle.sin(),
                        )
                    })
                    .collect()
            }

            Self::Rect { width, height } => {
                if width <= 0.0 || height <= 0.0 {
                    return Vec::new();
                }

                let (half_width, half_height) = (width / 2.0, height / 2.0);

                vec![
                    at(centre.x - half_width, centre.y - half_height),
                    at(centre.x + half_width, centre.y - half_height),
                    at(centre.x + half_width, centre.y + half_height),
                    at(centre.x - half_width, centre.y + half_height),
                ]
            }
        }
    }
}

const MAX_SAG: f32 = 0.25;
const MIN_CIRCLE_STEPS: u32 = 8;
const MAX_CIRCLE_STEPS: u32 = 128;

pub(crate) fn circle_steps(radius: f32) -> u32 {
    if radius <= MAX_SAG {
        return MIN_CIRCLE_STEPS;
    }

    let steps = PI / (1.0 - MAX_SAG / radius).clamp(-1.0, 1.0).acos();

    (steps.ceil() as u32).clamp(MIN_CIRCLE_STEPS, MAX_CIRCLE_STEPS)
}
