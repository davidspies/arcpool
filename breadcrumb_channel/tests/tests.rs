use std::sync::atomic::{self, AtomicUsize};
use std::sync::Arc;

use breadcrumb_channel::{channel, TryRecvError};

#[derive(Debug)]
struct DropTracker {
    clone_count: Arc<AtomicUsize>,
}

impl DropTracker {
    fn new() -> Self {
        DropTracker {
            clone_count: Arc::new(AtomicUsize::new(1)),
        }
    }

    fn clone_count(&self) -> usize {
        self.clone_count.load(atomic::Ordering::SeqCst)
    }
}

impl Drop for DropTracker {
    fn drop(&mut self) {
        let count = self.clone_count.fetch_sub(1, atomic::Ordering::SeqCst);
        if count == 0 {
            panic!("DropTracker dropped with clone count 0");
        }
    }
}

impl Clone for DropTracker {
    fn clone(&self) -> Self {
        self.clone_count.fetch_add(1, atomic::Ordering::SeqCst);
        DropTracker {
            clone_count: self.clone_count.clone(),
        }
    }
}

#[tokio::test]
async fn test_send_and_recv() {
    let (sender, mut receiver) = channel();

    // Send a value through the channel
    assert_eq!(sender.send(42), Ok(()));

    // Receive the value from the channel
    assert_eq!(receiver.recv().await, Some(42));
}

#[tokio::test]
async fn test_multiple_receivers() {
    let (sender, mut receiver1) = channel();
    let mut receiver2 = sender.subscribe();

    // Send a value through the channel
    assert_eq!(sender.send("hello"), Ok(()));

    // Receive the value from the first receiver
    assert_eq!(receiver1.recv().await, Some("hello"));

    // Receive the value from the second receiver
    assert_eq!(receiver2.recv().await, Some("hello"));
}

#[tokio::test]
async fn test_receiver_disconnected() {
    let (sender, receiver) = channel::<&str>();

    // Drop the receiver
    drop(receiver);

    // Sending a value should return an error
    assert_eq!(sender.send("test"), Err("test"));
}

#[tokio::test]
async fn test_try_recv() {
    let (sender, mut receiver) = channel();

    // Sending a value
    assert_eq!(sender.send(42), Ok(()));

    // try_recv should return the value
    assert_eq!(receiver.try_recv(), Ok(42));

    // try_recv should return TryRecvError::Empty if the channel is empty
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));

    // Drop the sender
    drop(sender);

    // try_recv should return TryRecvError::Disconnected if the sender is dropped
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[tokio::test]
async fn test_sent_and_received_value_dropped() {
    let (sender, mut receiver) = channel();
    let tracker = DropTracker::new();

    // Send the DropTracker through the channel
    assert_eq!(sender.send(tracker.clone()).unwrap(), ());

    // Receive the DropTracker from the channel
    let received_tracker = receiver.recv().await.unwrap();
    drop(received_tracker);

    assert!(tracker.clone_count() == 1);
}

#[tokio::test]
async fn test_sent_and_not_received_value_dropped() {
    let (sender, receiver) = channel::<DropTracker>();
    let tracker = DropTracker::new();

    // Send the DropTracker through the channel
    assert_eq!(sender.send(tracker.clone()).unwrap(), ());

    // Drop the receiver without receiving the value
    drop(receiver);

    // Ensure the sent DropTracker is dropped
    assert!(tracker.clone_count() == 1);
}
