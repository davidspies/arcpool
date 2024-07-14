use std::{
    collections::VecDeque,
    ops::{Index, IndexMut},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct StableIndex(usize);
impl StableIndex {
    pub fn next_idx(self) -> StableIndex {
        StableIndex(self.0 + 1)
    }
}

/// As long as the element is not popped, the index returned by [push_back](Self::push_back)
/// will continue to point to the pushed element.
pub struct StableQueue<T> {
    inner: VecDeque<T>,
    head: StableIndex,
    fixed_capacity: Option<usize>,
}

impl<T> StableQueue<T> {
    pub fn new_unbounded() -> Self {
        Self {
            inner: VecDeque::new(),
            head: StableIndex(0),
            fixed_capacity: None,
        }
    }

    /// If initialized with a fixed capacity, the queue cannot grow beyond that capacity
    /// and the address in memory of elements inserted into the queue will not change.
    pub fn with_fixed_capacity(cap: usize) -> Self {
        Self {
            inner: VecDeque::with_capacity(cap),
            head: StableIndex(0),
            fixed_capacity: Some(cap),
        }
    }

    pub fn front(&self) -> Option<&T> {
        self.inner.front()
    }

    pub fn front_mut(&mut self) -> Option<&mut T> {
        self.inner.front_mut()
    }

    pub fn back(&self) -> Option<&T> {
        self.inner.back()
    }

    pub fn back_mut(&mut self) -> Option<&mut T> {
        self.inner.back_mut()
    }

    pub fn push_back(&mut self, value: T) -> Result<StableIndex, T> {
        if Some(self.len()) == self.fixed_capacity {
            return Err(value);
        }
        let i = self.back_idx().map_or(self.head, StableIndex::next_idx);
        self.inner.push_back(value);
        Ok(i)
    }

    pub fn pop_front(&mut self) -> Option<T> {
        let popped = self.inner.pop_front()?;
        self.head = self.head.next_idx();
        Some(popped)
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    pub fn get(&self, StableIndex(physical_idx): StableIndex) -> Option<&T> {
        let StableIndex(head) = self.head;
        if physical_idx < head {
            return None;
        }
        let ind = physical_idx - head;
        self.inner.get(ind)
    }

    pub fn get_mut(&mut self, StableIndex(physical_idx): StableIndex) -> Option<&mut T> {
        let StableIndex(head) = self.head;
        if physical_idx < head {
            return None;
        }
        let ind = physical_idx - head;
        self.inner.get_mut(ind)
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
        Some(StableIndex(head + (self.len() - 1)))
    }

    pub fn fixed_capacity(&self) -> Option<usize> {
        self.fixed_capacity
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
