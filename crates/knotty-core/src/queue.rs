//! The finite event queue and what goes in it.

/// Which clipboard a write is bound for.
///
/// The engine normalizes each protocol's own selectors onto these three
/// before a write reaches us, so they are all a write can name.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardTarget {
    /// The system clipboard.
    Standard = 0,
    /// The selection clipboard.
    Selection = 1,
    /// The primary selection.
    Primary = 2,
}

/// Something the child did whose happening is the whole of its meaning.
///
/// State goes in the snapshot, where only the newest value matters. What
/// lands here is what a consumer cannot read back off a screen, so missing it
/// is missing it for good. cf. `02-ffi.md`
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    /// The child rang the bell.
    Bell,
    /// The child asked for text to be put on a clipboard.
    ClipboardWrite {
        /// Which clipboard it is bound for.
        target: ClipboardTarget,
        /// What to put there.
        text: String,
    },
}

/// How many events may wait to be taken before further ones are dropped.
///
/// A consumer drains the whole queue on every wake, so a backlog this deep
/// means the child is producing faster than anything can act on. Dropping is
/// safe by construction: nothing a screen has to get right is in here.
const EVENT_QUEUE_CAP: usize = 64;

/// Events waiting for the app, a count of the ones that did not fit, and
/// whether anything has arrived since the session last asked.
#[derive(Debug, Default)]
pub struct EventQueue {
    events: Vec<Event>,
    dropped: u64,
    /// Whether anything reached the queue since this was last asked.
    arrived: bool,
}

impl EventQueue {
    /// Queue `event`, or count it dropped when the queue is at its cap.
    pub fn push(&mut self, event: Event) {
        self.arrived = true;
        if self.events.len() >= EVENT_QUEUE_CAP {
            self.dropped = self.dropped.saturating_add(1);
            return;
        }
        self.events.push(event);
    }

    /// Whether an event arrived since this was last asked.
    ///
    /// Asking clears it, the rule the writer queue's overrun flag already
    /// follows. A dropped event counts as an arrival: what this answers is
    /// whether there is something to come and look at, and a queue overflowing
    /// is that too.
    pub fn take_arrival(&mut self) -> bool {
        std::mem::take(&mut self.arrived)
    }

    /// Take everything queued, oldest first, along with how many events were
    /// dropped since the last take.
    ///
    /// The count empties with the queue, so one overrun is reported once
    /// rather than held against every later take — the rule the writer
    /// queue's overrun flag already follows.
    pub fn take(&mut self) -> (Vec<Event>, u64) {
        (
            std::mem::take(&mut self.events),
            std::mem::take(&mut self.dropped),
        )
    }
}
