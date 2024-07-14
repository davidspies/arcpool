use std::sync::{Arc as StdArc, Barrier, Mutex};
use std::thread;

use bitvec::bitvec;

use arc_queue_pool::{Arc, ArcPool};

#[test]
fn test_arc_pool_creation() {
    let pool: ArcPool<i32> = ArcPool::new();
    assert!(Arc::ref_count(&pool.alloc(42)) == 1);

    let pool_with_capacity: ArcPool<String> = ArcPool::with_capacity(10);
    assert!(Arc::ref_count(&pool_with_capacity.alloc("test".to_string())) == 1);
}

#[test]
fn test_arc_allocation() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);

    assert_eq!(*arc1, 1);
    assert_eq!(*arc2, 2);
    assert_eq!(*arc3, 3);
}

#[test]
fn test_arc_cloning() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(42);
    let arc2 = arc1.clone();

    assert_eq!(Arc::ref_count(&arc1), 2);
    assert_eq!(Arc::ref_count(&arc2), 2);
}

#[test]
fn test_arc_next() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);

    assert_eq!(*Arc::next(&arc1).unwrap(), 2);
    assert_eq!(*Arc::next(&arc2).unwrap(), 3);
    assert!(Arc::next(&arc3).is_none());
}

#[test]
fn test_arc_into_inner_and_next() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let mut arc3 = pool.alloc(3);

    // This should fail because arc1 and arc2 still exist
    let result = Arc::into_inner_and_next(arc3);
    assert!(result.is_none());

    drop(arc1);
    arc3 = pool.alloc(4); // Reallocate arc3
                          // This should still fail because arc2 exists
    let result = Arc::into_inner_and_next(arc3);
    assert!(result.is_none());

    drop(arc2);
    arc3 = pool.alloc(5); // Reallocate arc3 again
                          // Now this should succeed
    let (value, next) = Arc::into_inner_and_next(arc3).unwrap();
    assert_eq!(value, 5);
    assert!(next.is_none());
}

#[test]
fn test_arc_try_unwrap_and_next() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);
    let arc3_clone = arc3.clone();

    // This should fail because arc3 has multiple references
    assert!(Arc::try_unwrap_and_next(arc3).is_err());

    drop(arc3_clone);
    let arc3 = pool.alloc(3); // Reallocate arc3
                              // This should still fail because arc1 and arc2 exist
    assert!(Arc::try_unwrap_and_next(arc3).is_err());

    drop(arc1);
    drop(arc2);
    let arc3 = pool.alloc(3); // Reallocate arc3 again
                              // Now this should succeed
    let (value, next) = Arc::try_unwrap_and_next(arc3).unwrap();
    assert_eq!(value, 3);
    assert!(next.is_none());
}

#[test]
fn test_arc_ref_count() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc = pool.alloc(42);

    assert_eq!(Arc::ref_count(&arc), 1);

    let arc_clone = arc.clone();
    assert_eq!(Arc::ref_count(&arc), 2);
    assert_eq!(Arc::ref_count(&arc_clone), 2);

    drop(arc_clone);
    assert_eq!(Arc::ref_count(&arc), 1);
}

#[test]
fn test_arc_deref() {
    let pool: ArcPool<String> = ArcPool::new();
    let arc = pool.alloc("Hello, world!".to_string());

    assert_eq!(arc.len(), 13);
    assert_eq!(&*arc, "Hello, world!");
}

#[test]
fn test_complex_scenario() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);
    let arc4 = pool.alloc(4);

    assert_eq!(*Arc::next(&arc1).unwrap(), 2);
    assert_eq!(*Arc::next(&arc2).unwrap(), 3);
    assert_eq!(*Arc::next(&arc3).unwrap(), 4);
    assert!(Arc::next(&arc4).is_none());

    drop(arc1);
    drop(arc2);

    let (value, next) = Arc::into_inner_and_next(arc3).unwrap();
    assert_eq!(value, 3);
    assert_eq!(**next.as_ref().unwrap(), 4);

    // arc4 should now be the only one left
    assert!(Arc::next(&arc4).is_none());
    drop(next);
    let (value, next) = Arc::into_inner_and_next(arc4).unwrap();
    assert_eq!(value, 4);
    assert!(next.is_none());
}

#[test]
fn test_arc_try_unwrap_and_next_with_next() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);

    // Drop arc1 to allow unwrapping of arc2
    drop(arc1);

    // This should succeed and return Some for next
    match Arc::try_unwrap_and_next(arc2) {
        Ok((value, next)) => {
            assert_eq!(value, 2);
            assert!(next.is_some());
            let next_arc = next.unwrap();
            assert_eq!(*next_arc, 3);
        }
        Err(_) => panic!("Expected Ok, got Err"),
    }

    // arc3 should still be valid and the last in the queue
    assert!(Arc::next(&arc3).is_none());
}

#[test]
fn test_arc_into_inner_and_next_with_clone() {
    let pool: ArcPool<i32> = ArcPool::new();
    let arc1 = pool.alloc(1);
    let arc2 = pool.alloc(2);
    let arc3 = pool.alloc(3);

    // Create a clone of arc2
    let arc2_clone = arc2.clone();

    // Drop arc1 to ensure it's not preventing arc2 from being unwrapped
    drop(arc1);

    // Attempt to unwrap arc2
    let result = Arc::into_inner_and_next(arc2);

    // This should return None because arc2_clone still exists
    assert!(result.is_none());

    // Verify that arc2_clone and arc3 are still valid
    assert_eq!(*arc2_clone, 2);
    assert_eq!(*arc3, 3);

    // Verify that the next relationship is still intact
    assert_eq!(*Arc::next(&arc2_clone).unwrap(), 3);
}

#[test]
fn stress_test_arc_into_inner_and_next() {
    const NUM_ARCS: usize = 100;
    const NUM_THREADS: usize = 1000;

    let pool: ArcPool<usize> = ArcPool::new();
    let arcs: Vec<_> = (0..NUM_ARCS).map(|i| pool.alloc(i)).collect();

    let barrier = StdArc::new(Barrier::new(NUM_THREADS + 1));
    let visited = StdArc::new(Mutex::new(bitvec![0; NUM_ARCS]));

    let threads: Vec<_> = (0..NUM_THREADS)
        .map(|i| {
            let mut current_arc = arcs[i % NUM_ARCS].clone();
            let barrier = barrier.clone();
            let visited = visited.clone();

            thread::spawn(move || {
                barrier.wait();

                loop {
                    match Arc::into_inner_and_next(current_arc) {
                        Some((value, next)) => {
                            let mut visited = visited.lock().unwrap();
                            assert!(!visited[value]);
                            visited.set(value, true);
                            if let Some(next_arc) = next {
                                current_arc = next_arc;
                            } else {
                                break;
                            }
                        }
                        None => break,
                    }
                    thread::yield_now();
                }
            })
        })
        .collect();

    drop(arcs);

    barrier.wait();

    for thread in threads {
        thread.join().unwrap();
    }

    let final_visited = visited.lock().unwrap();
    assert_eq!(final_visited.count_ones(), NUM_ARCS);
}
