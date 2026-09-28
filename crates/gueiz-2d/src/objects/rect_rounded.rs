use std::marker::PhantomData;

use gueiz_gpu::vertex::Vertex;

use crate::object::{Object, create_object};
use crate::objects::polygon_rounded::{CornerData, CornerType, outline};
use crate::objects::rect::corner;
use crate::paint_type::PaintType;

pub struct RectRounded<State> {
    object: Object,
    corner_datum: Vec<CornerData>,
    _state: PhantomData<State>,
}

pub struct Init;
pub struct PaintTypeSet;
pub struct Vertex1Set;
pub struct Vertex2Set;
pub struct Vertex3Set;
pub struct DiagonalSet;
pub struct End;

impl<State> RectRounded<State> {
    fn advance<Next>(self) -> RectRounded<Next> {
        RectRounded {
            object: self.object,
            corner_datum: self.corner_datum,
            _state: PhantomData,
        }
    }
}

impl RectRounded<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            corner_datum: Vec::new(),
            _state: PhantomData,
        }
    }

    pub fn paint_type(mut self, paint_type: PaintType) -> RectRounded<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl RectRounded<PaintTypeSet> {
    pub fn vertex1(mut self, corner_type: CornerType, vertex1: Vertex) -> RectRounded<Vertex1Set> {
        self.corner_datum.push(CornerData::new(corner_type, vertex1));
        self.advance()
    }
    
    pub fn from(mut self, corner_type: CornerType, vertex: Vertex) -> RectRounded<DiagonalSet> {
        self.corner_datum.push(CornerData::new(corner_type, vertex));
        self.advance()
    }
}

impl RectRounded<Vertex1Set> {
    pub fn vertex2(mut self, corner_type: CornerType, vertex2: Vertex) -> RectRounded<Vertex2Set> {
        self.corner_datum.push(CornerData::new(corner_type, vertex2));
        self.advance()
    }
}

impl RectRounded<Vertex2Set> {
    pub fn vertex3(mut self, corner_type: CornerType, vertex3: Vertex) -> RectRounded<Vertex3Set> {
        self.corner_datum.push(CornerData::new(corner_type, vertex3));
        self.advance()
    }
}

impl RectRounded<Vertex3Set> {
    pub fn vertex4(mut self, corner_type: CornerType, vertex4: Vertex) -> RectRounded<End> {
        self.corner_datum.push(CornerData::new(corner_type, vertex4));
        self.advance()
    }
}

impl RectRounded<DiagonalSet> {
    pub fn to(self, vertex: Vertex) -> RectRounded<End> {
        self.close(vertex)
    }

    pub fn to_wh(self, vertex_wh: Vertex) -> RectRounded<End> {
        let from = self.start().vertex;

        let mut to = vertex_wh;
        to.x = from.x + vertex_wh.x;
        to.y = from.y + vertex_wh.y;

        self.close(to)
    }

    fn start(&self) -> &CornerData {
        self.corner_datum
            .first()
            .expect("from が必ず設定されている必要があります。")
    }

    fn close(mut self, to: Vertex) -> RectRounded<End> {
        let corner_type = self.start().corner_type;
        let from = self.start().vertex;

        for vertex in [
            corner(from, to, to.x, from.y),
            to,
            corner(from, to, from.x, to.y),
        ] {
            self.corner_datum.push(CornerData::new(corner_type, vertex));
        }

        self.advance()
    }
}

impl RectRounded<End> {
    pub fn end(mut self) -> Object {
        let corner_datum = std::mem::take(&mut self.corner_datum);

        for vertex in outline(&corner_datum) {
            self.object.put_vertex(vertex);
        }

        self.object.end();
        self.object
    }
}
