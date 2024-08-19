use std::{
    cmp::Ordering,
    fmt::{self, Debug, Formatter},
    hash::{Hash, Hasher},
    ops::Deref,
    sync::{Arc as StdArc, Weak},
};

use consume_on_drop::ConsumeOnDrop;
use derive_where::derive_where;
use parking_lot::{RwLock, RwLockUpgradableReadGuard};

use self::inner::{ArcInner, ArcPoolInner};

pub use self::inner::ArcIndex;

mod inner;

/// Stores all the allocations for the [Arc]s together in chunks of contiguous memory.
pub struct ArcPool<T>(RwLock<StdArc<ArcPoolInner<T>>>);

unsafe impl<T: Send + Sync> Send for ArcPool<T> {}
unsafe impl<T: Send + Sync> Sync for ArcPool<T> {}

impl<T> Default for ArcPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ArcPool<T> {
    pub fn new() -> Self {
        Self::with_capacity(0)
    }

    pub fn is_empty(&self) -> bool {
        self.0.read().is_empty()
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(RwLock::new(StdArc::new(ArcPoolInner::with_capacity(
            cap,
            0,
            Weak::new(),
        ))))
    }

    pub fn alloc(&self, mut value: T) -> Arc<T> {
        let read_guard = self.0.read();
        value = match read_guard.try_alloc(value) {
            Ok(arc_inner) => return Arc(ConsumeOnDrop::new(arc_inner)),
            Err(value) => value,
        };
        drop(read_guard);
        let read_guard = self.0.upgradable_read();
        let inner = StdArc::new(ArcPoolInner::with_capacity(
            (read_guard.capacity() * 2).max(1),
            read_guard.offset() + read_guard.capacity(),
            StdArc::downgrade(&read_guard),
        ));
        read_guard.set_next(inner.clone());
        let arc_inner = inner.try_alloc(value).unwrap_or_else(|_| unreachable!());
        *RwLockUpgradableReadGuard::upgrade(read_guard) = inner;
        Arc(ConsumeOnDrop::new(arc_inner))
    }
}

/// A reference-counted pointer to a value of type `T` stored in an [ArcPool].
/// Does not support weak references.
#[derive_where(Clone)]
pub struct Arc<T>(ConsumeOnDrop<ArcInner<T>>);

impl<T> Arc<T> {
    pub fn get_mut(this: &mut Self) -> Option<&mut T> {
        this.0.get_mut()
    }

    pub fn into_inner(Self(inner): Self) -> Option<T> {
        ConsumeOnDrop::into_inner(inner).into_inner()
    }

    pub fn try_unwrap(Self(inner): Self) -> Result<T, Self> {
        match ConsumeOnDrop::into_inner(inner).try_unwrap() {
            Ok(value) => Ok(value),
            Err(inner) => Err(Self(ConsumeOnDrop::new(inner))),
        }
    }

    pub fn into_index(self) -> ArcIndex<T> {
        ArcInner::into_index(ConsumeOnDrop::into_inner(self.0))
    }

    pub fn ptr_eq(this: &Self, other: &Self) -> bool {
        ArcInner::ptr_eq(&this.0, &other.0)
    }

    /// # Safety
    /// Must be the same pool as the one that created the Arc
    pub unsafe fn from_index(pool: &ArcPool<T>, index: ArcIndex<T>) -> Self {
        Self(ConsumeOnDrop::new(ArcInner::from_index(
            pool.0.read().clone(),
            index,
        )))
    }

    /// # Safety
    /// Must be the same pool as the one that created the Arc
    pub unsafe fn clone_from_index(pool: &ArcPool<T>, index: &ArcIndex<T>) -> Self {
        Self(ConsumeOnDrop::new(ArcInner::clone_from_index(
            pool.0.read().clone(),
            index,
        )))
    }

    pub fn ref_count(this: &Self) -> usize {
        this.0.ref_count()
    }

    /// Returns `true` if there are no other `Arc`s pointing to the same value.
    /// 
    /// This is equivalent to `Arc::ref_count(this) == 1` but may be more performant.
    pub fn is_unique(this: &Self) -> bool {
        this.0.is_unique()
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T: Debug> Debug for Arc<T> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        (**self).fmt(f)
    }
}

impl<T: Hash> Hash for Arc<T> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (**self).hash(state)
    }
}

impl<T: PartialEq> PartialEq for Arc<T> {
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<T: Eq> Eq for Arc<T> {}

impl<T: PartialOrd> PartialOrd for Arc<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<T: Ord> Ord for Arc<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        (**self).cmp(&**other)
    }
}
