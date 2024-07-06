use std::sync::atomic::{self, AtomicUsize};

use derive_where::derive_where;
use stable_queue::{StableIndex, StableQueue};

pub(super) struct ArcPoolInner<T>(StableQueue<StableQueue<Entry<T>>>);

#[derive_where(Clone)]
pub(super) struct ArcData<T> {
    ptr: *const Entry<T>,
    outer_index: StableIndex,
    inner_index: StableIndex,
}

pub(super) struct Entry<T> {
    refcount: AtomicUsize,
    pub(super) value: T,
}

impl<T> ArcPoolInner<T> {
    pub(super) fn with_capacity(cap: usize) -> Self {
        let mut queue = StableQueue::new_unbounded();
        queue
            .push_back(StableQueue::with_fixed_capacity(cap))
            .unwrap_or_else(|_| unreachable!());
        Self(queue)
    }

    pub(super) fn alloc_data(&mut self, value: T) -> ArcData<T> {
        let mut outer_index = self.0.back_idx().unwrap();
        let mut queue = self.0.back_mut().unwrap();
        let back_cap = queue.fixed_capacity().unwrap();
        let new_entry = Entry {
            refcount: AtomicUsize::new(1),
            value,
        };
        let inner_index = match queue.push_back(new_entry) {
            Ok(index) => index,
            Err(rejected) => {
                outer_index = self
                    .0
                    .push_back(StableQueue::with_fixed_capacity(back_cap * 2))
                    .unwrap_or_else(|_| unreachable!());
                queue = self.0.back_mut().unwrap();
                queue.push_back(rejected).unwrap_or_else(|_| unreachable!())
            }
        };
        ArcData {
            ptr: &queue[inner_index],
            outer_index,
            inner_index,
        }
    }

    pub(super) fn pop_front_and_arc_next(&mut self) -> (T, Option<ArcData<T>>) {
        let queue = self.0.front_mut().unwrap();
        let Entry { refcount, value } = queue.pop_front().unwrap();
        debug_assert_eq!(refcount.into_inner(), 0);
        if queue.is_empty() {
            if self.0.len() == 1 {
                return (value, None);
            }
            self.0.pop_front();
        }
        let outer_index = self.0.front_idx().unwrap();
        let queue = self.0.front_mut().unwrap();
        let inner_index = queue.front_idx().unwrap();
        let ptr = queue.front().unwrap();
        ptr.refcount.fetch_add(1, atomic::Ordering::Relaxed);
        let ptr = ptr as *const _;
        let data = ArcData {
            ptr,
            outer_index,
            inner_index,
        };
        (value, Some(data))
    }

    pub(super) unsafe fn next(&self, data: &ArcData<T>) -> Option<ArcData<T>> {
        let new_outer_index;
        let new_ptr;
        let mut new_inner_index = data.inner_index.next_idx();
        match self.0[data.outer_index].get(new_inner_index) {
            Some(ptr) => {
                new_outer_index = data.outer_index;
                new_ptr = ptr;
            }
            None => {
                new_outer_index = data.outer_index.next_idx();
                let next_queue = self.0.get(new_outer_index)?;
                new_inner_index = next_queue.front_idx().unwrap();
                new_ptr = next_queue.front().unwrap();
            }
        }
        new_ptr.refcount.fetch_add(1, atomic::Ordering::Relaxed);
        Some(ArcData {
            ptr: new_ptr,
            outer_index: new_outer_index,
            inner_index: new_inner_index,
        })
    }

    pub(super) fn front_entry(&self) -> Option<&Entry<T>> {
        self.0.front().unwrap().front()
    }

    pub(super) fn pop_front_while_dropped(&mut self) {
        loop {
            let Some(Entry { refcount, value: _ }) = self.front_entry() else {
                break;
            };
            if refcount.load(atomic::Ordering::Relaxed) > 0 {
                break;
            }
            atomic::fence(atomic::Ordering::Acquire);
            drop(self.pop_front_entry().unwrap());
        }
    }

    fn pop_front_entry(&mut self) -> Option<Entry<T>> {
        let front_queue = self.0.front_mut().unwrap();
        let result = front_queue.pop_front()?;
        if front_queue.is_empty() && self.0.len() > 1 {
            self.0.pop_front();
        }
        Some(result)
    }
}

impl<T> ArcData<T> {
    pub(super) unsafe fn refcount(&self) -> &AtomicUsize {
        &(*self.ptr).refcount
    }

    pub(super) fn ptr(&self) -> *const Entry<T> {
        self.ptr
    }
}
