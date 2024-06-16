use std::sync::OnceLock;

use derive_where::derive_where;
use parking_lot::Mutex;
use tokio::sync::watch;

use crate::{Arc, ArcPool};

pub struct Sender<T> {
    node_pool: ArcPool<Node<T>>,
    next: Mutex<Arc<Node<T>>>,
    notify: watch::Sender<()>,
}

impl<T> Sender<T> {
    pub fn send(&self, value: T) -> Result<(), T> {
        if self.notify.is_closed() {
            return Err(value);
        }
        let mut next = self.next.lock();
        let next_arc = self.node_pool.alloc(Node::default());
        next.set(value, next_arc.clone());
        *next = next_arc;
        let _ = self.notify.send(());
        Ok(())
    }

    pub fn subscribe(&self) -> Receiver<T> {
        Receiver {
            next: self.next.lock().clone(),
            notify: self.notify.subscribe(),
        }
    }

    pub async fn closed(&self) {
        self.notify.closed().await
    }
}

#[derive_where(Clone)]
pub struct Receiver<T> {
    next: Arc<Node<T>>,
    notify: watch::Receiver<()>,
}

pub enum TryRecvError {
    Empty,
    Disconnected,
}

impl<T: Clone> Receiver<T> {
    pub async fn recv(&mut self) -> Option<T> {
        loop {
            if let Some(result) = self.try_next() {
                return Some(result);
            }
            self.notify.changed().await.ok()?;
        }
    }

    pub fn try_recv(&mut self) -> Result<T, TryRecvError> {
        self.try_next().ok_or_else(|| {
            if self.notify.has_changed().is_err() {
                TryRecvError::Disconnected
            } else {
                TryRecvError::Empty
            }
        })
    }

    fn try_next(&mut self) -> Option<T> {
        let (value, next_node) = self.next.get()?;
        let value = value.clone();
        self.next = next_node.clone();
        Some(value)
    }
}

#[derive_where(Default)]
struct Node<T>(OnceLock<(T, Arc<Node<T>>)>);

impl<T> Node<T> {
    fn set(&self, value: T, next: Arc<Node<T>>) {
        self.0.set((value, next)).unwrap_or_else(|_| {
            panic!("Node::set: called twice on the same node");
        });
    }
    fn get(&self) -> Option<&(T, Arc<Node<T>>)> {
        self.0.get()
    }
}

pub fn channel<T>() -> (Sender<T>, Receiver<T>) {
    let (tx, rx) = watch::channel(());
    let node_pool = ArcPool::new();
    let initial = node_pool.alloc(Node::default());
    (
        Sender {
            node_pool,
            next: Mutex::new(initial.clone()),
            notify: tx,
        },
        Receiver {
            next: initial,
            notify: rx,
        },
    )
}
