use crate::{Arc, ArcIndex, ArcPool};

#[derive(Clone, Copy)]
pub struct DropNormally;

pub trait Consumer<T> {
    fn consume(&self, value: T);
}

impl<C: Consumer<T>, T> Consumer<T> for std::sync::Arc<C> {
    fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

pub trait UnsafeConsumer<T> {
    unsafe fn consume(&self, value: T);
}

impl<C: UnsafeConsumer<T>, T> UnsafeConsumer<T> for std::sync::Arc<C> {
    unsafe fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

#[derive(Clone, Copy)]
pub struct Safe<C>(pub C);

impl<C: Consumer<T>, T> UnsafeConsumer<T> for Safe<C> {
    unsafe fn consume(&self, value: T) {
        self.0.consume(value)
    }
}

impl<T> Consumer<T> for DropNormally {
    fn consume(&self, _: T) {}
}

impl<T> UnsafeConsumer<ArcIndex> for ArcPool<T> {
    unsafe fn consume(&self, value: ArcIndex) {
        drop(Arc::from_index(self, value))
    }
}
