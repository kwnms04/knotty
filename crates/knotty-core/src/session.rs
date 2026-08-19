//! Session lifecycle and the publish path.

use std::panic::{self, AssertUnwindSafe};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::io::{self, Pty, Waker};
use crate::listener::Listener;
use crate::mailbox::Mailbox;
use crate::queue::{Event, EventQueue};
use crate::snapshot::{ScreenState, Snapshot};
use crate::vt::Terminal;
use crate::wake::{Debt, Wake};
use crate::writer::WriteQueue;
use crate::{Error, Result};

/// A selection's two endpoints, in viewport coordinates.
///
/// Both ends are inclusive, and either may come first: the pair records which
/// way the selection was made, not which end is topmost.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRange {
    /// Column of the first endpoint.
    pub start_x: u16,
    /// Row of the first endpoint.
    pub start_y: u16,
    /// Column of the second endpoint.
    pub end_x: u16,
    /// Row of the second endpoint.
    pub end_y: u16,
    /// Whether the endpoints are opposite corners of a block rather than the
    /// ends of a run of text.
    pub rectangle: bool,
}

/// What has become of a session's child.
///
/// Kept apart from whether the session itself still works. The two are
/// different facts, and a child still running behind a session whose thread
/// panicked is a real pairing — an app warns before closing on this one and
/// stops taking input on the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChildState {
    /// There is no child. A detached session is fed by its caller and has
    /// none.
    None,
    /// Still running.
    Running,
    /// Gone, with what it ended by — or 128 plus the signal that ended it,
    /// which is the one number a shell reports either by.
    Exited(i32),
}

/// What the shared exit cell reads as while the child is still running.
///
/// A code is a byte, or 128 plus a signal number, so nothing negative is one.
const STILL_RUNNING: i32 = -1;

/// How long a synchronized output block may hold a wake back before it is
/// given up on.
///
/// Generous on purpose, because the wrong firing is the common one: a block
/// crossing a slow link takes its time and is doing nothing wrong, and a
/// timeout short enough to catch it tears an honest screen every time it
/// fires. The program that really leaves a block open is rare, and costs one
/// tear once. Not a setting — an app has nothing to go on that would make it a
/// better answer than this one.
const SYNC_TIMEOUT: Duration = Duration::from_secs(1);

/// A terminal session.
///
/// It owns no thread and no child process of its own: [`feed`] runs the VT
/// engine on the calling thread. A detached session is one used directly; a
/// [`PtySession`] is this same session with a thread and a child around it,
/// and everything past the parser is the same code either way.
///
/// [`feed`]: Session::feed
pub struct Session {
    terminal: Terminal,
    // Shared rather than owned outright: a PTY session's consumer takes from
    // this on its own thread while the I/O thread publishes into it. The
    // mailbox is the only thing here that crosses, which is what adr/0003
    // bought.
    mailbox: Arc<Mailbox<Snapshot>>,
    // What the last capture said about the screen outside the grid, so that a
    // title or cursor change on an otherwise still screen still publishes.
    last_screen: ScreenState,
    // Shared with the engine callback, which outlives any single call and so
    // cannot borrow the session — and, in a PTY session, with the app, which
    // queues into it from its own thread. That second sharer is why the queue
    // carries a lock of its own.
    writes: Arc<WriteQueue>,
    // What the last drain handed out. Kept alive here because the boundary
    // lends the bytes rather than copying them.
    drained: Vec<u8>,
    // Shared with the engine callbacks, for the same reason the writer queue
    // is — and with the app, which is what drains it whether or not a PTY is
    // behind the session.
    events: Arc<Mutex<EventQueue>>,
    // How the consumer is told to come and look. Not the engine's business:
    // when a frame gets drawn is between the session and whoever draws.
    // Shared, because a PTY session's consumer registers from its own thread
    // while this one pays from the I/O thread.
    wake: Arc<Debt>,
    // News this round produced that the consumer has not been told of, while a
    // synchronized output block is holding it back. It waits here rather than
    // on the debt: what the debt holds is payable the moment a consumer turns
    // up, and this is not — the block is what says so, and only this side can
    // ask. cf. `03-core.md` C5
    held_back: bool,
    // When the owed wake was first held back by an open synchronized output
    // block, or `None` when nothing is being held back. This is the clock the
    // timeout runs on, and it is read by whoever waits — which is the I/O
    // thread. A detached session has no such waiter, so its blocks are never
    // given up on. cf. `03-core.md` C5
    held_since: Option<Instant>,
    // Whether the block now open has been given up on. While it stands nothing
    // is held back; closing the block clears it.
    given_up_on_block: bool,
}

impl Session {
    /// Create a session, its engine, and its queues.
    ///
    /// The engine's handles are single-threaded, so whatever thread calls this
    /// is the only one that may drive the session afterwards.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self> {
        let writes = Arc::new(WriteQueue::default());
        // What the app has to be told rather than shown. Neither a bell nor a
        // clipboard write leaves a mark on the screen, so a consumer that
        // misses one has no second way to learn it happened.
        let events = Arc::new(Mutex::new(EventQueue::default()));
        let listener = Listener::new(Arc::clone(&writes), Arc::clone(&events));

        Ok(Self {
            terminal: Terminal::new(cols, rows, max_scrollback, listener)?,
            mailbox: Arc::new(Mailbox::new()),
            last_screen: ScreenState::default(),
            writes,
            drained: Vec::new(),
            events,
            wake: Arc::new(Debt::default()),
            held_back: false,
            held_since: None,
            given_up_on_block: false,
        })
    }

    /// Set what to call when the session has something new to be taken, or
    /// clear it with `None`.
    ///
    /// The call is made on the thread that drove the session, from inside the
    /// call that published — so it may do nothing but wake its own thread.
    /// Re-entering the session from it would re-enter state the running call
    /// still holds. A wake that fell due while nobody was registered is paid
    /// here, before this returns.
    pub fn set_wake(&mut self, wake: Option<Wake>) {
        self.wake.register(wake);
    }

    /// Queue `bytes` for the child, without waiting for them to get there.
    ///
    /// # Errors
    ///
    /// [`Error::WriteQueueFull`] when they did not fit, in which case none of
    /// them were queued: a prefix of what the user typed reaching the child is
    /// worse than none of it.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if self.writes.try_push(bytes) {
            return Ok(());
        }
        Err(Error::WriteQueueFull)
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// Publishes a snapshot: the selection is part of what a consumer draws.
    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> Result<()> {
        self.terminal.set_selection(range)?;
        self.publish(false)
    }

    /// Process `bytes` to completion on the calling thread, publishing at most
    /// one snapshot.
    ///
    /// # Errors
    ///
    /// [`Error::WriteQueueFull`] when the terminal's answers did not fit in
    /// the writer queue. The screen is published either way: what the child
    /// missed hearing does not make the frame wrong.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.terminal.feed(bytes);

        // Read before publishing, which can return early: an overrun left
        // standing would surface on some later feed that overran nothing.
        let overran = self.writes.take_overrun();
        self.publish(false)?;

        if overran {
            return Err(Error::WriteQueueFull);
        }
        Ok(())
    }

    /// Queue the child's end for the app and publish.
    ///
    /// The caller is whoever read the terminal to its end, which is what makes
    /// the order right: everything the child printed went through [`feed`]
    /// before this. Publishing after the event is queued is what carries the
    /// wake — a consumer that came for the frame drains the event beside it.
    ///
    /// The wake is paid even inside a synchronized output block, which is the
    /// one place that rule bends. A block the child left open can never close,
    /// and no round follows this one to carry what was held back — so
    /// suppressing here would leave the news of the exit in a queue nobody was
    /// told to come for. cf. `03-core.md` C5
    ///
    /// The frame goes out whether or not the screen moved, for the same reason:
    /// a child that ends without printing leaves the grid exactly as it was,
    /// and the snapshot is where the exit is the truth a consumer may not lose.
    /// Without a frame to carry it, the last one anybody holds goes on saying
    /// the child is running.
    ///
    /// [`feed`]: Session::feed
    pub(crate) fn note_child_exit(&mut self, code: i32) -> Result<()> {
        self.events
            .lock()
            .expect("event queue lock")
            .push(Event::ChildExited { code });
        self.publish(true)?;
        self.pay_wake();
        Ok(())
    }

    /// Take the bytes queued for the child, emptying the queue.
    ///
    /// The slice stays valid until the next take or until the session is
    /// dropped.
    pub fn take_writes(&mut self) -> &[u8] {
        self.drained = self.writes.take();
        &self.drained
    }

    /// Take the events queued for the app, emptying the queue, along with how
    /// many were dropped for want of room since the last take.
    ///
    /// A dropped event never makes the screen wrong: everything that has to
    /// be true is in the snapshot.
    pub fn take_events(&mut self) -> (Vec<Event>, u64) {
        self.events.lock().expect("event queue lock").take()
    }

    /// Take the latest snapshot, emptying the mailbox.
    ///
    /// Returns `None` when nothing has been published since the last take.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.mailbox.take()
    }

    /// Capture the terminal and publish it, unless nothing changed, then wake
    /// the consumer if the round left it anything.
    ///
    /// `even_if_unchanged` publishes a frame the screen did not move for, which
    /// is for news that is not the screen's own. cf. [`note_child_exit`]
    ///
    /// [`note_child_exit`]: Session::note_child_exit
    fn publish(&mut self, even_if_unchanged: bool) -> Result<()> {
        // An event is as much reason to wake as a frame is, and a bell marks
        // no cell — so a screen that did not move can still leave something
        // to take.
        let mut something_to_take = self.events.lock().expect("event queue lock").take_arrival();
        if let Some(mut snapshot) = self
            .terminal
            .capture(&self.last_screen, even_if_unchanged)?
        {
            self.last_screen = snapshot.screen.clone();
            // The mailbox keeps only the newest snapshot, so publishing over
            // an unconsumed one drops it. Carry its change marks across, or a
            // consumer that misses a frame is told less changed than did.
            if let Some(dropped) = self.mailbox.take() {
                snapshot.absorb_marks_of(&dropped);
            }
            self.mailbox.publish(snapshot);
            something_to_take = true;
        }
        self.emit_wake(something_to_take)
    }

    /// Wake the consumer if it is owed one, unless a synchronized output block
    /// is open.
    ///
    /// A block is the child saying its screen is mid-draw, so waking inside
    /// one hands over a half-drawn frame. Held back is the wake, not the
    /// publication: the mailbox keeps only the newest snapshot, so the frames
    /// published inside a block are passed over unseen rather than queued up.
    /// And what is held back is a single owed wake rather than one per frame,
    /// which is what makes the end of a block exactly one.
    ///
    /// A block the child never closes is [`wait_out_sync_block`]'s to break,
    /// after which nothing is held back until the block closes — so what is
    /// checked here is the block's standing and not only that one is open.
    ///
    /// What is read here is the mode as the round left it, not every time it
    /// moved during the round: the engine reports no mode change, and scanning
    /// the bytes ourselves would take the parser back off it. So a round that
    /// closes one block and opens the next holds the first one's wake until
    /// the second closes. The mailbox keeps only the newest snapshot, so what
    /// that costs is the delay and not the frame, and the timeout bounds it.
    ///
    /// A block given up on comes back from it by that same reading — a round
    /// that ends outside a block — so one that closes the given-up block and
    /// opens the next in the same breath carries the giving-up into it. That
    /// is as close as this gets to telling one block from the next: the round
    /// is the only boundary anything here can see.
    ///
    /// [`wait_out_sync_block`]: Session::wait_out_sync_block
    fn emit_wake(&mut self, owed: bool) -> Result<()> {
        self.held_back |= owed;
        // Asked whether or not anything is owed: a block given up on may well
        // close on a round that has nothing to show, and the rule has to come
        // back with it either way.
        let open = self.terminal.sync_output_open()?;
        if !open {
            self.given_up_on_block = false;
        }

        if self.held_back && open && !self.given_up_on_block {
            // The clock starts at the first wake held back rather than at the
            // opening of the block: before there is something to show, a
            // timeout has nothing to release.
            self.held_since.get_or_insert_with(Instant::now);
            return Ok(());
        }

        self.pay_wake();
        Ok(())
    }

    /// Give up on a block that has held a wake back too long, answering with
    /// how much longer the one still open may hold it.
    ///
    /// `None` means there is nothing to come back for: no wake is being held,
    /// or the one that was has just gone out. Whoever waits on the terminal
    /// waits no longer than what this answers, and asks again when it returns.
    ///
    /// The block is given up on rather than let through a frame per timeout: a
    /// frame each time over turns a frozen screen into a slow one, and that
    /// program's screen is wrong either way. Drawing it at the speed the user
    /// can watch it go wrong is the better of the two.
    pub(crate) fn wait_out_sync_block(&mut self) -> Option<Duration> {
        let held = self.held_since?;
        match SYNC_TIMEOUT.checked_sub(held.elapsed()) {
            Some(left) if !left.is_zero() => Some(left),
            _ => {
                self.given_up_on_block = true;
                self.pay_wake();
                None
            }
        }
    }

    /// Let what was held back become the consumer's due, and settle it if
    /// anyone is there to take it.
    ///
    /// Whoever calls it, a wake on its way out is no longer a wake being held
    /// back — including the child's exit, which pays one from outside the
    /// rule. cf. `note_child_exit`
    ///
    /// What nobody was there to take stays on the debt rather than here, so
    /// the next consumer to register is told about it and this side is done
    /// with it.
    fn pay_wake(&mut self) {
        self.held_since = None;
        if std::mem::take(&mut self.held_back) {
            self.wake.owe();
        }
        self.wake.settle();
    }
}

/// Run a session's I/O loop and wind it up after.
///
/// A loop that gave up mid-round and one that panicked are the same news to the
/// app: nothing is driving the engine any more, so the session is broken rather
/// than merely finished. Both leave the mark and tell the consumer once, so a
/// window that has stopped working stops looking like one that works. A loop
/// that returned of its own accord — the child ended, or the session is being
/// released — left nothing to say.
///
/// Nothing of its own is done to the child. Returning from here drops the
/// terminal, and dropping it is what puts a child down and collects it — the
/// same path a released session takes, reached without a line of its own.
/// Piling a timeout and a kill onto code that is already running wrong leaves
/// nowhere to go when that fails too. cf. `03-core.md` C8
fn wind_up(broken: &AtomicBool, wake: &Debt, run: impl FnOnce() -> Result<()>) {
    // Unwind safety is asserted rather than proved: what the loop was holding
    // is dropped as this returns, and what it shared is left where it stands
    // for the app to take. Nothing here touches the engine again.
    if matches!(panic::catch_unwind(AssertUnwindSafe(run)), Ok(Ok(()))) {
        return;
    }

    broken.store(true, Ordering::Relaxed);
    // Stored before the telling, so a consumer that comes for it finds the
    // mark already set. Owed rather than called outright, because a break with
    // nobody registered is the news that matters most on the way back.
    wake.owe();
    wake.settle();
}

/// A session with a child process behind a pseudoterminal.
///
/// One thread per session owns the engine and everything that touches it: the
/// engine's handles are single-threaded, so they never leave it. What
/// crosses back to the app is the mailbox, the event queue, and the wake —
/// none of which is the engine's. cf. `docs/adr/0003-snapshot-mailbox.md`
///
/// Input goes the other way as a request rather than a call: it is put where
/// the thread will find it, and the thread applies it. So the calls below
/// return as soon as the request is queued, and the engine's own answer to it
/// is not theirs to report.
pub struct PtySession {
    mailbox: Arc<Mailbox<Snapshot>>,
    events: Arc<Mutex<EventQueue>>,
    // The session's own, shared rather than reached for: registering a
    // consumer must not have to cross to a thread that may be mid-parse.
    wake: Arc<Debt>,
    // Requests the I/O thread applies to the engine on the app's behalf. Only
    // the selection travels this way: what is bound for the child goes in the
    // queue below, which both threads reach.
    selection: Sender<Option<SelectionRange>>,
    // The bytes waiting for the child, whoever queued them. Shared with the
    // session on the I/O thread rather than counted on each side, so the cap
    // is checked where the bytes are.
    writes: Arc<WriteQueue>,
    // What the child ended by, or `STILL_RUNNING`. Written by the I/O thread
    // before it says the child is gone, so a consumer that comes for that
    // telling finds the code already here.
    child_exit: Arc<AtomicI32>,
    // Whether the I/O thread gave up mid-round. Written by that thread as it
    // leaves, and the only way the app's side learns that nothing is driving
    // the engine any more.
    broken: Arc<AtomicBool>,
    waker: Waker,
    stopping: Arc<AtomicBool>,
    // Taken in `drop`, which is the only place this is `None`.
    thread: Option<JoinHandle<()>>,
}

impl PtySession {
    /// Start `program` with `args` behind a pseudoterminal of `cols` by
    /// `rows`, and put a thread on it.
    pub fn new(
        program: &[u8],
        args: &[Vec<u8>],
        cols: u16,
        rows: u16,
        max_scrollback: usize,
    ) -> Result<Self> {
        let (mut terminal, waker) = Pty::spawn(program, args, cols, rows)?;
        let (selection, arriving) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let child_exit = Arc::new(AtomicI32::new(STILL_RUNNING));
        let broken = Arc::new(AtomicBool::new(false));
        let (started, start) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("knotty-io".to_owned())
            .spawn({
                let stopping = Arc::clone(&stopping);
                let child_exit = Arc::clone(&child_exit);
                let broken = Arc::clone(&broken);
                move || {
                    // Built here rather than handed in: the engine's handles
                    // are single-threaded, so this thread has to be the one
                    // that makes them.
                    let mut session = match Session::new(cols, rows, max_scrollback) {
                        Ok(session) => session,
                        Err(error) => {
                            let _ = started.send(Err(error));
                            return;
                        }
                    };
                    // The debt the app registers against is the session's own:
                    // one wake, whichever side of the thread it is settled
                    // from. cf. `03-core.md` C5
                    let wake = Arc::clone(&session.wake);
                    let writes = Arc::clone(&session.writes);
                    let crossing = (
                        Arc::clone(&session.mailbox),
                        Arc::clone(&session.events),
                        Arc::clone(&writes),
                        Arc::clone(&wake),
                    );
                    if started.send(Ok(crossing)).is_err() {
                        return;
                    }
                    wind_up(&broken, &wake, || {
                        io::run(
                            &mut session,
                            &mut terminal,
                            &arriving,
                            &writes,
                            &child_exit,
                            &stopping,
                        )
                    });
                }
            })
            .map_err(Error::from)?;

        // The thread reports how its own construction went, so a session that
        // could not build its engine fails here rather than looking alive.
        let (mailbox, events, writes, wake) = start.recv().map_err(|_| Error::Io)??;
        Ok(Self {
            mailbox,
            events,
            wake,
            selection,
            writes,
            child_exit,
            broken,
            waker,
            stopping,
            thread: Some(thread),
        })
    }

    /// What has become of the child.
    ///
    /// A session with a PTY behind it always has one, so this is never
    /// [`ChildState::None`].
    pub fn child(&self) -> ChildState {
        match self.child_exit.load(Ordering::Relaxed) {
            STILL_RUNNING => ChildState::Running,
            code => ChildState::Exited(code),
        }
    }

    /// Whether the session's thread gave up mid-round, leaving nothing to
    /// drive the engine.
    ///
    /// What it published before that stands. What it would have published
    /// since does not exist.
    pub fn broken(&self) -> bool {
        self.broken.load(Ordering::Relaxed)
    }

    /// How many bytes are still waiting for the child.
    ///
    /// Everything queued and not yet handed to the terminal, whoever queued it
    /// — what the app wrote and what the terminal answered. This is the count
    /// [`write`] refuses against, and it falls back to zero as the child keeps
    /// up.
    ///
    /// It is also the one place a caller can watch a write reach the terminal,
    /// which is what the B5 bench times against. Nothing else says so: the
    /// write itself happens on the I/O thread and tells no one. Reading it
    /// takes no lock, so watching it in a spin does not hold up the hand-over
    /// being watched for.
    ///
    /// [`write`]: PtySession::write
    pub fn backlog(&self) -> usize {
        self.writes.waiting()
    }

    /// Set what to call when the session has something new to be taken, or
    /// clear it with `None`.
    ///
    /// The call is made on the session's own I/O thread, so it may do nothing
    /// but wake the thread that registered it. A wake that fell due while
    /// nobody was registered is paid here, before this returns.
    pub fn set_wake(&self, wake: Option<Wake>) {
        self.wake.register(wake);
    }

    /// Queue `bytes` for the child.
    ///
    /// # Errors
    ///
    /// [`Error::WriteQueueFull`] when the child has left more waiting than the
    /// queue holds, in which case none of `bytes` was queued.
    ///
    /// [`Error::Io`] once the session's thread is gone, which it is once the
    /// child is: nothing is left to hand the queue to the terminal.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        // The thread is what drains the queue, so bytes queued after it has
        // gone have nowhere left to go. Saying so beats swallowing them —
        // the same answer `set_selection` gives, which the queue's own
        // emptiness would otherwise hide here.
        if self.thread.as_ref().is_some_and(JoinHandle::is_finished) {
            return Err(Error::Io);
        }
        // Queued into the same place the terminal's own answers go, and
        // refused against what is already there — which is what the child has
        // not read, the case the cap exists for. cf. `02-ffi.md`
        if !self.writes.try_push(bytes) {
            return Err(Error::WriteQueueFull);
        }
        // The I/O thread waits on room in the terminal only while something is
        // queued for it, so a queue that just stopped being empty has to say
        // so.
        self.waker.nudge();
        Ok(())
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// An endpoint outside the viewport is not reported: by the time the
    /// thread finds that out, this call has long returned.
    pub fn set_selection(&self, range: Option<SelectionRange>) -> Result<()> {
        // The thread is gone once the child is, and what this carried has
        // nowhere left to go. Saying so beats swallowing it.
        self.selection.send(range).map_err(|_| Error::Io)?;
        self.waker.nudge();
        Ok(())
    }

    /// Take the events queued for the app, emptying the queue, along with how
    /// many were dropped for want of room since the last take.
    pub fn take_events(&self) -> (Vec<Event>, u64) {
        self.events.lock().expect("event queue lock").take()
    }

    /// Take the latest snapshot, emptying the mailbox.
    ///
    /// Returns `None` when nothing has been published since the last take.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.mailbox.take()
    }
}

impl Drop for PtySession {
    /// Stop the thread and wait for it to let go of the terminal.
    ///
    /// The thread owns the terminal, so waiting for it is also what waits for
    /// the child to be put down and collected — by the time this returns,
    /// nothing of the session is still running.
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Relaxed);
        // Stored first, so a thread that is between rounds rather than in the
        // wait still finds the flag set when it gets there.
        self.waker.nudge();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Instant;

    use super::{Debt, Duration, PtySession, SYNC_TIMEOUT, Session, wind_up};
    use crate::queue::Event;
    use crate::writer::CAP;
    use crate::{Error, Result};

    fn session() -> Session {
        Session::new(80, 24, 0).expect("a session")
    }

    /// A consumer that counts what it was told, which is all a real one may do
    /// from inside the call.
    fn listening() -> (Debt, Arc<AtomicU32>) {
        let told = Arc::new(AtomicU32::new(0));
        let wake = Debt::default();
        wake.register(Some(Box::new({
            let told = Arc::clone(&told);
            move || {
                told.fetch_add(1, Ordering::Relaxed);
            }
        })));

        (wake, told)
    }

    /// The I/O thread is the only thing driving a PTY session's engine, so a
    /// panic in it takes the session with it — and the app has to be told,
    /// since the last thing it heard was a screen that looked fine.
    #[test]
    fn a_loop_that_panics_leaves_the_session_broken_and_tells_the_consumer_once() {
        let broken = AtomicBool::new(false);
        let (wake, told) = listening();

        // Reaching the next line at all is half the point: a panic escaping
        // the thread would take no one with it, but the session it left
        // behind would look alive for good.
        wind_up(&broken, &wake, || panic!("on purpose"));

        assert!(broken.load(Ordering::Relaxed), "the break went unrecorded");
        assert_eq!(told.load(Ordering::Relaxed), 1);
    }

    /// A round that failed left the session as unattended as a panic did: the
    /// loop is gone either way.
    #[test]
    fn a_loop_that_gave_up_mid_round_leaves_the_session_broken() {
        let broken = AtomicBool::new(false);
        let (wake, told) = listening();

        wind_up(&broken, &wake, || Err(Error::Io));

        assert!(broken.load(Ordering::Relaxed));
        assert_eq!(told.load(Ordering::Relaxed), 1);
    }

    /// The ordinary way out: the child ended, or the session is being
    /// released. Nothing broke, and a consumer told otherwise would put up a
    /// dead window over a session that merely finished.
    #[test]
    fn a_loop_that_returned_of_its_own_accord_leaves_the_session_alone() {
        let broken = AtomicBool::new(false);
        let (wake, told) = listening();

        wind_up(&broken, &wake, || Result::Ok(()));

        assert!(!broken.load(Ordering::Relaxed));
        assert_eq!(told.load(Ordering::Relaxed), 0);
    }

    /// A session and the count of what its consumer was told.
    fn watched() -> (Session, Arc<AtomicU32>) {
        let mut session = session();
        let told = Arc::new(AtomicU32::new(0));
        session.set_wake(Some(Box::new({
            let told = Arc::clone(&told);
            move || {
                told.fetch_add(1, Ordering::Relaxed);
            }
        })));

        (session, told)
    }

    /// Open a block and draw inside it, which is a wake held back.
    fn holding() -> (Session, Arc<AtomicU32>) {
        let (mut session, told) = watched();
        session
            .feed(b"\x1b[?2026hhalf drawn")
            .expect("the feed to finish");
        assert_eq!(told.load(Ordering::Relaxed), 0, "the block held nothing");

        (session, told)
    }

    /// Put the block's clock back far enough that its time is up.
    ///
    /// The alternative is sleeping out the real timeout, which is a second of
    /// test time to learn what the field already says.
    fn overdue(session: &mut Session) {
        session.held_since = Some(Instant::now() - SYNC_TIMEOUT);
    }

    /// A child that opens a block and never closes it would otherwise hold the
    /// screen for good. The wake it held back goes out once, and the screen
    /// starts moving again.
    #[test]
    fn a_block_open_past_the_timeout_wakes_the_consumer() {
        let (mut session, told) = holding();

        overdue(&mut session);
        assert!(
            session.wait_out_sync_block().is_none(),
            "a block past its time was still being waited on",
        );

        assert_eq!(told.load(Ordering::Relaxed), 1);
    }

    /// A block given up on is given up on for good, not let through a frame
    /// per timeout. cf. [`Session::wait_out_sync_block`]
    #[test]
    fn a_block_given_up_on_holds_nothing_back() {
        let (mut session, told) = holding();
        overdue(&mut session);
        session.wait_out_sync_block();

        session.feed(b"more").expect("the feed to finish");
        session.feed(b"and more").expect("the feed to finish");

        assert_eq!(told.load(Ordering::Relaxed), 3, "the block held again");
        assert!(
            session.wait_out_sync_block().is_none(),
            "a block given up on was being waited on again",
        );
    }

    /// A child that closes its block is behaving, and gets the suppression it
    /// asks for afterwards — being given up on once is not held against it.
    #[test]
    fn a_block_that_closes_puts_the_suppression_back() {
        let (mut session, told) = holding();
        overdue(&mut session);
        session.wait_out_sync_block();

        session.feed(b"\x1b[?2026l").expect("the feed to finish");
        let settled = told.load(Ordering::Relaxed);
        session
            .feed(b"\x1b[?2026hhalf drawn again")
            .expect("the feed to finish");

        assert_eq!(
            told.load(Ordering::Relaxed),
            settled,
            "a block after a given-up one was not held back",
        );
    }

    /// What the round costs, written down: closing the given-up block and
    /// opening the next in one feed leaves the mode open at the end of it, and
    /// the giving-up rides along into a block that did nothing wrong. The
    /// round is the only boundary this side can see, so this is the edge of
    /// what "the block closed" can mean — not a case worth scanning bytes for.
    #[test]
    fn a_block_opened_in_the_same_breath_as_the_last_one_closed_stays_given_up_on() {
        let (mut session, told) = holding();
        overdue(&mut session);
        session.wait_out_sync_block();
        let settled = told.load(Ordering::Relaxed);

        session
            .feed(b"\x1b[?2026l\x1b[?2026hhalf drawn again")
            .expect("the feed to finish");

        assert!(
            told.load(Ordering::Relaxed) > settled,
            "the round closed a block and opened one, and the mode says open",
        );
    }

    /// A consumer arriving mid-block is still a consumer arriving mid-block:
    /// the debt it is paid on registering is what fell due, and a wake the
    /// block is sitting on has not. Paying it there would hand over the
    /// half-drawn screen the suppression exists to keep back. cf.
    /// `03-core.md` C5
    #[test]
    fn a_wake_a_block_is_holding_back_is_not_paid_to_a_consumer_registering() {
        let (mut session, _) = holding();

        let told = Arc::new(AtomicU32::new(0));
        session.set_wake(Some(Box::new({
            let told = Arc::clone(&told);
            move || {
                told.fetch_add(1, Ordering::Relaxed);
            }
        })));

        assert_eq!(
            told.load(Ordering::Relaxed),
            0,
            "registering let a held-back wake out",
        );

        session.feed(b"\x1b[?2026l").expect("the feed to finish");
        assert_eq!(told.load(Ordering::Relaxed), 1, "the block let nothing out");
    }

    /// The ordinary block, which is every block: it closes long before its
    /// time is up, and the timeout is something it never meets.
    #[test]
    fn a_block_that_closes_in_time_never_comes_due() {
        let (mut session, told) = holding();

        assert!(
            session
                .wait_out_sync_block()
                .is_some_and(|left| !left.is_zero()),
            "a block that just opened was already out of time",
        );
        assert_eq!(told.load(Ordering::Relaxed), 0);

        session.feed(b"\x1b[?2026l").expect("the feed to finish");

        assert_eq!(told.load(Ordering::Relaxed), 1);
        assert!(
            session.wait_out_sync_block().is_none(),
            "a closed block was still being waited on",
        );
    }

    /// What the rules themselves say is `listener`'s to test. What is left
    /// here is the one thing a literal cannot stand in for: the engine sends a
    /// write with no representations as a null array, which is not a slice at
    /// any length, and the facade turning that into an empty one is what makes
    /// the refusal reachable at all. cf. ADR 0012
    #[test]
    fn a_clipboard_write_with_no_representations_leaves_the_clipboard_alone() {
        let mut session = session();

        session.feed(b"\x1b]52;c;\x07").expect("the feed to finish");

        let (events, dropped) = session.take_events();
        assert!(events.is_empty(), "a clear request reached the app");
        assert_eq!(dropped, 0);
    }

    /// The other half of the wiring: a write that is refused nowhere comes out
    /// on the event queue the listener was built around.
    #[test]
    fn a_clipboard_write_that_is_utf8_still_arrives() {
        let mut session = session();

        // `aGk=` is base64 for "hi".
        session
            .feed(b"\x1b]52;c;aGk=\x07")
            .expect("the feed to finish");

        let (events, _) = session.take_events();
        assert!(matches!(
            events.as_slice(),
            [Event::ClipboardWrite { text, .. }] if text == "hi"
        ));
    }

    /// How long the thread is given to wind up before it counts as one that
    /// never does.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// The thread is the only thing that drains the queue, so bytes queued
    /// after it has gone would sit there for good. What a consumer hears
    /// instead is the refusal — the same one `set_selection` gives, and the
    /// pairing that broke when this path stopped going through the channel.
    #[test]
    fn a_write_with_no_thread_left_to_carry_it_says_so() {
        let session = PtySession::new(b"/bin/sh", &[b"-c".to_vec(), b"exit 0".to_vec()], 4, 1, 0)
            .expect("a session whose child ends at once");

        // Spun on rather than slept through: what is waited for is the thread
        // winding up, and it has no telling of its own.
        let start = Instant::now();
        loop {
            match session.write(b"typed") {
                Err(Error::Io) => break,
                // A full queue is the cap doing its work rather than a
                // refusal: nothing drains it once the child is gone, so on a
                // busy runner it can fill before the thread winds up. Neither
                // answer is the one waited for, and `is_finished` is checked
                // before the queueing, so `Error::Io` still comes out of a
                // queue with no room left. cf. `02-ffi.md`
                Ok(()) | Err(Error::WriteQueueFull) => assert!(
                    start.elapsed() < PATIENCE,
                    "the thread is long gone and a write still did not say so",
                ),
                Err(other) => panic!("a live queue refused a write with {other:?}"),
            }
        }
    }

    /// The cap keeps a child that has stopped reading from growing the queue
    /// without bound. What the unit tests cannot reach is this path: both ends
    /// of a PTY session queueing into the one queue while the I/O thread
    /// drains it. So what is watched here is the ceiling holding while all
    /// three are at it.
    #[test]
    fn the_queue_bound_holds_against_a_child_that_never_reads() {
        let session = PtySession::new(b"/bin/sh", &[b"-c".to_vec(), b"sleep 30".to_vec()], 4, 1, 0)
            .expect("a session with a child that never reads");

        let chunk = vec![b'x'; 64 * 1024];
        assert!(session.write(&chunk).is_ok(), "the first write was refused");

        // Four times the cap, so a queue growing unchecked would be far past
        // it by the end.
        for _ in 1..(4 * CAP / chunk.len()) {
            // A refusal is one of the two right answers — it means the queue
            // was full, which is the cap doing its work. Growing past it is
            // the wrong one.
            let _ = session.write(&chunk);
            let waiting = session.backlog();
            assert!(
                waiting <= CAP,
                "the queue holds {waiting} bytes, past {CAP}"
            );
        }
    }
}
