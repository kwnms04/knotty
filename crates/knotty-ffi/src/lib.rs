//! The C ABI. The generated header (`include/knotty.h`) is the source of truth.
//!
//! Every identifier that crosses the boundary is an opaque pointer or a
//! `repr(C)` struct, and no VT engine type appears here.

use std::ffi::c_void;
use std::ptr;

use knotty_core::{Error, Event, Session, Snapshot};

/// The snapshot's POD types. A C consumer gets these from the header; this
/// re-export is how a Rust consumer names the same layouts.
pub use knotty_core::{
    Attribute, Cell, ClipboardTarget, Cursor, CursorShape, Dirty, Rgb, Row, RowFlag,
    SelectionRange, Underline,
};

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

impl From<&Event> for KtEvent {
    fn from(event: &Event) -> Self {
        match event {
            Event::Bell => Self {
                kind: KtEventKind::Bell,
                clipboard_target: ClipboardTarget::Standard,
                text: KtText {
                    bytes: ptr::null(),
                    len: 0,
                },
            },
            Event::ClipboardWrite { target, text } => Self {
                kind: KtEventKind::ClipboardWrite,
                clipboard_target: *target,
                text: text.as_str().into(),
            },
        }
    }
}

/// What a session calls when it has something new to be taken.
///
/// `userdata` comes back exactly as it was handed to [`kt_session_set_wake`].
///
/// The call is made on the thread that drove the session, from inside the call
/// that published. **It may do nothing but wake its own thread** — a call back
/// across this boundary re-enters a session the running call still holds.
pub type KtWake = Option<extern "C" fn(userdata: *mut c_void)>;

/// ABI version of this library.
///
/// A caller reads the constant from the header it compiled against and
/// compares it with [`kt_abi_version`]. Mismatch means header and library
/// disagree about layouts, and the caller must not proceed.
pub const KT_ABI_VERSION: u32 = 6;

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
    /// The call is only for a session with no PTY behind it. No such session
    /// exists yet; the contract is fixed now so it cannot move later.
    NotDetached = 8,
    /// The queue of bytes bound for the child is at its cap, and what did not
    /// fit was dropped. Reported once per overrun, so a later call succeeding
    /// does not mean the dropped bytes came back.
    WriteQueueFull = 9,
}

impl From<Error> for KtStatus {
    fn from(error: Error) -> Self {
        match error {
            Error::Engine => Self::Engine,
            Error::TooLarge => Self::TooLarge,
            Error::OutOfRange => Self::OutOfRange,
            Error::WriteQueueFull => Self::WriteQueueFull,
        }
    }
}

/// Opaque handle to a session.
pub struct KtSession {
    session: Session,
    defunct: bool,
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
    fn guard(&mut self, call: impl FnOnce(&mut Session) -> KtStatus) -> KtStatus {
        let status = guarded(KtStatus::Panicked, || call(&mut self.session));
        if status == KtStatus::Panicked {
            self.defunct = true;
        }
        status
    }

    /// The same, for a call that gives the session input — which a defunct
    /// session no longer takes.
    fn drive(&mut self, call: impl FnOnce(&mut Session) -> Result<(), Error>) -> KtStatus {
        if self.defunct {
            return KtStatus::Defunct;
        }

        self.guard(|session| match call(session) {
            Ok(()) => KtStatus::Ok,
            Err(error) => error.into(),
        })
    }
}

/// Run a boundary call, returning `fallback` if it panics.
///
/// Unwinding into C is undefined, so nothing may leave here by that route.
fn guarded<T>(fallback: T, call: impl FnOnce() -> T) -> T {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(call)).unwrap_or(fallback)
}

/// Opaque handle to a snapshot.
pub struct KtSnapshot(Snapshot);

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
    if out.is_null() {
        return KtStatus::NullArgument;
    }
    unsafe { *out = ptr::null_mut() };
    guarded(KtStatus::Panicked, || {
        match Session::new_detached(cols, rows, max_scrollback) {
            Ok(session) => {
                let handle = KtSession {
                    session,
                    defunct: false,
                    events: Vec::new(),
                    event_views: Vec::new(),
                };
                unsafe { *out = Box::into_raw(Box::new(handle)) };
                KtStatus::Ok
            }
            Err(error) => error.into(),
        }
    })
}

/// Release a session. Null is a no-op.
///
/// # Safety
///
/// `session` must come from [`kt_session_new_detached`] and must not be used
/// afterwards.
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
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };
    let bytes = if len == 0 {
        &[][..]
    } else if bytes.is_null() {
        return KtStatus::NullArgument;
    } else {
        unsafe { std::slice::from_raw_parts(bytes, len) }
    };

    session.drive(|session| session.feed(bytes))
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
/// What was published while no callback was registered stays owed, and the
/// next publication carries it — so a consumer that attaches late is told
/// there is something to take rather than having to know to look.
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
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };

    session.guard(|session| {
        session.set_wake(wake.map(|wake| Box::new(move || wake(userdata)) as Box<dyn Fn()>));
        KtStatus::Ok
    })
}

/// Select a range of the viewport, or clear the selection by passing null.
///
/// Publishes a snapshot, since the selection is part of what a consumer draws.
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
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };

    let range = unsafe { range.as_ref() }.copied();
    session.drive(|session| session.set_selection(range))
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
    if out.is_null() {
        return KtStatus::NullArgument;
    }
    // What a call that never gets to the queue leaves behind. A successful
    // drain overwrites this, empty or not, with the session's own buffer.
    unsafe {
        *out = KtBytes {
            bytes: ptr::null(),
            len: 0,
        }
    };
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };

    session.guard(|session| {
        let queued = session.take_writes();
        unsafe {
            *out = KtBytes {
                bytes: queued.as_ptr(),
                len: queued.len(),
            }
        };
        KtStatus::Ok
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
    if out.is_null() {
        return KtStatus::NullArgument;
    }
    // What a call that never gets to the queue leaves behind. A successful
    // drain overwrites this, empty or not.
    unsafe {
        *out = KtEvents {
            events: ptr::null(),
            len: 0,
            dropped: 0,
        }
    };
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };

    // Taken out of the guarded call rather than inside it: what comes back
    // has to be stored on the handle, which the guard has already borrowed.
    let mut taken = None;
    let status = session.guard(|session| {
        taken = Some(session.take_events());
        KtStatus::Ok
    });
    let Some((events, dropped)) = taken else {
        return status;
    };

    // Dropping what the last drain lent is what invalidates its pointers,
    // which is the contract above.
    session.events = events;
    session.event_views = session.events.iter().map(KtEvent::from).collect();
    unsafe {
        *out = KtEvents {
            events: session.event_views.as_ptr(),
            len: session.event_views.len(),
            dropped,
        }
    };
    KtStatus::Ok
}

/// Take the latest snapshot, emptying the session's mailbox.
///
/// Returns `KT_STATUS_NO_VALUE` when nothing has been published since the
/// last take. On success `out` receives an owned handle, to be released with
/// [`kt_snapshot_free`]; otherwise it receives null.
///
/// Works on a defunct session: what it holds is the last state that was
/// right, and handing that back is the whole point of keeping it.
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
    if out.is_null() {
        return KtStatus::NullArgument;
    }
    unsafe { *out = ptr::null_mut() };
    let Some(session) = (unsafe { session.as_mut() }) else {
        return KtStatus::NullArgument;
    };

    // Guarded rather than driven: a defunct session still hands back what it
    // last published, which is the whole reason for keeping it.
    session.guard(|session| match session.take_snapshot() {
        Some(snapshot) => {
            unsafe { *out = Box::into_raw(Box::new(KtSnapshot(snapshot))) };
            KtStatus::Ok
        }
        None => KtStatus::NoValue,
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
    if out.is_null() {
        return KtStatus::NullArgument;
    }
    let Some(snapshot) = (unsafe { snapshot.as_ref() }) else {
        return KtStatus::NullArgument;
    };

    guarded(KtStatus::Panicked, || {
        unsafe {
            *out = KtSnapshotView {
                cols: snapshot.0.cols,
                rows: snapshot.0.rows,
                dirty: snapshot.0.dirty,
                has_selection: snapshot.0.has_selection,
                cells: snapshot.0.cells.as_ptr(),
                row_state: snapshot.0.row_state.as_ptr(),
                graphemes: snapshot.0.graphemes.as_ptr(),
                grapheme_count: snapshot.0.graphemes.len(),
                cursor: snapshot.0.screen.cursor,
                title: snapshot.0.screen.title.as_str().into(),
                pwd: snapshot.0.screen.pwd.as_str().into(),
            }
        };
        KtStatus::Ok
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A panic has to start somewhere real. `guard` is where every call that
    /// touches a session goes through, so panicking there is the same thing
    /// happening to a session that a bug in the core would do.
    fn panic_in(session: *mut KtSession) -> KtStatus {
        unsafe { &mut *session }.guard(|_| panic!("on purpose"))
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

        let mut snapshot = ptr::null_mut();
        assert_eq!(
            unsafe { kt_session_take_snapshot(session, &mut snapshot) },
            KtStatus::Ok,
            "a defunct session still hands back what it last published",
        );

        let mut view = std::mem::MaybeUninit::<KtSnapshotView>::uninit();
        assert_eq!(
            unsafe { kt_snapshot_view(snapshot, view.as_mut_ptr()) },
            KtStatus::Ok,
        );
        let view = unsafe { view.assume_init() };
        assert_eq!(unsafe { (*view.cells).codepoint }, u32::from(b'o'));

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

        unsafe { kt_session_free(session) };
    }
}
