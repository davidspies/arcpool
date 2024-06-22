use arc_slab_pool::{Arc, ArcIndex, ArcPool};
use consume_on_drop::ConsumeOnDrop;
use derive_where::derive_where;
use tokio::sync::watch;

use self::consumer::{DropNormally, Safe};
use self::inner::{ReceiverInner, SenderInner};
use self::node::Node;

pub use self::consumer::{Consumer, UnsafeConsumer};

mod inner;
mod node;

pub mod consumer;

pub struct Sender<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    inner: ConsumeOnDrop<SenderInner<T, C>>,
    notify: watch::Sender<()>,
}

impl<T, C: Consumer<T>> Sender<T, Safe<C>> {
    pub fn send(&self, value: T) -> Result<(), T> {
        unsafe { self.unsafe_send(value) }
    }
}

impl<T, C: UnsafeConsumer<T>> Sender<T, C> {
    pub unsafe fn unsafe_send(&self, value: T) -> Result<(), T> {
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

    pub fn consumer(&self) -> &C {
        &self.inner.consumer()
    }
}

#[derive_where(Clone; C)]
pub struct Receiver<T, C: UnsafeConsumer<T> = Safe<DropNormally>> {
    inner: ConsumeOnDrop<ReceiverInner<T, C>>,
    notify: watch::Receiver<()>,
}

impl<T, C: UnsafeConsumer<T>> Receiver<T, C> {
    pub async fn recv(&mut self) -> Option<T> {
        self.notify.mark_unchanged();
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

    pub fn consumer(&self) -> &C {
        &self.inner.consumer()
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
    let (tx, rx) = watch::channel(());
    let node_pool = std::sync::Arc::new(ArcPool::new());
    let initial = node_pool.alloc(Node::default());
    (
        Sender {
            inner: ConsumeOnDrop::new(SenderInner::new(
                node_pool.clone(),
                initial.clone(),
                consumer.clone(),
            )),
            notify: tx,
        },
        Receiver {
            inner: ConsumeOnDrop::new(ReceiverInner::new(node_pool, initial, consumer)),
            notify: rx,
        },
    )
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

    pub fn consumer(&self) -> &std::sync::Arc<ArcPool<T>> {
        self.0.consumer()
    }
}

impl<T> ArcReceiver<T> {
    pub async fn recv(&mut self) -> Option<Arc<T>> {
        let arc_index = self.0.recv().await?;
        let pool = self.0.consumer();
        Some(unsafe { Arc::from_index(pool, arc_index) })
    }

    pub fn try_recv(&mut self) -> Result<Arc<T>, TryRecvError> {
        let arc_index = self.0.try_recv()?;
        let pool = self.0.consumer();
        Ok(unsafe { Arc::from_index(pool, arc_index) })
    }

    pub fn consumer(&self) -> &std::sync::Arc<ArcPool<T>> {
        self.0.consumer()
    }
}
