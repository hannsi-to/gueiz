use std::f32::consts::TAU;
use std::marker::PhantomData;
use gueiz_gpu::vertex::Vertex;
use crate::object::{create_object, Object};
use crate::paint_type::PaintType;

pub struct Circle<State> {
    object: Object,
    circle_data: CircleData,
    _state: PhantomData<State>,
}

struct CircleData {
    vertex_center: Option<Vertex>,
    radius_x: Option<f32>,
    radius_y: Option<f32>,
    start_angle: f32,
    end_angle: f32,
    segment_count: usize,
    outer_colors: Vec<(f32, f32, f32, f32)>
}

pub struct Init;
pub struct PaintTypeSet;
pub struct VertexCenterSet;
pub struct End;

impl<State> Circle<State> {
    fn advance<Next>(self) -> Circle<Next> {
        Circle {
            object: self.object,
            circle_data: self.circle_data,
            _state: PhantomData,
        }
    }
}


impl Circle<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            circle_data: CircleData {
                vertex_center: None,
                radius_x: None,
                radius_y: None,
                start_angle: 0.0,
                end_angle: 360.0_f32.to_radians(),
                segment_count: 16,
                outer_colors: Vec::new(),
            },
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> Circle<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl Circle<PaintTypeSet> {
    pub fn vertex_center(mut self, vertex_center: Vertex) -> Circle<VertexCenterSet> {
        self.circle_data.vertex_center = Some(vertex_center);
        self.advance()
    }
}

impl Circle<VertexCenterSet> {
    pub fn radius(mut self, radius: f32) -> Circle<VertexCenterSet> {
        self.circle_data.radius_x = Some(radius);
        self.circle_data.radius_y = Some(radius);
        self.advance()
    }

    pub fn radius_x(mut self, radius_x: f32) -> Circle<VertexCenterSet> {
        self.circle_data.radius_x = Some(radius_x);
        self.advance()
    }

    pub fn radius_y(mut self, radius_y: f32) -> Circle<VertexCenterSet> {
        self.circle_data.radius_y = Some(radius_y);
        self.advance()
    }

    pub fn start_angle(mut self, start_angle: f32) -> Circle<VertexCenterSet> {
        self.circle_data.start_angle = start_angle;
        self.advance()
    }

    pub fn end_angle(mut self, end_angle: f32) -> Circle<VertexCenterSet> {
        self.circle_data.end_angle = end_angle;
        self.advance()
    }

    pub fn start_angle_degrees(mut self, start_angle_degrees: f32) -> Circle<VertexCenterSet> {
        self.circle_data.start_angle = start_angle_degrees.to_radians();
        self.advance()
    }

    pub fn end_angle_degrees(mut self, end_angle_degrees: f32) -> Circle<VertexCenterSet> {
        self.circle_data.end_angle = end_angle_degrees.to_radians();
        self.advance()
    }

    pub fn segment_count(mut self, segment_count: usize) -> Circle<VertexCenterSet> {
        self.circle_data.segment_count = segment_count;
        self.advance()
    }

    pub fn add_outer_color(mut self, outer_color: (f32, f32, f32, f32)) -> Circle<VertexCenterSet> {
        self.circle_data.outer_colors.push(outer_color);
        self.advance()
    }

    pub fn end_circle_option(self) -> Circle<End> {
        self.advance()
    }
}

impl Circle<End> {
    pub fn end(mut self) -> Object {
        for vertex in outline(&self.circle_data) {
            self.object.put_vertex(vertex);
        }

        self.object.end();
        self.object
    }
}

fn outline(circle_data: &CircleData) -> Vec<Vertex> {
    let centre = circle_data
        .vertex_center
        .expect("vertex_center が必ず設定されている必要があります。");

    let radius_x = circle_data
        .radius_x
        .or(circle_data.radius_y)
        .expect("radius が必ず設定されている必要があります。");
    let radius_y = circle_data.radius_y.or(circle_data.radius_x).unwrap();

    let usable = |radius: f32| radius.is_finite() && radius > 0.0;

    if !usable(radius_x) || !usable(radius_y) {
        return Vec::new();
    }

    let sweep = circle_data.end_angle - circle_data.start_angle;

    if !sweep.is_finite() || sweep == 0.0 {
        return Vec::new();
    }

    let whole = sweep.abs() >= TAU - 1e-4;
    let sweep = if whole { TAU.copysign(sweep) } else { sweep };

    let segments = circle_data.segment_count.max(if whole { 3 } else { 1 });

    let points = if whole { segments } else { segments + 1 };

    let mut vertices = Vec::with_capacity(points + 1);

    if !whole {
        vertices.push(centre);
    }

    for step in 0..points {
        let ratio = step as f32 / segments as f32;
        let angle = circle_data.start_angle + sweep * ratio;

        let mut vertex = centre;
        vertex.x = centre.x + radius_x * angle.cos();
        vertex.y = centre.y + radius_y * angle.sin();

        if let Some((r, g, b, a)) = rim_color(&circle_data.outer_colors, ratio, whole) {
            vertex.r = r;
            vertex.g = g;
            vertex.b = b;
            vertex.a = a;
        }

        vertices.push(vertex);
    }

    vertices
}

fn rim_color(
    outer_colors: &[(f32, f32, f32, f32)],
    ratio: f32,
    whole: bool,
) -> Option<(f32, f32, f32, f32)> {
    match outer_colors.len() {
        0 => None,
        1 => Some(outer_colors[0]),
        count => {
            let span = if whole { count } else { count - 1 } as f32;
            let position = (ratio * span).clamp(0.0, span);
            let index = position.floor() as usize;

            let from = outer_colors[index % count];
            let to = outer_colors[(index + 1) % count];
            let blend = position - index as f32;

            let mix = |a: f32, b: f32| a + (b - a) * blend;

            Some((
                mix(from.0, to.0),
                mix(from.1, to.1),
                mix(from.2, to.2),
                mix(from.3, to.3),
            ))
        }
    }
}
