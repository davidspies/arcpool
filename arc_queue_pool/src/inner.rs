use std::ptr;
use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc as StdArc;

use consume_on_drop::Consume;
use derive_where::derive_where;
use parking_lot::RwLock;
use stable_queue::{StableIndex, StableQueue};

#[derive_where(Clone)]
pub struct ArcPool<T>(StdArc<RwLock<StableQueue<StableQueue<Entry<T>>>>>);

struct Entry<T> {
    refcount: AtomicUsize,
    value: T,
}

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
        let mut queue = StableQueue::new_unbounded();
        queue
            .push_back(StableQueue::with_fixed_capacity(cap))
            .unwrap_or_else(|_| unreachable!());
        Self(StdArc::new(RwLock::new(queue)))
    }

    pub(crate) fn alloc_inner(&self, value: T) -> ArcInner<T> {
        let mut guard = self.0.write();
        let mut outer_index = guard.back_idx().unwrap();
        let mut queue = guard.back_mut().unwrap();
        let back_cap = queue.fixed_capacity().unwrap();
        let new_entry = Entry {
            refcount: AtomicUsize::new(1),
            value,
        };
        let inner_index = match queue.push_back(new_entry) {
            Ok(index) => index,
            Err(rejected) => {
                outer_index = guard
                    .push_back(StableQueue::with_fixed_capacity(back_cap * 2))
                    .unwrap_or_else(|_| unreachable!());
                queue = guard.back_mut().unwrap();
                queue.push_back(rejected).unwrap_or_else(|_| unreachable!())
            }
        };
        ArcInner {
            pool: self.clone(),
            ptr: &queue[inner_index],
            outer_index,
            inner_index,
        }
    }

    fn pop_front_and_arc_next(self) -> (T, Option<ArcInner<T>>) {
        let mut guard = self.0.write();
        let queue = guard.front_mut().unwrap();
        let Entry { refcount, value } = queue.pop_front().unwrap();
        debug_assert_eq!(refcount.into_inner(), 0);
        if queue.is_empty() {
            if guard.len() == 1 {
                return (value, None);
            }
            guard.pop_front();
        }
        let outer_index = guard.front_idx().unwrap();
        let queue = guard.front_mut().unwrap();
        let inner_index = queue.front_idx().unwrap();
        let ptr = queue.front().unwrap();
        ptr.refcount.fetch_add(1, atomic::Ordering::Relaxed);
        let ptr = ptr as *const _;
        drop(guard);
        (
            value,
            Some(ArcInner {
                pool: self,
                ptr,
                outer_index,
                inner_index,
            }),
        )
    }
}

pub(crate) struct ArcInner<T> {
    pool: ArcPool<T>,
    ptr: *const Entry<T>,
    outer_index: StableIndex,
    inner_index: StableIndex,
}

impl<T> ArcInner<T> {
    pub(crate) fn next(&self) -> Option<Self> {
        let guard = self.pool.0.read();
        let new_outer_index;
        let new_ptr;
        let mut new_inner_index = self.inner_index.next_idx();
        match guard[self.outer_index].get(new_inner_index) {
            Some(ptr) => {
                new_outer_index = self.outer_index;
                new_ptr = ptr;
            }
            None => {
                new_outer_index = self.outer_index.next_idx();
                let next_queue = guard.get(new_outer_index)?;
                new_inner_index = next_queue.front_idx().unwrap();
                new_ptr = next_queue.front().unwrap();
            }
        }
        new_ptr.refcount.fetch_add(1, atomic::Ordering::Relaxed);
        Some(ArcInner {
            pool: self.pool.clone(),
            ptr: new_ptr,
            outer_index: new_outer_index,
            inner_index: new_inner_index,
        })
    }

    pub(crate) fn into_inner_and_next(self) -> Option<(T, Option<ArcInner<T>>)> {
        let Entry { refcount, value: _ } = unsafe { &*self.ptr };
        let mut cur_count = refcount.load(atomic::Ordering::Relaxed);
        while cur_count > 1 {
            match refcount.compare_exchange_weak(
                cur_count,
                cur_count - 1,
                atomic::Ordering::Release,
                atomic::Ordering::Relaxed,
            ) {
                Ok(_) => return None,
                Err(count) => cur_count = count,
            }
        }
        let guard = self.pool.0.read();
        refcount
            .compare_exchange(1, 0, atomic::Ordering::Acquire, atomic::Ordering::Relaxed)
            .unwrap();
        let front_ptr = guard.front().unwrap().front().unwrap();
        if !ptr::addr_eq(front_ptr, self.ptr) {
            return None;
        }
        drop(guard);
        Some(self.pool.pop_front_and_arc_next())
    }

    pub(crate) fn try_unwrap_and_next(self) -> Result<(T, Option<ArcInner<T>>), ArcInner<T>> {
        let Entry { refcount, value: _ } = unsafe { &*self.ptr };
        let cur_count = refcount.load(atomic::Ordering::Relaxed);
        if cur_count > 1 {
            return Err(self);
        }
        let guard = self.pool.0.read();
        let front_ptr = guard.front().unwrap().front().unwrap();
        if !ptr::addr_eq(front_ptr, self.ptr) {
            drop(guard);
            return Err(self);
        }
        drop(guard);
        refcount
            .compare_exchange(1, 0, atomic::Ordering::Acquire, atomic::Ordering::Relaxed)
            .unwrap();
        Ok(self.pool.pop_front_and_arc_next())
    }

    pub(crate) fn get(&self) -> &T {
        &unsafe { &*self.ptr }.value
    }

    pub(crate) fn ref_count(&self) -> usize {
        unsafe { &*self.ptr }
            .refcount
            .load(atomic::Ordering::Relaxed)
    }
}

impl<T> Clone for ArcInner<T> {
    fn clone(&self) -> Self {
        let &Self {
            ref pool,
            ptr,
            outer_index,
            inner_index,
        } = self;
        unsafe { &*ptr }
            .refcount
            .fetch_add(1, atomic::Ordering::Relaxed);
        ArcInner {
            pool: pool.clone(),
            ptr,
            outer_index,
            inner_index,
        }
    }
}

impl<T> Consume for ArcInner<T> {
    fn consume(self) {
        let old_count = unsafe { &*self.ptr }
            .refcount
            .fetch_sub(1, atomic::Ordering::Release);
        if old_count > 1 {
            return;
        }
        let guard = self.pool.0.read();
        let front_ptr = guard.front().unwrap().front();
        if !front_ptr.is_some_and(|ptr| ptr::addr_eq(ptr, self.ptr)) {
            return;
        }
        drop(guard);
        let mut guard = self.pool.0.write();
        loop {
            let front_queue = guard.front_mut().unwrap();
            let Some(Entry { refcount, value: _ }) = front_queue.front() else {
                break;
            };
            if refcount.load(atomic::Ordering::Acquire) > 0 {
                break;
            }
            drop(front_queue.pop_front().unwrap());
            if front_queue.is_empty() {
                if guard.len() == 1 {
                    break;
                }
                guard.pop_front();
            }
        }
    }
}
