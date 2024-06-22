use std::{
    collections::VecDeque,
    ops::Deref,
    ptr,
    sync::atomic::{self, AtomicUsize},
};

use derive_where::derive_where;
use parking_lot::RwLock;
use stable_queue::{StableIndex, StableQueue};

#[derive_where(Clone)]
pub struct ArcPool<T>(std::sync::Arc<RwLock<VecDeque<StableQueue<(AtomicUsize, T)>>>>);

impl<T> ArcPool<T> {
    pub fn new() -> Self {
        Self::with_capacity(8)
    }

    pub fn with_capacity(cap: usize) -> Self {
        Self(std::sync::Arc::new(RwLock::new(VecDeque::from_iter([
            StableQueue::with_capacity(cap.max(1)),
        ]))))
    }

    pub fn alloc(&self, value: T) -> Arc<T> {
        let mut guard = self.0.write();
        let mut back_queue = guard.back_mut().unwrap();
        let back_cap = back_queue.capacity();
        let index = match back_queue.push_back((AtomicUsize::new(1), value)) {
            Ok(index) => index,
            Err(rejected) => {
                guard.push_back(StableQueue::with_capacity(back_cap * 2));
                back_queue = guard.back_mut().unwrap();
                back_queue
                    .push_back(rejected)
                    .unwrap_or_else(|_| unreachable!())
            }
        };
        Arc {
            pool: self.clone(),
            ptr: &back_queue[index],
            index,
        }
    }
}

pub struct Arc<T> {
    pool: ArcPool<T>,
    ptr: *const (AtomicUsize, T),
    index: StableIndex,
}

impl<T> Arc<T> {
    pub fn next(this: &Self) -> Option<Self> {
        let guard = this.pool.0.read();
        let (my_queue_ind, my_queue) = guard
            .iter()
            .enumerate()
            .rev()
            .find(|(_i, queue)| {
                queue
                    .get(this.index)
                    .is_some_and(|item| ptr::addr_eq(item, this.ptr))
            })
            .unwrap();
        let next_queue;
        let next_idx;
        match my_queue.increment_index(this.index) {
            Some(new_idx) => {
                next_queue = my_queue;
                next_idx = new_idx;
            }
            None => {
                next_queue = guard.get(my_queue_ind + 1)?;
                next_idx = next_queue.front_idx().unwrap();
            }
        }
        let next_ptr = &next_queue[next_idx];
        let (ref_count, _value) = next_ptr;
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Some(Arc {
            pool: this.pool.clone(),
            ptr: next_ptr,
            index: next_idx,
        })
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        let (_, value) = unsafe { &*self.ptr };
        value
    }
}

impl<T> Clone for Arc<T> {
    fn clone(&self) -> Self {
        let (ref_count, _) = unsafe { &*self.ptr };
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Arc {
            pool: self.pool.clone(),
            ptr: self.ptr,
            index: self.index,
        }
    }
}

impl<T> Drop for Arc<T> {
    fn drop(&mut self) {
        let (ref_count, _) = unsafe { &*self.ptr };
        if ref_count.fetch_sub(1, atomic::Ordering::Release) != 1 {
            return;
        }
        let guard = self.pool.0.read();
        let front_ptr = guard.front().unwrap().front().unwrap();
        if !ptr::addr_eq(front_ptr, self.ptr) {
            return;
        }
        drop(guard);
        let mut guard = self.pool.0.write();
        loop {
            let front_queue = guard.front_mut().unwrap();
            let Some((refcount, _val)) = front_queue.front() else {
                break;
            };
            if refcount.load(atomic::Ordering::Acquire) != 0 {
                break;
            }
            front_queue.pop_front().unwrap();
            if front_queue.is_empty() {
                if guard.len() == 1 {
                    break;
                }
                guard.pop_front();
            }
        }
    }
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}
