use std::{
    mem::{self, MaybeUninit},
    sync::{Arc as StdArc, OnceLock},
};

use arc_queue_pool::{Arc as QueueArc, ArcPool as ArcQueuePool};
use arc_slice_pool::{Arc as SliceArc, ArcPool as ArcSlicePool};
use consume_on_drop::ConsumeOnDrop;
use derive_where::derive_where;
use tokio::sync::watch;

use self::consumer::{DropNormally, Safe};
use self::inner::ReceiverInner;

pub use self::consumer::{Consumer, UnsafeConsumer};

mod inner;

pub mod arc;
pub mod consumer;

pub struct Sender<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    next_node_tx: watch::Sender<SliceArc<OnceLock<QueueArc<T>>>>,
    consumer: C,
    node_pool: ArcQueuePool<T>,
    notify_pool: StdArc<ArcSlicePool<OnceLock<QueueArc<T>>>>,
}

impl<T, C: Consumer<T>> Sender<T, Safe<C>> {
    pub fn send(&self, value: T) -> Result<(), T> {
        unsafe { self.unsafe_send(value) }
    }
}

impl<T, C: UnsafeConsumer<T>> Sender<T, C> {
    /// # Safety
    /// The value must be a valid value for the consumer the channel was created with.
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
        inner::consume_node(self.consumer(), this_node);
        Ok(())
    }

    pub fn subscribe(&self) -> Receiver<T, C>
    where
        C: Clone,
    {
        Receiver {
            inner: ConsumeOnDrop::new(ReceiverInner::new(
                self.next_node_tx.borrow().clone(),
                self.consumer.clone(),
                self.notify_pool.clone(),
            )),
            next_node_rx: self.next_node_tx.subscribe(),
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
    inner: ConsumeOnDrop<ReceiverInner<T, C>>,
    next_node_rx: watch::Receiver<SliceArc<OnceLock<QueueArc<T>>>>,
}

impl<T, C: UnsafeConsumer<T>> Receiver<T, C> {
    pub async fn recv(&mut self) -> Option<T> {
        self.next_node_rx.mark_unchanged();
        loop {
            if let Some(result) = self.inner.try_next(&self.next_node_rx) {
                return Some(result);
            }
            self.next_node_rx.changed().await.ok()?;
        }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        match self.inner.try_next(&self.next_node_rx) {
            Some(result) => Ok(result),
            None => {
                if self.next_node_rx.has_changed().is_err() {
                    Err(TryRecvError::Disconnected)
                } else {
                    Err(TryRecvError::Empty)
                }
            }
        }
    }

    pub fn consumer(&self) -> &C {
        self.inner.consumer()
    }
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
    let node_pool = ArcQueuePool::new();
    let notify_pool = StdArc::new(ArcSlicePool::new());
    let next_node = notify_pool.alloc(OnceLock::new());
    let (next_node_tx, next_node_rx) = watch::channel(next_node.clone());
    let sender = Sender {
        next_node_tx,
        consumer: consumer.clone(),
        node_pool,
        notify_pool: notify_pool.clone(),
    };
    let receiver = Receiver {
        inner: ConsumeOnDrop::new(ReceiverInner::new(next_node, consumer, notify_pool)),
        next_node_rx,
    };
    (sender, receiver)
}
