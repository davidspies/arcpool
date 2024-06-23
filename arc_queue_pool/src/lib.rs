use std::{
    collections::VecDeque,
    ops::Deref,
    ptr,
    sync::atomic::{self, AtomicUsize},
};

use consume_on_drop::{Consume, ConsumeOnDrop};
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
        Arc(ConsumeOnDrop::new(ArcInner {
            pool: self.clone(),
            ptr: &back_queue[index],
            index,
        }))
    }
}

#[derive_where(Clone)]
pub struct Arc<T>(ConsumeOnDrop<ArcInner<T>>);

impl<T> Arc<T> {
    pub fn next(this: &Self) -> Option<Self> {
        this.0.next().map(|inner| Self(ConsumeOnDrop::new(inner)))
    }

    pub fn into_inner(this: Self) -> Option<T> {
        ConsumeOnDrop::into_inner(this.0).into_inner()
    }

    pub fn into_inner_and_next(this: Self) -> Option<(T, Option<Self>)> {
        let (inner, next) = ConsumeOnDrop::into_inner(this.0).into_inner_and_next()?;
        Some((inner, next.map(|inner| Self(ConsumeOnDrop::new(inner)))))
    }
}

impl<T> Deref for Arc<T> {
    type Target = T;

    fn deref(&self) -> &T {
        self.0.get()
    }
}

struct ArcInner<T> {
    pool: ArcPool<T>,
    ptr: *const (AtomicUsize, T),
    index: StableIndex,
}

impl<T> ArcInner<T> {
    fn next(&self) -> Option<Self> {
        let guard = self.pool.0.read();
        let (my_queue_ind, my_queue) = guard
            .iter()
            .enumerate()
            .rev()
            .find(|(_i, queue)| {
                queue
                    .get(self.index)
                    .is_some_and(|item| ptr::addr_eq(item, self.ptr))
            })
            .unwrap();
        let next_queue;
        let next_idx;
        match my_queue.increment_index(self.index) {
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
        Some(ArcInner {
            pool: self.pool.clone(),
            ptr: next_ptr,
            index: next_idx,
        })
    }

    fn into_inner(self) -> Option<T> {
        let (ref_count, _) = unsafe { &*self.ptr };
        let mut cur_count = ref_count.load(atomic::Ordering::Relaxed);
        while cur_count > 1 {
            match ref_count.compare_exchange_weak(
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
        ref_count
            .compare_exchange(1, 0, atomic::Ordering::Acquire, atomic::Ordering::Relaxed)
            .unwrap();
        let front_ptr = guard.front().unwrap().front().unwrap();
        if !ptr::addr_eq(front_ptr, self.ptr) {
            return None;
        }
        drop(guard);
        let mut result = None;
        let mut guard = self.pool.0.write();
        loop {
            let front_queue = guard.front_mut().unwrap();
            let Some((refcount, _val)) = front_queue.front() else {
                break;
            };
            if refcount.load(atomic::Ordering::Acquire) != 0 {
                break;
            }
            let (_refcount, val) = front_queue.pop_front().unwrap();
            result.get_or_insert(val);
            if front_queue.is_empty() {
                if guard.len() == 1 {
                    break;
                }
                guard.pop_front();
            }
        }
        Some(result.unwrap())
    }

    fn into_inner_and_next(self) -> Option<(T, Option<ArcInner<T>>)> {
        let (ref_count, _) = unsafe { &*self.ptr };
        let mut cur_count = ref_count.load(atomic::Ordering::Relaxed);
        while cur_count > 1 {
            match ref_count.compare_exchange_weak(
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
        ref_count
            .compare_exchange(1, 0, atomic::Ordering::Acquire, atomic::Ordering::Relaxed)
            .unwrap();
        let front_ptr = guard.front().unwrap().front().unwrap();
        if !ptr::addr_eq(front_ptr, self.ptr) {
            return None;
        }
        drop(guard);
        let mut guard = self.pool.0.write();
        let front_queue = guard.front_mut().unwrap();
        let (refcount, _val) = front_queue.front().unwrap();
        debug_assert_eq!(refcount.load(atomic::Ordering::Relaxed), 0);
        let (_refcount, result) = front_queue.pop_front().unwrap();
        if front_queue.is_empty() {
            if guard.len() == 1 {
                return Some((result, None));
            }
            guard.pop_front();
        }
        let front_queue = guard.front_mut().unwrap();
        let index = front_queue.front_idx().unwrap();
        let next_ptr = front_queue.front().unwrap();
        let (ref_count, _value) = next_ptr;
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Some((
            result,
            Some(ArcInner {
                pool: self.pool.clone(),
                ptr: next_ptr,
                index,
            }),
        ))
    }

    fn get(&self) -> &T {
        let (_, value) = unsafe { &*self.ptr };
        value
    }
}

impl<T> Clone for ArcInner<T> {
    fn clone(&self) -> Self {
        let (ref_count, _) = unsafe { &*self.ptr };
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        ArcInner {
            pool: self.pool.clone(),
            ptr: self.ptr,
            index: self.index,
        }
    }
}

impl<T> Consume for ArcInner<T> {
    fn consume(self) {
        self.into_inner();
    }
}

unsafe impl<T: Send + Sync> Send for Arc<T> {}
unsafe impl<T: Send + Sync> Sync for Arc<T> {}
