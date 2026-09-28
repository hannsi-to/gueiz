use std::marker::PhantomData;
use gueiz_gpu::vertex::Vertex;
use crate::object::{create_object, Object};
use crate::paint_type::PaintType;

pub struct Triangle<State> {
    object: Object,
    _state: PhantomData<State>,
}

pub struct Init;
pub struct PaintTypeSet;
pub struct Vertex1Set;
pub struct Vertex2Set;
pub struct End;

impl<State> Triangle<State> {
    fn advance<Next>(self) -> Triangle<Next> {
        Triangle {
            object: self.object,
            _state: PhantomData,
        }
    }
}

impl Triangle<Init> {
    pub fn new(name: &str) -> Self {
        Self {
            object: create_object(name),
            _state: PhantomData,
        }
    }
    
    pub fn paint_type(mut self, paint_type: PaintType) -> Triangle<PaintTypeSet> {
        self.object.begin(paint_type);
        self.advance()
    }
}

impl Triangle<PaintTypeSet> {
    pub fn vertex1(mut self, vertex1: Vertex) -> Triangle<Vertex1Set> {
        self.object.put_vertex(vertex1);
        self.advance()
    }
}

impl Triangle<Vertex1Set> {
    pub fn vertex2(mut self, vertex2: Vertex) -> Triangle<Vertex2Set> {
        self.object.put_vertex(vertex2);
        self.advance()
    }
}

impl Triangle<Vertex2Set> {
    pub fn vertex3(mut self, vertex3: Vertex) -> Triangle<End> {
        self.object.put_vertex(vertex3);
        self.advance()
    }
}

impl Triangle<End> {
    pub fn end(mut self) -> Object {
        self.object.end();
        self.object
    }
}