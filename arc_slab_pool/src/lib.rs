use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ops::Deref,
    sync::{
        atomic::{self, AtomicUsize},
        OnceLock,
    },
};

use consume_on_drop::{Consume, ConsumeOnDrop};
use derive_where::derive_where;
use parking_lot::{Mutex, RwLock, RwLockUpgradableReadGuard};

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
            read_guard.mem.len() * 2,
            read_guard.offset + read_guard.mem.len(),
            std::sync::Arc::downgrade(&read_guard),
        ));
        read_guard
            .next
            .set(inner.clone())
            .unwrap_or_else(|_| unreachable!());
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

struct ArcInner<T> {
    pool: std::sync::Arc<ArcPoolInner<T>>,
    index: usize,
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}

pub struct ArcIndex(usize, ConsumeOnDrop<Panicker>);

struct Panicker;

impl Consume for Panicker {
    fn consume(self) {
        panic!("ArcIndex dropped without from_index")
    }
}

impl<T> Arc<T> {
    pub fn into_inner(Self { inner, _phantom }: Self) -> Option<T> {
        ConsumeOnDrop::into_inner(inner).into_inner()
    }

    pub fn into_index(self) -> ArcIndex {
        ArcInner::into_index(ConsumeOnDrop::into_inner(self.inner))
    }

    pub unsafe fn from_index(pool: &ArcPool<T>, index: ArcIndex) -> Self {
        Self {
            inner: ConsumeOnDrop::new(ArcInner::from_index(&pool, index)),
            _phantom: PhantomData,
        }
    }

    pub unsafe fn clone_from_index(pool: &ArcPool<T>, index: &ArcIndex) -> Self {
        Self {
            inner: ConsumeOnDrop::new(ArcInner::clone_from_index(&pool, index)),
            _phantom: PhantomData,
        }
    }
}

impl<T> ArcInner<T> {
    fn into_inner(self) -> Option<T> {
        let (ref_count, ptr) = &self.pool.mem[self.index];
        if ref_count.fetch_sub(1, atomic::Ordering::Release) > 1 {
            return None;
        }
        atomic::fence(atomic::Ordering::Acquire);
        let ptr = ptr.get();
        let result = mem::replace(unsafe { &mut *ptr }, MaybeUninit::uninit());
        self.pool.free_list.lock().push(self.index);
        Some(unsafe { result.assume_init() })
    }

    fn into_index(this: Self) -> ArcIndex {
        unsafe { std::sync::Arc::increment_strong_count(&this.pool) }
        ArcIndex(this.pool.offset + this.index, ConsumeOnDrop::new(Panicker))
    }

    unsafe fn from_index(pool: &ArcPool<T>, index: ArcIndex) -> Self {
        let result = Self::reconstruct_from_ref(pool, &index);
        std::sync::Arc::decrement_strong_count(&result.pool);
        let ArcIndex(_, panicker) = index;
        let _ = ConsumeOnDrop::into_inner(panicker);
        result
    }

    unsafe fn clone_from_index(pool: &ArcPool<T>, index: &ArcIndex) -> Self {
        let result = Self::reconstruct_from_ref(pool, index);
        let (ref_count, _) = &result.pool.mem[result.index];
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        result
    }

    unsafe fn reconstruct_from_ref(
        ArcPool(pool): &ArcPool<T>,
        &ArcIndex(index, _): &ArcIndex,
    ) -> Self {
        let mut pool = pool.read().clone();
        while index < pool.offset {
            pool = pool.prev.upgrade().unwrap();
        }
        Self {
            index: index - pool.offset,
            pool,
        }
    }
}

impl<T> Clone for ArcInner<T> {
    fn clone(&self) -> Self {
        let (ref_count, _ptr) = &self.pool.mem[self.index];
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Self {
            pool: self.pool.clone(),
            index: self.index,
        }
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &*self.inner
    }
}

impl<T> Deref for ArcInner<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let (_ref_count, ptr) = &self.pool.mem[self.index];
        let ptr = ptr.get();
        unsafe { &(*ptr).assume_init_ref() }
    }
}

impl<T> Consume for ArcInner<T> {
    fn consume(self) {
        drop(self.into_inner());
    }
}

struct ArcPoolInner<T> {
    free_list: Mutex<Vec<usize>>,
    mem: Box<[(AtomicUsize, UnsafeCell<MaybeUninit<T>>)]>,
    offset: usize,
    prev: std::sync::Weak<ArcPoolInner<T>>,
    next: OnceLock<std::sync::Arc<ArcPoolInner<T>>>,
}

impl<T> ArcPoolInner<T> {
    fn with_capacity(cap: usize, offset: usize, prev: std::sync::Weak<ArcPoolInner<T>>) -> Self {
        let cap = cap.max(1);
        Self {
            free_list: Mutex::new((0..cap).rev().collect()),
            mem: (0..cap)
                .map(|_| (AtomicUsize::new(0), UnsafeCell::new(MaybeUninit::uninit())))
                .collect(),
            offset,
            prev,
            next: OnceLock::new(),
        }
    }

    fn try_alloc(self: &std::sync::Arc<Self>, value: T) -> Result<ArcInner<T>, T> {
        let index = match self.free_list.lock().pop() {
            Some(index) => index,
            None => return Err(value),
        };
        let (refcount, ptr) = &self.mem[index];
        refcount.fetch_add(1, atomic::Ordering::Relaxed);
        let ptr = ptr.get();
        unsafe { &mut *ptr }.write(value);
        Ok(ArcInner {
            pool: self.clone(),
            index,
        })
    }
}