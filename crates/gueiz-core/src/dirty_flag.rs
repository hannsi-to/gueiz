pub struct DirtyFlag<T> {
    object: T,
    dirty: bool,
}

impl<T: PartialEq> DirtyFlag<T> {
    pub fn new(object: T) -> Self {
        Self {
            object,
            dirty: false,
        }
    }

    pub fn set(&mut self, value: T) -> bool {
        if self.object != value {
            self.object = value;
            self.dirty = true;
            true
        } else {
            false
        }
    }

    pub fn mark_dirty(&mut self) {
        self.dirty = true;
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn clear(&mut self) {
        self.dirty = false;
    }

    pub fn get(&self) -> &T {
        &self.object
    }
}