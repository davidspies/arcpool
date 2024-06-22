use arc_slab_pool::{Arc, ArcIndex, ArcPool};

#[derive(Clone, Copy)]
pub struct DropNormally;

pub trait Consumer<T> {
    fn clone_value(&self, value: &T) -> T;
    fn consume(&self, value: T);
}

impl<C: Consumer<T>, T> Consumer<T> for std::sync::Arc<C> {
    fn clone_value(&self, value: &T) -> T {
        (**self).clone_value(value)
    }
    fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

pub trait UnsafeConsumer<T> {
    #[allow(clippy::missing_safety_doc)]
    unsafe fn clone_value(&self, value: &T) -> T;
    #[allow(clippy::missing_safety_doc)]
    unsafe fn consume(&self, value: T);
}

impl<C: UnsafeConsumer<T>, T> UnsafeConsumer<T> for std::sync::Arc<C> {
    unsafe fn clone_value(&self, value: &T) -> T {
        (**self).clone_value(value)
    }
    unsafe fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

#[derive(Clone, Copy)]
pub struct Safe<C>(pub C);

impl<C: Consumer<T>, T> UnsafeConsumer<T> for Safe<C> {
    unsafe fn clone_value(&self, value: &T) -> T {
        self.0.clone_value(value)
    }
    unsafe fn consume(&self, value: T) {
        self.0.consume(value)
    }
}

impl<T: Clone> Consumer<T> for DropNormally {
    fn clone_value(&self, value: &T) -> T {
        value.clone()
    }
    fn consume(&self, _: T) {}
}

impl<T> UnsafeConsumer<ArcIndex> for ArcPool<T> {
    unsafe fn clone_value(&self, value: &ArcIndex) -> ArcIndex {
        Arc::into_index(Arc::clone_from_index(self, value))
    }
    unsafe fn consume(&self, value: ArcIndex) {
        drop(Arc::from_index(self, value))
    }
}
