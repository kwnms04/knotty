//! Session lifecycle and the publish path.

use std::cell::RefCell;
use std::panic::{self, AssertUnwindSafe};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use libghostty_vt::screen::Screen;
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::{
    ClipboardLocation, ClipboardWriteError, ConformanceLevel, DeviceAttributeFeature,
    DeviceAttributes, DeviceType, Mode, Point, PointCoordinate, PrimaryDeviceAttributes,
    SecondaryDeviceAttributes, TertiaryDeviceAttributes,
};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use crate::io::{self, Input, Pty, Waker};
use crate::mailbox::Mailbox;
use crate::queue::{ClipboardTarget, Event, EventQueue};
use crate::snapshot::{self, ScreenState, Snapshot};
use crate::{Error, Result};

/// What a session calls when it has something new to be taken.
///
/// `Send` because a PTY session makes the call from its own I/O thread, which
/// is not the thread that registered it.
pub type Wake = Box<dyn Fn() + Send>;

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

/// How many bytes may wait for the PTY before further writes are dropped.
///
/// A child that never reads is the case this exists for: without a cap the
/// queue grows until the process dies.
const WRITE_QUEUE_CAP: usize = 8 * 1024 * 1024;

/// The largest clipboard payload knotty will carry.
///
/// A write past it is refused whole rather than cut short: nothing tells the
/// user their copy arrived as its first megabyte, so a truncated clipboard is
/// worse than one that did not change. With the queue's cap this is also what
/// bounds what undrained events can hold.
const CLIPBOARD_TEXT_CAP: usize = 1024 * 1024;

/// The one representation knotty takes off a clipboard write.
///
/// The engine normalizes OSC 52 and iTerm2's copy sequence into the same
/// shape, and both carry this. v1 has no rich clipboard to put anything else
/// on, so a write offering no plain text has nothing for us.
const CLIPBOARD_MIME: &[u8] = b"text/plain";

/// What an ENQ is answered with: knotty's name.
///
/// An answerback reaches the child as if it were typed, so it stays a fixed
/// string of ours — nothing that reaches the screen can steer what is sent.
const ANSWERBACK: &str = "knotty";

/// Name and version, the payload of the XTVERSION answer.
///
/// Programs pick a code path by this, so it has to be knotty's rather than the
/// `libghostty` the engine falls back to.
const VERSION_REPORT: &str = concat!("knotty ", env!("CARGO_PKG_VERSION"));

/// What knotty answers a device attributes query with.
///
/// A VT220 with color, which is what the engine implements. DA2's firmware
/// field is a version number and stays 0 while knotty is 0.x — it is set by
/// hand, not from the crate version. DA3's unit id is meaningless for an
/// emulator.
const DEVICE_ATTRIBUTES: DeviceAttributes = DeviceAttributes {
    primary: PrimaryDeviceAttributes::new(
        ConformanceLevel::VT220,
        &[DeviceAttributeFeature::ANSI_COLOR],
    ),
    secondary: SecondaryDeviceAttributes {
        device_type: DeviceType::VT220,
        firmware_version: 0,
        rom_cartridge: 0,
    },
    tertiary: TertiaryDeviceAttributes { unit_id: 0 },
};

/// How the engine answers a title query: `OSC l <title> ST`.
const TITLE_REPORT_PREFIX: &[u8] = b"\x1b]l";

/// The same answer carrying no title.
const EMPTY_TITLE_REPORT: &[u8] = b"\x1b]l\x1b\\";

/// Empty out an answer that would carry writable state back to the child.
///
/// A title query (`CSI 21 t`) is answered with the title, and a PTY write is
/// keyboard input as far as the shell can tell — so output nobody trusts can
/// set the title to a command, query it back, and have it typed. The engine
/// emits this one answer with no callback to fill in, which is why it is
/// caught here on the wire rather than refused earlier. The answer still goes
/// out: a program waiting on one must not hang. cf.
/// `docs/adr/0007-input-security.md`.
///
/// The icon label query is the other half of the pair the ADR names. Its
/// report is not written here because nothing emits one: an answer the filter
/// never sees costs a prefix to match and proves nothing when it does.
//
// `03-core.md` gives this to the `listener` module, which does not exist yet;
// until it does, it sits beside the queue it feeds.
fn sanitized_answer(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(TITLE_REPORT_PREFIX) {
        return EMPTY_TITLE_REPORT;
    }
    bytes
}

impl From<ClipboardLocation> for ClipboardTarget {
    fn from(location: ClipboardLocation) -> Self {
        match location {
            ClipboardLocation::Standard => Self::Standard,
            ClipboardLocation::Selection => Self::Selection,
            ClipboardLocation::Primary => Self::Primary,
        }
    }
}

/// Bytes on their way to the child.
///
/// Every write the terminal makes lands here, so nothing waits on a PTY that
/// may not be ready — or, in a detached session, that does not exist.
//
// `03-core.md` gives this to the `io` module, which owns the event loop and
// the file descriptor. Neither exists yet, so it waits here.
#[derive(Debug, Default)]
struct WriteQueue {
    bytes: Vec<u8>,
    /// Whether bytes were dropped for want of room.
    overran: bool,
}

impl WriteQueue {
    /// Append `bytes`, or report that there was no room for them.
    fn try_push(&mut self, bytes: &[u8]) -> bool {
        if self.bytes.len() + bytes.len() > WRITE_QUEUE_CAP {
            return false;
        }
        self.bytes.extend_from_slice(bytes);
        true
    }

    /// The same for what the engine answers, which has no caller standing by
    /// to be told: the drop is remembered instead.
    fn push(&mut self, bytes: &[u8]) {
        if !self.try_push(bytes) {
            self.overran = true;
        }
    }

    /// Whether bytes have been dropped since this was last asked.
    ///
    /// Asking clears it, so one overrun is reported once rather than held
    /// against every later call.
    fn take_overrun(&mut self) -> bool {
        std::mem::take(&mut self.overran)
    }
}

/// A terminal session.
///
/// It owns no thread and no child process of its own: [`feed`] runs the VT
/// engine on the calling thread. A detached session is one used directly; a
/// [`PtySession`] is this same session with a thread and a child around it,
/// and everything past the parser is the same code either way.
///
/// [`feed`]: Session::feed
pub struct Session {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    // Shared rather than owned outright: a PTY session's consumer takes from
    // this on its own thread while the I/O thread publishes into it. The
    // mailbox is the only thing here that crosses, which is what adr/0003
    // bought.
    mailbox: Arc<Mailbox<Snapshot>>,
    // Which screen the selection was made on, or None when there is none.
    //
    // A snapshot has to say whether a selection exists even when no visible
    // row falls inside one, and the engine's own answer is out of reach: the
    // C API has it, but the pinned safe wrapper keeps the raw handle private.
    // So knotty keeps its own record. See `Session::has_selection` for what
    // that costs.
    selection_screen: Option<Screen>,
    // What the last capture said about the screen outside the grid, so that a
    // title or cursor change on an otherwise still screen still publishes.
    last_screen: ScreenState,
    // Shared with the engine callback, which outlives any single call and so
    // cannot borrow the session. One thread drives both, so the borrows never
    // overlap.
    writes: Rc<RefCell<WriteQueue>>,
    // What the last drain handed out. Kept alive here because the boundary
    // lends the bytes rather than copying them.
    drained: Vec<u8>,
    // Shared with the engine callbacks, for the same reason the writer queue
    // is — and with the app, which is what drains it whether or not a PTY is
    // behind the session. That second sharer is why this is a lock and the
    // writer queue is not.
    events: Arc<Mutex<EventQueue>>,
    // How the consumer is told to come and look, and whether it is owed a
    // telling. Neither is the engine's business: when a frame gets drawn is
    // between the session and whoever draws.
    wake: Option<Wake>,
    wake_owed: bool,
}

impl Session {
    /// Create a session, its engine, and its queues.
    ///
    /// The engine's handles are single-threaded, so whatever thread calls this
    /// is the only one that may drive the session afterwards.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self> {
        let mut terminal = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback,
        })?;
        let render = RenderState::new()?;

        let writes = Rc::new(RefCell::new(WriteQueue::default()));
        terminal.on_pty_write({
            let writes = Rc::clone(&writes);
            move |_, bytes| writes.borrow_mut().push(sanitized_answer(bytes))
        })?;

        // What the core knows about itself, and nothing more. The pixel size
        // is the renderer's to answer and the color scheme the app's, so those
        // queries get no callback and the engine stays silent on them — knotty
        // does not invent a value it cannot know. One callback covers all
        // three size reports, so the size in cells goes silent with the two in
        // pixels rather than being answered alongside made-up pixels.
        //
        // `03-core.md` gives these to the `listener` module, which does not
        // exist yet; until it does, they sit at the one place a session wires
        // the engine up.
        terminal.on_device_attributes(|_| Some(DEVICE_ATTRIBUTES))?;
        terminal.on_enquiry(|_| Some(ANSWERBACK))?;
        terminal.on_xtversion(|_| Some(VERSION_REPORT))?;

        // What the app has to be told rather than shown. Neither leaves a
        // mark on the screen, so a consumer that misses one has no second way
        // to learn it happened.
        let events = Arc::new(Mutex::new(EventQueue::default()));
        terminal.on_bell({
            let events = Arc::clone(&events);
            move |_| events.lock().expect("event queue lock").push(Event::Bell)
        })?;
        terminal.on_clipboard_write({
            let events = Arc::clone(&events);
            move |_, write| {
                // The refusals below are how the engine's callback says no.
                // OSC 52 carries no acknowledgement, so nothing reaches the
                // child either way — what they buy is that the app is never
                // handed a payload it should not act on.
                //
                // A write carrying no representations at all is how the engine
                // asks for the clipboard to be cleared. It lands here as no
                // matching representation, and refusing it is the answer we
                // want: acting on it faithfully would wipe what the user last
                // copied on the say-so of the child.
                let Some(content) = write
                    .contents()
                    .find(|content| content.mime == CLIPBOARD_MIME)
                else {
                    return Err(ClipboardWriteError::Unsupported);
                };
                // A representation of no length is an explicit empty one,
                // which the engine's contract says does not clear the
                // clipboard. An app that wrote it faithfully would wipe what
                // the user last copied, so it stops here rather than going
                // out as a copy of nothing.
                if content.data.is_empty() {
                    return Ok(());
                }
                if content.data.len() > CLIPBOARD_TEXT_CAP {
                    return Err(ClipboardWriteError::Denied);
                }
                // The payload is base64 of whatever the child chose to send,
                // so it is bytes and nothing promises they are text. What is
                // not UTF-8 is malformed as `text/plain`, and the event
                // carries `KtText`, which promises UTF-8 to the app. So it is
                // refused here rather than decoded lossily. cf. ADR 0012
                let Ok(text) = str::from_utf8(content.data) else {
                    return Err(ClipboardWriteError::InvalidData);
                };

                events
                    .lock()
                    .expect("event queue lock")
                    .push(Event::ClipboardWrite {
                        target: write.location().into(),
                        text: text.to_owned(),
                    });
                Ok(())
            }
        })?;

        // A clipboard read gets no callback because the engine drops the
        // request without telling us: there is nothing to answer, so nothing
        // goes out. Answering would hand the child whatever the user last
        // copied. cf. `docs/adr/0007-input-security.md`

        Ok(Self {
            terminal,
            render,
            mailbox: Arc::new(Mailbox::new()),
            selection_screen: None,
            last_screen: ScreenState::default(),
            writes,
            drained: Vec::new(),
            events,
            wake: None,
            wake_owed: false,
        })
    }

    /// Set what to call when the session has something new to be taken, or
    /// clear it with `None`.
    ///
    /// The call is made on the thread that drove the session, from inside the
    /// call that published — so it may do nothing but wake its own thread.
    /// Re-entering the session from it would re-enter state the running call
    /// still holds.
    pub fn set_wake(&mut self, wake: Option<Wake>) {
        self.wake = wake;
    }

    /// Queue `bytes` for the child, without waiting for them to get there.
    ///
    /// # Errors
    ///
    /// [`Error::WriteQueueFull`] when they did not fit, in which case none of
    /// them were queued: a prefix of what the user typed reaching the child is
    /// worse than none of it.
    pub fn write(&mut self, bytes: &[u8]) -> Result<()> {
        if self.writes.borrow_mut().try_push(bytes) {
            return Ok(());
        }
        Err(Error::WriteQueueFull)
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// Publishes a snapshot: the selection is part of what a consumer draws.
    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> Result<()> {
        match range {
            Some(range) => {
                let at = |x, y| Point::Viewport(PointCoordinate { x, y: u32::from(y) });
                let start = self
                    .terminal
                    .grid_ref(at(range.start_x, range.start_y))
                    .map_err(|_| Error::OutOfRange)?;
                let end = self
                    .terminal
                    .grid_ref(at(range.end_x, range.end_y))
                    .map_err(|_| Error::OutOfRange)?;
                self.terminal
                    .set_selection(Some(&Selection::new(start, end, range.rectangle)))?;
            }
            None => {
                self.terminal.set_selection(None)?;
            }
        }
        self.selection_screen = match range {
            Some(_) => Some(self.terminal.active_screen()?),
            None => None,
        };

        self.publish(false)
    }

    /// Whether a selection exists.
    ///
    /// The engine's selection belongs to the active screen and is dropped when
    /// that changes, so knotty's record only holds while the screen does. A
    /// sequence that resets the terminal outright also drops the selection and
    /// is not detectable here, so this can read true for a while after such a
    /// reset. Exposing the engine's own answer needs the wrapper to hand out
    /// the raw handle.
    fn has_selection(&self) -> Result<bool> {
        Ok(match self.selection_screen {
            Some(screen) => screen == self.terminal.active_screen()?,
            None => false,
        })
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
        self.terminal.vt_write(bytes);

        // Read before publishing, which can return early: an overrun left
        // standing would surface on some later feed that overran nothing.
        let overran = self.writes.borrow_mut().take_overrun();
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
        self.drained = std::mem::take(&mut self.writes.borrow_mut().bytes);
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
        let has_selection = self.has_selection()?;
        // An event is as much reason to wake as a frame is, and a bell marks
        // no cell — so a screen that did not move can still leave something
        // to take.
        let mut something_to_take = self.events.lock().expect("event queue lock").take_arrival();
        if let Some(mut snapshot) = snapshot::capture(
            &mut self.render,
            &self.terminal,
            &self.last_screen,
            even_if_unchanged,
        )? {
            snapshot.has_selection = has_selection;
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
    /// A block the child never closes is a timeout's to break, and a timeout
    /// needs a clock. A detached session runs on its caller's thread and has
    /// none, so that arrives with the I/O thread.
    ///
    /// What is read here is the mode as the round left it, not every time it
    /// moved during the round: the engine reports no mode change, and scanning
    /// the bytes ourselves would take the parser back off it. So a round that
    /// closes one block and opens the next holds the first one's wake until
    /// the second closes. The mailbox keeps only the newest snapshot, so what
    /// that costs is the delay and not the frame — and the timeout is its
    /// upper bound once there is one.
    fn emit_wake(&mut self, owed: bool) -> Result<()> {
        self.wake_owed |= owed;
        if !self.wake_owed || self.terminal.mode(Mode::SYNC_OUTPUT)? {
            return Ok(());
        }
        self.pay_wake();
        Ok(())
    }

    /// Settle the owed wake, if one is owed and anyone is there to take it.
    ///
    /// Owed with nobody to tell stays owed, so a consumer that registers late
    /// is told about what it was not there for rather than having to know to
    /// look.
    fn pay_wake(&mut self) {
        if !self.wake_owed {
            return;
        }
        let Some(wake) = &self.wake else {
            return;
        };

        self.wake_owed = false;
        wake();
    }
}

/// The consumer of a PTY session, and whether it is owed a telling.
///
/// The debt lives here rather than on the session inside, because that
/// session's callback is a trampoline into this and so is never absent — it
/// would settle a debt on behalf of a consumer that is not there. Keeping it
/// out here is what lets a consumer registering late be told about what it was
/// not there for. cf. `03-core.md` C5
#[derive(Default)]
struct Consumer {
    wake: Option<Wake>,
    owed: bool,
}

impl Consumer {
    /// Tell the consumer to come and look, or remember that it is owed one.
    fn tell(&mut self) {
        match &self.wake {
            Some(wake) => wake(),
            None => self.owed = true,
        }
    }
}

/// Run a session's I/O loop and settle up after it.
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
fn settle(broken: &AtomicBool, consumer: &Mutex<Consumer>, run: impl FnOnce() -> Result<()>) {
    // Unwind safety is asserted rather than proved: what the loop was holding
    // is dropped as this returns, and what it shared is left where it stands
    // for the app to take. Nothing here touches the engine again.
    if matches!(panic::catch_unwind(AssertUnwindSafe(run)), Ok(Ok(()))) {
        return;
    }

    broken.store(true, Ordering::Relaxed);
    consumer.lock().expect("consumer lock").tell();
}

/// A session with a child process behind a pseudoterminal.
///
/// One thread per session owns the engine and everything that touches it: the
/// safe wrapper's handles are single-threaded, so they never leave it. What
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
    // Kept here rather than on the session, so that registering one does not
    // have to reach across to a thread that may be mid-parse.
    consumer: Arc<Mutex<Consumer>>,
    input: Sender<Input>,
    // How many bytes are waiting for the child, whoever queued them. Both
    // ends add and the I/O thread takes away as the terminal accepts, because
    // a queue with an end on each thread cannot be counted on either alone.
    backlog: Arc<AtomicUsize>,
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
        let (input, arriving) = mpsc::channel();
        let stopping = Arc::new(AtomicBool::new(false));
        let backlog = Arc::new(AtomicUsize::new(0));
        let child_exit = Arc::new(AtomicI32::new(STILL_RUNNING));
        let broken = Arc::new(AtomicBool::new(false));
        let consumer = Arc::new(Mutex::new(Consumer::default()));
        let (started, start) = mpsc::channel();

        let thread = thread::Builder::new()
            .name("knotty-io".to_owned())
            .spawn({
                let stopping = Arc::clone(&stopping);
                let backlog = Arc::clone(&backlog);
                let child_exit = Arc::clone(&child_exit);
                let broken = Arc::clone(&broken);
                let consumer = Arc::clone(&consumer);
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
                    // Always set, so the session inside never holds a debt of
                    // its own — `Consumer` is what holds one instead.
                    session.set_wake(Some(Box::new({
                        let consumer = Arc::clone(&consumer);
                        move || consumer.lock().expect("consumer lock").tell()
                    })));

                    let crossing = (Arc::clone(&session.mailbox), Arc::clone(&session.events));
                    if started.send(Ok(crossing)).is_err() {
                        return;
                    }
                    settle(&broken, &consumer, || {
                        io::run(
                            &mut session,
                            &mut terminal,
                            &arriving,
                            &backlog,
                            &child_exit,
                            &stopping,
                        )
                    });
                }
            })
            .map_err(Error::from)?;

        // The thread reports how its own construction went, so a session that
        // could not build its engine fails here rather than looking alive.
        let (mailbox, events) = start.recv().map_err(|_| Error::Io)??;
        Ok(Self {
            mailbox,
            events,
            consumer,
            input,
            backlog,
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

    /// Set what to call when the session has something new to be taken, or
    /// clear it with `None`.
    ///
    /// The call is made on the session's own I/O thread, so it may do nothing
    /// but wake the thread that registered it. A wake that fell due while
    /// nobody was registered is paid here, before this returns.
    pub fn set_wake(&self, wake: Option<Wake>) {
        let mut consumer = self.consumer.lock().expect("consumer lock");
        consumer.wake = wake;
        // Taken rather than read: a wake cleared while a debt stands leaves
        // the debt, which `tell` puts back.
        if std::mem::take(&mut consumer.owed) {
            consumer.tell();
        }
    }

    /// Queue `bytes` for the child.
    ///
    /// # Errors
    ///
    /// [`Error::WriteQueueFull`] when the child has left more waiting than the
    /// queue holds, in which case none of `bytes` was queued.
    pub fn write(&self, bytes: &[u8]) -> Result<()> {
        // The backlog is only ever taken from as the terminal accepts, so what
        // this refuses against is what the child has not read — the case the
        // cap exists for. cf. `02-ffi.md`
        //
        // Read and added to with no lock between: calls on one session are
        // serialized by the boundary's own contract, so the only other hand
        // here is the I/O thread's, and a subtraction it makes in between only
        // makes this answer more generous.
        if self.backlog.load(Ordering::Relaxed) + bytes.len() > WRITE_QUEUE_CAP {
            return Err(Error::WriteQueueFull);
        }
        self.backlog.fetch_add(bytes.len(), Ordering::Relaxed);
        self.hand_over(Input::Write(bytes.to_vec()))
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// An endpoint outside the viewport is not reported: by the time the
    /// thread finds that out, this call has long returned.
    pub fn set_selection(&self, range: Option<SelectionRange>) -> Result<()> {
        self.hand_over(Input::Selection(range))
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

    /// Put `input` where the I/O thread will find it, and make it look.
    fn hand_over(&self, input: Input) -> Result<()> {
        // The thread is gone once the child is, and what this carried has
        // nowhere left to go. Saying so beats swallowing it.
        self.input.send(input).map_err(|_| Error::Io)?;
        self.waker.nudge();
        Ok(())
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

    use super::{Consumer, Mutex, Session, settle};
    use crate::queue::Event;
    use crate::{Error, Result};

    fn session() -> Session {
        Session::new(80, 24, 0).expect("a session")
    }

    /// A consumer that counts what it was told, which is all a real one may do
    /// from inside the call.
    fn listening() -> (Mutex<Consumer>, Arc<AtomicU32>) {
        let told = Arc::new(AtomicU32::new(0));
        let consumer = Consumer {
            wake: Some(Box::new({
                let told = Arc::clone(&told);
                move || {
                    told.fetch_add(1, Ordering::Relaxed);
                }
            })),
            owed: false,
        };

        (Mutex::new(consumer), told)
    }

    /// The I/O thread is the only thing driving a PTY session's engine, so a
    /// panic in it takes the session with it — and the app has to be told,
    /// since the last thing it heard was a screen that looked fine.
    #[test]
    fn a_loop_that_panics_leaves_the_session_broken_and_tells_the_consumer_once() {
        let broken = AtomicBool::new(false);
        let (consumer, told) = listening();

        // Reaching the next line at all is half the point: a panic escaping
        // the thread would take no one with it, but the session it left
        // behind would look alive for good.
        settle(&broken, &consumer, || panic!("on purpose"));

        assert!(broken.load(Ordering::Relaxed), "the break went unrecorded");
        assert_eq!(told.load(Ordering::Relaxed), 1);
    }

    /// A round that failed left the session as unattended as a panic did: the
    /// loop is gone either way.
    #[test]
    fn a_loop_that_gave_up_mid_round_leaves_the_session_broken() {
        let broken = AtomicBool::new(false);
        let (consumer, told) = listening();

        settle(&broken, &consumer, || Err(Error::Io));

        assert!(broken.load(Ordering::Relaxed));
        assert_eq!(told.load(Ordering::Relaxed), 1);
    }

    /// The ordinary way out: the child ended, or the session is being
    /// released. Nothing broke, and a consumer told otherwise would put up a
    /// dead window over a session that merely finished.
    #[test]
    fn a_loop_that_returned_of_its_own_accord_leaves_the_session_alone() {
        let broken = AtomicBool::new(false);
        let (consumer, told) = listening();

        settle(&broken, &consumer, || Result::Ok(()));

        assert!(!broken.load(Ordering::Relaxed));
        assert_eq!(told.load(Ordering::Relaxed), 0);
    }

    /// The fuzzer cannot stand in for these: neither input crashes once the
    /// binding layer stops building invalid values out of them, so what is
    /// under test is a refusal, not a survival. cf. ADR 0012
    #[test]
    fn a_clipboard_write_that_is_not_utf8_is_refused() {
        let mut session = session();

        // `//4=` is base64 for FF FE, which is not UTF-8.
        session
            .feed(b"\x1b]52;c;//4=\x07")
            .expect("the feed to finish");

        let (events, dropped) = session.take_events();
        assert!(events.is_empty(), "non-UTF-8 payload reached the app");
        assert_eq!(dropped, 0);
    }

    #[test]
    fn a_clipboard_write_with_no_representations_leaves_the_clipboard_alone() {
        let mut session = session();

        session.feed(b"\x1b]52;c;\x07").expect("the feed to finish");

        let (events, dropped) = session.take_events();
        assert!(events.is_empty(), "a clear request reached the app");
        assert_eq!(dropped, 0);
    }

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
}
