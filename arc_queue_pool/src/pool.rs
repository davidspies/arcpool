use std::ptr;
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc as StdArc;

use consume_on_drop::Consume;
use derive_where::derive_where;
use parking_lot::RwLock;

use self::inner::{ArcData, ArcPoolInner};

mod inner;

#[derive_where(Clone)]
pub struct ArcPool<T>(StdArc<RwLock<ArcPoolInner<T>>>);

impl<T> Default for ArcPool<T> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T> ArcPool<T> {
    pub fn new() -> Self {
        Self::with_capacity(1)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(StdArc::new(RwLock::new(ArcPoolInner::with_capacity(cap))))
    }

    pub(super) fn alloc_inner(&self, value: T) -> ArcInner<T> {
        ArcInner {
            pool: self.clone(),
            data: self.0.write().alloc_data(value),
        }
    }

    unsafe fn try_unwrap_and_next(&self, data: &ArcData<T>) -> Option<(T, Option<ArcInner<T>>)> {
        let cur_count = data.refcount().load(atomic::Ordering::Relaxed);
        if cur_count > 1 {
            return None;
        }
        atomic::fence(atomic::Ordering::Acquire);
        let guard = self.0.read();
        let front_ptr = guard.front_entry().unwrap();
        if !ptr::addr_eq(front_ptr, data.ptr()) {
            return None;
        }
        drop(guard);
        data.refcount()
            .compare_exchange(1, 0, atomic::Ordering::Relaxed, atomic::Ordering::Relaxed)
            .unwrap();
        let (value, data) = self.0.write().pop_front_and_arc_next();
        Some((
            value,
            data.map(|data| ArcInner {
                pool: self.clone(),
                data,
            }),
        ))
    }

    unsafe fn into_inner_and_next(self, data: ArcData<T>) -> Option<(T, Option<ArcInner<T>>)> {
        let is_unique = is_unique_or_else_decr(data.refcount());
        if !is_unique {
            return None;
        }
        let guard = self.0.read();
        let old_count = data.refcount().fetch_sub(1, atomic::Ordering::Release);
        // Can happen as a result of another thread calling `next`
        if old_count > 1 {
            return None;
        }
        atomic::fence(atomic::Ordering::Acquire);
        let front_ptr = guard.front_entry().unwrap();
        if !ptr::addr_eq(front_ptr, data.ptr()) {
            return None;
        }
        drop(guard);
        let (value, data) = self.0.write().pop_front_and_arc_next();
        Some((value, data.map(|data| ArcInner { pool: self, data })))
    }
}

pub(super) struct ArcInner<T> {
    pool: ArcPool<T>,
    data: ArcData<T>,
}

unsafe impl<T: Send + Sync> Send for ArcInner<T> {}
unsafe impl<T: Send + Sync> Sync for ArcInner<T> {}

impl<T> ArcInner<T> {
    pub(super) fn next(&self) -> Option<Self> {
        let read_guard = self.pool.0.read();
        Some(ArcInner {
            pool: self.pool.clone(),
            data: unsafe { read_guard.next(&self.data) }?,
        })
    }

    pub(super) fn into_inner_and_next(self) -> Option<(T, Option<ArcInner<T>>)> {
        unsafe { self.pool.into_inner_and_next(self.data) }
    }

    pub(super) fn try_unwrap_and_next(self) -> Result<(T, Option<ArcInner<T>>), ArcInner<T>> {
        unsafe { self.pool.try_unwrap_and_next(&self.data).ok_or(self) }
    }

    pub(super) fn get(&self) -> &T {
        &unsafe { &*self.data.ptr() }.value
    }

    pub(super) fn ref_count(&self) -> usize {
        unsafe { self.data.refcount() }.load(atomic::Ordering::Relaxed)
    }
}

fn is_unique_or_else_decr(refcount: &AtomicUsize) -> bool {
    let mut cur_count = refcount.load(atomic::Ordering::Relaxed);
    while cur_count > 1 {
        match refcount.compare_exchange_weak(
            cur_count,
            cur_count - 1,
            atomic::Ordering::Release,
            atomic::Ordering::Relaxed,
        ) {
            Ok(_) => return false,
            Err(count) => cur_count = count,
        }
    }
    true
}

impl<T> Clone for ArcInner<T> {
    fn clone(&self) -> Self {
        let Self { pool, data } = self;
        unsafe { data.refcount() }.fetch_add(1, atomic::Ordering::Relaxed);
        ArcInner {
            pool: pool.clone(),
            data: data.clone(),
        }
    }
}

impl<T> Consume for ArcInner<T> {
    fn consume(self) {
        let old_count = unsafe { self.data.refcount() }.fetch_sub(1, atomic::Ordering::Release);
        if old_count > 1 {
            return;
        }
        atomic::fence(atomic::Ordering::Acquire);
        let guard = self.pool.0.read();
        let front_ptr = guard.front_entry();
        if !front_ptr.is_some_and(|ptr| ptr::addr_eq(ptr, self.data.ptr())) {
            return;
        }
        drop(guard);
        self.pool.0.write().pop_front_while_dropped();
    }
}
