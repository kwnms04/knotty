//! Session lifecycle and the publish path.

use std::cell::RefCell;
use std::rc::Rc;

use libghostty_vt::screen::Screen;
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::{
    ClipboardLocation, ClipboardWriteError, ConformanceLevel, DeviceAttributeFeature,
    DeviceAttributes, DeviceType, Point, PointCoordinate, PrimaryDeviceAttributes,
    SecondaryDeviceAttributes, TertiaryDeviceAttributes,
};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use crate::mailbox::Mailbox;
use crate::queue::{ClipboardTarget, Event, EventQueue};
use crate::snapshot::{self, ScreenState, Snapshot};
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
    fn push(&mut self, bytes: &[u8]) {
        if self.bytes.len() + bytes.len() > WRITE_QUEUE_CAP {
            self.overran = true;
            return;
        }
        self.bytes.extend_from_slice(bytes);
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
/// A detached session owns no thread and no child process: [`feed`] runs the
/// VT engine on the calling thread. Everything past the parser — conversion
/// and mailbox publication — is the path a PTY session will take too.
///
/// [`feed`]: Session::feed
pub struct Session {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    mailbox: Mailbox<Snapshot>,
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
    // is.
    events: Rc<RefCell<EventQueue>>,
}

impl Session {
    /// Create a session with no PTY behind it.
    pub fn new_detached(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self> {
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
        let events = Rc::new(RefCell::new(EventQueue::default()));
        terminal.on_bell({
            let events = Rc::clone(&events);
            move |_| events.borrow_mut().push(Event::Bell)
        })?;
        terminal.on_clipboard_write({
            let events = Rc::clone(&events);
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

                events.borrow_mut().push(Event::ClipboardWrite {
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
            mailbox: Mailbox::new(),
            selection_screen: None,
            last_screen: ScreenState::default(),
            writes,
            drained: Vec::new(),
            events,
        })
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

        self.publish()
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
        self.publish()?;

        if overran {
            return Err(Error::WriteQueueFull);
        }
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
        self.events.borrow_mut().take()
    }

    /// Take the latest snapshot, emptying the mailbox.
    ///
    /// Returns `None` when nothing has been published since the last take.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.mailbox.take()
    }

    /// Capture the terminal and publish it, unless nothing changed.
    fn publish(&mut self) -> Result<()> {
        let has_selection = self.has_selection()?;
        if let Some(mut snapshot) =
            snapshot::capture(&mut self.render, &self.terminal, &self.last_screen)?
        {
            snapshot.has_selection = has_selection;
            self.last_screen = snapshot.screen.clone();
            // The mailbox keeps only the newest snapshot, so publishing over
            // an unconsumed one drops it. Carry its change marks across, or a
            // consumer that misses a frame is told less changed than did.
            if let Some(dropped) = self.mailbox.take() {
                snapshot.absorb_marks_of(&dropped);
            }
            self.mailbox.publish(snapshot);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::Session;
    use crate::queue::Event;

    fn session() -> Session {
        Session::new_detached(80, 24, 0).expect("a detached session")
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
