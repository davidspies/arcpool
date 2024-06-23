use std::{
    collections::VecDeque,
    mem,
    ops::{Index, IndexMut},
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct StableIndex(usize);

pub struct StableQueue<T> {
    inner: VecDeque<T>,
    head: StableIndex,
}

impl<T> StableQueue<T> {
    pub fn with_capacity(cap: usize) -> Self {
        if mem::size_of::<T>() == 0 {
            panic!("Zero-sized types are not supported");
        }
        Self {
            inner: VecDeque::with_capacity(cap),
            head: StableIndex(0),
        }
    }

    pub fn front(&self) -> Option<&T> {
        self.inner.front()
    }

    pub fn push_back(&mut self, value: T) -> Result<StableIndex, T> {
        if self.len() == self.capacity() {
            return Err(value);
        }
        let i = self
            .back_idx()
            .map_or(self.head, |i| self.increment_helper(i));
        self.inner.push_back(value);
        Ok(i)
    }

    pub fn pop_front(&mut self) -> Option<T> {
        let popped = self.inner.pop_front()?;
        self.head = self.increment_helper(self.head);
        Some(popped)
    }

    pub fn capacity(&self) -> usize {
        self.inner.capacity()
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn get(&self, StableIndex(physical_idx): StableIndex) -> Option<&T> {
        let StableIndex(head) = self.head;
        let ind = if physical_idx < head {
            self.capacity() - head + physical_idx
        } else {
            physical_idx - head
        };
        self.inner.get(ind)
    }

    pub fn get_mut(&mut self, StableIndex(physical_idx): StableIndex) -> Option<&mut T> {
        let StableIndex(head) = self.head;
        let ind = if physical_idx < head {
            self.capacity() - head + physical_idx
        } else {
            physical_idx - head
        };
        self.inner.get_mut(ind)
    }

    /// Undefined behavior for a StableIndex that came from a different StableQueue
    pub fn increment_index(&self, idx: StableIndex) -> Option<StableIndex> {
        if Some(idx) == self.back_idx() {
            None
        } else {
            Some(self.increment_helper(idx))
        }
    }

    fn increment_helper(&self, StableIndex(mut idx): StableIndex) -> StableIndex {
        idx += 1;
        if idx == self.capacity() {
            idx = 0;
        }
        StableIndex(idx)
    }

    pub fn front_idx(&self) -> Option<StableIndex> {
        if self.is_empty() {
            None
        } else {
            Some(self.head)
        }
    }

    pub fn back_idx(&self) -> Option<StableIndex> {
        if self.is_empty() {
            return None;
        }
        let StableIndex(head) = self.head;
        let i = if self.capacity() - head < self.len() - 1 {
            head.wrapping_add(self.len() - 1)
                .wrapping_sub(self.capacity())
        } else {
            head + (self.len() - 1)
        };
        Some(StableIndex(i))
    }
}

impl<T> Index<StableIndex> for StableQueue<T> {
    type Output = T;

    fn index(&self, idx: StableIndex) -> &Self::Output {
        self.get(idx).unwrap()
    }
}

impl<T> IndexMut<StableIndex> for StableQueue<T> {
    fn index_mut(&mut self, idx: StableIndex) -> &mut Self::Output {
        self.get_mut(idx).unwrap()
    }
}
