use std::{mem, sync::OnceLock};

use consume_on_drop::{Consume, ConsumeOnDrop};
use derive_where::derive_where;
use parking_lot::Mutex;
use tokio::sync::watch;

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

pub struct Sender<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    inner: ConsumeOnDrop<SenderInner<T, C>>,
    notify: watch::Sender<()>,
}

struct SenderInner<T, C: UnsafeConsumer<T>> {
    node_pool: std::sync::Arc<ArcPool<Node<T>>>,
    next: Mutex<Arc<Node<T>>>,
    consumer: C,
}

impl<T, C: UnsafeConsumer<T>> Consume for SenderInner<T, C> {
    fn consume(self) {
        drop_with(self.next.into_inner(), &self.node_pool, &self.consumer);
    }
}

impl<T, C: UnsafeConsumer<T>> Sender<T, C> {
    pub fn send(&self, value: T) -> Result<(), T> {
        if self.notify.is_closed() {
            return Err(value);
        }
        self.inner.send(value);
        let _ = self.notify.send(());
        Ok(())
    }

    pub fn subscribe(&self) -> Receiver<T, C>
    where
        C: Clone,
    {
        Receiver {
            inner: ConsumeOnDrop::new(self.inner.subscribe()),
            notify: self.notify.subscribe(),
        }
    }

    pub async fn closed(&self) {
        self.notify.closed().await
    }
}

impl<T, C: UnsafeConsumer<T>> SenderInner<T, C> {
    fn send(&self, value: T) {
        let mut next = self.next.lock();
        let next_arc = self.node_pool.alloc(Node::default());
        next.set(value, next_arc.clone());
        let prev_next = mem::replace(&mut *next, next_arc);
        drop(next);
        drop_with(prev_next, &self.node_pool, &self.consumer);
    }

    fn subscribe(&self) -> ReceiverInner<T, C>
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
pub struct Receiver<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    inner: ConsumeOnDrop<ReceiverInner<T, C>>,
    notify: watch::Receiver<()>,
}

impl<T: Clone> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            if let Some(result) = self.inner.try_next() {
                return Some(result);
            }
            self.notify.changed().await.ok()?;
        }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.inner.try_next().ok_or_else(|| {
            if self.notify.has_changed().is_err() {
                TryRecvError::Disconnected
            } else {
                TryRecvError::Empty
            }
        })
    }
}

#[derive_where(Clone; C)]
struct ReceiverInner<T, C> {
    node_pool: std::sync::Arc<ArcPool<Node<T>>>,
    next: Arc<Node<T>>,
    consumer: C,
}

impl<T, C: UnsafeConsumer<T>> Consume for ReceiverInner<T, C> {
    fn consume(self) {
        drop_with(self.next, &self.node_pool, &self.consumer)
    }
}

pub enum TryRecvError {
    Empty,
    Disconnected,
}

impl<T: Clone, C: UnsafeConsumer<T>> ReceiverInner<T, C> {
    fn try_next(&mut self) -> Option<T> {
        let (value, next_node) = self.next.get()?;
        let value = value.clone();
        let new_next = unsafe { Arc::clone_from_index(&self.node_pool, next_node) };
        let old_next = mem::replace(&mut self.next, new_next);
        drop_with(old_next, &self.node_pool, &self.consumer);
        Some(value)
    }
}

fn drop_with<T>(
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

#[derive_where(Default)]
struct Node<T>(OnceLock<(T, ArcIndex)>);

impl<T> Node<T> {
    fn set(&self, value: T, next: Arc<Node<T>>) {
        self.0
            .set((value, Arc::into_index(next)))
            .unwrap_or_else(|_| {
                panic!("Node::set: called twice on the same node");
            });
    }
    fn get(&self) -> Option<&(T, ArcIndex)> {
        self.0.get()
    }
    fn into_inner(self) -> Option<(T, ArcIndex)> {
        self.0.into_inner()
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    channel_with_consumer(DropNormally)
}

pub fn channel_with_consumer<T, C: Consumer<T> + Clone>(
    consumer: C,
) -> (Sender<T, Safe<C>>, Receiver<T, Safe<C>>) {
    unsafe { channel_with_unsafe_consumer(Safe(consumer)) }
}

pub unsafe fn channel_with_unsafe_consumer<T, C: UnsafeConsumer<T> + Clone>(
    consumer: C,
) -> (Sender<T, C>, Receiver<T, C>) {
    let (tx, rx) = watch::channel(());
    let node_pool = std::sync::Arc::new(ArcPool::new());
    let initial = node_pool.alloc(Node::default());
    (
        Sender {
            inner: ConsumeOnDrop::new(SenderInner {
                node_pool: node_pool.clone(),
                next: Mutex::new(initial.clone()),
                consumer: consumer.clone(),
            }),
            notify: tx,
        },
        Receiver {
            inner: ConsumeOnDrop::new(ReceiverInner {
                node_pool,
                next: initial,
                consumer,
            }),
            notify: rx,
        },
    )
}
