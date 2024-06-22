use std::sync::OnceLock;

use arc_slice_pool::{Arc, ArcIndex, ArcPool};
use derive_where::derive_where;

use super::consumer::UnsafeConsumer;

#[derive_where(Default)]
pub(super) struct Node<T>(OnceLock<(T, ArcIndex)>);

impl<T> Node<T> {
    pub(super) fn set(&self, value: T, next: Arc<Node<T>>) {
        self.0
            .set((value, Arc::into_index(next)))
            .unwrap_or_else(|_| panic!("Node::set: called twice on the same node"));
    }
    pub(super) fn get(&self) -> Option<&(T, ArcIndex)> {
        self.0.get()
    }
    pub(super) fn into_inner(self) -> Option<(T, ArcIndex)> {
        self.0.into_inner()
    }
}

pub(super) unsafe fn drop_with<T>(
    mut node: Arc<Node<T>>,
    node_pool: &std::sync::Arc<ArcPool<Node<T>>>,
    consumer: &impl UnsafeConsumer<T>,
) {
    loop {
        let Some(raw_node) = Arc::into_inner(node) else {
            return;
        };
        let Some((value, arc_index)) = raw_node.into_inner() else {
            return;
        };
        unsafe { consumer.consume(value) };
        node = unsafe { Arc::from_index(node_pool, arc_index) };
    }
}
