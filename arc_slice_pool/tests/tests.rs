use std::sync::atomic::{AtomicUsize, Ordering};

use arc_slice_pool::{Arc, ArcPool};

#[test]
fn arc_pool_alloc() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    assert_eq!(*arc, 42);
}

#[test]
fn arc_pool_alloc_multiple() {
    let pool = ArcPool::with_capacity(2);
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);
    assert_eq!(*arc1, 1);
    assert_eq!(*arc2, 2);
    assert_eq!(*arc3, 3);
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
fn arc_into_inner() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    let value = Arc::into_inner(arc);
    assert_eq!(value, Some(42));
}

#[test]
fn arc_try_unwrap() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    let result = Arc::try_unwrap(arc);
    assert_eq!(result, Ok(42));

    let arc = pool.alloc(42);
    let _arc_clone = arc.clone();
    let result = Arc::try_unwrap(arc);
    assert!(result.is_err());
}

#[test]
fn arc_into_index_and_from_index() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    let index = Arc::into_index(arc);
    let arc2 = unsafe { Arc::from_index(&pool, index) };
    assert_eq!(*arc2, 42);
}

#[test]
fn arc_clone_from_index() {
    let pool = ArcPool::new();
    let arc = pool.alloc(42);
    let index = Arc::into_index(arc);
    let arc2 = unsafe { Arc::clone_from_index(&pool, &index) };
    assert_eq!(*arc2, 42);
    unsafe { Arc::from_index(&pool, index) };
}

#[test]
fn arc_drop() {
    static DROP_COUNT: AtomicUsize = AtomicUsize::new(0);

    struct Droppable;

    impl Drop for Droppable {
        fn drop(&mut self) {
            DROP_COUNT.fetch_add(1, Ordering::SeqCst);
        }
    }

    let pool = ArcPool::new();
    {
        let _arc = pool.alloc(Droppable);
        assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 0);
    }
    assert_eq!(DROP_COUNT.load(Ordering::SeqCst), 1);
}

#[test]
fn arc_pool_thread_safety() {
    use std::sync::Arc as StdArc;
    use std::thread;

    let pool = StdArc::new(ArcPool::new());
    let threads: Vec<_> = (0..10)
        .map(|i| {
            let pool = StdArc::clone(&pool);
            thread::spawn(move || {
                let arc = pool.alloc(i);
                assert_eq!(*arc, i);
            })
        })
        .collect();

    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn arc_from_index_after_growth() {
    let pool = ArcPool::with_capacity(2); // Start with a small capacity

    // Create initial Arcs and save their indices
    let arc1 = pool.alloc(1);
    let index1 = Arc::into_index(arc1);
    let arc2 = pool.alloc(2);
    let index2 = Arc::into_index(arc2);

    // Force pool growth by allocating more Arcs than the initial capacity
    let _other_arcs = Vec::from_iter((3..10).map(|i| pool.alloc(i)));

    // Now try to retrieve the original Arcs using their old indices
    let retrieved_arc1 = unsafe { Arc::from_index(&pool, index1) };
    let retrieved_arc2 = unsafe { Arc::from_index(&pool, index2) };

    // Verify that the retrieved Arcs contain the correct values
    assert_eq!(*retrieved_arc1, 1);
    assert_eq!(*retrieved_arc2, 2);

    // Ensure that the retrieved Arcs are valid and can be cloned
    let cloned_arc1 = retrieved_arc1.clone();
    let cloned_arc2 = retrieved_arc2.clone();

    assert_eq!(*cloned_arc1, 1);
    assert_eq!(*cloned_arc2, 2);
}
