use std::marker::PhantomData;

use gueiz_gpu::vertex::Vertex;

use crate::object::{Object, create_object};
use crate::paint_type::PaintType;

pub struct Rect<State> {
    object: Object,
    _state: PhantomData<State>,
}

pub struct Init;
pub struct PaintTypeSet;
pub struct Vertex1Set;
pub struct Vertex2Set;
pub struct Vertex3Set;
pub struct DiagonalSet;
pub struct End;

impl<State> Rect<State> {
    fn advance<Next>(self) -> Rect<Next> {
        Rect {
            object: self.object,
            _state: PhantomData,
        }
    }
}

impl Rect<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> Rect<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl Rect<PaintTypeSet> {
    pub fn vertex1(mut self, vertex1: Vertex) -> Rect<Vertex1Set> {
        self.object.put_vertex(vertex1);
        self.advance()
    }

    pub fn from(mut self, vertex: Vertex) -> Rect<DiagonalSet> {
        self.object.put_vertex(vertex);
        self.advance()
    }
}

impl Rect<DiagonalSet> {
    pub fn to(self, vertex: Vertex) -> Rect<End> {
        self.close(vertex)
    }

    pub fn to_wh(self, vertex_wh: Vertex) -> Rect<End> {
        let from = self.start();

        let mut to = vertex_wh;
        to.x = from.x + vertex_wh.x;
        to.y = from.y + vertex_wh.y;

        self.close(to)
    }

    fn start(&self) -> Vertex {
        *self
            .object
            .vertices()
            .first()
            .expect("from が必ず設定されている必要があります。")
    }

    fn close(mut self, to: Vertex) -> Rect<End> {
        let from = self.start();

        self.object.put_vertex(corner(from, to, to.x, from.y));
        self.object.put_vertex(to);
        self.object.put_vertex(corner(from, to, from.x, to.y));

        self.advance()
    }
}

impl Rect<Vertex1Set> {
    pub fn vertex2(mut self, vertex2: Vertex) -> Rect<Vertex2Set> {
        self.object.put_vertex(vertex2);
        self.advance()
    }
}

impl Rect<Vertex2Set> {
    pub fn vertex3(mut self, vertex3: Vertex) -> Rect<Vertex3Set> {
        self.object.put_vertex(vertex3);
        self.advance()
    }
}

impl Rect<Vertex3Set> {
    pub fn vertex4(mut self, vertex4: Vertex) -> Rect<End> {
        self.object.put_vertex(vertex4);
        self.advance()
    }
}

impl Rect<End> {
    pub fn end(mut self) -> Object {
        self.object.end();
        self.object
    }
}

pub(crate) fn corner(from: Vertex, to: Vertex, x: f32, y: f32) -> Vertex {
    let u = if x == to.x { to.u } else { from.u };
    let v = if y == to.y { to.v } else { from.v };

    let middle = |a: f32, b: f32| (a + b) / 2.0;

    Vertex::new_position_color_uv_normal(
        x,
        y,
        middle(from.z, to.z),
        middle(from.r, to.r),
        middle(from.g, to.g),
        middle(from.b, to.b),
        middle(from.a, to.a),
        u,
        v,
        middle(from.n_x, to.n_x),
        middle(from.n_y, to.n_y),
        middle(from.n_z, to.n_z),
    )
}
