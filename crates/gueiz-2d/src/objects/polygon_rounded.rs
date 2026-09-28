use std::f32::consts::{FRAC_PI_2, TAU};
use std::marker::PhantomData;

use gueiz_gpu::vertex::Vertex;
use crate::object::{create_object, Object};
use crate::objects::points::circle_steps;
use crate::paint_type::PaintType;

pub struct PolygonRounded<State> {
    object: Object,
    corner_datum: Vec<CornerData>,
    _state: PhantomData<State>,
}

#[derive(Clone, Copy)]
#[derive(PartialEq)]
#[derive(Debug)]
pub enum CornerType {
    None{},
    Circle{
        radius: f32,
    },
    Ellipse{
        radius_x: f32,
        radius_y: f32,
    },
    Bevel {
        radius: f32,
    },
    Chanfer {
        radius_x: f32,
        radius_y: f32,
    },
    Inset {
        radius_x: f32,
        radius_y: f32,
    },
    Concave {
        radius_x: f32,
        radius_y: f32,
    }
}

pub(crate) struct CornerData {
    pub(crate) vertex: Vertex,
    pub(crate) corner_type: CornerType,
}

impl CornerData {
    pub(crate) fn new(corner_type: CornerType, vertex: Vertex) -> Self {
        Self {
            vertex,
            corner_type,
        }
    }
}

pub struct Init;
pub struct PaintTypeSet;
pub struct Vertex1Set;
pub struct Vertex2Set;
pub struct VertexSet;
pub struct End;

impl<State> PolygonRounded<State> {
    fn advance<Next>(self) -> PolygonRounded<Next> {
        PolygonRounded {
            object: self.object,
            corner_datum: self.corner_datum,
            _state: PhantomData,
        }
    }
}

impl PolygonRounded<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            corner_datum: Vec::new(),
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> PolygonRounded<Vertex1Set> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl PolygonRounded<Vertex1Set> {
    pub fn vertex(mut self, corner_type: CornerType, vertex: Vertex) -> PolygonRounded<Vertex2Set> {
        self.corner_datum.push(
            CornerData {
                corner_type,
                vertex,
            }
        );
        self.advance()
    }
}

impl PolygonRounded<Vertex2Set> {
    pub fn vertex(mut self, corner_type: CornerType, vertex: Vertex) -> PolygonRounded<VertexSet> {
        self.corner_datum.push(
            CornerData {
                corner_type,
                vertex,
            }
        );
        self.advance()
    }
}

impl PolygonRounded<VertexSet> {
    pub fn vertex(mut self, corner_type: CornerType, vertex: Vertex) -> PolygonRounded<VertexSet> {
        self.corner_datum.push(
            CornerData {
                corner_type,
                vertex,
            }
        );
        self.advance()
    }

    pub fn last_vertex(mut self, corner_type: CornerType, vertex: Vertex) -> PolygonRounded<End> {
        self.corner_datum.push(
            CornerData {
                corner_type,
                vertex,
            }
        );
        self.advance()
    }
}

impl PolygonRounded<End> {
    pub fn end(mut self) -> Object {
        let corner_datum = std::mem::take(&mut self.corner_datum);

        for vertex in outline(&corner_datum) {
            self.object.put_vertex(vertex);
        }

        self.object.end();
        self.object
    }
}

pub(crate) fn outline(corner_datum: &[CornerData]) -> Vec<Vertex> {
    let count = corner_datum.len();

    if count < 3 {
        return corner_datum.iter().map(|data| data.vertex).collect();
    }

    let neighbours = |index: usize| {
        (
            corner_datum[(index + count - 1) % count].vertex,
            corner_datum[index].vertex,
            corner_datum[(index + 1) % count].vertex,
        )
    };

    let cuts: Vec<Cut> = (0..count)
        .map(|index| {
            let (previous, corner, next) = neighbours(index);
            Cut::measure(corner_datum[index].corner_type, previous, corner, next)
        })
        .collect();

    let factors = shrink_factors(corner_datum, &cuts);

    let mut vertices = Vec::new();

    for index in 0..count {
        let (previous, corner, next) = neighbours(index);
        let cut = &cuts[index];
        let factor = factors[index];

        let towards = |neighbour: Vertex, distance: f32| {
            let span = length(neighbour.x - corner.x, neighbour.y - corner.y);

            if span > 0.0 {
                blend(corner, neighbour, (distance * factor / span).clamp(0.0, 1.0))
            } else {
                corner
            }
        };

        let entry = towards(previous, cut.in_distance);
        let exit = towards(next, cut.out_distance);

        for &([x, y], ratio) in &cut.points {
            let mut vertex = blend(entry, exit, ratio);
            vertex.x = corner.x + x * factor;
            vertex.y = corner.y + y * factor;
            vertices.push(vertex);
        }
    }

    vertices
}

struct Cut {
    points: Vec<([f32; 2], f32)>,
    in_distance: f32,
    out_distance: f32,
}

impl Cut {
    fn sharp() -> Self {
        Self {
            points: vec![([0.0, 0.0], 0.0)],
            in_distance: 0.0,
            out_distance: 0.0,
        }
    }

    fn measure(corner_type: CornerType, previous: Vertex, corner: Vertex, next: Vertex) -> Self {
        let (radius_x, radius_y) = match corner_type {
            CornerType::None {} => return Self::sharp(),

            CornerType::Circle { radius } | CornerType::Bevel { radius } => (radius, radius),

            CornerType::Ellipse { radius_x, radius_y }
            | CornerType::Chanfer { radius_x, radius_y }
            | CornerType::Inset { radius_x, radius_y }
            | CornerType::Concave { radius_x, radius_y } => (radius_x, radius_y),
        };

        let Some(wedge) = Wedge::measure(previous, corner, next, radius_x, radius_y) else {
            return Self::sharp();
        };

        let (entry, exit) = (wedge.entry(), wedge.exit());

        let points = match corner_type {
            CornerType::None {} => vec![([0.0, 0.0], 0.0)],

            CornerType::Circle { .. } | CornerType::Ellipse { .. } => wedge.round(),

            CornerType::Bevel { .. } | CornerType::Chanfer { .. } => {
                vec![(entry, 0.0), (exit, 1.0)]
            }

            CornerType::Inset { .. } => vec![
                (entry, 0.0),
                ([entry[0] + exit[0], entry[1] + exit[1]], 0.5),
                (exit, 1.0),
            ],

            CornerType::Concave { .. } => wedge.scoop(),
        };

        Self {
            in_distance: length(entry[0], entry[1]),
            out_distance: length(exit[0], exit[1]),
            points,
        }
    }
}

struct Wedge {
    entry_direction: [f32; 2],
    exit_direction: [f32; 2],
    tangent: f32,
    sine: f32,
    cosine: f32,
    radius_x: f32,
    radius_y: f32,
}

const MIN_SINE: f32 = 1e-4;

impl Wedge {
    fn measure(
        previous: Vertex,
        corner: Vertex,
        next: Vertex,
        radius_x: f32,
        radius_y: f32,
    ) -> Option<Self> {
        let usable = |radius: f32| radius.is_finite() && radius > 0.0;

        if !usable(radius_x) || !usable(radius_y) {
            return None;
        }

        let towards = |neighbour: Vertex| {
            normalise(
                (neighbour.x - corner.x) / radius_x,
                (neighbour.y - corner.y) / radius_y,
            )
        };

        let entry_direction = towards(previous)?;
        let exit_direction = towards(next)?;

        let cosine = (entry_direction[0] * exit_direction[0]
            + entry_direction[1] * exit_direction[1])
            .clamp(-1.0, 1.0);
        let sine = (entry_direction[0] * exit_direction[1]
            - entry_direction[1] * exit_direction[0])
            .abs();

        if sine < MIN_SINE {
            return None;
        }

        Some(Self {
            entry_direction,
            exit_direction,
            tangent: (1.0 + cosine) / sine,
            sine,
            cosine,
            radius_x,
            radius_y,
        })
    }

    fn unscale(&self, x: f32, y: f32) -> [f32; 2] {
        [x * self.radius_x, y * self.radius_y]
    }

    fn entry(&self) -> [f32; 2] {
        self.unscale(
            self.entry_direction[0] * self.tangent,
            self.entry_direction[1] * self.tangent,
        )
    }

    fn exit(&self) -> [f32; 2] {
        self.unscale(
            self.exit_direction[0] * self.tangent,
            self.exit_direction[1] * self.tangent,
        )
    }

    fn angle(&self) -> f32 {
        self.cosine.acos()
    }

    fn round(&self) -> Vec<([f32; 2], f32)> {
        let centre = [
            (self.entry_direction[0] + self.exit_direction[0]) / self.sine,
            (self.entry_direction[1] + self.exit_direction[1]) / self.sine,
        ];

        let start = [
            self.entry_direction[0] * self.tangent - centre[0],
            self.entry_direction[1] * self.tangent - centre[1],
        ];
        let finish = [
            self.exit_direction[0] * self.tangent - centre[0],
            self.exit_direction[1] * self.tangent - centre[1],
        ];

        let sweep = (start[0] * finish[0] + start[1] * finish[1])
            .clamp(-1.0, 1.0)
            .acos();
        let turn = if start[0] * finish[1] - start[1] * finish[0] >= 0.0 {
            1.0
        } else {
            -1.0
        };

        let steps = arc_steps(sweep, self.radius_x.max(self.radius_y));

        (0..=steps)
            .map(|step| {
                let ratio = step as f32 / steps as f32;
                let (sine, cosine) = (turn * sweep * ratio).sin_cos();

                (
                    self.unscale(
                        centre[0] + start[0] * cosine - start[1] * sine,
                        centre[1] + start[0] * sine + start[1] * cosine,
                    ),
                    ratio,
                )
            })
            .collect()
    }

    fn scoop(&self) -> Vec<([f32; 2], f32)> {
        let (entry, exit) = (self.entry(), self.exit());
        let steps = arc_steps(self.angle(), self.radius_x.max(self.radius_y));

        (0..=steps)
            .map(|step| {
                let ratio = step as f32 / steps as f32;
                let (sine, cosine) = (FRAC_PI_2 * ratio).sin_cos();

                (
                    [
                        entry[0] * cosine + exit[0] * sine,
                        entry[1] * cosine + exit[1] * sine,
                    ],
                    ratio,
                )
            })
            .collect()
    }
}

fn shrink_factors(corner_datum: &[CornerData], cuts: &[Cut]) -> Vec<f32> {
    let count = corner_datum.len();
    let mut factors = vec![1.0f32; count];

    for index in 0..count {
        let ahead = (index + 1) % count;

        let from = corner_datum[index].vertex;
        let to = corner_datum[ahead].vertex;

        let span = length(to.x - from.x, to.y - from.y);
        let want = cuts[index].out_distance + cuts[ahead].in_distance;

        if want > span {
            let factor = if want > 0.0 { span / want } else { 1.0 };

            factors[index] = factors[index].min(factor);
            factors[ahead] = factors[ahead].min(factor);
        }
    }

    factors
}

fn arc_steps(sweep: f32, radius: f32) -> u32 {
    let full = circle_steps(radius) as f32;

    ((full * sweep / TAU).ceil() as u32).max(1)
}

fn normalise(x: f32, y: f32) -> Option<[f32; 2]> {
    let span = length(x, y);

    if span > 0.0 && span.is_finite() {
        Some([x / span, y / span])
    } else {
        None
    }
}

fn length(x: f32, y: f32) -> f32 {
    (x * x + y * y).sqrt()
}

fn blend(from: Vertex, to: Vertex, ratio: f32) -> Vertex {
    let mix = |a: f32, b: f32| a + (b - a) * ratio;

    Vertex::new_position_color_uv_normal(
        mix(from.x, to.x),
        mix(from.y, to.y),
        mix(from.z, to.z),
        mix(from.r, to.r),
        mix(from.g, to.g),
        mix(from.b, to.b),
        mix(from.a, to.a),
        mix(from.u, to.u),
        mix(from.v, to.v),
        mix(from.n_x, to.n_x),
        mix(from.n_y, to.n_y),
        mix(from.n_z, to.n_z),
    )
}