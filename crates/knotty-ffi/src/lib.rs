//! The C ABI. The generated header (`include/knotty.h`) is the source of truth.
//!
//! Every identifier that crosses the boundary is an opaque pointer or a
//! `repr(C)` struct, and no VT engine type appears here.

use std::ptr;

use knotty_core::{Error, Session, Snapshot};

/// The snapshot's POD types. A C consumer gets these from the header; this
/// re-export is how a Rust consumer names the same layouts.
pub use knotty_core::{Attribute, Cell, Rgb, Underline};

/// ABI version of this library.
///
/// A caller reads the constant from the header it compiled against and
/// compares it with [`kt_abi_version`]. Mismatch means header and library
/// disagree about layouts, and the caller must not proceed.
pub const KT_ABI_VERSION: u32 = 2;

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
}

impl From<Error> for KtStatus {
    fn from(error: Error) -> Self {
        match error {
            Error::Engine => Self::Engine,
        }
    }
}

/// Opaque handle to a session.
pub struct KtSession(Session);

/// Opaque handle to a snapshot.
pub struct KtSnapshot(Snapshot);

/// Borrowed view of a snapshot's contents.
///
/// `cells` points at `rows * cols` cells in row-major order and stays valid
/// until the snapshot is freed.
#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct KtSnapshotView {
    /// Viewport width in cells.
    pub cols: u16,
    /// Viewport height in cells.
    pub rows: u16,
    /// Row-major grid of `rows * cols` cells.
    pub cells: *const Cell,
}

/// Return the ABI version this library was built with.
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
    match Session::new_detached(cols, rows, max_scrollback) {
        Ok(session) => {
            unsafe { *out = Box::into_raw(Box::new(KtSession(session))) };
            KtStatus::Ok
        }
        Err(error) => error.into(),
    }
}

/// Release a session. Null is a no-op.
///
/// # Safety
///
/// `session` must come from [`kt_session_new_detached`] and must not be used
/// afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_session_free(session: *mut KtSession) {
    if !session.is_null() {
        drop(unsafe { Box::from_raw(session) });
    }
}

/// Feed `len` bytes to a detached session.
///
/// Processes the whole buffer on the calling thread before returning, and
/// publishes at most one snapshot.
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

    match session.0.feed(bytes) {
        Ok(()) => KtStatus::Ok,
        Err(error) => error.into(),
    }
}

/// Take the latest snapshot, emptying the session's mailbox.
///
/// Returns `KT_STATUS_NO_VALUE` when nothing has been published since the
/// last take. On success `out` receives an owned handle, to be released with
/// [`kt_snapshot_free`]; otherwise it receives null.
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
    let Some(session) = (unsafe { session.as_ref() }) else {
        return KtStatus::NullArgument;
    };

    match session.0.take_snapshot() {
        Some(snapshot) => {
            unsafe { *out = Box::into_raw(Box::new(KtSnapshot(snapshot))) };
            KtStatus::Ok
        }
        None => KtStatus::NoValue,
    }
}

/// Release a snapshot. Null is a no-op.
///
/// # Safety
///
/// `snapshot` must come from [`kt_session_take_snapshot`] and must not be
/// used afterwards.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kt_snapshot_free(snapshot: *mut KtSnapshot) {
    if !snapshot.is_null() {
        drop(unsafe { Box::from_raw(snapshot) });
    }
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

    unsafe {
        *out = KtSnapshotView {
            cols: snapshot.0.cols,
            rows: snapshot.0.rows,
            cells: snapshot.0.cells.as_ptr(),
        }
    };
    KtStatus::Ok
}
