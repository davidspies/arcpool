use std::sync::Arc as StdArc;

use arc_slice_pool::{Arc, ArcIndex, ArcPool};

#[derive(Clone, Copy)]
pub struct DropNormally;

/// A [Consumer] holds metadata needed to clone and consume values of type [T].
pub trait Consumer<T> {
    fn clone_value(&self, value: &T) -> T;
    fn consume(&self, value: T);
}

impl<C: Consumer<T>, T> Consumer<T> for StdArc<C> {
    fn clone_value(&self, value: &T) -> T {
        (**self).clone_value(value)
    }
    fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

/// An [UnsafeConsumer] is like a [Consumer], but it has some notion of "valid" values which it
/// can clone and consume. Unlike with [Consumer], The validity of a value is not reflected in the
/// type system.
///
/// If a value is not valid, the behavior of [UnsafeConsumer::clone_value] and
/// [UnsafeConsumer::consume] is undefined.
///
/// If a value _is_ valid, [UnsafeConsumer::clone_value] should return another valid value for
/// this consumer.
pub trait UnsafeConsumer<T> {
    #[allow(clippy::missing_safety_doc)]
    unsafe fn clone_value(&self, value: &T) -> T;
    #[allow(clippy::missing_safety_doc)]
    unsafe fn consume(&self, value: T);
}

impl<C: UnsafeConsumer<T>, T> UnsafeConsumer<T> for StdArc<C> {
    unsafe fn clone_value(&self, value: &T) -> T {
        (**self).clone_value(value)
    }
    unsafe fn consume(&self, value: T) {
        (**self).consume(value)
    }
}

/// A newtype wrapper which makes it possible to use a safe [Consumer] anywhere an [UnsafeConsumer]
/// is expected.
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

impl<T> UnsafeConsumer<ArcIndex<T>> for ArcPool<T> {
    unsafe fn clone_value(&self, value: &ArcIndex<T>) -> ArcIndex<T> {
        Arc::into_index(Arc::clone_from_index(self, value))
    }
    unsafe fn consume(&self, value: ArcIndex<T>) {
        drop(Arc::from_index(self, value))
    }
}
