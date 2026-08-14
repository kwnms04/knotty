//! Single-slot latest-wins mailbox.

use std::sync::Mutex;

/// A one-slot handoff between a producer and a consumer.
///
/// Publishing over an unconsumed value discards it, so the slot always holds
/// the newest value and never grows. Consumption is destructive: once taken,
/// the slot is empty until the next publish.
#[derive(Debug)]
pub struct Mailbox<T>(Mutex<Option<T>>);

impl<T> Mailbox<T> {
    /// Create an empty mailbox.
    #[must_use]
    pub fn new() -> Self {
        Self(Mutex::new(None))
    }

    /// Put `value` in the slot, dropping whatever was there.
    pub fn publish(&self, value: T) {
        *self.0.lock().expect("mailbox lock") = Some(value);
    }

    /// Empty the slot, returning its value if one was published since the
    /// last take.
    pub fn take(&self) -> Option<T> {
        self.0.lock().expect("mailbox lock").take()
    }
}

impl<T> Default for Mailbox<T> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::Mailbox;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    /// A payload that reports its own destruction, so the test can tell
    /// "discarded" apart from "leaked".
    struct Tracked {
        seq: usize,
        dropped: Arc<AtomicUsize>,
    }

    impl Drop for Tracked {
        fn drop(&mut self) {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    #[test]
    fn publish_over_an_unconsumed_value_discards_it() {
        let mailbox = Mailbox::new();

        mailbox.publish(1);
        mailbox.publish(2);

        assert_eq!(mailbox.take(), Some(2));
        assert_eq!(mailbox.take(), None);
    }

    #[test]
    fn concurrent_producer_and_consumer_never_see_a_stale_value() {
        const PUBLISHED: usize = 10_000;

        let mailbox = Arc::new(Mailbox::new());
        let dropped = Arc::new(AtomicUsize::new(0));

        let producer = thread::spawn({
            let mailbox = Arc::clone(&mailbox);
            let dropped = Arc::clone(&dropped);
            move || {
                for seq in 0..PUBLISHED {
                    mailbox.publish(Tracked {
                        seq,
                        dropped: Arc::clone(&dropped),
                    });
                }
            }
        });

        let mut last: Option<usize> = None;
        loop {
            // Read "finished" before draining: if the producer was already
            // done, the drain below cannot miss a later publish.
            let finished = producer.is_finished();
            while let Some(value) = mailbox.take() {
                assert!(
                    last.is_none_or(|previous| value.seq > previous),
                    "took {} after {last:?} — the slot handed back a stale value",
                    value.seq,
                );
                last = Some(value.seq);
            }
            if finished {
                break;
            }
        }
        producer.join().unwrap();

        assert_eq!(last, Some(PUBLISHED - 1), "the final publish was lost");
        assert_eq!(
            dropped.load(Ordering::Relaxed),
            PUBLISHED,
            "overwritten values must be dropped, not leaked",
        );
    }
}
