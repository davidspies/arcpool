use std::{mem, sync::OnceLock};

use consume_on_drop::Consume;
use derive_where::derive_where;
use tokio::sync::watch;

use crate::consumer::UnsafeConsumer;

#[derive_where(Clone; C)]
pub(super) struct ReceiverInner<T, C: UnsafeConsumer<T>> {
    next_node: arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>,
    consumer: C,
    notify_pool: std::sync::Arc<arc_slice_pool::ArcPool<OnceLock<arc_queue_pool::Arc<T>>>>,
}

impl<T, C: UnsafeConsumer<T>> Consume for ReceiverInner<T, C> {
    fn consume(self) {
        let Some(node) = arc_slice_pool::Arc::into_inner(self.next_node) else {
            return;
        };
        let Some(mut node) = OnceLock::into_inner(node) else {
            return;
        };
        loop {
            let Some((val, next_node)) = arc_queue_pool::Arc::into_inner_and_next(node) else {
                break;
            };
            unsafe { self.consumer.consume(val) };
            let Some(next_node) = next_node else { break };
            node = next_node
        }
    }
}

impl<T, C: UnsafeConsumer<T>> ReceiverInner<T, C> {
    pub(super) fn new(
        next_node: arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>,
        consumer: C,
        notify_pool: std::sync::Arc<arc_slice_pool::ArcPool<OnceLock<arc_queue_pool::Arc<T>>>>,
    ) -> Self {
        Self {
            next_node,
            consumer,
            notify_pool,
        }
    }

    pub(super) fn try_next(
        &mut self,
        next_node_rx: &watch::Receiver<arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>>,
    ) -> Option<T> {
        let node = self.next_node.get()?;
        let sender_next_node = next_node_rx.borrow().clone();
        let next_node = match arc_queue_pool::Arc::next(node) {
            Some(next_node) => self.notify_pool.alloc(OnceLock::from(next_node)),
            None => sender_next_node,
        };
        let replaced_node = mem::replace(&mut self.next_node, next_node);
        match arc_slice_pool::Arc::try_unwrap(replaced_node) {
            Ok(replaced_node) => {
                let inner_node = replaced_node.into_inner().unwrap();
                match arc_queue_pool::Arc::try_unwrap_and_next(inner_node) {
                    Ok((result, _next_inner_node)) => Some(result),
                    Err(inner_node) => {
                        let result = unsafe { self.consumer.clone_value(&inner_node) };
                        // Handles unlikely race condition where the remaining instances of the inner node
                        // are dropped while we're cloning the value:
                        if let Some((inner_value, _next_inner_node)) =
                            arc_queue_pool::Arc::into_inner_and_next(inner_node)
                        {
                            unsafe { self.consumer.consume(inner_value) };
                        }
                        Some(result)
                    }
                }
            }
            Err(replaced_node) => {
                let inner_node = replaced_node.get().unwrap();
                let result = unsafe { self.consumer.clone_value(inner_node) };
                unsafe { consume_node(self.consumer(), replaced_node) };
                Some(result)
            }
        }
    }

    pub(super) fn consumer(&self) -> &C {
        &self.consumer
    }
}

/// # Safety
/// Instance of T must be valid for the consumer
pub(super) unsafe fn consume_node<T>(
    consumer: &impl UnsafeConsumer<T>,
    node: arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>,
) {
    let Some(node) = arc_slice_pool::Arc::into_inner(node) else {
        return;
    };
    let node = OnceLock::into_inner(node).unwrap();
    let Some((value, next)) = arc_queue_pool::Arc::into_inner_and_next(node) else {
        return;
    };
    consumer.consume(value);
    debug_assert!(!next.is_some_and(|next| arc_queue_pool::Arc::ref_count(&next) == 1));
}
