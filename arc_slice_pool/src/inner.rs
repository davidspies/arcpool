use std::{
    cell::UnsafeCell,
    marker::PhantomData,
    mem::{self, MaybeUninit},
    ops::Deref,
    sync::{
        atomic::{self, AtomicUsize},
        Arc as StdArc, OnceLock, Weak,
    },
};

use consume_on_drop::Consume;
use parking_lot::Mutex;

/// A smaller representation of a [crate::Arc] that does not know which
/// [crate::ArcPool] it came from. Behaves similarly to the raw ptr used in
/// [from_raw](StdArc::from_raw) and [into_raw](StdArc::into_raw).
pub struct ArcIndex<T> {
    index: usize,
    phantom: PhantomData<*const T>,
}

unsafe impl<T: Send + Sync> Send for ArcIndex<T> {}
unsafe impl<T: Send + Sync> Sync for ArcIndex<T> {}

impl<T> Drop for ArcIndex<T> {
    fn drop(&mut self) {
        panic!("ArcIndex dropped without from_index")
    }
}

pub(super) struct ArcPoolInner<T> {
    free_list: Mutex<Vec<usize>>,
    mem: Box<[(AtomicUsize, UnsafeCell<MaybeUninit<T>>)]>,
    offset: usize,
    prev: Weak<ArcPoolInner<T>>,
    next: OnceLock<StdArc<ArcPoolInner<T>>>,
}

impl<T> ArcPoolInner<T> {
    pub(super) fn with_capacity(cap: usize, offset: usize, prev: Weak<ArcPoolInner<T>>) -> Self {
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

    pub(super) fn try_alloc(self: &StdArc<Self>, value: T) -> Result<ArcInner<T>, T> {
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
            phantom: PhantomData,
        })
    }

    pub(super) fn capacity(&self) -> usize {
        self.mem.len()
    }

    pub(super) fn offset(&self) -> usize {
        self.offset
    }

    pub(super) fn set_next(&self, next: StdArc<ArcPoolInner<T>>) {
        self.next
            .set(next)
            .unwrap_or_else(|_| panic!("next already set"))
    }

    pub(super) fn is_empty(&self) -> bool {
        self.free_list.lock().len() == self.mem.len() && self.prev.weak_count() == 0
    }
}

pub(super) struct ArcInner<T> {
    pool: StdArc<ArcPoolInner<T>>,
    index: usize,
    phantom: PhantomData<*const T>,
}

unsafe impl<T: Send + Sync> Send for ArcInner<T> {}
unsafe impl<T: Send + Sync> Sync for ArcInner<T> {}

impl<T> Clone for ArcInner<T> {
    fn clone(&self) -> Self {
        let (ref_count, _ptr) = &self.pool.mem[self.index];
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Self {
            pool: self.pool.clone(),
            index: self.index,
            phantom: PhantomData,
        }
    }
}

impl<T> Deref for ArcInner<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let (_ref_count, ptr) = &self.pool.mem[self.index];
        let ptr = ptr.get();
        unsafe { (*ptr).assume_init_ref() }
    }
}

impl<T> Consume for ArcInner<T> {
    fn consume(self) {
        drop(self.into_inner());
    }
}

impl<T> ArcInner<T> {
    pub(super) fn get_mut(&mut self) -> Option<&mut T> {
        let (ref_count, ptr) = &self.pool.mem[self.index];
        if ref_count.load(atomic::Ordering::Relaxed) != 1 {
            return None;
        }
        atomic::fence(atomic::Ordering::Acquire);
        let ptr = ptr.get();
        Some(unsafe { (*ptr).assume_init_mut() })
    }

    pub(super) fn into_inner(self) -> Option<T> {
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

    pub(super) fn ref_count(&self) -> usize {
        let (ref_count, _ptr) = &self.pool.mem[self.index];
        ref_count.load(atomic::Ordering::Acquire)
    }

    pub(super) fn is_unique(&self) -> bool {
        let (ref_count, _ptr) = &self.pool.mem[self.index];
        let ref_count = ref_count.load(atomic::Ordering::Relaxed);
        let is_unique = ref_count == 1;
        if is_unique {
            atomic::fence(atomic::Ordering::Acquire);
        }
        is_unique
    }

    pub(super) fn try_unwrap(self) -> Result<T, Self> {
        let (ref_count, _ptr) = &self.pool.mem[self.index];
        match ref_count.compare_exchange_weak(
            1,
            0,
            atomic::Ordering::Acquire,
            atomic::Ordering::Relaxed,
        ) {
            Ok(_) => {}
            Err(_) => return Err(self),
        }
        let (_ref_count, ptr) = &self.pool.mem[self.index];
        let ptr = ptr.get();
        let result = mem::replace(unsafe { &mut *ptr }, MaybeUninit::uninit());
        self.pool.free_list.lock().push(self.index);
        Ok(unsafe { result.assume_init() })
    }

    pub(super) fn into_index(this: Self) -> ArcIndex<T> {
        unsafe { StdArc::increment_strong_count(StdArc::as_ptr(&this.pool)) }
        ArcIndex {
            index: this.pool.offset + this.index,
            phantom: PhantomData,
        }
    }

    pub(super) unsafe fn from_index(pool: StdArc<ArcPoolInner<T>>, index: ArcIndex<T>) -> Self {
        let result = Self::reconstruct_from_ref(pool, &index);
        StdArc::decrement_strong_count(StdArc::as_ptr(&result.pool));
        mem::forget(index);
        result
    }

    pub(super) unsafe fn clone_from_index(
        pool: StdArc<ArcPoolInner<T>>,
        index: &ArcIndex<T>,
    ) -> Self {
        let result = Self::reconstruct_from_ref(pool, index);
        let (ref_count, _) = &result.pool.mem[result.index];
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        result
    }

    pub(super) fn ptr_eq(&self, other: &Self) -> bool {
        StdArc::ptr_eq(&self.pool, &other.pool) && self.index == other.index
    }

    unsafe fn reconstruct_from_ref(
        mut pool: StdArc<ArcPoolInner<T>>,
        &ArcIndex { index, phantom: _ }: &ArcIndex<T>,
    ) -> Self {
        let mut offset = pool.offset;
        while index < offset {
            pool = pool.prev.upgrade().unwrap();
            offset = pool.offset;
        }
        Self {
            index: index - offset,
            pool,
            phantom: PhantomData,
        }
    }
}
