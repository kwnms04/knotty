//! The public C ABI is the seam: everything here goes through the same
//! entry points a real consumer calls.

use std::mem::MaybeUninit;
use std::ptr;

use knotty_ffi::{
    KtSession, KtSnapshot, KtSnapshotView, KtStatus, kt_abi_version, kt_session_feed,
    kt_session_free, kt_session_new_detached, kt_session_take_snapshot, kt_snapshot_free,
    kt_snapshot_view,
};

fn detached(cols: u16, rows: u16) -> *mut KtSession {
    let mut session = ptr::null_mut();
    let status = unsafe { kt_session_new_detached(cols, rows, 0, &mut session) };
    assert_eq!(status, KtStatus::Ok);
    assert!(!session.is_null());
    session
}

fn feed(session: *mut KtSession, bytes: &[u8]) {
    let status = unsafe { kt_session_feed(session, bytes.as_ptr(), bytes.len()) };
    assert_eq!(status, KtStatus::Ok);
}

fn take(session: *mut KtSession) -> *mut KtSnapshot {
    let mut snapshot = ptr::null_mut();
    let status = unsafe { kt_session_take_snapshot(session, &mut snapshot) };
    assert_eq!(status, KtStatus::Ok);
    assert!(!snapshot.is_null());
    snapshot
}

fn view(snapshot: *const KtSnapshot) -> KtSnapshotView {
    let mut view = MaybeUninit::<KtSnapshotView>::uninit();
    let status = unsafe { kt_snapshot_view(snapshot, view.as_mut_ptr()) };
    assert_eq!(status, KtStatus::Ok);
    unsafe { view.assume_init() }
}

/// Read a cell the way a consumer does: index into the flat grid.
fn codepoint_at(view: &KtSnapshotView, row: u16, col: u16) -> u32 {
    let index = usize::from(row) * usize::from(view.cols) + usize::from(col);
    assert!(index < usize::from(view.rows) * usize::from(view.cols));
    unsafe { (*view.cells.add(index)).codepoint }
}

/// The handshake a consumer performs at startup, against the header text it
/// would have compiled against rather than against the Rust constant.
#[test]
fn the_library_reports_the_abi_version_the_header_declares() {
    let header = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../include/knotty.h"
    ))
    .expect("read include/knotty.h");
    let declared: u32 = header
        .lines()
        .find_map(|line| line.strip_prefix("#define KT_ABI_VERSION "))
        .expect("the header declares KT_ABI_VERSION")
        .trim()
        .parse()
        .expect("KT_ABI_VERSION is a number");

    assert_eq!(kt_abi_version(), declared);
}

#[test]
fn feeding_ascii_puts_it_in_the_grid() {
    let session = detached(8, 3);

    feed(session, b"hi\r\nthere");

    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!((view.cols, view.rows), (8, 3));
    for (col, expected) in "hi".chars().enumerate() {
        assert_eq!(codepoint_at(&view, 0, col as u16), expected as u32);
    }
    for (col, expected) in "there".chars().enumerate() {
        assert_eq!(codepoint_at(&view, 1, col as u16), expected as u32);
    }
    assert_eq!(codepoint_at(&view, 0, 2), 0, "untouched cells stay empty");
    assert_eq!(codepoint_at(&view, 2, 0), 0, "untouched rows stay empty");

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_snapshot_outlives_the_session_that_published_it() {
    let session = detached(4, 1);
    feed(session, b"ok");
    let snapshot = take(session);

    unsafe { kt_session_free(session) };

    let view = view(snapshot);
    assert_eq!(codepoint_at(&view, 0, 0), u32::from(b'o'));

    unsafe { kt_snapshot_free(snapshot) };
}

#[test]
fn taking_twice_reports_no_value_the_second_time() {
    let session = detached(4, 1);
    feed(session, b"x");

    let snapshot = take(session);
    unsafe { kt_snapshot_free(snapshot) };

    let mut second = ptr::null_mut();
    let status = unsafe { kt_session_take_snapshot(session, &mut second) };
    assert_eq!(status, KtStatus::NoValue);
    assert!(second.is_null());

    unsafe { kt_session_free(session) };
}

#[test]
fn a_feed_that_changes_nothing_publishes_nothing() {
    let session = detached(4, 1);
    feed(session, b"x");
    unsafe { kt_snapshot_free(take(session)) };

    feed(session, b"");

    let mut snapshot = ptr::null_mut();
    let status = unsafe { kt_session_take_snapshot(session, &mut snapshot) };
    assert_eq!(status, KtStatus::NoValue);

    unsafe { kt_session_free(session) };
}

#[test]
fn taking_before_anything_is_fed_reports_no_value() {
    let session = detached(4, 1);

    let mut snapshot = ptr::null_mut();
    let status = unsafe { kt_session_take_snapshot(session, &mut snapshot) };

    assert_eq!(status, KtStatus::NoValue);
    assert!(snapshot.is_null());

    unsafe { kt_session_free(session) };
}

#[test]
fn null_arguments_are_reported_rather_than_dereferenced() {
    assert_eq!(
        unsafe { kt_session_new_detached(4, 1, 0, ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_feed(ptr::null_mut(), b"x".as_ptr(), 1) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_snapshot_view(ptr::null(), ptr::null_mut()) },
        KtStatus::NullArgument,
    );

    // Freeing null is a no-op, not a crash.
    unsafe { kt_session_free(ptr::null_mut()) };
    unsafe { kt_snapshot_free(ptr::null_mut()) };
}
