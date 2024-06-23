use std::{
    mem::{self, MaybeUninit},
    sync::OnceLock,
};

use arc_slice_pool::{Arc, ArcIndex, ArcPool};
use derive_where::derive_where;
use tokio::sync::watch;

use self::consumer::{DropNormally, Safe};

pub use self::consumer::{Consumer, UnsafeConsumer};

pub mod consumer;

pub struct Sender<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    next_node_tx: watch::Sender<arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>>,
    consumer: C,
    node_pool: arc_queue_pool::ArcPool<T>,
    notify_pool: std::sync::Arc<arc_slice_pool::ArcPool<OnceLock<arc_queue_pool::Arc<T>>>>,
}

impl<T, C: Consumer<T>> Sender<T, Safe<C>> {
    pub fn send(&self, value: T) -> Result<(), T> {
        unsafe { self.unsafe_send(value) }
    }
}

impl<T, C: UnsafeConsumer<T>> Sender<T, C> {
    /// # Safety
    /// The value must be a valid value for the consumer
    pub unsafe fn unsafe_send(&self, value: T) -> Result<(), T> {
        if self.next_node_tx.is_closed() {
            return Err(value);
        }
        let node = self.node_pool.alloc(value);
        let next_node = self.notify_pool.alloc(OnceLock::new());
        let mut this_node = MaybeUninit::uninit();
        self.next_node_tx.send_modify(|cur_node| {
            cur_node.set(node).unwrap_or_else(|_| unreachable!());
            this_node.write(mem::replace(cur_node, next_node));
        });
        let this_node = this_node.assume_init();
        consume_node(self.consumer(), this_node);
        Ok(())
    }

    pub fn subscribe(&self) -> Receiver<T, C>
    where
        C: Clone,
    {
        Receiver {
            next_node: self.next_node_tx.borrow().clone(),
            consumer: self.consumer.clone(),
            next_node_rx: self.next_node_tx.subscribe(),
            notify_pool: self.notify_pool.clone(),
        }
    }

    pub async fn closed(&self) {
        self.next_node_tx.closed().await
    }

    pub fn consumer(&self) -> &C {
        &self.consumer
    }
}

#[derive_where(Clone; C)]
pub struct Receiver<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    next_node: arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>,
    consumer: C,
    next_node_rx: watch::Receiver<arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>>,
    notify_pool: std::sync::Arc<arc_slice_pool::ArcPool<OnceLock<arc_queue_pool::Arc<T>>>>,
}

impl<T, C: UnsafeConsumer<T>> Receiver<T, C> {
    pub async fn recv(&mut self) -> Option<T> {
        self.next_node_rx.mark_unchanged();
        loop {
            if let Some(result) = self.try_next() {
                return Some(result);
            }
            self.next_node_rx.changed().await.ok()?;
        }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match self.try_next() {
            Some(result) => return Ok(result),
            None => {
                if self.next_node_rx.has_changed().is_err() {
                    Err(TryRecvError::Disconnected)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }

    fn try_next(&mut self) -> Option<T> {
        let node = self.next_node.get()?;
        let result = unsafe { self.consumer.clone_value(node) };
        let sender_next_node = self.next_node_rx.borrow().clone();
        let next_node = match arc_queue_pool::Arc::next(node) {
            Some(next_node) => {
                drop(sender_next_node); // Safe because we still have a clone or earlier node in the queue
                self.notify_pool.alloc(OnceLock::from(next_node))
            }
            None => sender_next_node,
        };
        let replaced_node = mem::replace(&mut self.next_node, next_node);
        unsafe { consume_node(self.consumer(), replaced_node) };
        Some(result)
    }

    pub fn consumer(&self) -> &C {
        &self.consumer
    }
}

/// # Safety
/// Instance of T must be valid for the consumer
unsafe fn consume_node<T>(
    consumer: &impl UnsafeConsumer<T>,
    node: arc_slice_pool::Arc<OnceLock<arc_queue_pool::Arc<T>>>,
) {
    let Some(node) = arc_slice_pool::Arc::into_inner(node) else {
        return;
    };
    let node = OnceLock::into_inner(node).unwrap();
    let Some(value) = arc_queue_pool::Arc::into_inner(node) else {
        return;
    };
    consumer.consume(value)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TryRecvError {
    Empty,
    Disconnected,
}

pub fn channel<T: Clone>() -> (Sender<T>, Receiver<T>) {
    channel_with_consumer(Safe(DropNormally))
}

pub fn channel_with_consumer<T, C: UnsafeConsumer<T> + Clone>(
    consumer: C,
) -> (Sender<T, C>, Receiver<T, C>) {
    let node_pool = arc_queue_pool::ArcPool::new();
    let notify_pool = std::sync::Arc::new(arc_slice_pool::ArcPool::new());
    let next_node = notify_pool.alloc(OnceLock::new());
    let (next_node_tx, next_node_rx) = watch::channel(next_node.clone());
    let sender = Sender {
        next_node_tx,
        consumer: consumer.clone(),
        node_pool,
        notify_pool: notify_pool.clone(),
    };
    let receiver = Receiver {
        next_node,
        consumer,
        next_node_rx,
        notify_pool,
    };
    (sender, receiver)
}

pub struct ArcSender<T>(Sender<ArcIndex, std::sync::Arc<ArcPool<T>>>);
pub struct ArcReceiver<T>(Receiver<ArcIndex, std::sync::Arc<ArcPool<T>>>);

pub fn arc_channel<T>() -> (ArcSender<T>, ArcReceiver<T>) {
    let arc_pool = std::sync::Arc::new(ArcPool::new());
    let (sender, receiver) = channel_with_consumer(arc_pool);
    (ArcSender(sender), ArcReceiver(receiver))
}

impl<T> ArcSender<T> {
    pub fn send(&self, value: T) -> Result<(), T> {
        let pool = self.0.consumer();
        let arc = pool.alloc(value);
        let arc_index = Arc::into_index(arc);
        let rejected = match unsafe { self.0.unsafe_send(arc_index) } {
            Ok(()) => return Ok(()),
            Err(rejected) => rejected,
        };
        let reconstructed = unsafe { Arc::from_index(pool, rejected) };
        Err(Arc::into_inner(reconstructed).unwrap())
    }

    pub fn subscribe(&self) -> ArcReceiver<T> {
        ArcReceiver(self.0.subscribe())
    }

    pub async fn closed(&self) {
        self.0.closed().await
    }

    pub fn pool(&self) -> &std::sync::Arc<ArcPool<T>> {
        self.0.consumer()
    }
}

impl<T> ArcReceiver<T> {
    pub async fn recv(&mut self) -> Option<Arc<T>> {
        let arc_index = self.0.recv().await?;
        Some(unsafe { Arc::from_index(self.pool(), arc_index) })
    }

    pub fn try_recv(&mut self) -> Result<Arc<T>, TryRecvError> {
        let arc_index = self.0.try_recv()?;
        Ok(unsafe { Arc::from_index(self.pool(), arc_index) })
    }

    pub fn pool(&self) -> &std::sync::Arc<ArcPool<T>> {
        self.0.consumer()
    }
}
