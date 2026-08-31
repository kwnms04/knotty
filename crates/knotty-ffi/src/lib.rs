//! The C ABI. The generated header (`include/knotty.h`) is the source of truth.
//!
//! Every identifier that crosses the boundary is an opaque pointer or a
//! `repr(C)` struct, and no VT engine type appears here.

use std::ffi::c_void;
use std::ptr;

mod entry;

use knotty_core::{
    ChildState, Error, Event, KeyEvent, MouseEvent, PtySession, Session, Snapshot, Wake, WheelEvent,
};

/// The snapshot's POD types. A C consumer gets these from the header; this
/// re-export is how a Rust consumer names the same layouts.
pub use knotty_core::{
    Attribute, Cell, ClipboardTarget, Cursor, CursorShape, Dirty, Rgb, Row, RowFlag,
    SelectionRange, SelectionUnit, Underline,
};

/// The input path's POD types, re-exported for the same reason.
pub use knotty_core::{Key, KeyAction, Modifier, MouseAction, MouseButton};

/// Borrowed UTF-8, valid for as long as whatever lent it stays put.
///
/// Not null-terminated: read `len` bytes. What has been taken out of the text
/// is the lending field's to say: a snapshot's title and working directory
/// have had their control characters removed and so hold no interior nulls,
/// a clipboard payload is whatever the child asked to copy.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtText {
    /// The bytes.
    pub bytes: *const u8,
    /// How many of them.
    pub len: usize,
}

impl From<&str> for KtText {
    fn from(text: &str) -> Self {
        Self {
            bytes: text.as_ptr(),
            len: text.len(),
        }
    }
}

/// Borrowed bytes, valid until the call that lent them is made again.
///
/// Not a string: these are whatever the terminal put on the wire, and nothing
/// promises they are text. Read `len` bytes.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtBytes {
    /// The bytes.
    pub bytes: *const u8,
    /// How many of them.
    pub len: usize,
}

/// Which kind of event a [`KtEvent`] is, and so which of its fields carry
/// anything.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KtEventKind {
    /// The child rang the bell.
    Bell = 0,
    /// The child asked for text to be put on a clipboard.
    ClipboardWrite = 1,
    /// The child is gone.
    ChildExited = 2,
}

/// One thing that happened, whose happening is the whole of its meaning.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtEvent {
    /// Which kind of event this is.
    pub kind: KtEventKind,
    /// Which clipboard the text is bound for. Set only for a clipboard write.
    pub clipboard_target: ClipboardTarget,
    /// What to put on that clipboard, borrowed for as long as the run it came
    /// in is. Empty for any other kind.
    ///
    /// Nothing has been taken out of it: it is what the child asked to copy,
    /// control characters and all. Stripping those would eat the newlines out
    /// of a copied paragraph, and untrusted bytes are made safe where they
    /// re-enter — the paste path. cf. `docs/adr/0007-input-security.md`
    pub text: KtText,
    /// What the child exited with, or 128 plus the signal that ended it — the
    /// one number a shell reports either by. Set only for a child's exit, and
    /// 0 for any other kind.
    pub exit_code: i32,
}

/// Borrowed run of events, valid until the call that lent them is made again.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtEvents {
    /// The events, oldest first.
    pub events: *const KtEvent,
    /// How many of them.
    pub len: usize,
    /// How many events were dropped for want of room since the last take. A
    /// dropped event never makes the screen wrong: everything that has to be
    /// true is in the snapshot. The count empties with the queue, so one
    /// overrun is reported once.
    pub dropped: u64,
}

impl KtEvent {
    /// An event of `kind` carrying nothing, for the kind to fill in what it
    /// does carry.
    ///
    /// Every field is set on every event, whether or not its kind uses it — so
    /// what a kind leaves alone reads as the empty value its field documents
    /// rather than as whatever the last event put there.
    fn of(kind: KtEventKind) -> Self {
        Self {
            kind,
            clipboard_target: ClipboardTarget::Standard,
            text: KtText {
                bytes: ptr::null(),
                len: 0,
            },
            exit_code: 0,
        }
    }
}

impl From<&Event> for KtEvent {
    fn from(event: &Event) -> Self {
        match event {
            Event::Bell => Self::of(KtEventKind::Bell),
            Event::ClipboardWrite { target, text } => Self {
                clipboard_target: *target,
                text: text.as_str().into(),
                ..Self::of(KtEventKind::ClipboardWrite)
            },
            Event::ChildExited { code } => Self {
                exit_code: *code,
                ..Self::of(KtEventKind::ChildExited)
            },
        }
    }
}

/// A key event on its way in, before anything has decided what bytes it is.
///
/// The physical key rather than the character: the same key is `A` on a US
/// layout and `Ф` on a Russian one, so `⌃A` is the same place on the keyboard
/// either way. What the layout made of it travels as `text` beside it.
///
/// Which bytes it comes to is the core's to answer, because the modes it
/// depends on are the terminal's and reading them out here would read them as
/// of some earlier frame. cf. `docs/adr/0017-semantic-input-events.md`
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtKeyEvent {
    /// Which way the key moved. Only a press or a repeat encodes anything.
    pub action: KeyAction,
    /// Which key it was. `KT_KEY_UNIDENTIFIED` is refused rather than
    /// encoded.
    pub key: Key,
    /// What was held down, as `KtModifier` bits.
    pub mods: u16,
    /// Which of those the layout already spent on `text`, as `KtModifier`
    /// bits. Option making `å` out of `⌥A` on macOS is one: the modifier was
    /// held, but it is not one the terminal should encode a second time.
    pub consumed_mods: u16,
    /// Whether an input method is mid-composition. Keys are held back while
    /// it is, which is what keeps half a syllable out of the child.
    pub composing: bool,
    /// What the layout made of the key, as UTF-8, empty where it made
    /// nothing. Borrowed for the length of the call.
    ///
    /// Neither control characters nor a platform's own function key codes
    /// belong here — C0 and DEL, and on macOS the private use area
    /// `U+F700`–`U+F8FF` that AppKit puts in `NSEvent.characters` for the
    /// arrows and the F keys. The core derives all of those from the key and
    /// the modifiers, and one arriving as text is one that would be encoded
    /// twice. Leave it empty for them.
    pub text: KtText,
}

/// Whether a session has a child and what has become of it.
///
/// Read apart from [`KtSessionState`]: the two are different facts, and a
/// session whose thread panicked with its child still running is a real
/// pairing. What decides whether closing the window needs a warning is this
/// one; what decides whether the window still takes input is the other.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KtChildState {
    /// There is no child. A session with no PTY behind it is fed by its caller
    /// and has none.
    None = 0,
    /// The child is still running.
    Running = 1,
    /// The child is gone, and `child_exit_code` says what by.
    Exited = 2,
}

/// Whether a session still works.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KtSessionState {
    /// Working.
    Ok = 0,
    /// Something inside it panicked. It keeps the last screen it published and
    /// takes no more input, which comes back as `KT_STATUS_DEFUNCT`.
    Broken = 1,
}

/// What a session calls when it has something new to be taken.
///
/// `userdata` comes back exactly as it was handed to [`kt_session_set_wake`].
///
/// The call is made on the thread that drove the session, from inside the call
/// that published — the caller's own thread for a detached session, the
/// session's I/O thread for one with a PTY behind it. **It may do nothing but
/// wake its own thread**: a call back across this boundary re-enters a session
/// the running call still holds.
pub type KtWake = Option<extern "C" fn(userdata: *mut c_void)>;

/// The caller's opaque pointer, on its way to whichever thread publishes.
///
/// A PTY session wakes from its own I/O thread, so the pointer crosses one.
/// Nothing here reads it — what it points at is the caller's to keep alive,
/// which is the promise [`kt_session_set_wake`] already asks for.
struct Userdata(*mut c_void);

// SAFETY: the pointer is carried, never dereferenced. Whatever it names is the
// caller's to synchronize, and the wake contract already forbids the callback
// from doing more than flagging its own thread.
unsafe impl Send for Userdata {}

impl Userdata {
    /// Hand the pointer back for the one call it is for.
    ///
    /// A method rather than a field read, so that a closure capturing this
    /// captures the wrapper — reaching for the field directly would capture
    /// the bare pointer and lose the promise above.
    fn get(&self) -> *mut c_void {
        self.0
    }
}

/// ABI version of this library.
///
/// A caller reads the constant from the header it compiled against and
/// compares it with [`kt_abi_version`]. Mismatch means header and library
/// disagree about layouts, and the caller must not proceed.
pub const KT_ABI_VERSION: u32 = 8;

/// Outcome of a call across the boundary.
#[repr(i32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KtStatus {
    /// The call succeeded.
    Ok = 0,
    /// Nothing was available. Not a failure.
    NoValue = 1,
    /// A required pointer argument was null.
    NullArgument = 2,
    /// The VT engine rejected the operation.
    Engine = 3,
    /// The terminal's state is bigger than a snapshot can describe.
    TooLarge = 4,
    /// A coordinate fell outside the terminal.
    OutOfRange = 5,
    /// Something inside the core panicked. The call did nothing useful and
    /// the session it was made on is now defunct.
    Panicked = 6,
    /// The session already panicked. It keeps its last good snapshot but
    /// takes no more input.
    Defunct = 7,
    /// The call is only for a session with no PTY behind it. One with a PTY
    /// has its own thread doing what the call would have done.
    NotDetached = 8,
    /// The queue of bytes bound for the child is at its cap, and what did not
    /// fit was dropped. Reported once per overrun, so a later call succeeding
    /// does not mean the dropped bytes came back.
    WriteQueueFull = 9,
    /// An operating system call failed — opening a terminal, starting a child,
    /// or talking to one already started.
    Io = 10,
    /// A key event named no key. Nothing was queued for the child, and the
    /// caller has a mapping to fill in rather than a key that has no bytes:
    /// keys that encode to nothing are answered with `KT_STATUS_OK`.
    UnidentifiedKey = 11,
}

impl From<Error> for KtStatus {
    fn from(error: Error) -> Self {
        match error {
            Error::Engine => Self::Engine,
            Error::TooLarge => Self::TooLarge,
            Error::OutOfRange => Self::OutOfRange,
            Error::WriteQueueFull => Self::WriteQueueFull,
            Error::Io => Self::Io,
        }
    }
}

/// What a fallible core call comes back across the boundary as.
fn status(result: Result<(), Error>) -> KtStatus {
    match result {
        Ok(()) => KtStatus::Ok,
        Err(error) => error.into(),
    }
}

/// Who drives the engine behind a session: the caller, or a thread of the
/// session's own.
///
/// That is the whole of the difference between the two shapes a session comes
/// in, and the calls below are where it is answered — so that no entry point
/// has to know which it is holding.
enum Driver {
    /// No PTY behind it: the caller feeds the engine itself.
    Detached(Session),
    /// A child process behind a pseudoterminal, with a thread on it.
    Pty(PtySession),
}

impl Driver {
    /// Feed bytes to the engine.
    ///
    /// A PTY session takes its input from its child, so there is nothing here
    /// for a caller to push in. cf. `03-core.md` C7
    fn feed(&mut self, bytes: &[u8]) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.feed(bytes)),
            Self::Pty(_) => KtStatus::NotDetached,
        }
    }

    /// Drain the bytes queued for the child.
    ///
    /// A PTY session's I/O thread is already draining that queue into the
    /// terminal, so nothing is left for a caller to take. cf. `03-core.md` C7
    fn take_writes(&mut self) -> Result<&[u8], KtStatus> {
        match self {
            Self::Detached(session) => Ok(session.take_writes()),
            Self::Pty(_) => Err(KtStatus::NotDetached),
        }
    }

    /// Queue bytes for the child.
    fn write(&mut self, bytes: &[u8]) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.write(bytes)),
            Self::Pty(session) => status(session.write(bytes)),
        }
    }

    /// Encode a key and queue what it comes to for the child.
    fn key(&mut self, event: KeyEvent) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.key(&event)),
            Self::Pty(session) => status(session.key(event)),
        }
    }

    /// Encode a mouse event and queue what it comes to for the child.
    fn mouse(&mut self, event: MouseEvent) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.mouse(&event)),
            Self::Pty(session) => status(session.mouse(event)),
        }
    }

    /// Turn the wheel, which the terminal makes one of three things of.
    fn wheel(&mut self, event: WheelEvent) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.wheel(&event)),
            Self::Pty(session) => status(session.wheel(event)),
        }
    }

    /// Tell the child the window gained or lost focus, if it asked to hear.
    fn focus(&mut self, gained: bool) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.focus(gained)),
            Self::Pty(session) => status(session.focus(gained)),
        }
    }

    /// Resize the grid, with how big one cell now is in pixels.
    fn resize(&mut self, cols: u16, rows: u16, cell_width: u32, cell_height: u32) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.resize(cols, rows, cell_width, cell_height)),
            Self::Pty(session) => status(session.resize(cols, rows, cell_width, cell_height)),
        }
    }

    fn set_selection(&mut self, range: Option<SelectionRange>) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.set_selection(range)),
            Self::Pty(session) => status(session.set_selection(range)),
        }
    }

    /// Select from a gesture's anchor cell out to the cell it is over now.
    fn select(
        &mut self,
        anchor: (u16, u16),
        cell: (u16, u16),
        unit: SelectionUnit,
        rectangle: bool,
    ) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.select(anchor, cell, unit, rectangle)),
            Self::Pty(session) => status(session.select(anchor, cell, unit, rectangle)),
        }
    }

    /// The selection as plain text, or `None` when nothing is selected.
    fn copy_selection(&mut self) -> Result<Option<Vec<u8>>, Error> {
        match self {
            Self::Detached(session) => session.copy_selection(),
            Self::Pty(session) => session.copy_selection(),
        }
    }

    /// Move the viewport, up positive.
    fn scroll_viewport(&mut self, lines: i32) -> KtStatus {
        match self {
            Self::Detached(session) => status(session.scroll_viewport(lines)),
            Self::Pty(session) => status(session.scroll_viewport(lines)),
        }
    }

    fn set_wake(&mut self, wake: Option<Wake>) {
        match self {
            Self::Detached(session) => session.set_wake(wake),
            Self::Pty(session) => session.set_wake(wake),
        }
    }

    fn take_events(&mut self) -> (Vec<Event>, u64) {
        match self {
            Self::Detached(session) => session.take_events(),
            Self::Pty(session) => session.take_events(),
        }
    }

    fn take_snapshot(&self) -> Option<Snapshot> {
        match self {
            Self::Detached(session) => session.take_snapshot(),
            Self::Pty(session) => session.take_snapshot(),
        }
    }

    /// What has become of the session's child.
    fn child(&self) -> ChildState {
        match self {
            // Nothing is behind a detached session: the caller is what feeds
            // it, and a caller is not a child.
            Self::Detached(_) => ChildState::None,
            Self::Pty(session) => session.child(),
        }
    }

    /// Whether the session's own thread gave up, which only a session with a
    /// thread can do.
    fn broken(&self) -> bool {
        match self {
            Self::Detached(_) => false,
            Self::Pty(session) => session.broken(),
        }
    }
}

/// Opaque handle to a session.
pub struct KtSession {
    driver: Driver,
    /// Whether a call at this boundary panicked. A session with a thread of
    /// its own can break out of reach of any call, which the driver answers
    /// for; both are the same news, and [`KtSession::is_defunct`] is where
    /// they meet.
    defunct: bool,
    /// What the last copy came to. Kept alive for the reason a detached
    /// session's drained writes are: the run lent to the caller points into
    /// it.
    copied: Vec<u8>,
    /// What the last event drain took. Kept alive because the run lent to the
    /// caller borrows the text out of it rather than copying it.
    events: Vec<Event>,
    /// The lent run itself, pointing into `events`.
    event_views: Vec<KtEvent>,
}

impl KtSession {
    /// Run a call on the session, giving up on it if the call panics.
    ///
    /// A panic means an invariant broke somewhere we cannot see, so there is
    /// no way to know that carrying on is safe. What the session has already
    /// published stays where it is, because a screen that is stale beats a
    /// screen that empties.
    fn guard(&mut self, call: impl FnOnce(&mut Driver) -> KtStatus) -> KtStatus {
        let status = guarded(KtStatus::Panicked, || call(&mut self.driver));
        if status == KtStatus::Panicked {
            self.defunct = true;
        }
        status
    }

    /// The same, for a call that gives the session input — which a defunct
    /// session no longer takes.
    fn drive(&mut self, call: impl FnOnce(&mut Driver) -> KtStatus) -> KtStatus {
        if self.is_defunct() {
            return KtStatus::Defunct;
        }

        self.guard(call)
    }

    /// Whether the session is past working, whether it broke under a call made
    /// here or on a thread of its own.
    fn is_defunct(&self) -> bool {
        self.defunct || self.driver.broken()
    }
}

/// Wrap `session` in a handle the boundary can hand out.
fn handle(driver: Driver) -> *mut KtSession {
    Box::into_raw(Box::new(KtSession {
        driver,
        defunct: false,
        copied: Vec::new(),
        events: Vec::new(),
        event_views: Vec::new(),
    }))
}

/// Run a boundary call, returning `fallback` if it panics.
///
/// Unwinding into C is undefined, so nothing may leave here by that route.
fn guarded<T>(fallback: T, call: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).unwrap_or(fallback)
}

/// Opaque handle to a snapshot.
///
/// The screen is what the session published. The two states beside it are what
/// the session said of itself when the snapshot was taken, and they travel with
/// it so that a consumer draws one consistent answer rather than asking a
/// session that has moved on since.
pub struct KtSnapshot {
    frame: Snapshot,
    child: ChildState,
    session: KtSessionState,
}

/// Borrowed view of a snapshot's contents.
///
/// The pointers stay valid until the snapshot is freed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtSnapshotView {
    /// Viewport width in cells.
    pub cols: u16,
    /// Viewport height in cells.
    pub rows: u16,
    /// How much of the grid changed since the last snapshot. Can be
    /// `KT_DIRTY_CLEAN` when what changed was outside the grid.
    pub dirty: Dirty,
    /// Whether a selection exists. A selection scrolled out of the viewport
    /// still exists, so this is not the same as no row being selected.
    pub has_selection: bool,
    /// Row-major grid of `rows * cols` cells.
    pub cells: *const Cell,
    /// One entry per row: its flags and, where selected, its columns.
    pub row_state: *const Row,
    /// Codepoints for cells whose cluster did not fit in one cell. A cell
    /// carrying `KT_ATTRIBUTE_OVERFLOW` holds the index of its run's length
    /// here; the codepoints follow, base first.
    pub graphemes: *const u32,
    /// Number of entries in `graphemes`, lengths included.
    pub grapheme_count: usize,
    /// Where the cursor is and how it looks.
    pub cursor: Cursor,
    /// Window title, control characters already removed.
    pub title: KtText,
    /// Working directory as an absolute path, control characters already
    /// removed.
    pub pwd: KtText,
    /// Whether the session has a child and whether it is still running. This
    /// is the truth about the child: the exit is an event as well, but events
    /// can be dropped and this cannot.
    pub child_state: KtChildState,
    /// Whether the session still works. A broken one keeps the screen it has
    /// and refuses input.
    pub session_state: KtSessionState,
    /// What the child exited with, or 128 plus the signal that ended it — the
    /// one number a shell reports either by. Set only when `child_state` is
    /// `KT_CHILD_STATE_EXITED`, and 0 otherwise.
    pub child_exit_code: i32,
}

/// Return the ABI version this library was built with.
// Not guarded: reading a constant is the one thing here that cannot panic.
#[unsafe(no_mangle)]
pub extern "C" fn kt_abi_version() -> u32 {
    KT_ABI_VERSION
}

/// Create a session with no PTY behind it.
///
/// On success writes an owned handle to `out`, to be released with
/// [`kt_session_free`]. On failure `out` receives null.
///
/// # Safety
///
/// `out` must be a valid, writable pointer to a `KtSession *`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_new_detached(
    cols: u16,
    rows: u16,
    max_scrollback: usize,
    out: *mut *mut KtSession,
) -> KtStatus {
    entry::answer(|| {
        let out = unsafe { entry::out(out, ptr::null_mut()) }?;

        Ok(guarded(KtStatus::Panicked, || {
            match Session::new(cols, rows, max_scrollback) {
                Ok(session) => {
                    *out = handle(Driver::Detached(session));
                    KtStatus::Ok
                }
                Err(error) => error.into(),
            }
        }))
    })
}

/// Create a session with a child process behind a pseudoterminal.
///
/// `argv` is the command to run: `argv[0]` is the program, the rest its
/// arguments, and each is a run of bytes rather than a null-terminated string.
/// An empty `argv` names nothing to run and is reported as a missing argument.
/// The child starts knowing the size it was given here, so its first frame is
/// already the right shape.
///
/// The session gets a thread of its own, which reads the terminal, feeds the
/// engine, publishes, and hands the child what [`kt_session_write`] queued. A
/// call that reaches past that thread to what it owns — [`kt_session_feed`],
/// [`kt_session_take_writes`] — is refused with `KT_STATUS_NOT_DETACHED`.
///
/// On success writes an owned handle to `out`, to be released with
/// [`kt_session_free`]. On failure `out` receives null.
///
/// # Safety
///
/// `argv` must point at `argc` readable `KtText`s, each of which must point at
/// its own `len` readable bytes — null only where that length is 0. `out` must
/// be a valid, writable pointer to a `KtSession *`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_new_pty(
    cols: u16,
    rows: u16,
    max_scrollback: usize,
    argv: *const KtText,
    argc: usize,
    out: *mut *mut KtSession,
) -> KtStatus {
    entry::answer(|| {
        let out = unsafe { entry::out(out, ptr::null_mut()) }?;
        if argv.is_null() || argc == 0 {
            return Err(KtStatus::NullArgument);
        }

        // Copied rather than borrowed: the thread that runs the command
        // outlives this call, and what the caller lent does not have to.
        let argv: Vec<Vec<u8>> = unsafe { std::slice::from_raw_parts(argv, argc) }
            .iter()
            .map(|argument| unsafe { entry::borrowed(argument.bytes, argument.len) })
            .map(|argument| argument.map(<[u8]>::to_vec))
            .collect::<Result<_, _>>()?;
        let (program, args) = argv.split_first().expect("argc is not zero");

        Ok(guarded(KtStatus::Panicked, || {
            match PtySession::new(program, args, cols, rows, max_scrollback) {
                Ok(session) => {
                    *out = handle(Driver::Pty(session));
                    KtStatus::Ok
                }
                Err(error) => error.into(),
            }
        }))
    })
}

/// Release a session, stopping its I/O thread if it has one. Null is a no-op.
///
/// # Safety
///
/// `session` must come from [`kt_session_new_detached`] or
/// [`kt_session_new_pty`] and must not be used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_free(session: *mut KtSession) {
    guarded((), || {
        if !session.is_null() {
            drop(unsafe { Box::from_raw(session) });
        }
    });
}

/// Feed `len` bytes to a detached session.
///
/// Processes the whole buffer on the calling thread before returning, and
/// publishes at most one snapshot. A session with a PTY behind it takes its
/// input from that PTY, so this returns `KT_STATUS_NOT_DETACHED` for one.
///
/// Returns `KT_STATUS_WRITE_QUEUE_FULL` when the terminal's answers to what
/// was fed did not fit in the writer queue. The snapshot is published either
/// way: what the child missed hearing does not make the frame wrong.
///
/// # Safety
///
/// `session` must be a live handle, and `bytes` must point at `len` readable
/// bytes. `bytes` may be null only when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_feed(
    session: *mut KtSession,
    bytes: *const u8,
    len: usize,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;
        let bytes = unsafe { entry::borrowed(bytes, len) }?;

        Ok(session.drive(|driver| driver.feed(bytes)))
    })
}

/// Queue `len` bytes for the session's child.
///
/// Returns as soon as they are queued rather than waiting on the child to
/// read them: a session with a PTY behind it hands them over on its own
/// thread, and a detached one has them collected by
/// [`kt_session_take_writes`] alongside what the terminal answered.
///
/// Returns `KT_STATUS_WRITE_QUEUE_FULL` when they did not fit, in which case
/// none of them were queued — a prefix of what the user typed reaching the
/// child is worse than none of it.
///
/// # Safety
///
/// `session` must be a live handle, and `bytes` must point at `len` readable
/// bytes. `bytes` may be null only when `len` is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_write(
    session: *mut KtSession,
    bytes: *const u8,
    len: usize,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;
        let bytes = unsafe { entry::borrowed(bytes, len) }?;

        Ok(session.drive(|driver| driver.write(bytes)))
    })
}

/// Encode a key event and queue what it comes to for the session's child.
///
/// The encoding is the core's, taken with the terminal's own modes in hand:
/// the same arrow key is `ESC [ A` at a prompt and `ESC O A` in an editor
/// that asked for cursor key application mode, and a caller never has to know
/// which. cf. `docs/adr/0017-semantic-input-events.md`
///
/// A key that comes to nothing queues nothing and answers `KT_STATUS_OK` — a
/// bare modifier, a release, and every key at all while an input method is
/// composing. A key that names nothing answers
/// `KT_STATUS_UNIDENTIFIED_KEY` instead, so that a mapping missing from the
/// caller is heard about where it happens rather than found later in a key
/// that quietly does nothing.
///
/// A key also brings the viewport back to the active area, the way every
/// terminal does: a screen scrolled back into the history is one the next
/// command would run off the bottom of, and output arriving does not bring it
/// down — that is what having scrolled back is for.
///
/// A detached session encodes on the calling thread, so it answers
/// `KT_STATUS_WRITE_QUEUE_FULL` when the bytes did not fit, as
/// [`kt_session_write`] does. A session with a PTY behind it encodes on its
/// own thread and is past answering by the time it finds out, the way
/// [`kt_session_set_selection`] is — and a queue that full on one of those is
/// the loop's own to shed, since the loop is the only thing that drains it.
///
/// # Safety
///
/// `session` must be a live handle, and `event` must point at a readable
/// `KtKeyEvent` whose text points at its own `len` readable bytes — null only
/// where that length is 0.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_key(
    session: *mut KtSession,
    event: *const KtKeyEvent,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;
        let event = unsafe { entry::at(event) }?;
        // Checked here rather than deeper down because it is the one thing
        // about a key that needs no terminal to answer — and a session with a
        // PTY behind it encodes on its own thread, where there is no longer
        // anybody to answer to.
        if event.key == Key::Unidentified {
            return Err(KtStatus::UnidentifiedKey);
        }
        let text = unsafe { entry::borrowed(event.text.bytes, event.text.len) }?;

        // Copied rather than borrowed: a session with a PTY behind it applies
        // this on its own thread, which outlives the call the text was lent
        // for.
        let event = KeyEvent {
            action: event.action,
            key: event.key,
            mods: event.mods,
            consumed_mods: event.consumed_mods,
            text: text.to_vec(),
            composing: event.composing,
        };
        Ok(session.drive(|driver| driver.key(event)))
    })
}

/// Hand the session a mouse event over the cell at `x`, `y`.
///
/// Cells rather than pixels: turning one into the other wants the metrics,
/// and those belong where the display is. `x` counts from the left of the
/// viewport and `y` from the top, and a position past the edge is clamped to
/// it.
///
/// **Nothing is queued while the child has not asked to hear about the
/// mouse**, which is most of the time and answers `KT_STATUS_OK` all the
/// same: a click at a shell prompt is the terminal's, and the mode saying so
/// is read here rather than above — the sequence that turns reporting on is
/// output, and a click arriving right behind it has to be read against what
/// that left. cf. `docs/adr/0017-semantic-input-events.md`
///
/// Which of the five reporting formats a click that does report is written in
/// is the terminal's too, as is whether a motion reports at all.
///
/// `mods` is `KtModifier` bits. `button` may be `KT_MOUSE_BUTTON_NONE` only
/// for a motion, which is what a pointer moving with nothing held is.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_mouse(
    session: *mut KtSession,
    action: MouseAction,
    button: MouseButton,
    mods: u16,
    x: u16,
    y: u16,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        let event = MouseEvent {
            action,
            button,
            mods,
            x,
            y,
        };
        Ok(session.drive(|driver| driver.mouse(event)))
    })
}

/// Turn the wheel over the cell at `x`, `y`.
///
/// **Both deltas are in lines**, and up and right are positive. A trackpad
/// reports its inertia in pixels and reports a great many of them; dividing
/// those by the height a line is drawn at belongs to whoever knows that
/// height, and calling here only when the count of lines changed is what
/// keeps a flick off this path a hundred times over.
///
/// What the child hears is one of three things, and the terminal is what says
/// which. With mouse reporting on it is a mouse code, one per line, because a
/// program that asked to hear about the mouse asked about this too. On the
/// alternate screen with alternate scroll left on it is cursor keys, which is
/// how a pager that never asked for the mouse still scrolls — and they are
/// the same arrows the keyboard sends, application mode included. Otherwise
/// it is nobody's but the terminal's: the viewport moves into the scrollback
/// and a snapshot is published, so the app holds no scroll position of its
/// own.
///
/// `mods` is `KtModifier` bits.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_wheel(
    session: *mut KtSession,
    delta_x: i32,
    delta_y: i32,
    x: u16,
    y: u16,
    mods: u16,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        let event = WheelEvent {
            delta_x,
            delta_y,
            mods,
            x,
            y,
        };
        Ok(session.drive(|driver| driver.wheel(event)))
    })
}

/// Tell the session the window gained or lost focus.
///
/// **Nothing is queued while focus reporting is off**, which is the usual
/// case and answers `KT_STATUS_OK`. The gate is here for the reason
/// [`kt_session_mouse`]'s is: the mode belongs to the terminal, and reading
/// it above would read it as of some earlier frame.
///
/// vim's `autoread` lives down this path — a file changed by something else
/// is re-read when the window comes back.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_focus(session: *mut KtSession, gained: bool) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.drive(|driver| driver.focus(gained)))
    })
}

/// Resize the terminal's grid, and say how big one cell now is in pixels.
///
/// The primary screen reflows: a line longer than the new width folds rather
/// than losing its tail, and widening unfolds it again. The alternate screen
/// does not — a full-screen program redraws itself for the size it is told
/// about, which is what the resize tells it.
///
/// **Call this only when what it carries has changed.** Reflow is one of the
/// two exceptions to the boundary's non-blocking contract and costs what the
/// scrollback is long, so a window being dragged must not reach here on every
/// pixel. Only the counts moving rewraps anything — a call carrying nothing
/// but a new cell size costs none of it — but keeping the idle calls out is
/// the caller's. cf. `docs/02-ffi.md`
///
/// The pixel size is a cell's, not the grid's. It is what an in-band size
/// report carries and what fills in the pseudoterminal's own pixel fields, so
/// that a program asking the terminal how big it is in pixels gets an answer
/// rather than a zero. Zero is how a caller says it does not know.
///
/// A session with a PTY behind it also tells its child, which is the
/// `SIGWINCH` that makes an editor redraw. Reflow and that telling are one
/// request on its own thread, so this returns once it is queued and an
/// engine that refused the size comes back as a frame that did not change
/// shape rather than as a status.
///
/// A grid of no columns or no rows is refused with `KT_STATUS_ENGINE`.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_resize(
    session: *mut KtSession,
    cols: u16,
    rows: u16,
    cell_width: u32,
    cell_height: u32,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.drive(|driver| driver.resize(cols, rows, cell_width, cell_height)))
    })
}

/// Register what a session calls when it has something new to be taken, or
/// clear it by passing null.
///
/// Called once per publication that left something behind — a new snapshot, a
/// new event, or both. A feed that changed nothing calls nothing, so a
/// consumer that draws on this never draws a frame it did not need.
///
/// Wakes coalesce, so on each one take the snapshot and drain the queues until
/// they are empty.
///
/// While the child holds a synchronized output block open the call is held
/// back, and the close of the block makes it exactly once — a frame published
/// inside a block is a half-drawn screen, and the newest is the only one a
/// consumer would have got anyway.
///
/// What fell due while no callback was registered stays owed, and registering
/// one pays it before this call returns — so a consumer that attaches late is
/// told there is something to take rather than having to know to look. A wake
/// a synchronized output block is holding back has not fallen due yet, and
/// goes out with the close of the block as it would have anyway.
///
/// # Safety
///
/// `session` must be a live handle. `userdata` is never read here, only handed
/// back, but whatever it points at must outlive the session or be cleared out
/// of it first.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_set_wake(
    session: *mut KtSession,
    wake: KtWake,
    userdata: *mut c_void,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.guard(|driver| {
            driver.set_wake(wake.map(|wake| {
                let userdata = Userdata(userdata);
                Box::new(move || wake(userdata.get())) as Wake
            }));
            KtStatus::Ok
        }))
    })
}

/// Select a range of the viewport, or clear the selection by passing null.
///
/// Publishes a snapshot, since the selection is part of what a consumer draws.
///
/// A session with a PTY behind it applies this on its own thread, so the call
/// returns once the request is queued and an endpoint outside the viewport
/// comes back as a wake with nothing new selected rather than as
/// `KT_STATUS_OUT_OF_RANGE`.
///
/// # Safety
///
/// `session` must be a live handle, and `range` must be null or point at a
/// readable `KtSelectionRange`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_set_selection(
    session: *mut KtSession,
    range: *const SelectionRange,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;
        // Null is how a caller clears the selection, so this is the one
        // pointer here that is allowed to be one.
        let range = unsafe { range.as_ref() }.copied();

        Ok(session.drive(|driver| driver.set_selection(range)))
    })
}

/// Select from the cell a gesture began on out to the cell it is over now,
/// measured in `unit`.
///
/// **Both ends together, not one.** A word or a line is widened from both, so
/// a call naming only the cell under the pointer would have nothing to widen
/// from — and the selection collapses the moment the pointer crosses a space.
/// The anchor is the app's to keep, along with the click count `unit` comes
/// from and whether a drag is under way; where the boundaries fall is the
/// engine's, and the app never counts one itself. cf.
/// `docs/adr/0017-semantic-input-events.md`
///
/// The pair also records which way the drag went, so dragging back past the
/// anchor reverses the selection rather than emptying it. `rectangle` makes
/// the two ends opposite corners of a block instead of the ends of a run of
/// text.
///
/// The selection the engine installs is tracked, so output scrolling it into
/// the scrollback leaves it over the same text.
///
/// **Nothing is selected while the child has asked to hear about the mouse**,
/// which answers `KT_STATUS_OK` all the same: a drag inside an editor is the
/// editor's, and a highlight of the terminal's own over its selection would
/// be two answers to one drag. The mode is read beside the terminal for the
/// reason [`kt_session_mouse`]'s is.
///
/// Coordinates are viewport cells counted from the top left, and one past an
/// edge is clamped to it — a drag out of the window is a pointer past the
/// edge, and the edge is what it means.
///
/// Publishes a snapshot when something was selected, since the selection is
/// part of what a consumer draws. [`kt_session_set_selection`] stays what it
/// is: the path for a selection nobody gestured — ⌘A and the like — and it is
/// ungated in both directions. Letting a selection go is never the program's
/// business, so a click that clears one clears it whether or not the child is
/// hearing about the mouse.
///
/// A session with a PTY behind it applies this on its own thread, so the call
/// returns once the request is queued and an endpoint outside the viewport
/// comes back as a wake with nothing new selected rather than as
/// `KT_STATUS_OUT_OF_RANGE`.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_select(
    session: *mut KtSession,
    anchor_x: u16,
    anchor_y: u16,
    x: u16,
    y: u16,
    unit: SelectionUnit,
    rectangle: bool,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.drive(|driver| driver.select((anchor_x, anchor_y), (x, y), unit, rectangle)))
    })
}

/// Take the selection as plain text.
///
/// `out` receives a run borrowed from the session, valid until the next call
/// to this function on it or until the session is freed. Nothing selected is
/// `KT_STATUS_NO_VALUE` with an empty run, which is not a failure — it is the
/// answer a copy with no selection gets.
///
/// Plain text and nothing else. Folded lines come back as the one line they
/// were typed as, and trailing blanks are trimmed, which is what makes a
/// paste of a copied paragraph the paragraph. The engine can write VT and
/// HTML too; v1's clipboard carries `text/plain`.
///
/// **A session with a PTY behind it waits for its thread here**, which is the
/// one call at this boundary that does. Every other one puts a request down
/// and lets the frame carry the answer; this one has an answer that is not
/// the screen. The wait is a round of that thread's loop, and the call is a
/// key the user pressed once.
///
/// # Safety
///
/// `session` must be a live handle and `out` must be a valid, writable
/// pointer to a `KtBytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_copy_selection(
    session: *mut KtSession,
    out: *mut KtBytes,
) -> KtStatus {
    entry::answer(|| {
        // What a call that never reaches the selection leaves behind, written
        // before anything that can fail.
        let out = unsafe {
            entry::out(
                out,
                KtBytes {
                    bytes: ptr::null(),
                    len: 0,
                },
            )
        }?;
        let session = unsafe { entry::at_mut(session) }?;
        // What `drive` does, spelled out: a defunct session takes no more
        // calls, and a panic in this one makes it defunct. It cannot be
        // `drive` itself, which answers with a status and this one has a
        // value to bring back as well.
        if session.is_defunct() {
            return Err(KtStatus::Defunct);
        }

        let mut taken = None;
        let status = session.guard(|driver| match driver.copy_selection() {
            Ok(text) => {
                taken = text;
                KtStatus::Ok
            }
            Err(error) => error.into(),
        });
        if status != KtStatus::Ok {
            return Ok(status);
        }
        let Some(text) = taken else {
            return Ok(KtStatus::NoValue);
        };

        session.copied = text;
        *out = KtBytes {
            bytes: session.copied.as_ptr(),
            len: session.copied.len(),
        };
        Ok(KtStatus::Ok)
    })
}

/// Move the viewport `lines` lines into the scrollback, up positive.
///
/// What a selection drag out of the window asks for. It cannot be an event:
/// the pointer has stopped moving and the screen still has to keep coming, so
/// the timer that calls this is the app's — and so is deciding which way, out
/// of where the pointer left. cf. `docs/05-swift-app.md`
///
/// Clamped by the engine at either end, so asking to go past the top of the
/// history or the bottom of the active area does nothing and publishes
/// nothing.
///
/// # Safety
///
/// `session` must be a live handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_scroll_viewport(
    session: *mut KtSession,
    lines: i32,
) -> KtStatus {
    entry::answer(|| {
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.drive(|driver| driver.scroll_viewport(lines)))
    })
}

/// Take the bytes a detached session has queued for its child, emptying the
/// queue.
///
/// `out` receives a run borrowed from the session, valid until the next call
/// to this function on it or until the session is freed. A length of 0 means
/// nothing was queued, which is not a failure. A session with a PTY behind it
/// has its own reader draining the queue, so this returns
/// `KT_STATUS_NOT_DETACHED` for one.
///
/// Works on a defunct session, for the same reason taking its snapshot does:
/// what it queued before it broke is still what it queued.
///
/// # Safety
///
/// `session` must be a live handle and `out` must be a valid, writable
/// pointer to a `KtBytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_take_writes(
    session: *mut KtSession,
    out: *mut KtBytes,
) -> KtStatus {
    entry::answer(|| {
        // What a call that never gets to the queue leaves behind. A successful
        // drain overwrites it, empty or not, with the session's own buffer.
        let out = unsafe {
            entry::out(
                out,
                KtBytes {
                    bytes: ptr::null(),
                    len: 0,
                },
            )
        }?;
        let session = unsafe { entry::at_mut(session) }?;

        Ok(session.guard(|driver| match driver.take_writes() {
            Ok(queued) => {
                *out = KtBytes {
                    bytes: queued.as_ptr(),
                    len: queued.len(),
                };
                KtStatus::Ok
            }
            Err(refusal) => refusal,
        }))
    })
}

/// Take the events a session has queued for the app, emptying the queue.
///
/// `out` receives a run borrowed from the session, valid until the next call
/// to this function on it or until the session is freed, along with the
/// number of events dropped for want of room since the last take. A length of
/// 0 means nothing was queued, which is not a failure.
///
/// Unlike the writer queue this is not a detached-only drain: events are the
/// app's to consume, and a session with a PTY behind it has no one else to
/// consume them. Drain until the queue is empty on every wake.
///
/// Works on a defunct session, for the same reason taking its snapshot does:
/// what it queued before it broke is still what it queued.
///
/// # Safety
///
/// `session` must be a live handle and `out` must be a valid, writable
/// pointer to a `KtEvents`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_take_events(
    session: *mut KtSession,
    out: *mut KtEvents,
) -> KtStatus {
    entry::answer(|| {
        // What a call that never gets to the queue leaves behind. A successful
        // drain overwrites it, empty or not.
        let out = unsafe {
            entry::out(
                out,
                KtEvents {
                    events: ptr::null(),
                    len: 0,
                    dropped: 0,
                },
            )
        }?;
        let session = unsafe { entry::at_mut(session) }?;

        // Taken out of the guarded call rather than inside it: what comes back
        // has to be stored on the handle, which the guard has already
        // borrowed.
        let mut taken = None;
        let status = session.guard(|driver| {
            taken = Some(driver.take_events());
            KtStatus::Ok
        });
        let Some((events, dropped)) = taken else {
            return Ok(status);
        };

        // Dropping what the last drain lent is what invalidates its pointers,
        // which is the contract above.
        session.events = events;
        session.event_views = session.events.iter().map(KtEvent::from).collect();
        *out = KtEvents {
            events: session.event_views.as_ptr(),
            len: session.event_views.len(),
            dropped,
        };
        Ok(KtStatus::Ok)
    })
}

/// Take the latest snapshot, emptying the session's mailbox.
///
/// Returns `KT_STATUS_NO_VALUE` when nothing has been published since the
/// last take, or `KT_STATUS_DEFUNCT` when nothing has been published and the
/// session is past working — a broken session publishes no more, so a bare
/// "nothing new" would be the last thing a consumer ever heard from one. On
/// success `out` receives an owned handle, to be released with
/// [`kt_snapshot_free`]; otherwise it receives null.
///
/// Works on a defunct session: what it holds is the last state that was
/// right, and handing that back is the whole point of keeping it. The snapshot
/// says so — a session that broke while its child went on running reports both
/// on the frame it hands over.
///
/// # Safety
///
/// `session` must be a live handle and `out` must be a valid, writable
/// pointer to a `KtSnapshot *`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_take_snapshot(
    session: *mut KtSession,
    out: *mut *mut KtSnapshot,
) -> KtStatus {
    entry::answer(|| {
        let out = unsafe { entry::out(out, ptr::null_mut()) }?;
        let session = unsafe { entry::at_mut(session) }?;

        // Taken out of the guarded call rather than inside it, the way the
        // event drain is: what the frame is stamped with has to be read off
        // the session the guard has already borrowed.
        //
        // Guarded rather than driven: a defunct session still hands back what
        // it last published, which is the whole reason for keeping it.
        let mut taken = None;
        let status = session.guard(|driver| {
            taken = driver.take_snapshot();
            KtStatus::Ok
        });
        let Some(frame) = taken else {
            // A break with an empty mailbox is the one case where the state
            // has no frame to ride on, and no frame is coming to carry it
            // later.
            return Ok(match status {
                KtStatus::Ok if session.is_defunct() => KtStatus::Defunct,
                KtStatus::Ok => KtStatus::NoValue,
                // The take itself is what broke, and that is what to report.
                panicked => panicked,
            });
        };

        // Read after the frame rather than before it. The I/O thread writes
        // the child's end down before publishing the frame that carries it, so
        // a frame taken after that write is one whose state is already set —
        // and reading first could hand that very frame over stamped as though
        // the child were still running, with no frame after it to put that
        // right. The other way round costs a screen one frame behind its own
        // state, which the next take settles. cf. `03-core.md` C6
        let child = session.driver.child();
        let state = if session.is_defunct() {
            KtSessionState::Broken
        } else {
            KtSessionState::Ok
        };

        *out = Box::into_raw(Box::new(KtSnapshot {
            frame,
            child,
            session: state,
        }));
        Ok(KtStatus::Ok)
    })
}

/// Release a snapshot. Null is a no-op.
///
/// # Safety
///
/// `snapshot` must come from [`kt_session_take_snapshot`] and must not be
/// used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_snapshot_free(snapshot: *mut KtSnapshot) {
    guarded((), || {
        if !snapshot.is_null() {
            drop(unsafe { Box::from_raw(snapshot) });
        }
    });
}

/// Fill `out` with a view of the snapshot's contents.
///
/// # Safety
///
/// `snapshot` must be a live handle and `out` must be a valid, writable
/// pointer to a `KtSnapshotView`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_snapshot_view(
    snapshot: *const KtSnapshot,
    out: *mut KtSnapshotView,
) -> KtStatus {
    entry::answer(|| {
        let out = unsafe { entry::at_mut(out) }?;
        let snapshot = unsafe { entry::at(snapshot) }?;

        // The code is carried in a field of its own, so a kind that has none
        // reads as 0 rather than as whatever the last one left — the same rule
        // an event is filled in by.
        let (child_state, child_exit_code) = match snapshot.child {
            ChildState::None => (KtChildState::None, 0),
            ChildState::Running => (KtChildState::Running, 0),
            ChildState::Exited(code) => (KtChildState::Exited, code),
        };

        Ok(guarded(KtStatus::Panicked, || {
            *out = KtSnapshotView {
                cols: snapshot.frame.cols,
                rows: snapshot.frame.rows,
                dirty: snapshot.frame.dirty,
                has_selection: snapshot.frame.has_selection,
                cells: snapshot.frame.cells.as_ptr(),
                row_state: snapshot.frame.row_state.as_ptr(),
                graphemes: snapshot.frame.graphemes.as_ptr(),
                grapheme_count: snapshot.frame.graphemes.len(),
                cursor: snapshot.frame.screen.cursor,
                title: snapshot.frame.screen.title.as_str().into(),
                pwd: snapshot.frame.screen.pwd.as_str().into(),
                child_state,
                session_state: snapshot.session,
                child_exit_code,
            };
            KtStatus::Ok
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic has to start somewhere real. `guard` is where every call that
    /// touches a session goes through, so panicking there is the same thing
    /// happening to a session that a bug in the core would do.
    ///
    /// A panic on a PTY session's own thread reaches the same state by the
    /// other road, and the core is where that one is tested — no call of the
    /// app's is anywhere near it when it happens.
    fn panic_in(session: *mut KtSession) -> KtStatus {
        unsafe { &mut *session }.guard(|_| panic!("on purpose"))
    }

    /// Read a snapshot the way a consumer does. What it points at stays the
    /// snapshot's, so the caller frees that after this and not before.
    fn view_of(snapshot: *mut KtSnapshot) -> KtSnapshotView {
        let mut view = std::mem::MaybeUninit::<KtSnapshotView>::uninit();
        assert_eq!(
            unsafe { kt_snapshot_view(snapshot, view.as_mut_ptr()) },
            KtStatus::Ok,
        );
        unsafe { view.assume_init() }
    }

    fn take(session: *mut KtSession) -> *mut KtSnapshot {
        let mut snapshot = ptr::null_mut();
        assert_eq!(
            unsafe { kt_session_take_snapshot(session, &mut snapshot) },
            KtStatus::Ok,
        );
        snapshot
    }

    fn detached() -> *mut KtSession {
        let mut session = ptr::null_mut();
        assert_eq!(
            unsafe { kt_session_new_detached(4, 1, 0, &mut session) },
            KtStatus::Ok,
        );
        session
    }

    #[test]
    fn a_panic_comes_back_as_a_status() {
        let session = detached();

        // Reaching the next line at all is most of the point: unwinding into
        // C would have taken the process with it.
        assert_eq!(panic_in(session), KtStatus::Panicked);

        unsafe { kt_session_free(session) };
    }

    #[test]
    fn a_panicked_session_keeps_its_last_good_snapshot() {
        let session = detached();
        assert_eq!(
            unsafe { kt_session_feed(session, b"ok".as_ptr(), 2) },
            KtStatus::Ok,
        );
        assert_eq!(panic_in(session), KtStatus::Panicked);

        // A defunct session still hands back what it last published.
        let snapshot = take(session);
        let view = view_of(snapshot);

        assert_eq!(unsafe { (*view.cells).codepoint }, u32::from(b'o'));
        assert_eq!(
            view.session_state,
            KtSessionState::Broken,
            "the screen came back with nothing to say it can no longer be trusted",
        );

        unsafe { kt_snapshot_free(snapshot) };
        unsafe { kt_session_free(session) };
    }

    /// The frame is what carries the state, and a broken session publishes no
    /// more of them — so a consumer that had already taken the last one would
    /// hear "nothing new" and go on drawing a window that is dead. The refusal
    /// is what it hears instead.
    #[test]
    fn a_broken_session_with_nothing_left_to_hand_over_says_that_rather_than_no_value() {
        let session = detached();
        assert_eq!(
            unsafe { kt_session_feed(session, b"ok".as_ptr(), 2) },
            KtStatus::Ok,
        );
        assert_eq!(panic_in(session), KtStatus::Panicked);

        // The one frame there was, taken the way a consumer takes it.
        unsafe { kt_snapshot_free(take(session)) };

        let mut snapshot = ptr::null_mut();
        assert_eq!(
            unsafe { kt_session_take_snapshot(session, &mut snapshot) },
            KtStatus::Defunct,
        );
        assert!(snapshot.is_null());

        unsafe { kt_session_free(session) };
    }

    /// A detached session is fed by its caller, so there is no child to be
    /// warned about on the way out — which is not the same fact as a child
    /// that has ended.
    #[test]
    fn a_detached_session_reports_no_child_and_a_session_that_works() {
        let session = detached();
        assert_eq!(
            unsafe { kt_session_feed(session, b"ok".as_ptr(), 2) },
            KtStatus::Ok,
        );

        let snapshot = take(session);
        let view = view_of(snapshot);

        assert_eq!(view.child_state, KtChildState::None);
        assert_eq!(view.child_exit_code, 0);
        assert_eq!(view.session_state, KtSessionState::Ok);

        unsafe { kt_snapshot_free(snapshot) };
        unsafe { kt_session_free(session) };
    }

    #[test]
    fn a_panicked_session_takes_no_more_input() {
        let session = detached();
        assert_eq!(panic_in(session), KtStatus::Panicked);

        // Defunct rather than Panicked: nothing blew up this time, the
        // session is simply already gone.
        assert_eq!(
            unsafe { kt_session_feed(session, b"x".as_ptr(), 1) },
            KtStatus::Defunct,
        );
        assert_eq!(
            unsafe { kt_session_set_selection(session, ptr::null()) },
            KtStatus::Defunct,
        );
        assert_eq!(
            unsafe {
                kt_session_key(
                    session,
                    &KtKeyEvent {
                        action: KeyAction::Press,
                        key: Key::A,
                        mods: 0,
                        consumed_mods: 0,
                        composing: false,
                        text: KtText {
                            bytes: ptr::null(),
                            len: 0,
                        },
                    },
                )
            },
            KtStatus::Defunct,
        );

        unsafe { kt_session_free(session) };
    }
}
