use std::{
    ops::Deref,
    ptr,
    sync::atomic::{self, AtomicUsize},
};

use consume_on_drop::{Consume, ConsumeOnDrop};
use derive_where::derive_where;
use parking_lot::RwLock;
use stable_queue::{StableIndex, StableQueue};

#[derive_where(Clone)]
pub struct ArcPool<T>(std::sync::Arc<RwLock<StableQueue<StableQueue<(AtomicUsize, T)>>>>);

impl<T> ArcPool<T> {
    pub fn new() -> Self {
        Self::with_capacity(8)
    }

    pub fn with_capacity(cap: usize) -> Self {
        let mut queue = StableQueue::new_unbounded();
        queue
            .push_back(StableQueue::with_fixed_capacity(cap))
            .unwrap_or_else(|_| unreachable!());
        Self(std::sync::Arc::new(RwLock::new(queue)))
    }

    pub fn alloc(&self, value: T) -> Arc<T> {
        let mut guard = self.0.write();
        let mut back_idx = guard.back_idx().unwrap();
        let mut back_queue = guard.back_mut().unwrap();
        let back_cap = back_queue.fixed_capacity().unwrap();
        let index = match back_queue.push_back((AtomicUsize::new(1), value)) {
            Ok(index) => index,
            Err(rejected) => {
                back_idx = guard
                    .push_back(StableQueue::with_fixed_capacity(back_cap * 2))
                    .unwrap_or_else(|_| unreachable!());
                back_queue = guard.back_mut().unwrap();
                back_queue
                    .push_back(rejected)
                    .unwrap_or_else(|_| unreachable!())
            }
        };
        Arc(ConsumeOnDrop::new(ArcInner {
            pool: self.clone(),
            ptr: &back_queue[index],
            outer_index: back_idx,
            inner_index: index,
        }))
    }

    fn pop_front_and_arc_next(self) -> (T, Option<ArcInner<T>>) {
        let mut guard = self.0.write();
        let front_queue = guard.front_mut().unwrap();
        let (refcount, _val) = front_queue.front().unwrap();
        debug_assert_eq!(refcount.load(atomic::Ordering::Relaxed), 0);
        let (_refcount, result) = front_queue.pop_front().unwrap();
        if front_queue.is_empty() {
            if guard.len() == 1 {
                return (result, None);
            }
            guard.pop_front();
        }
        let front_queue_idx = guard.front_idx().unwrap();
        let front_queue = guard.front_mut().unwrap();
        let index = front_queue.front_idx().unwrap();
        let next_ptr = front_queue.front().unwrap();
        let (ref_count, _value) = next_ptr;
        let next_ptr = next_ptr as *const _;
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        drop(guard);
        (
            result,
            Some(ArcInner {
                pool: self,
                ptr: next_ptr,
                outer_index: front_queue_idx,
                inner_index: index,
            }),
        )
    }
}

#[derive_where(Clone)]
pub struct Arc<T>(ConsumeOnDrop<ArcInner<T>>);

impl<T> Arc<T> {
    pub fn next(this: &Self) -> Option<Self> {
        this.0.next().map(|inner| Self(ConsumeOnDrop::new(inner)))
    }

    pub fn into_inner_and_next(this: Self) -> Option<(T, Option<Self>)> {
        let (value, next) = ConsumeOnDrop::into_inner(this.0).into_inner_and_next()?;
        Some((value, next.map(|inner| Self(ConsumeOnDrop::new(inner)))))
    }

    pub fn try_unwrap_and_next(this: Self) -> Result<(T, Option<Self>), Self> {
        match ConsumeOnDrop::into_inner(this.0).try_unwrap_and_next() {
            Ok((value, next)) => Ok((value, next.map(|inner| Self(ConsumeOnDrop::new(inner))))),
            Err(inner) => Err(Self(ConsumeOnDrop::new(inner))),
        }
    }

    pub fn ref_count(this: &Self) -> usize {
        let (ref_count, _) = unsafe { &*this.0.ptr };
        ref_count.load(atomic::Ordering::Relaxed)
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
    outer_index: StableIndex,
    inner_index: StableIndex,
}

impl<T> ArcInner<T> {
    fn next(&self) -> Option<Self> {
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
        let (ref_count, _value) = new_ptr;
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
        Some(ArcInner {
            pool: self.pool.clone(),
            ptr: new_ptr,
            outer_index: new_outer_index,
            inner_index: new_inner_index,
        })
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
        Some(self.pool.pop_front_and_arc_next())
    }

    fn try_unwrap_and_next(self) -> Result<(T, Option<ArcInner<T>>), ArcInner<T>> {
        let (ref_count, _) = unsafe { &*self.ptr };
        let cur_count = ref_count.load(atomic::Ordering::Relaxed);
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
        ref_count
            .compare_exchange(1, 0, atomic::Ordering::Acquire, atomic::Ordering::Relaxed)
            .unwrap();
        Ok(self.pool.pop_front_and_arc_next())
    }

    fn get(&self) -> &T {
        let (_, value) = unsafe { &*self.ptr };
        value
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
        let (ref_count, _) = unsafe { &*ptr };
        ref_count.fetch_add(1, atomic::Ordering::Relaxed);
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
        let (ref_count, _) = unsafe { &*self.ptr };
        if ref_count.fetch_sub(1, atomic::Ordering::Release) > 1 {
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
            let Some((refcount, _val)) = front_queue.front() else {
                break;
            };
            if refcount.load(atomic::Ordering::Acquire) > 0 {
                break;
            }
            let (_refcount, val) = front_queue.pop_front().unwrap();
            drop(val);
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
