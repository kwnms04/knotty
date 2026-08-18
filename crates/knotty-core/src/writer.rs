//! The bytes bound for the child, and the cap on how many may wait.
//!
//! One queue, whoever queued into it: what the app wrote and what the terminal
//! answered. Both ends of a PTY session reach it — the app's thread pushes and
//! the I/O thread hands over — so the cap is checked where the bytes live
//! rather than beside each caller. cf. `03-core.md` C1
//!
//! A detached session has no I/O thread, and takes the queue back by asking
//! for it. cf. `03-core.md` C7

use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::Result;

/// How many bytes may wait for the child before further writes are dropped.
///
/// A child that never reads is the case this exists for: without a cap the
/// queue grows until the process dies. What it promises is that bound and not
/// a refusal. cf. `docs/adr/0013-one-writer-queue.md`
pub(crate) const CAP: usize = 8 * 1024 * 1024;

/// Bytes on their way to the child.
///
/// Every write the app makes and every answer the terminal gives lands here,
/// so nothing waits on a PTY that may not be ready — or, in a detached
/// session, that does not exist.
#[derive(Debug, Default)]
pub struct WriteQueue {
    queued: Mutex<Queued>,
    /// How many bytes are waiting, readable without taking the lock.
    ///
    /// A mirror of the buffer's own length rather than a count of its own:
    /// it is set from that length inside every call that changes it, so the
    /// two cannot drift. It exists because whoever watches the queue drain
    /// watches it in a spin — and a watcher that took the lock to look would
    /// be holding up the very hand-over it is waiting for.
    waiting: AtomicUsize,
}

/// The queue itself and whether anything has been dropped from it.
#[derive(Debug, Default)]
struct Queued {
    bytes: Vec<u8>,
    /// Whether bytes were dropped for want of room.
    overran: bool,
}

impl WriteQueue {
    /// Append `bytes`, or report that there was no room for them.
    pub fn try_push(&self, bytes: &[u8]) -> bool {
        self.append(&mut self.lock(), bytes)
    }

    /// The same for what the terminal answers, which has no caller standing
    /// by to be told: the drop is remembered instead.
    ///
    /// Refusing and remembering happen under the one lock, so the queue is
    /// never seen having dropped something without saying so.
    pub fn push(&self, bytes: &[u8]) {
        let mut queued = self.lock();
        if !self.append(&mut queued, bytes) {
            queued.overran = true;
        }
    }

    /// Put `bytes` on the end of a queue already locked, or answer that they
    /// did not fit.
    ///
    /// Nothing is queued when they do not fit: a prefix of what the user
    /// typed reaching the child is worse than none of it.
    fn append(&self, queued: &mut Queued, bytes: &[u8]) -> bool {
        if queued.bytes.len() + bytes.len() > CAP {
            return false;
        }
        queued.bytes.extend_from_slice(bytes);
        self.waiting.store(queued.bytes.len(), Ordering::Relaxed);
        true
    }

    /// Whether bytes have been dropped since this was last asked.
    ///
    /// Asking clears it, so one overrun is reported once rather than held
    /// against every later call.
    pub fn take_overrun(&self) -> bool {
        std::mem::take(&mut self.lock().overran)
    }

    /// How many bytes are still waiting for the child.
    pub fn waiting(&self) -> usize {
        self.waiting.load(Ordering::Relaxed)
    }

    /// Hand what is queued to `take_some`, and drop as much of it as was
    /// taken.
    ///
    /// The bytes are lent rather than copied out, so the queue is locked for
    /// the length of the call — which is what keeps the order right: what the
    /// app queues while a hand-over is in flight lands behind what is being
    /// handed over, not in front of it. The call is expected to be a
    /// non-blocking write and nothing longer.
    ///
    /// # Errors
    ///
    /// Whatever `take_some` reports, in which case nothing was dropped.
    pub fn drain_with(&self, take_some: impl FnOnce(&[u8]) -> Result<usize>) -> Result<()> {
        let mut queued = self.lock();
        if queued.bytes.is_empty() {
            return Ok(());
        }

        let taken = take_some(&queued.bytes)?;
        queued.bytes.drain(..taken);
        self.waiting.store(queued.bytes.len(), Ordering::Relaxed);
        Ok(())
    }

    /// Take everything queued, emptying the queue.
    ///
    /// For a session with no I/O thread to drain it, where asking is the
    /// whole of the drain. cf. `03-core.md` C7
    pub fn take(&self) -> Vec<u8> {
        let mut queued = self.lock();
        let taken = std::mem::take(&mut queued.bytes);
        self.waiting.store(queued.bytes.len(), Ordering::Relaxed);
        taken
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Queued> {
        self.queued.lock().expect("writer queue lock")
    }
}

#[cfg(test)]
mod tests {
    use super::{CAP, WriteQueue};
    use crate::Error;

    #[test]
    fn a_write_that_does_not_fit_is_refused_whole() {
        let queue = WriteQueue::default();
        assert!(queue.try_push(&vec![b'x'; CAP - 1]));

        assert!(!queue.try_push(b"two"), "two bytes fit in one byte of room");

        assert_eq!(
            queue.waiting(),
            CAP - 1,
            "a refused write was queued anyway"
        );
    }

    /// The terminal's answers have no caller to refuse: the drop is what is
    /// reported, and once.
    #[test]
    fn an_answer_that_does_not_fit_is_reported_once() {
        let queue = WriteQueue::default();
        assert!(queue.try_push(&vec![b'x'; CAP]));

        queue.push(b"answer");

        assert!(queue.take_overrun(), "the overrun went unrecorded");
        assert!(!queue.take_overrun(), "the overrun was reported twice");
    }

    /// The terminal takes what it has room for and no more, so the rest has to
    /// wait where it is — in front of whatever is queued next.
    #[test]
    fn what_a_hand_over_left_behind_stays_in_front_of_what_comes_after() {
        let queue = WriteQueue::default();
        queue.try_push(b"first");

        queue
            .drain_with(|bytes| {
                assert_eq!(bytes, b"first");
                Ok(2)
            })
            .expect("the hand-over to finish");
        queue.try_push(b"second");

        assert_eq!(queue.waiting(), b"rstsecond".len());
        assert_eq!(queue.take(), b"rstsecond");
    }

    /// A hand-over that failed handed nothing over, and what it was given has
    /// to still be there for the round that follows.
    #[test]
    fn a_hand_over_that_failed_drops_nothing() {
        let queue = WriteQueue::default();
        queue.try_push(b"typed");

        let refused = queue.drain_with(|_| Err(Error::Io));

        assert_eq!(refused, Err(Error::Io));
        assert_eq!(queue.waiting(), b"typed".len());
        assert_eq!(queue.take(), b"typed");
    }

    /// The count is what the queue is watched by, so it has to follow every
    /// way the queue can change — including the take that empties it.
    #[test]
    fn the_count_follows_the_queue() {
        let queue = WriteQueue::default();
        assert_eq!(queue.waiting(), 0);

        queue.try_push(b"typed");
        assert_eq!(queue.waiting(), b"typed".len());

        queue.take();
        assert_eq!(queue.waiting(), 0);
    }
}
