use arc_slice_pool::{Arc, ArcIndex, ArcPool};
use derive_where::derive_where;

use crate::TryRecvError;

pub struct Sender<T>(crate::Sender<ArcIndex<T>, std::sync::Arc<ArcPool<T>>>);

#[derive_where(Clone)]
pub struct Receiver<T>(crate::Receiver<ArcIndex<T>, std::sync::Arc<ArcPool<T>>>);

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let arc_pool = std::sync::Arc::new(ArcPool::new());
    let (sender, receiver) = crate::channel_with_consumer(arc_pool);
    (Sender(sender), Receiver(receiver))
}

impl<T> Sender<T> {
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

    pub fn subscribe(&self) -> Receiver<T> {
        Receiver(self.0.subscribe())
    }

    pub async fn closed(&self) {
        self.0.closed().await
    }

    pub fn pool(&self) -> &std::sync::Arc<ArcPool<T>> {
        self.0.consumer()
    }
}

impl<T> Receiver<T> {
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
