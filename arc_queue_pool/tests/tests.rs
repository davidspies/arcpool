use std::sync::atomic::Ordering;
use std::sync::{atomic::AtomicUsize, Arc as StdArc};
use std::thread;

use arc_queue_pool::{Arc, ArcPool};

#[test]
fn arc_pool_alloc() {
    let pool = ArcPool::new();
    let arc1 = pool.alloc(42);
    let arc2 = pool.alloc(84);

    assert_eq!(*arc1, 42);
    assert_eq!(*arc2, 84);
}

#[test]
fn arc_clone() {
    let pool = ArcPool::new();
    let arc1 = pool.alloc(42);
    let arc2 = arc1.clone();

    assert_eq!(*arc1, 42);
    assert_eq!(*arc2, 42);
}

#[test]
fn arc_drop() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    drop(arc);

    // Allocate a new value to potentially reuse the slot
    let arc2 = pool.alloc(84);
    assert_eq!(*arc2, 84);
}

#[test]
fn arc_next() {
    let pool = ArcPool::new();
    let arc1 = pool.alloc(1);
    let _arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);

    let next1 = Arc::next(&arc1).unwrap();
    let next2 = Arc::next(&next1).unwrap();

    assert_eq!(*next1, 2);
    assert_eq!(*next2, 3);
    assert!(Arc::next(&arc3).is_none());
}

#[test]
fn arc_into_inner() {
    let pool = ArcPool::new();
    let counter = DropCounter::new();
    {
        let arc = pool.alloc(counter.clone());
        let _inner = Arc::into_inner_and_next(arc).unwrap();
    } // _inner is dropped here
    assert_eq!(counter.count(), 1);
}

#[test]
fn multiple_queues() {
    let pool = ArcPool::with_capacity(2);
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3); // This should create a new queue

    assert_eq!(*arc1, 1);
    assert_eq!(*arc2, 2);
    assert_eq!(*arc3, 3);
}

#[test]
fn threaded_usage() {
    let pool = StdArc::new(ArcPool::new());
    let threads: Vec<_> = (0..10)
        .map(|i| {
            let pool = pool.clone();
            thread::spawn(move || {
                let arc = pool.alloc(i);
                assert_eq!(*arc, i);
                let (inner, _next) = Arc::into_inner_and_next(arc)?;
                Some(inner)
            })
        })
        .collect();

    for (i, thread) in threads.into_iter().enumerate() {
        let result = thread.join().unwrap();
        assert_eq!(result, Some(i as i32));
    }
}

#[test]
fn large_allocation() {
    let pool = ArcPool::new();
    let arcs: Vec<_> = (0..1000).map(|i| pool.alloc(i)).collect();

    for (i, arc) in arcs.iter().enumerate() {
        assert_eq!(**arc, i as i32);
    }

    // Test that we can still allocate after many allocations
    let new_arc = pool.alloc(1000);
    assert_eq!(*new_arc, 1000);
}

#[derive(Clone)]
struct DropCounter {
    count: StdArc<AtomicUsize>,
}

impl DropCounter {
    fn new() -> Self {
        DropCounter {
            count: StdArc::new(AtomicUsize::new(0)),
        }
    }

    fn count(&self) -> usize {
        self.count.load(Ordering::SeqCst)
    }
}

impl Drop for DropCounter {
    fn drop(&mut self) {
        self.count.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn single_arc_drop() {
    let pool = ArcPool::new();
    let counter = DropCounter::new();
    {
        let _arc = pool.alloc(counter.clone());
    } // _arc is dropped here
    assert_eq!(counter.count(), 1);
}

#[test]
fn multiple_arc_drops() {
    let pool = ArcPool::new();
    let counter = DropCounter::new();
    {
        let arc1 = pool.alloc(counter.clone());
        let arc2 = arc1.clone();
        assert!(Arc::next(&arc1).is_none());

        // Drop them in different orders
        drop(arc2);
        drop(arc1);
    }
    assert_eq!(counter.count(), 1);
}

#[test]
fn multiple_queues_drop() {
    let pool = ArcPool::with_capacity(2);
    let counter = DropCounter::new();
    {
        let arc1 = pool.alloc(counter.clone());
        let arc2 = pool.alloc(counter.clone());
        let arc3 = pool.alloc(counter.clone()); // This should create a new queue
        let arc4 = Arc::next(&arc1).unwrap();
        let arc5 = Arc::next(&arc2).unwrap();
        assert!(Arc::next(&arc3).is_none());

        // Drop in mixed order
        drop(arc2);
        drop(arc4);
        drop(arc1);
        drop(arc3);
        drop(arc5);
    }
    assert_eq!(counter.count(), 3);
}

#[test]
fn threaded_drops() {
    let pool = StdArc::new(ArcPool::new());
    let counter = StdArc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..10)
        .map(|_| {
            let pool = pool.clone();
            let counter = counter.clone();
            std::thread::spawn(move || {
                let dropper = DropCounter { count: counter };
                let arc = pool.alloc(dropper);
                let _arc2 = arc.clone();
                drop(arc);
                // _arc2 is dropped when the thread ends
            })
        })
        .collect();

    for thread in threads {
        thread.join().unwrap();
    }

    assert_eq!(counter.load(Ordering::SeqCst), 10);
}
