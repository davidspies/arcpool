use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc as StdArc;

use arc_slice_pool::Arc;
use breadcrumb_channel::{arc_channel, TryRecvError};
use tokio::time::{sleep, Duration};

#[tokio::test]
async fn test_arc_channel_creation() {
    let (sender, mut receiver) = arc_channel::<i32>();
    assert!(sender.send(42).is_ok());
    assert_eq!(receiver.recv().await.map(|arc| *arc), Some(42));
}

#[tokio::test]
async fn test_multiple_receivers() {
    let (sender, mut receiver1) = arc_channel::<String>();
    let mut receiver2 = sender.subscribe();

    sender.send("Hello".to_string()).unwrap();

    assert_eq!(*receiver1.recv().await.unwrap(), "Hello");
    assert_eq!(*receiver2.recv().await.unwrap(), "Hello");
}

#[tokio::test]
async fn test_try_recv_empty() {
    let (_sender, mut receiver) = arc_channel::<i32>();
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));
}

#[tokio::test]
async fn test_try_recv_disconnected() {
    let (sender, mut receiver) = arc_channel::<i32>();
    drop(sender);
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Disconnected));
}

#[tokio::test]
async fn test_sender_closed() {
    let (sender, receiver) = arc_channel::<i32>();

    tokio::spawn(async move {
        sleep(Duration::from_millis(100)).await;
        drop(receiver);
    });

    sender.closed().await;
}

#[tokio::test]
async fn test_lazy_clone() {
    let (sender, mut receiver1) = arc_channel::<Vec<i32>>();
    let mut receiver2 = sender.subscribe();

    sender.send(vec![1, 2, 3]).unwrap();

    let arc1 = receiver1.recv().await.unwrap();
    let arc2 = receiver2.recv().await.unwrap();

    assert!(Arc::ptr_eq(&arc1, &arc2));
}

#[tokio::test]
async fn test_send_after_receiver_dropped() {
    let (sender, receiver) = arc_channel::<i32>();
    drop(receiver);
    assert!(sender.send(42).is_err());
}

#[tokio::test]
async fn test_multiple_sends() {
    let (sender, mut receiver) = arc_channel::<i32>();

    for i in 0..10 {
        sender.send(i).unwrap();
    }

    for i in 0..10 {
        assert_eq!(*receiver.recv().await.unwrap(), i);
    }
}

#[tokio::test]
async fn test_receiver_recv_after_sender_dropped() {
    let (sender, mut receiver) = arc_channel::<i32>();
    sender.send(42).unwrap();
    drop(sender);

    assert_eq!(*receiver.recv().await.unwrap(), 42);
    assert!(receiver.recv().await.is_none());
}

#[tokio::test]
async fn test_recv_await() {
    let (sender, mut receiver) = arc_channel::<String>();

    let send_task = tokio::spawn(async move {
        sleep(Duration::from_millis(100)).await;
        sender.send("Delayed message".to_string()).unwrap();
    });

    let receive_task = tokio::spawn(async move {
        let start = std::time::Instant::now();
        let received = receiver.recv().await.unwrap();
        let elapsed = start.elapsed();
        (received, elapsed)
    });

    let (send_result, receive_result) = tokio::join!(send_task, receive_task);
    send_result.unwrap();
    let (received, elapsed) = receive_result.unwrap();

    assert_eq!(*received, "Delayed message");
    assert!(elapsed >= Duration::from_millis(100));
}

#[derive(Debug)]
struct Trackable {
    id: usize,
    drop_counter: StdArc<AtomicUsize>,
}

impl Trackable {
    fn new(id: usize, drop_counter: StdArc<AtomicUsize>) -> Self {
        Self { id, drop_counter }
    }
}

impl Drop for Trackable {
    fn drop(&mut self) {
        self.drop_counter.fetch_add(1, Ordering::SeqCst);
    }
}

#[tokio::test]
async fn test_message_drop_on_lagging_receiver_removal() {
    let drop_counter = StdArc::new(AtomicUsize::new(0));
    let (sender, receiver1) = arc_channel::<Trackable>();
    let mut receiver2 = sender.subscribe();

    // Send 10 messages
    for i in 0..10 {
        sender
            .send(Trackable::new(i, drop_counter.clone()))
            .unwrap();
    }

    // Receive 5 messages from receiver2, making receiver1 lag behind
    for i in 0..5 {
        assert_eq!(receiver2.recv().await.unwrap().id, i);
    }

    // At this point, no messages should have been dropped
    assert_eq!(drop_counter.load(Ordering::SeqCst), 0);

    // Drop the lagging receiver
    drop(receiver1);

    // Send one more message to trigger cleanup
    sender
        .send(Trackable::new(10, drop_counter.clone()))
        .unwrap();

    // Verify that the first 5 messages have been dropped
    assert_eq!(drop_counter.load(Ordering::SeqCst), 5);

    // Receive the remaining messages from receiver2
    for i in 5..11 {
        assert_eq!(receiver2.recv().await.unwrap().id, i);
    }

    // Drop everything and verify all messages are dropped
    drop(receiver2);

    assert_eq!(drop_counter.load(Ordering::SeqCst), 11);
}

#[tokio::test]
async fn test_last_receiver_dropped_after_sender_caught_up() {
    let drop_counter = StdArc::new(AtomicUsize::new(0));
    let (sender, mut receiver) = arc_channel::<Trackable>();

    // Send 5 messages
    for i in 0..5 {
        sender
            .send(Trackable::new(i, drop_counter.clone()))
            .unwrap();
    }

    // Verify no messages have been dropped yet
    assert_eq!(drop_counter.load(Ordering::SeqCst), 0);

    // Receive all messages
    for i in 0..5 {
        assert_eq!(receiver.recv().await.unwrap().id, i);
    }

    // Verify all messages have been dropped
    assert_eq!(drop_counter.load(Ordering::SeqCst), 5);

    // Drop the sender
    drop(sender);

    // Drop the receiver
    drop(receiver);

    // Verify no double-drops occurred
    assert_eq!(drop_counter.load(Ordering::SeqCst), 5);
}

#[tokio::test]
async fn test_last_receiver_dropped_after_sender_not_caught_up() {
    let drop_counter = StdArc::new(AtomicUsize::new(0));
    let (sender, mut receiver) = arc_channel::<Trackable>();

    // Send 10 messages
    for i in 0..10 {
        sender
            .send(Trackable::new(i, drop_counter.clone()))
            .unwrap();
    }

    // Receive only 5 messages, leaving the receiver not caught up
    for i in 0..5 {
        assert_eq!(receiver.recv().await.unwrap().id, i);
    }

    // Drop the sender
    drop(sender);

    // Verify only the received messages have been dropped
    assert_eq!(drop_counter.load(Ordering::SeqCst), 5);

    // Drop the receiver
    drop(receiver);

    // Verify all messages have been dropped
    assert_eq!(drop_counter.load(Ordering::SeqCst), 10);
}

#[tokio::test]
async fn test_try_recv_success() {
    let (sender, mut receiver) = arc_channel::<i32>();

    // Send a value
    sender.send(42).unwrap();

    // Use try_recv to get the value immediately
    assert_eq!(*receiver.try_recv().unwrap(), 42);

    // Verify that the channel is now empty
    assert_eq!(receiver.try_recv(), Err(TryRecvError::Empty));

    // Send another value
    sender.send(100).unwrap();

    // Verify that we can still receive normally
    assert_eq!(*receiver.recv().await.unwrap(), 100);
}

#[tokio::test]
async fn test_try_recv_with_lagging_receiver() {
    let (sender, mut receiver1) = arc_channel::<i32>();
    let mut receiver2 = receiver1.clone();

    // Send a value
    sender.send(42).unwrap();

    // Use try_recv on receiver1
    assert_eq!(*receiver1.try_recv().unwrap(), 42);


    // Send another value
    sender.send(100).unwrap();

    // Use try_recv on receiver1 again
    assert_eq!(*receiver1.try_recv().unwrap(), 100);

    // Verify that receiver2 can still receive the values
    assert_eq!(*receiver2.recv().await.unwrap(), 42);
    assert_eq!(*receiver2.recv().await.unwrap(), 100);

    // Verify that both receivers now see the channel as empty
    assert!(matches!(receiver1.try_recv(), Err(TryRecvError::Empty)));
    assert!(matches!(receiver2.try_recv(), Err(TryRecvError::Empty)));
}
