use std::{marker::PhantomData, ops::Deref};

use consume_on_drop::ConsumeOnDrop;
use derive_where::derive_where;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use self::inner::{ArcInner, ArcPoolInner};

pub use self::inner::ArcIndex;

mod inner;

pub struct ArcPool<T>(RwLock<std::sync::Arc<ArcPoolInner<T>>>);

unsafe impl<T: Send + Sync> Send for ArcPool<T> {}
unsafe impl<T: Send + Sync> Sync for ArcPool<T> {}

impl<T> Default for ArcPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ArcPool<T> {
    pub fn new() -> Self {
        Self::with_capacity(8)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(RwLock::new(std::sync::Arc::new(
            ArcPoolInner::with_capacity(cap, 0, std::sync::Weak::new()),
        )))
    }

    pub fn alloc(&self, mut value: T) -> Arc<T> {
        let read_guard = self.0.read();
        value = match read_guard.try_alloc(value) {
            Ok(arc_inner) => {
                return Arc {
                    inner: ConsumeOnDrop::new(arc_inner),
                    _phantom: PhantomData,
                }
            }
            Err(value) => value,
        };
        drop(read_guard);
        let read_guard = self.0.upgradable_read();
        let inner = std::sync::Arc::new(ArcPoolInner::with_capacity(
            read_guard.capacity() * 2,
            read_guard.offset() + read_guard.capacity(),
            std::sync::Arc::downgrade(&read_guard),
        ));
        read_guard.set_next(inner.clone());
        let arc_inner = inner.try_alloc(value).unwrap_or_else(|_| unreachable!());
        *RwLockUpgradableReadGuard::upgrade(read_guard) = inner;
        Arc {
            inner: ConsumeOnDrop::new(arc_inner),
            _phantom: PhantomData,
        }
    }
}

#[derive_where(Clone)]
pub struct Arc<T> {
    inner: ConsumeOnDrop<ArcInner<T>>,
    _phantom: PhantomData<*const T>, // Arc<T> should not be `Send` unless T is `Send` and `Sync`
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

impl<T> Arc<T> {
    pub fn into_inner(Self { inner, _phantom }: Self) -> Option<T> {
        ConsumeOnDrop::into_inner(inner).into_inner()
    }

    pub fn into_index(self) -> ArcIndex {
        ArcInner::into_index(ConsumeOnDrop::into_inner(self.inner))
    }

    /// # Safety
    /// Must be the same pool as the one that created the Arc
    pub unsafe fn from_index(pool: &ArcPool<T>, index: ArcIndex) -> Self {
        Self {
            inner: ConsumeOnDrop::new(ArcInner::from_index(pool.0.read().clone(), index)),
            _phantom: PhantomData,
        }
    }

    /// # Safety
    /// Must be the same pool as the one that created the Arc
    pub unsafe fn clone_from_index(pool: &ArcPool<T>, index: &ArcIndex) -> Self {
        Self {
            inner: ConsumeOnDrop::new(ArcInner::clone_from_index(pool.0.read().clone(), index)),
            _phantom: PhantomData,
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
