use std::mem;

use arc_slab_pool::{Arc, ArcPool};
use consume_on_drop::Consume;
use derive_where::derive_where;
use parking_lot::Mutex;

use super::consumer::UnsafeConsumer;
use super::node::{self, Node};

pub(super) struct SenderInner<T, C: UnsafeConsumer<T>> {
    node_pool: std::sync::Arc<ArcPool<Node<T>>>,
    next: Mutex<Arc<Node<T>>>,
    consumer: C,
}

impl<T, C: UnsafeConsumer<T>> Consume for SenderInner<T, C> {
    fn consume(self) {
        unsafe { node::drop_with(self.next.into_inner(), &self.node_pool, &self.consumer) }
    }
}

impl<T, C: UnsafeConsumer<T>> SenderInner<T, C> {
    pub(super) fn new(
        node_pool: std::sync::Arc<ArcPool<Node<T>>>,
        next: Arc<Node<T>>,
        consumer: C,
    ) -> Self {
        Self {
            node_pool,
            next: Mutex::new(next),
            consumer,
        }
    }

    pub(super) unsafe fn send(&self, value: T) {
        let mut next = self.next.lock();
        let next_arc = self.node_pool.alloc(Node::default());
        next.set(value, next_arc.clone());
        let prev_next = mem::replace(&mut *next, next_arc);
        drop(next);
        node::drop_with(prev_next, &self.node_pool, &self.consumer);
    }

    pub(super) fn subscribe(&self) -> ReceiverInner<T, C>
    where
        C: Clone,
    {
        ReceiverInner {
            node_pool: self.node_pool.clone(),
            next: self.next.lock().clone(),
            consumer: self.consumer.clone(),
        }
    }
}

#[derive_where(Clone; C)]
pub(super) struct ReceiverInner<T, C> {
    node_pool: std::sync::Arc<ArcPool<Node<T>>>,
    next: Arc<Node<T>>,
    consumer: C,
}

impl<T, C: UnsafeConsumer<T>> Consume for ReceiverInner<T, C> {
    fn consume(self) {
        unsafe { node::drop_with(self.next, &self.node_pool, &self.consumer) }
    }
}

impl<T, C: UnsafeConsumer<T>> ReceiverInner<T, C> {
    pub(super) fn new(
        node_pool: std::sync::Arc<ArcPool<Node<T>>>,
        next: Arc<Node<T>>,
        consumer: C,
    ) -> Self {
        Self {
            node_pool,
            next,
            consumer,
        }
    }

    pub(super) fn try_next(&mut self) -> Option<T>
    where
        T: Clone,
    {
        let (value, next_node) = self.next.get()?;
        let value = value.clone();
        let new_next = unsafe { Arc::clone_from_index(&self.node_pool, next_node) };
        let old_next = mem::replace(&mut self.next, new_next);
        unsafe { node::drop_with(old_next, &self.node_pool, &self.consumer) };
        Some(value)
    }
}
