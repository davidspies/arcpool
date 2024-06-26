use std::{
    cell::UnsafeCell,
    mem::{self, MaybeUninit},
    ops::Deref,
    sync::{
        atomic::{self, AtomicUsize},
        Arc as StdArc, OnceLock, Weak,
    },
};

use consume_on_drop::{Consume, ConsumeOnDrop};
use parking_lot::Mutex;

pub struct ArcIndex(usize, ConsumeOnDrop<Panicker>);

struct Panicker;

impl Consume for Panicker {
    fn consume(self) {
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
}

pub(super) struct ArcInner<T> {
    pool: StdArc<ArcPoolInner<T>>,
    index: usize,
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

    pub(super) fn into_index(this: Self) -> ArcIndex {
        unsafe { StdArc::increment_strong_count(StdArc::as_ptr(&this.pool)) }
        ArcIndex(this.pool.offset + this.index, ConsumeOnDrop::new(Panicker))
    }

    pub(super) unsafe fn from_index(pool: StdArc<ArcPoolInner<T>>, index: ArcIndex) -> Self {
        let result = Self::reconstruct_from_ref(pool, &index);
        StdArc::decrement_strong_count(StdArc::as_ptr(&result.pool));
        let ArcIndex(_, panicker) = index;
        let _panicker = ConsumeOnDrop::into_inner(panicker);
        result
    }

    pub(super) unsafe fn clone_from_index(pool: StdArc<ArcPoolInner<T>>, index: &ArcIndex) -> Self {
        let result = Self::reconstruct_from_ref(pool, index);
        let (ref_count, _) = &result.pool.mem[result.index];
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        result
    }

    unsafe fn reconstruct_from_ref(
        mut pool: StdArc<ArcPoolInner<T>>,
        &ArcIndex(index, _): &ArcIndex,
    ) -> Self {
        let mut offset = pool.offset;
        while index < offset {
            pool = pool.prev.upgrade().unwrap();
            offset = pool.offset;
        }
        Self {
            index: index - offset,
            pool,
        }
    }
}
