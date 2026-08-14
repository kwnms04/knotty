//! The C ABI. The generated header (`include/knotty.h`) is the source of truth.
//!
//! Every identifier that crosses the boundary is an opaque pointer or a
//! `repr(C)` struct, and no VT engine type appears here.

use std::ptr;

use knotty_core::{Error, Session, Snapshot};

/// The snapshot's POD types. A C consumer gets these from the header; this
/// re-export is how a Rust consumer names the same layouts.
pub use knotty_core::{
    Attribute, Cell, Cursor, CursorShape, Dirty, Rgb, Row, RowFlag, SelectionRange, Underline,
};

/// Borrowed UTF-8, valid until the snapshot it came from is freed.
///
/// Not null-terminated: read `len` bytes. Control characters have already been
/// removed, so there are no interior nulls either.
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
}

impl From<Error> for KtStatus {
    fn from(error: Error) -> Self {
        match error {
            Error::Engine => Self::Engine,
            Error::TooLarge => Self::TooLarge,
            Error::OutOfRange => Self::OutOfRange,
        }
    }
}

/// Opaque handle to a session.
pub struct KtSession {
    session: Session,
    defunct: bool,
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
