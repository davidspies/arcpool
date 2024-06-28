use stable_queue::StableQueue;

#[test]
fn new_unbounded() {
    let queue: StableQueue<i32> = StableQueue::new_unbounded();
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.fixed_capacity(), None);
}

#[test]
fn with_fixed_capacity() {
    let queue: StableQueue<i32> = StableQueue::with_fixed_capacity(5);
    assert!(queue.is_empty());
    assert_eq!(queue.len(), 0);
    assert_eq!(queue.fixed_capacity(), Some(5));
}

#[test]
fn push_back_and_front() {
    let mut queue = StableQueue::new_unbounded();
    let idx1 = queue.push_back(1).unwrap();
    let idx2 = queue.push_back(2).unwrap();

    assert_eq!(queue.len(), 2);
    assert_eq!(queue.front(), Some(&1));
    assert_eq!(queue.back(), Some(&2));
    assert_eq!(queue[idx1], 1);
    assert_eq!(queue[idx2], 2);
}

#[test]
fn pop_front() {
    let mut queue = StableQueue::new_unbounded();
    assert!(queue.pop_front().is_none());

    queue.push_back(1).unwrap();
    queue.push_back(2).unwrap();

    assert_eq!(queue.pop_front(), Some(1));
    assert_eq!(queue.len(), 1);
    assert_eq!(queue.front(), Some(&2));
}

#[test]
fn fixed_capacity_limit() {
    let mut queue = StableQueue::with_fixed_capacity(2);
    assert!(matches!(queue.push_back(1), Ok(_)));
    assert!(matches!(queue.push_back(2), Ok(_)));
    assert!(matches!(queue.push_back(3), Err(_)));
    assert_eq!(queue.len(), 2);
}

#[test]
fn front_and_back_idx() {
    let mut queue = StableQueue::new_unbounded();
    assert_eq!(queue.front_idx(), None);
    assert_eq!(queue.back_idx(), None);

    queue.push_back(1).unwrap();
    queue.push_back(2).unwrap();

    assert_eq!(queue[queue.front_idx().unwrap()], 1);
    assert_eq!(queue[queue.back_idx().unwrap()], 2);

    queue.pop_front();
    queue.push_back(3).unwrap();

    assert_eq!(queue[queue.front_idx().unwrap()], 2);
    assert_eq!(queue[queue.back_idx().unwrap()], 3);
}

#[test]
fn get_and_get_mut() {
    let mut queue = StableQueue::new_unbounded();
    let idx0 = queue.push_back(0).unwrap();
    let idx1 = queue.push_back(1).unwrap();
    let idx2 = queue.push_back(2).unwrap();
    queue.pop_front();

    assert_eq!(queue.get(idx1), Some(&1));
    assert_eq!(queue.get(idx2), Some(&2));
    assert_eq!(queue.get(idx0), None);

    *queue.get_mut(idx1).unwrap() = 10;
    assert_eq!(queue.get_mut(idx0), None);

    assert_eq!(queue.get(idx1), Some(&10));
}

#[test]
fn index_and_index_mut() {
    let mut queue = StableQueue::new_unbounded();
    let idx1 = queue.push_back(1).unwrap();
    let idx2 = queue.push_back(2).unwrap();

    assert_eq!(queue[idx1], 1);
    assert_eq!(queue[idx2], 2);

    queue[idx1] = 10;
    assert_eq!(queue[idx1], 10);
}

#[test]
#[should_panic]
fn index_out_of_bounds() {
    let mut queue = StableQueue::new_unbounded();
    let idx1 = queue.push_back(1).unwrap();
    queue.push_back(2).unwrap();
    queue.pop_front();
    let _ = queue[idx1]; // This should panic
}

#[test]
fn front_mut_and_back_mut() {
    let mut queue = StableQueue::new_unbounded();
    queue.push_back(1).unwrap();
    queue.push_back(2).unwrap();

    if let Some(front) = queue.front_mut() {
        *front = 10;
    }
    if let Some(back) = queue.back_mut() {
        *back = 20;
    }

    assert_eq!(queue.front(), Some(&10));
    assert_eq!(queue.back(), Some(&20));
}
