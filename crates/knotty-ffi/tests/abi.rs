//! The public C ABI is the seam: everything here goes through the same
//! entry points a real consumer calls.

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use rustix::process::{Pid, test_kill_process};

use knotty_ffi::{
    Attribute, Cell, ClipboardTarget, Cursor, CursorShape, Dirty, KtBytes, KtChildState,
    KtEventKind, KtEvents, KtSession, KtSessionState, KtSnapshot, KtSnapshotView, KtStatus, KtText,
    Rgb, Row, RowFlag, SelectionRange, Underline, kt_abi_version, kt_session_feed, kt_session_free,
    kt_session_new_detached, kt_session_new_pty, kt_session_set_selection, kt_session_set_wake,
    kt_session_take_events, kt_session_take_snapshot, kt_session_take_writes, kt_session_write,
    kt_snapshot_free, kt_snapshot_view,
};

/// Ghostty's own defaults. A change here is an upstream palette change, not a
/// knotty bug — but it moves every golden snapshot, so it must be deliberate.
const DEFAULT_FOREGROUND: Rgb = Rgb {
    r: 255,
    g: 255,
    b: 255,
};
const DEFAULT_BACKGROUND: Rgb = Rgb { r: 0, g: 0, b: 0 };
const PALETTE_RED: Rgb = Rgb {
    r: 204,
    g: 102,
    b: 102,
};
const PALETTE_BRIGHT_RED: Rgb = Rgb {
    r: 213,
    g: 78,
    b: 83,
};
const PALETTE_BLUE: Rgb = Rgb {
    r: 129,
    g: 162,
    b: 190,
};

fn detached(cols: u16, rows: u16) -> *mut KtSession {
    detached_with_scrollback(cols, rows, 0)
}

fn detached_with_scrollback(cols: u16, rows: u16, scrollback: usize) -> *mut KtSession {
    let mut session = ptr::null_mut();
    let status = unsafe { kt_session_new_detached(cols, rows, scrollback, &mut session) };
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
fn cell_at(view: &KtSnapshotView, row: u16, col: u16) -> Cell {
    let index = usize::from(row) * usize::from(view.cols) + usize::from(col);
    assert!(index < usize::from(view.rows) * usize::from(view.cols));
    unsafe { *view.cells.add(index) }
}

fn codepoint_at(view: &KtSnapshotView, row: u16, col: u16) -> u32 {
    cell_at(view, row, col).codepoint
}

fn row_state(view: &KtSnapshotView) -> Vec<Row> {
    (0..usize::from(view.rows))
        .map(|row| unsafe { *view.row_state.add(row) })
        .collect()
}

fn rows_with(view: &KtSnapshotView, flag: RowFlag) -> Vec<bool> {
    row_state(view)
        .iter()
        .map(|row| row.flags & flag as u8 != 0)
        .collect()
}

/// Read the selection the way a consumer does: beside the cells, never out of
/// them.
fn selected_columns(view: &KtSnapshotView) -> Vec<Option<(u16, u16)>> {
    row_state(view)
        .iter()
        .map(|row| {
            (row.flags & RowFlag::Selected as u8 != 0)
                .then_some((row.selection_start, row.selection_end))
        })
        .collect()
}

fn set_selection(session: *mut KtSession, range: Option<SelectionRange>) -> KtStatus {
    match range {
        Some(range) => unsafe { kt_session_set_selection(session, &range) },
        None => unsafe { kt_session_set_selection(session, ptr::null()) },
    }
}

/// Drain the writer queue, copying it out so the next drain cannot move it
/// under the caller.
fn writes(session: *mut KtSession) -> Vec<u8> {
    let mut queued = MaybeUninit::<KtBytes>::uninit();
    let status = unsafe { kt_session_take_writes(session, queued.as_mut_ptr()) };
    assert_eq!(status, KtStatus::Ok);

    let queued = unsafe { queued.assume_init() };
    if queued.len == 0 {
        return Vec::new();
    }
    unsafe { std::slice::from_raw_parts(queued.bytes, queued.len) }.to_vec()
}

/// What one event said, copied out so it outlives the drain that lent it.
#[derive(Debug, PartialEq, Eq)]
struct Happened {
    kind: KtEventKind,
    clipboard_target: ClipboardTarget,
    text: String,
    exit_code: i32,
}

fn bell() -> Happened {
    Happened {
        kind: KtEventKind::Bell,
        clipboard_target: ClipboardTarget::Standard,
        text: String::new(),
        exit_code: 0,
    }
}

fn clipboard_write(target: ClipboardTarget, text: &str) -> Happened {
    Happened {
        kind: KtEventKind::ClipboardWrite,
        clipboard_target: target,
        text: text.to_owned(),
        exit_code: 0,
    }
}

fn child_exited(code: i32) -> Happened {
    Happened {
        kind: KtEventKind::ChildExited,
        clipboard_target: ClipboardTarget::Standard,
        text: String::new(),
        exit_code: code,
    }
}

/// Drain the event queue, copying it out so the next drain cannot move it
/// under the caller. Returns what was queued and what was dropped.
fn events(session: *mut KtSession) -> (Vec<Happened>, u64) {
    let mut queued = MaybeUninit::<KtEvents>::uninit();
    let status = unsafe { kt_session_take_events(session, queued.as_mut_ptr()) };
    assert_eq!(status, KtStatus::Ok);

    let queued = unsafe { queued.assume_init() };
    if queued.len == 0 {
        return (Vec::new(), queued.dropped);
    }
    let taken = unsafe { std::slice::from_raw_parts(queued.events, queued.len) }
        .iter()
        .map(|event| Happened {
            kind: event.kind,
            clipboard_target: event.clipboard_target,
            text: match event.text.len {
                0 => String::new(),
                _ => text(event.text),
            },
            exit_code: event.exit_code,
        })
        .collect();
    (taken, queued.dropped)
}

/// All a real wake callback is allowed to do is set a flag its own thread
/// reads. Counting is that, plus enough to tell one wake from two.
extern "C" fn count_a_wake(userdata: *mut c_void) {
    unsafe { *userdata.cast::<u32>() += 1 };
}

/// Point the session at a counter and hand it back. The box stays put while
/// the session holds its address.
fn wake_counter(session: *mut KtSession) -> Box<u32> {
    let mut count = Box::new(0u32);
    let status =
        unsafe { kt_session_set_wake(session, Some(count_a_wake), (&raw mut *count).cast()) };
    assert_eq!(status, KtStatus::Ok);
    count
}

/// Read borrowed text the way a consumer does: bytes and a length, no
/// terminator.
fn text(text: KtText) -> String {
    let bytes = unsafe { std::slice::from_raw_parts(text.bytes, text.len) };
    String::from_utf8(bytes.to_vec()).expect("the boundary promises UTF-8")
}

/// What a snapshot says outside the grid, copied out so it outlives the
/// snapshot the view borrows from.
struct Screen {
    cursor: Cursor,
    title: String,
    pwd: String,
}

fn screen_of(view: &KtSnapshotView) -> Screen {
    Screen {
        cursor: view.cursor,
        title: text(view.title),
        pwd: text(view.pwd),
    }
}

/// Feed one burst to a fresh session and read back the screen state.
fn screen_after(bytes: &[u8]) -> Screen {
    let session = detached(8, 3);
    feed(session, bytes);
    let snapshot = take(session);

    let screen = screen_of(&view(snapshot));

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
    screen
}

fn selection(start: (u16, u16), end: (u16, u16)) -> SelectionRange {
    SelectionRange {
        start_x: start.0,
        start_y: start.1,
        end_x: end.0,
        end_y: end.1,
        rectangle: false,
    }
}

/// Read a cell's text the way a consumer does: straight from the cell unless
/// it says its codepoint is really an index into the grapheme table.
fn text_of(view: &KtSnapshotView, cell: &Cell) -> Vec<u32> {
    if cell.attributes & Attribute::Overflow as u16 == 0 {
        return vec![cell.codepoint];
    }

    let index = cell.codepoint as usize;
    assert!(index < view.grapheme_count);
    let len = unsafe { *view.graphemes.add(index) } as usize;
    assert!(
        index + 1 + len <= view.grapheme_count,
        "run runs off the table"
    );

    (0..len)
        .map(|offset| unsafe { *view.graphemes.add(index + 1 + offset) })
        .collect()
}

/// Feed one burst to a fresh session and read back the first row.
fn first_row_of(bytes: &[u8]) -> Vec<Cell> {
    first_row_and_text_of(bytes).0
}

/// The same, plus each cell's text resolved through the grapheme table.
fn first_row_and_text_of(bytes: &[u8]) -> (Vec<Cell>, Vec<Vec<u32>>) {
    let session = detached(12, 1);
    feed(session, bytes);
    let snapshot = take(session);
    let view = view(snapshot);

    let cells: Vec<Cell> = (0..view.cols).map(|col| cell_at(&view, 0, col)).collect();
    let text = cells.iter().map(|cell| text_of(&view, cell)).collect();

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
    (cells, text)
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
fn every_colour_source_arrives_as_resolved_rgb() {
    // Plain, one of the basic 16, one of the 256, true colour, then a
    // palette background.
    let row = first_row_of(b"P\x1b[31mB\x1b[38;5;9mI\x1b[38;2;10;20;30mT\x1b[0m\x1b[44mG");

    assert_eq!(row[0].foreground, DEFAULT_FOREGROUND, "unset foreground");
    assert_eq!(row[0].background, DEFAULT_BACKGROUND, "unset background");
    assert_eq!(row[1].foreground, PALETTE_RED, "SGR 31 resolved");
    assert_eq!(
        row[2].foreground, PALETTE_BRIGHT_RED,
        "SGR 38;5;N resolved out of the 256-colour palette",
    );
    assert_eq!(
        row[3].foreground,
        Rgb {
            r: 10,
            g: 20,
            b: 30
        },
        "true colour passes through",
    );
    assert_eq!(row[4].background, PALETTE_BLUE, "SGR 44 resolved");
}

#[test]
fn each_sgr_attribute_lands_in_its_own_bit() {
    for (sgr, attribute) in [
        ("1", Attribute::Bold),
        ("2", Attribute::Faint),
        ("3", Attribute::Italic),
        ("5", Attribute::Blink),
        ("7", Attribute::Inverse),
        ("8", Attribute::Invisible),
        ("9", Attribute::Strikethrough),
        ("53", Attribute::Overline),
    ] {
        let row = first_row_of(format!("\x1b[{sgr}mX").as_bytes());
        assert_eq!(row[0].attributes, attribute as u16, "SGR {sgr} alone");
    }

    // All at once: the bits must not tread on each other.
    let row = first_row_of(b"\x1b[1;2;3;5;7;8;9;53mX");
    assert_eq!(row[0].attributes, 0b1111_1111);
}

#[test]
fn underline_styles_are_distinguished() {
    for (sgr, expected) in [
        ("4", Underline::Single),
        ("21", Underline::Double),
        ("4:3", Underline::Curly),
        ("4:4", Underline::Dotted),
        ("4:5", Underline::Dashed),
        ("4;24", Underline::None),
    ] {
        let row = first_row_of(format!("\x1b[{sgr}mX").as_bytes());
        assert_eq!(row[0].underline, expected, "SGR {sgr}");
    }
}

#[test]
fn a_single_codepoint_stays_in_the_cell() {
    let (row, text) = first_row_and_text_of("a".as_bytes());

    assert_eq!(row[0].attributes & Attribute::Overflow as u16, 0);
    assert_eq!(row[0].codepoint, u32::from('a'));
    assert_eq!(text[0], vec![u32::from('a')]);
}

#[test]
fn clusters_that_do_not_fit_move_to_the_grapheme_table() {
    // A combining acute, then a ZWJ sequence. Ghostty gives each emoji of the
    // sequence its own wide cell, so the first carries man + ZWJ.
    let (row, text) = first_row_and_text_of("e\u{301}\u{1F468}\u{200D}\u{1F469}".as_bytes());

    assert_ne!(
        row[0].attributes & Attribute::Overflow as u16,
        0,
        "combining"
    );
    assert_eq!(text[0], vec![u32::from('e'), 0x0301]);

    assert_ne!(row[1].attributes & Attribute::Overflow as u16, 0, "ZWJ");
    assert_eq!(text[1], vec![0x1F468, 0x200D]);

    // That cell is wide as well as overflowing: the two are independent, and
    // its spacer is neither.
    assert_ne!(row[1].attributes & Attribute::Wide as u16, 0);
    assert_eq!(
        row[2].attributes,
        Attribute::WideTail as u16,
        "a spacer carries no cluster of its own",
    );

    // The index in the cell is an index, not a codepoint that happens to fit.
    assert_ne!(row[0].codepoint, row[1].codepoint);
}

#[test]
fn a_wide_character_marks_its_two_cells_differently() {
    let (row, text) = first_row_and_text_of("\u{D55C}a".as_bytes());

    assert_ne!(
        row[0].attributes & Attribute::Wide as u16,
        0,
        "leading cell"
    );
    assert_eq!(row[0].attributes & Attribute::WideTail as u16, 0);
    assert_eq!(text[0], vec![0xD55C]);

    assert_ne!(
        row[1].attributes & Attribute::WideTail as u16,
        0,
        "trailing"
    );
    assert_eq!(row[1].attributes & Attribute::Wide as u16, 0);
    assert_eq!(row[1].codepoint, 0, "the trailing cell holds no text");

    // The next character starts after both cells of the wide one.
    assert_eq!(row[2].codepoint, u32::from('a'));
    assert_eq!(
        row[2].attributes & (Attribute::Wide as u16 | Attribute::WideTail as u16),
        0
    );
}

#[test]
fn the_grapheme_table_is_rebuilt_for_every_snapshot() {
    let session = detached(4, 1);

    feed(session, "e\u{301}".as_bytes());
    let first = take(session);
    let first_view = view(first);
    let first_count = first_view.grapheme_count;
    assert_ne!(first_count, 0);
    unsafe { kt_snapshot_free(first) };

    // A second snapshot holding the same one cluster must be the same size,
    // not the first snapshot's table with more appended to it.
    feed(session, "\ra\u{301}".as_bytes());
    let second = take(session);
    let second_view = view(second);
    assert_eq!(second_view.grapheme_count, first_count);
    assert_eq!(
        text_of(&second_view, &cell_at(&second_view, 0, 0)),
        vec![u32::from('a'), 0x0301],
    );

    unsafe { kt_snapshot_free(second) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_change_to_some_rows_is_reported_as_partial_and_names_them() {
    let session = detached(8, 4);

    // The first frame has the whole screen to draw, so it is a full one.
    feed(session, b"one");
    let first = take(session);
    assert_eq!(view(first).dirty, Dirty::Full);
    assert_eq!(rows_with(&view(first), RowFlag::Dirty), vec![true; 4]);
    unsafe { kt_snapshot_free(first) };

    // Jump to row 2 and write there. Row 0 is dirty too because the cursor
    // left it; the rows nothing touched stay clean, which is what makes this
    // partial rather than full.
    feed(session, b"\x1b[3;1Hthree");
    let second = take(session);
    let second_view = view(second);

    assert_eq!(second_view.dirty, Dirty::Partial);
    assert_eq!(
        rows_with(&second_view, RowFlag::Dirty),
        vec![true, false, true, false]
    );

    unsafe { kt_snapshot_free(second) };
    unsafe { kt_session_free(session) };
}

#[test]
fn switching_to_the_alternate_screen_is_reported_as_full() {
    let session = detached(8, 3);
    feed(session, b"one");
    unsafe { kt_snapshot_free(take(session)) };

    feed(session, b"\x1b[?1049h");

    let snapshot = take(session);
    let view = view(snapshot);
    assert_eq!(view.dirty, Dirty::Full);
    assert_eq!(
        rows_with(&view, RowFlag::Dirty),
        vec![true; 3],
        "a full frame marks its rows too, so a consumer reading either layer redraws",
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_snapshot_dropped_before_anyone_saw_it_hands_over_its_dirty_rows() {
    let session = detached(8, 4);
    feed(session, b"one");
    unsafe { kt_snapshot_free(take(session)) };

    // Two feeds, one take. The first snapshot is overwritten before anyone
    // sees it, so the rows it marked have to show up in the one that replaced
    // it — the consumer never got a chance to redraw them.
    feed(session, b"\x1b[2;1Htwo");
    feed(session, b"\x1b[4;1Hfour");

    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!(
        rows_with(&view, RowFlag::Dirty),
        vec![true, true, false, true],
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn dirty_marks_do_not_carry_into_the_next_snapshot() {
    let session = detached(8, 4);

    feed(session, b"one");
    unsafe { kt_snapshot_free(take(session)) };
    feed(session, b"\x1b[3;1Hthree");
    unsafe { kt_snapshot_free(take(session)) };

    // Rows 0 and 2 were both marked in the snapshot just taken. Writing where
    // the cursor already sits leaves only row 2, so neither the whole-screen
    // marks from the first snapshot nor row 0's from the second survived.
    feed(session, b"X");
    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!(
        rows_with(&view, RowFlag::Dirty),
        vec![false, false, true, false]
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_row_says_whether_it_runs_on_into_the_next() {
    let session = detached(4, 3);

    // "abcdef" runs out of columns after four, so row 0 continues into row 1.
    // "gh" then ends at a newline instead.
    feed(session, b"abcdef\r\ngh");

    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!(
        rows_with(&view, RowFlag::Wrapped),
        vec![true, false, false],
        "only the row that ran out of columns is marked",
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_line_that_exactly_fills_a_row_and_then_ends_is_not_wrapped() {
    let session = detached(4, 2);

    feed(session, b"abcd\r\n");

    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!(
        rows_with(&view, RowFlag::Wrapped),
        vec![false, false],
        "filling the row is not the same as running past it",
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_selection_spanning_rows_is_carried_beside_the_cells() {
    let session = detached(6, 3);
    feed(session, b"abcdef\r\nghijkl\r\nmnopqr");
    unsafe { kt_snapshot_free(take(session)) };

    // From the middle of row 0 to the middle of row 1: row 0 is selected to
    // its end, row 1 from its start, row 2 not at all.
    assert_eq!(
        set_selection(session, Some(selection((2, 0), (3, 1)))),
        KtStatus::Ok,
    );

    let snapshot = take(session);
    let view = view(snapshot);

    assert!(view.has_selection);
    assert_eq!(
        selected_columns(&view),
        vec![Some((2, 5)), Some((0, 3)), None],
    );

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn selecting_does_not_touch_the_cells() {
    let session = detached(6, 2);
    feed(session, b"abcdef\r\nghijkl");

    let before = take(session);
    let before_view = view(before);
    let cells: Vec<Cell> = (0..2)
        .flat_map(|row| (0..6).map(move |col| (row, col)))
        .map(|(row, col)| cell_at(&before_view, row, col))
        .collect();
    unsafe { kt_snapshot_free(before) };

    set_selection(session, Some(selection((0, 0), (5, 1))));

    let after = take(session);
    let after_view = view(after);
    let after_cells: Vec<Cell> = (0..2)
        .flat_map(|row| (0..6).map(move |col| (row, col)))
        .map(|(row, col)| cell_at(&after_view, row, col))
        .collect();

    assert_eq!(cells, after_cells, "a selection is not part of a cell");
    assert!(after_view.has_selection);

    unsafe { kt_snapshot_free(after) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_selection_with_no_visible_row_is_not_the_same_as_no_selection() {
    let session = detached_with_scrollback(6, 3, 100);
    feed(session, b"abcdef\r\nghijkl\r\nmnopqr");
    unsafe { kt_snapshot_free(take(session)) };

    set_selection(session, Some(selection((2, 0), (3, 1))));
    let selected = take(session);
    assert!(view(selected).has_selection);
    unsafe { kt_snapshot_free(selected) };

    // Push what was selected into the scrollback. The selection is still set,
    // but no row on screen falls inside it any more.
    feed(session, b"\r\n\r\n\r\n\r\nzzz");
    let scrolled = take(session);
    let scrolled_view = view(scrolled);
    assert!(scrolled_view.has_selection, "the selection still exists");
    assert_eq!(selected_columns(&scrolled_view), vec![None, None, None]);
    unsafe { kt_snapshot_free(scrolled) };

    // Clearing it is a different state again, and the rows look the same.
    set_selection(session, None);
    let cleared = take(session);
    let cleared_view = view(cleared);
    assert!(!cleared_view.has_selection);
    assert_eq!(selected_columns(&cleared_view), vec![None, None, None]);

    unsafe { kt_snapshot_free(cleared) };
    unsafe { kt_session_free(session) };
}

#[test]
fn the_cursor_reports_where_it_is_and_how_it_looks() {
    let fresh = screen_after(b"");
    assert_eq!((fresh.cursor.x, fresh.cursor.y), (0, 0));
    assert!(fresh.cursor.visible);
    assert_eq!(fresh.cursor.shape, CursorShape::Block);

    let moved = screen_after(b"ab\r\ncd");
    assert_eq!((moved.cursor.x, moved.cursor.y), (2, 1));

    let hidden = screen_after(b"\x1b[?25l");
    assert!(!hidden.cursor.visible);

    for (sequence, shape) in [
        ("\x1b[1 q", CursorShape::Block),
        ("\x1b[3 q", CursorShape::Underline),
        ("\x1b[5 q", CursorShape::Bar),
    ] {
        let screen = screen_after(sequence.as_bytes());
        assert_eq!(screen.cursor.shape, shape, "DECSCUSR {sequence:?}");
    }
}

#[test]
fn the_title_and_working_directory_are_read_back() {
    assert_eq!(screen_after(b"\x1b]2;hello\x07").title, "hello");
    assert_eq!(screen_after(b"\x1b]0;zero\x07").title, "zero");

    // OSC 7 reports a URI; what a consumer wants is the path.
    assert_eq!(screen_after(b"\x1b]7;file:///tmp/x\x07").pwd, "/tmp/x");
    assert_eq!(
        screen_after(b"\x1b]7;file:///tmp/a%20b\x07").pwd,
        "/tmp/a b"
    );
    assert_eq!(
        screen_after(b"\x1b]7;file://myhost/tmp/x\x07").pwd,
        "/tmp/x",
        "the authority is dropped; knotty has no notion of which host it is on",
    );

    // OSC 1337 reports a bare path, which passes through untouched.
    assert_eq!(
        screen_after(b"\x1b]1337;CurrentDir=/tmp/y\x07").pwd,
        "/tmp/y"
    );

    // Anything that is not two hex digits is not an escape.
    assert_eq!(
        screen_after(b"\x1b]7;file:///tmp/100%25\x07").pwd,
        "/tmp/100%"
    );
    assert_eq!(
        screen_after(b"\x1b]7;file:///tmp/a%zz%2\x07").pwd,
        "/tmp/a%zz%2"
    );
    assert_eq!(
        screen_after(b"\x1b]7;file:///tmp/a%+A\x07").pwd,
        "/tmp/a%+A"
    );

    // A report that yields no absolute path publishes none.
    for report in [
        &b"\x1b]7;file://myhost\x07"[..],
        &b"\x1b]1337;CurrentDir=relative/dir\x07"[..],
        &b"\x1b]7;file:///tmp/%FF\x07"[..],
    ] {
        assert_eq!(screen_after(report).pwd, "", "{report:?}");
    }
}

#[test]
fn a_dropped_snapshot_hands_over_both_dirty_layers() {
    let session = detached(8, 2);
    // Get the opening full frame out of the way.
    feed(session, b"x");
    unsafe { kt_snapshot_free(take(session)) };

    // A grid change nobody took, then a title change that leaves the grid
    // alone. The rows the first one marked must still be marked, and the
    // global level has to agree — a consumer reading only that level would
    // otherwise skip the redraw those rows are asking for.
    feed(session, b"ab");
    feed(session, b"\x1b]2;hello\x07");

    let snapshot = take(session);
    let view = view(snapshot);

    assert_eq!(view.dirty, Dirty::Partial);
    assert_eq!(rows_with(&view, RowFlag::Dirty), vec![true, false]);
    assert_eq!(text(view.title), "hello");

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn control_characters_never_reach_the_title_or_the_working_directory() {
    // The engine drops C0 itself, but DEL and C1 reach us intact, and a
    // percent escape can smuggle a newline past the parser entirely.
    let screen = screen_after("\x1b]2;a\u{7f}b\u{85}c\x07".as_bytes());
    assert_eq!(screen.title, "abc");

    let smuggled = screen_after(b"\x1b]7;file:///tmp/a%0Ab%00c\x07");
    assert_eq!(
        smuggled.pwd, "/tmp/abc",
        "decoding happens first, so what it produces is sanitised too",
    );
}

#[test]
fn a_change_outside_the_grid_is_published_on_its_own() {
    let session = detached(8, 2);
    feed(session, b"ab");
    unsafe { kt_snapshot_free(take(session)) };

    // A title sequence leaves every row alone, so the engine reports nothing
    // dirty. It still has to reach a consumer.
    feed(session, b"\x1b]2;hello\x07");

    let snapshot = take(session);
    let view = view(snapshot);
    assert_eq!(view.dirty, Dirty::Clean, "no row changed");
    assert_eq!(text(view.title), "hello");

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_rectangular_selection_covers_the_same_columns_on_every_row() {
    let session = detached(6, 2);
    feed(session, b"abcdef\r\nghijkl");
    unsafe { kt_snapshot_free(take(session)) };

    set_selection(
        session,
        Some(SelectionRange {
            rectangle: true,
            ..selection((1, 0), (3, 1))
        }),
    );
    let block = take(session);
    assert_eq!(
        selected_columns(&view(block)),
        vec![Some((1, 3)), Some((1, 3))],
    );
    unsafe { kt_snapshot_free(block) };

    // The same endpoints read as a run of text instead: row 0 to its end,
    // row 1 from its start.
    set_selection(session, Some(selection((1, 0), (3, 1))));
    let linear = take(session);
    assert_eq!(
        selected_columns(&view(linear)),
        vec![Some((1, 5)), Some((0, 3))],
    );

    unsafe { kt_snapshot_free(linear) };
    unsafe { kt_session_free(session) };
}

#[test]
fn switching_screens_takes_the_selection_with_it() {
    let session = detached(6, 2);
    feed(session, b"abcdef\r\nghijkl");
    unsafe { kt_snapshot_free(take(session)) };

    set_selection(session, Some(selection((1, 0), (3, 0))));
    unsafe { kt_snapshot_free(take(session)) };

    // The engine's selection belongs to the active screen, so switching drops
    // it. Reporting it as still there would look like a selection scrolled out
    // of view, which is the one thing has_selection exists to rule out.
    feed(session, b"\x1b[?1049h");

    let snapshot = take(session);
    let view = view(snapshot);
    assert!(!view.has_selection);
    assert_eq!(selected_columns(&view), vec![None, None]);

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn resetting_the_terminal_takes_the_selection_with_it() {
    let session = detached(6, 2);
    feed(session, b"abcdef\r\nghijkl");
    unsafe { kt_snapshot_free(take(session)) };

    set_selection(session, Some(selection((1, 0), (3, 0))));
    unsafe { kt_snapshot_free(take(session)) };

    // `RIS` drops the selection without changing screens, so nothing knotty
    // could keep beside the engine would notice.
    feed(session, b"\x1bc");

    let snapshot = take(session);
    let view = view(snapshot);
    assert!(!view.has_selection);
    assert_eq!(selected_columns(&view), vec![None, None]);

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

#[test]
fn a_selection_endpoint_outside_the_viewport_is_reported_as_such() {
    let session = detached(4, 2);
    feed(session, b"ab");
    unsafe { kt_snapshot_free(take(session)) };

    assert_eq!(
        set_selection(session, Some(selection((0, 0), (99, 0)))),
        KtStatus::OutOfRange,
    );

    unsafe { kt_session_free(session) };
}

#[test]
fn changing_the_palette_recolours_cells_in_the_next_snapshot() {
    let session = detached(4, 1);
    feed(session, b"\x1b[31mX");

    let before = take(session);
    assert_eq!(cell_at(&view(before), 0, 0).foreground, PALETTE_RED);
    unsafe { kt_snapshot_free(before) };

    feed(session, b"\x1b]4;1;rgb:12/34/56\x07");

    let after = take(session);
    assert_eq!(
        cell_at(&view(after), 0, 0).foreground,
        Rgb {
            r: 0x12,
            g: 0x34,
            b: 0x56
        },
        "the cell still names palette 1, which now means something else",
    );

    unsafe { kt_snapshot_free(after) };
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

/// No callback of ours is attached to anything yet, and the engine still
/// answers a cursor position report by itself. That answer is the traffic
/// this queue exists to carry.
#[test]
fn what_the_engine_answers_on_its_own_lands_in_the_writer_queue() {
    let session = detached(4, 1);

    feed(session, b"\x1b[6n");

    assert_eq!(writes(session), b"\x1b[1;1R");

    unsafe { kt_session_free(session) };
}

#[test]
fn taking_the_queued_writes_empties_the_queue() {
    let session = detached(4, 1);
    feed(session, b"\x1b[6n");

    assert!(!writes(session).is_empty());
    assert!(
        writes(session).is_empty(),
        "the queue handed the same bytes twice"
    );

    unsafe { kt_session_free(session) };
}

#[test]
fn nothing_queued_is_an_empty_run_rather_than_a_failure() {
    let session = detached(4, 1);
    feed(session, b"ok");

    assert!(writes(session).is_empty());

    unsafe { kt_session_free(session) };
}

/// A TUI that asks what terminal it is talking to waits for the answer, so an
/// unanswered query is a hang rather than a missing feature.
///
/// These bytes are what knotty says it is, and the core now fills every one of
/// them in. The engine's own unset default says the same today — it quacks as a
/// VT220 too — so what this pins is the answer, and a change to it is an
/// upstream change knotty has to decide on rather than inherit.
#[test]
fn every_device_attributes_query_is_answered() {
    let session = detached(4, 1);

    feed(session, b"\x1b[c");
    assert_eq!(writes(session), b"\x1b[?62;22c", "DA1");

    feed(session, b"\x1b[>c");
    assert_eq!(writes(session), b"\x1b[>1;0;0c", "DA2");

    feed(session, b"\x1b[=c");
    assert_eq!(writes(session), b"\x1bP!|00000000\x1b\\", "DA3");

    unsafe { kt_session_free(session) };
}

/// A program picking a code path by terminal name has to be told knotty's,
/// not the engine's — the engine answers this one on its own with
/// `libghostty` when nothing fills it in.
#[test]
fn a_version_query_is_answered_with_knottys_own_name() {
    let session = detached(4, 1);

    feed(session, b"\x1b[>q");

    assert_eq!(
        writes(session),
        format!("\x1bP>|knotty {}\x1b\\", env!("CARGO_PKG_VERSION")).into_bytes(),
    );

    unsafe { kt_session_free(session) };
}

/// An old program checks that something is there by asking, so the answerback
/// is knotty's name rather than the silence an unfilled callback leaves.
#[test]
fn an_enquiry_is_answered_with_the_terminals_name() {
    let session = detached(4, 1);

    feed(session, b"\x05");

    assert_eq!(writes(session), b"knotty");

    unsafe { kt_session_free(session) };
}

/// The pixel size is the renderer's to know and the color scheme the app's,
/// so the core has nothing true to say and says nothing: a made-up value is
/// worse than none. cf. the M1 spec, `kwnms04/knotty#10`.
///
/// One engine callback covers all three size reports, and it cannot be filled
/// in without the cell pixel size — so the size in cells, which the core does
/// know, goes unanswered along with the two in pixels.
#[test]
fn queries_whose_answer_the_core_does_not_know_go_unanswered() {
    let session = detached(4, 1);

    // Cell pixel size, window pixel size, and text area size in cells.
    feed(session, b"\x1b[16t\x1b[14t\x1b[18t");
    // Color scheme.
    feed(session, b"\x1b[?996n");

    assert!(writes(session).is_empty());

    unsafe { kt_session_free(session) };
}

/// The engine reports the title with no callback of ours in the way, so
/// untrusted output can plant a command in the title and have the answer
/// delivered to the shell as keystrokes. cf. `docs/adr/0007-input-security.md`.
#[test]
fn a_title_query_is_answered_without_the_title_in_it() {
    let session = detached(4, 1);
    feed(session, b"\x1b]2;echo pwned\x07");

    // The title has to be set for the query to have anything to leak: a
    // filter kept honest by a broken setter is not kept honest at all.
    let snapshot = take(session);
    assert_eq!(text(view(snapshot).title), "echo pwned");
    unsafe { kt_snapshot_free(snapshot) };

    feed(session, b"\x1b[21t");

    assert_eq!(
        writes(session),
        b"\x1b]l\x1b\\",
        "the answer goes out carrying nothing — silence would hang a program \
         waiting on one",
    );

    unsafe { kt_session_free(session) };
}

/// The filter reads each answer on its way out, so one that merely looks like
/// a title report — same introducer, different report — has to pass whole.
#[test]
fn answers_other_than_the_title_report_are_left_alone() {
    let session = detached(4, 1);

    feed(session, b"\x1b]4;1;?\x1b\\");

    assert_eq!(writes(session), b"\x1b]4;1;rgb:cccc/6666/6666\x1b\\");

    unsafe { kt_session_free(session) };
}

/// Pushing and popping the title stack writes state rather than reading it
/// back, so nothing about it is the filter's business.
///
/// What the stack itself does is out of reach here: the pinned engine parses
/// both sequences and then drops them, keeping no stack, so a popped title
/// never returns. This holds them to what is observable — that neither is
/// answered on the wire, and neither disturbs the title.
#[test]
fn the_title_stack_is_driven_without_anything_being_answered() {
    let session = detached(4, 1);
    feed(session, b"\x1b]2;hello\x07");

    feed(session, b"\x1b[22;2t");
    feed(session, b"\x1b[23;2t");

    assert!(writes(session).is_empty(), "a write was answered");

    let snapshot = take(session);
    assert_eq!(text(view(snapshot).title), "hello");

    unsafe { kt_snapshot_free(snapshot) };
    unsafe { kt_session_free(session) };
}

/// The cap is what keeps a child that never reads from growing the queue
/// without bound, so overrunning it has to be its own status: a caller that
/// cannot tell it from a rejected sequence cannot tell the user either.
#[test]
fn a_writer_queue_over_its_cap_is_reported_apart_from_other_failures() {
    // The 8MB of 02-ffi.md, restated because the core keeps its own copy
    // private. If the two ever disagree, this is what notices.
    const CAP: usize = 8 * 1024 * 1024;

    let session = detached(4, 1);

    // XTVERSION: four bytes in, a name and version out. Repeat it and the
    // answers outrun the cap long before the bound.
    let queries = b"\x1b[>q".repeat(4096);
    let mut status = KtStatus::Ok;
    for _ in 0..1000 {
        status = unsafe { kt_session_feed(session, queries.as_ptr(), queries.len()) };
        if status != KtStatus::Ok {
            break;
        }
    }

    assert_eq!(status, KtStatus::WriteQueueFull);

    // Full to within one answer of the cap: a queue that reported overrunning
    // while nearly empty would be dropping bytes it had room for.
    let queued = writes(session).len();
    assert!(
        (CAP - 64..=CAP).contains(&queued),
        "reported the cap with {queued} bytes queued, not the {CAP} it holds",
    );
    assert_eq!(
        unsafe { kt_session_feed(session, b"ok".as_ptr(), 2) },
        KtStatus::Ok,
        "the overflow was reported once, not held against every later feed",
    );

    unsafe { kt_session_free(session) };
}

/// The bell leaves no mark on the screen, so the event is the only trace of
/// it there is: a consumer that misses it has no second way to find out.
#[test]
fn a_bell_arrives_as_an_event() {
    let session = detached(4, 1);

    feed(session, b"\x07");

    assert_eq!(events(session), (vec![bell()], 0));
    assert_eq!(
        events(session),
        (Vec::new(), 0),
        "taking the queue empties it",
    );

    unsafe { kt_session_free(session) };
}

/// Copying is the child's request and the app's to carry out, so the event
/// has to say both which clipboard and what to put on it.
#[test]
fn a_clipboard_write_carries_its_target_and_its_text() {
    let session = detached(4, 1);

    // OSC 52 with the base64 of "hello", "world" and "again" — one for each
    // clipboard a write can name, since the app puts them in different
    // places.
    feed(session, b"\x1b]52;c;aGVsbG8=\x07");
    feed(session, b"\x1b]52;p;d29ybGQ=\x07");
    feed(session, b"\x1b]52;s;YWdhaW4=\x07");

    assert_eq!(
        events(session),
        (
            vec![
                clipboard_write(ClipboardTarget::Standard, "hello"),
                clipboard_write(ClipboardTarget::Primary, "world"),
                clipboard_write(ClipboardTarget::Selection, "again"),
            ],
            0,
        ),
    );

    unsafe { kt_session_free(session) };
}

/// A payload past the cap is refused whole rather than cut short. Nothing
/// tells the user their copy arrived as its first megabyte, so a truncated
/// clipboard is worse than one that did not change.
#[test]
fn a_clipboard_payload_over_the_cap_is_refused_rather_than_cut_short() {
    // The 1MiB the core keeps private. If the two ever disagree, this is what
    // notices.
    const CAP: usize = 1024 * 1024;

    // Base64 of "aaa": four characters in, three bytes out, so a repeat of it
    // decodes to a payload of a size this test can aim.
    let payload = |bytes: usize| {
        [
            b"\x1b]52;c;".as_slice(),
            &b"YWFh".repeat(bytes / 3),
            b"\x07",
        ]
        .concat()
    };

    let session = detached(4, 1);

    feed(session, &payload(CAP));
    let (taken, dropped) = events(session);
    assert_eq!(taken.len(), 1, "a payload within the cap is carried");
    assert_eq!(dropped, 0);

    feed(session, &payload(CAP + 3));

    assert_eq!(
        events(session),
        (Vec::new(), 0),
        "the write is refused, not queued as a prefix of itself",
    );

    unsafe { kt_session_free(session) };
}

/// The queue is finite because a child can ring the bell faster than anyone
/// drains it. Losing events is allowed — everything a screen needs to be
/// right is in the snapshot — but losing them silently is not.
#[test]
fn events_past_the_cap_are_counted_rather_than_queued() {
    // The 64 the core keeps private, restated here for the same reason the
    // writer queue's cap is.
    const CAP: usize = 64;
    const OVER: usize = 10;

    let session = detached(4, 1);

    feed(session, &b"\x07".repeat(CAP + OVER));

    let (taken, dropped) = events(session);
    assert_eq!(taken.len(), CAP, "the queue held its cap and no more");
    assert_eq!(dropped, OVER as u64);
    assert_eq!(
        events(session),
        (Vec::new(), 0),
        "the count empties with the queue, so one overrun is reported once",
    );

    unsafe { kt_session_free(session) };
}

/// A child that could read the clipboard could read whatever the user last
/// copied — a password among it. The engine drops the request without telling
/// us, so there is nothing to answer with and nothing goes out.
#[test]
fn a_clipboard_read_request_gets_nothing_back() {
    let session = detached(4, 1);
    feed(session, b"\x1b]52;c;aGVsbG8=\x07");
    events(session);

    feed(session, b"\x1b]52;c;?\x07");

    assert!(writes(session).is_empty(), "an answer went to the child");
    assert_eq!(
        events(session),
        (Vec::new(), 0),
        "a read request is not a write and must not reach the app as one",
    );

    unsafe { kt_session_free(session) };
}

/// Without this a consumer has to poll to find out whether there is a frame,
/// which is the idle cost the whole design exists to avoid. A bell is in here
/// because it marks no cell: an event is as much reason to come and look as a
/// frame is.
#[test]
fn a_wake_says_when_there_is_something_to_take() {
    let session = detached(4, 1);
    let count = wake_counter(session);

    feed(session, b"x");
    assert_eq!(*count, 1, "a new screen went unannounced");

    feed(session, b"\x07");
    assert_eq!(*count, 2, "a bell changes no cell but is still an event");

    unsafe { kt_session_free(session) };
}

/// A wake the consumer cannot act on is a frame it draws for nothing. The
/// query here is the case that decides the rule: bytes came in and an answer
/// went out, but the screen did not move and nothing happened the app has to
/// hear about — so what the child is owed is not what the consumer is.
#[test]
fn a_feed_that_changes_nothing_wakes_nobody() {
    let session = detached(4, 1);
    let count = wake_counter(session);
    feed(session, b"x");
    let before = *count;

    feed(session, b"\x1b[c");

    assert!(!writes(session).is_empty(), "the query went unanswered");
    assert_eq!(*count, before, "queued writer traffic woke the consumer");

    unsafe { kt_session_free(session) };
}

/// Waking inside a block hands over the half-drawn screen the block exists to
/// hide, so what the consumer must observe is one wake at the close and none
/// before it.
#[test]
fn a_synchronized_output_block_holds_the_wake_back_until_it_closes() {
    let session = detached(4, 1);
    let count = wake_counter(session);

    feed(session, b"\x1b[?2026h");
    feed(session, b"a");
    feed(session, b"b");
    assert_eq!(*count, 0, "a half-drawn screen reached the consumer");

    feed(session, b"\x1b[?2026l");

    assert_eq!(*count, 1, "a closed block is one wake, not one per frame");

    // The next screen is announced as any other would be: closing the block
    // settled what was owed and left nothing behind.
    feed(session, b"c");
    assert_eq!(*count, 2);

    unsafe { kt_session_free(session) };
}

/// A block that opened and drew nothing owes nothing: what unblocks a held
/// wake is a frame, not the closing sequence.
#[test]
fn a_synchronized_output_block_with_nothing_in_it_wakes_nobody() {
    let session = detached(4, 1);
    let count = wake_counter(session);
    feed(session, b"x");
    let before = *count;

    feed(session, b"\x1b[?2026h\x1b[?2026l");

    assert_eq!(*count, before, "an empty block ended in a wake of its own");

    unsafe { kt_session_free(session) };
}

/// A consumer that goes away must be able to say so before it does, or the
/// session calls into what is no longer there. What it missed while away is
/// still owed, so it does not have to know to go looking on its way back.
#[test]
fn clearing_the_wake_stops_the_calls_without_losing_what_they_would_have_said() {
    let session = detached(4, 1);
    let count = wake_counter(session);
    feed(session, b"x");

    assert_eq!(
        unsafe { kt_session_set_wake(session, None, ptr::null_mut()) },
        KtStatus::Ok,
    );
    feed(session, b"y");
    assert_eq!(*count, 1, "a cleared callback was still called");

    // Registering is what pays it, and nothing in the feed after can add to
    // it — so the one wake the fresh counter sees is the one "y" was owed.
    let count = wake_counter(session);
    feed(session, b"");

    assert_eq!(*count, 1, "the screen drawn while nobody listened was lost");

    unsafe { kt_session_free(session) };
}

/// Whatever a shell prints, it prints on the child's own schedule: a fork, an
/// exec and a write have to happen before anything is on screen. So the check
/// is "eventually", with a bound long enough that only a real failure reaches
/// it.
const PATIENCE: Duration = Duration::from_secs(10);

/// A command as a consumer passes one: borrowed runs of text, program first.
fn pty(cols: u16, rows: u16, argv: &[&str]) -> *mut KtSession {
    let argv: Vec<KtText> = argv.iter().copied().map(KtText::from).collect();
    let mut session = ptr::null_mut();
    let status =
        unsafe { kt_session_new_pty(cols, rows, 0, argv.as_ptr(), argv.len(), &mut session) };
    assert_eq!(status, KtStatus::Ok);
    assert!(!session.is_null());
    session
}

fn write(session: *mut KtSession, bytes: &[u8]) {
    let status = unsafe { kt_session_write(session, bytes.as_ptr(), bytes.len()) };
    assert_eq!(status, KtStatus::Ok);
}

/// Take a snapshot if one has been published, rather than insisting on it.
fn take_if_any(session: *mut KtSession) -> Option<*mut KtSnapshot> {
    let mut snapshot = ptr::null_mut();
    match unsafe { kt_session_take_snapshot(session, &mut snapshot) } {
        KtStatus::Ok => Some(snapshot),
        KtStatus::NoValue => None,
        status => panic!("taking a snapshot answered {status:?}"),
    }
}

/// The screen as a person reads it: a cell holding nothing is a space.
fn screen_lines(view: &KtSnapshotView) -> Vec<String> {
    (0..view.rows)
        .map(|row| {
            (0..view.cols)
                .map(|col| match codepoint_at(view, row, col) {
                    0 => ' ',
                    codepoint => char::from_u32(codepoint).unwrap_or(' '),
                })
                .collect()
        })
        .collect()
}

/// Watch the screen until `read` finds what it is after, and answer with that.
///
/// Every snapshot carries the whole grid, so only the newest one has to be
/// looked at — a frame passed over cannot have held anything a later one does
/// not. `wanted` says what the wait was for, and is what a failure names.
fn wait_for_screen<T>(
    session: *mut KtSession,
    wanted: &str,
    read: impl Fn(&[String]) -> Option<T>,
) -> T {
    let deadline = Instant::now() + PATIENCE;
    let mut last = Vec::new();
    while Instant::now() < deadline {
        if let Some(snapshot) = take_if_any(session) {
            last = screen_lines(&view(snapshot));
            unsafe { kt_snapshot_free(snapshot) };
            if let Some(found) = read(&last) {
                return found;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("waited for {wanted}; the screen said {last:#?}");
}

/// Wait for some row of the screen to hold `wanted`.
fn wait_for(session: *mut KtSession, wanted: &str) {
    wait_for_screen(session, &format!("{wanted:?}"), |lines| {
        lines.iter().any(|line| line.contains(wanted)).then_some(())
    });
}

/// The wake counter a session with its own thread needs: the call arrives on
/// that thread, not on the one reading the count.
extern "C" fn count_a_wake_across_threads(userdata: *mut c_void) {
    unsafe { &*userdata.cast::<AtomicU32>() }.fetch_add(1, Ordering::Relaxed);
}

/// Point the session at a counter and hand it back. The box stays put while
/// the session holds its address.
fn shared_wake_counter(session: *mut KtSession) -> Box<AtomicU32> {
    let count = Box::new(AtomicU32::new(0));
    let status = unsafe {
        kt_session_set_wake(
            session,
            Some(count_a_wake_across_threads),
            (&raw const *count).cast_mut().cast(),
        )
    };
    assert_eq!(status, KtStatus::Ok);
    count
}

/// The milestone in one line: a shell actually comes up, and what it prints is
/// on the screen.
#[test]
fn a_pty_session_puts_what_its_child_printed_on_the_screen() {
    let session = pty(24, 4, &["/bin/sh", "-c", "printf 'up and running'"]);

    wait_for(session, "up and running");

    unsafe { kt_session_free(session) };
}

/// A child told the size only after it started has already drawn its first
/// frame to the wrong one, and many never redraw. So it is set on the terminal
/// before the fork.
#[test]
fn a_child_starts_knowing_the_size_of_the_window_it_is_in() {
    let session = pty(37, 9, &["/bin/sh", "-c", "stty size"]);

    // What `stty size` prints is rows then columns.
    wait_for(session, "9 37");

    unsafe { kt_session_free(session) };
}

/// The other half of the conversation: without this the terminal can only be
/// watched, not used.
#[test]
fn bytes_written_to_a_pty_session_reach_its_child() {
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "read line; printf 'heard:%s' \"$line\""],
    );

    // Carriage return rather than newline: it is what a keyboard sends, and
    // the terminal's line discipline is what turns it into the end of a line.
    write(session, b"typed\r");

    wait_for(session, "heard:typed");

    unsafe { kt_session_free(session) };
}

/// Without this a consumer has to poll a thread it does not own, which is the
/// idle cost the whole design exists to avoid.
#[test]
fn a_pty_session_wakes_its_consumer_when_there_is_something_to_take() {
    // A child that reads before it writes, so the callback is registered
    // before anything can be published.
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "read line; printf 'heard:%s' \"$line\""],
    );
    let count = shared_wake_counter(session);

    write(session, b"typed\r");
    wait_for(session, "heard:typed");

    assert!(
        count.load(Ordering::Relaxed) > 0,
        "a new screen went unannounced",
    );

    unsafe { kt_session_free(session) };
}

/// The session inside a PTY one always has a callback registered — the
/// trampoline out to the app — so it can never hold the debt itself. If it
/// settled one on the app's behalf, a consumer attaching late would sit on a
/// full mailbox it was never told about. cf. `03-core.md` C5
#[test]
fn a_pty_session_still_owes_a_wake_registered_after_the_screen_was_drawn() {
    let session = pty(24, 4, &["/bin/sh", "-c", "printf 'up and running'"]);
    // Drawn, published, and taken — all with nobody registered to be told.
    wait_for(session, "up and running");

    let count = shared_wake_counter(session);

    assert_eq!(
        count.load(Ordering::Relaxed),
        1,
        "the screen drawn while nobody listened was never announced",
    );

    unsafe { kt_session_free(session) };
}

/// Drive a consumer's own loop until the child's exit turns up: drain the
/// events, then take the newest screen. Answers with the exit and with the
/// screen as it stood when the exit was observed.
///
/// The screen is taken after the drain rather than before it, so what comes
/// back is what a consumer that has just been told "the child is gone" has to
/// draw — which is what C6's ordering invariant is about.
fn wait_for_exit(session: *mut KtSession) -> (Happened, Vec<String>) {
    let deadline = Instant::now() + PATIENCE;
    let mut screen = Vec::new();
    let mut ended = None;

    while ended.is_none() {
        assert!(
            Instant::now() < deadline,
            "waited for the child to end; the screen said {screen:#?}",
        );
        thread::sleep(Duration::from_millis(5));

        let (taken, _) = events(session);
        ended = taken
            .into_iter()
            .find(|event| event.kind == KtEventKind::ChildExited);
        if let Some(snapshot) = take_if_any(session) {
            screen = screen_lines(&view(snapshot));
            unsafe { kt_snapshot_free(snapshot) };
        }
    }

    (ended.expect("the loop runs until the exit arrives"), screen)
}

/// Watch the snapshots until one says what the wait was for, and answer with
/// what that one said.
///
/// Nothing here drains the event queue: what a snapshot carries has to be
/// readable by a consumer that only ever draws, since an event can be dropped
/// and the truth about the child cannot.
fn wait_for_snapshot<T>(
    session: *mut KtSession,
    wanted: &str,
    read: impl Fn(&KtSnapshotView) -> Option<T>,
) -> T {
    let deadline = Instant::now() + PATIENCE;
    let mut last = None;
    while Instant::now() < deadline {
        if let Some(snapshot) = take_if_any(session) {
            let view = view(snapshot);
            let found = read(&view);
            last = Some((view.child_state, view.child_exit_code, view.session_state));
            unsafe { kt_snapshot_free(snapshot) };
            if let Some(found) = found {
                return found;
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
    panic!("waited for {wanted}; the last snapshot said {last:?}");
}

/// Whether a process is still there, asked the way anything asks: a signal of
/// no signal.
///
/// A child that was killed but never collected is a zombie, and a zombie
/// answers this as alive — so this holds the session to collecting what it
/// kills, not only to killing it.
fn alive(pid: u32) -> bool {
    Pid::from_raw(pid as i32).is_some_and(|pid| test_kill_process(pid).is_ok())
}

/// Wait for the child to print its own process id, and answer with it.
///
/// The brackets are what makes a half-arrived number impossible to misread: a
/// screen showing the closing one is showing the whole of it.
fn pid_of_child(session: *mut KtSession) -> u32 {
    wait_for_screen(session, "the child's pid", |lines| {
        lines.iter().find_map(|line| {
            let (_, rest) = line.split_once("pid[")?;
            let (pid, _) = rest.split_once(']')?;
            pid.parse().ok()
        })
    })
}

/// Nothing on the screen says the shell has gone: a prompt that will never
/// come back looks exactly like one that will. So the exit is an event, and it
/// carries the code because that is what decides whether the window closes
/// quietly or stays up saying what went wrong. cf. `05-swift-app.md`
#[test]
fn a_child_that_ends_arrives_as_an_event_carrying_its_exit_code() {
    let session = pty(24, 4, &["/bin/sh", "-c", "exit 3"]);

    let (ended, _) = wait_for_exit(session);

    assert_eq!(ended, child_exited(3));

    unsafe { kt_session_free(session) };
}

/// A death by signal has no exit code of its own. It is reported as the shells
/// report one — 128 plus the signal — so that a consumer has a single number
/// to show and one meaning for it.
#[test]
fn a_child_killed_by_a_signal_reports_the_code_a_shell_would() {
    // SIGTERM, sent by the child to itself: no signal of knotty's is involved,
    // which is the point — nothing here installs a handler.
    let session = pty(24, 4, &["/bin/sh", "-c", "kill -TERM $$"]);

    let (ended, _) = wait_for_exit(session);

    assert_eq!(ended, child_exited(128 + 15));

    unsafe { kt_session_free(session) };
}

/// The event is the immediate telling and the snapshot is the truth: events
/// are finite and can be dropped, so a consumer that lost one would go on
/// drawing a window whose shell is gone as a working one.
///
/// The child here prints nothing, so the exit is the only thing there has ever
/// been to publish — a session that only published when the screen moved would
/// leave the last frame anybody holds saying the child is running.
#[test]
fn a_child_that_ends_says_so_on_the_snapshot_as_well() {
    let session = pty(24, 4, &["/bin/sh", "-c", "exit 7"]);

    let (state, code) = wait_for_snapshot(session, "the child to end", |view| {
        (view.child_state == KtChildState::Exited)
            .then_some((view.session_state, view.child_exit_code))
    });

    assert_eq!(code, 7);
    assert_eq!(
        state,
        KtSessionState::Ok,
        "a child that ended took the session with it",
    );

    unsafe { kt_session_free(session) };
}

/// The other half of the pair, and what an app asks before it closes a window:
/// a child still running is what makes the closing worth a warning.
#[test]
fn a_child_still_running_says_so_on_the_snapshot() {
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "printf 'up and running'; exec sleep 30"],
    );

    let code = wait_for_snapshot(session, "the first frame", |view| {
        (view.child_state == KtChildState::Running).then_some(view.child_exit_code)
    });

    assert_eq!(
        code, 0,
        "a running child was reported as ending with a code"
    );

    unsafe { kt_session_free(session) };
}

/// C6's ordering invariant. The exit is committed only after the terminal has
/// been read to its end, so a consumer told the child is gone has already been
/// shown everything it printed.
///
/// The child prints far more than one read takes, so the end arrives in a
/// round of its own rather than in the round that carried the output — which
/// is the shape a watcher of exits other than the read itself would have to
/// order by hand.
#[test]
fn everything_a_child_printed_is_on_screen_when_its_exit_arrives() {
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "seq 5000; printf 'the last line'; exit 0"],
    );

    let (ended, screen) = wait_for_exit(session);

    assert_eq!(ended.exit_code, 0);
    assert!(
        screen.iter().any(|line| line.contains("the last line")),
        "the exit outran the output; the screen said {screen:#?}",
    );

    unsafe { kt_session_free(session) };
}

/// A block the child left open can never close, and the round that would have
/// carried the wake it held back ends with the child. So the exit pays what is
/// owed itself — otherwise the last thing a window ever hears is silence, with
/// its shell gone and the news of it sitting in a queue. cf. `03-core.md` C5
#[test]
fn a_child_that_dies_inside_a_synchronized_block_still_wakes_its_consumer() {
    // The escape is written as the octal every shell's printf reads: an
    // escape spelled with a backslash-e is bash's alone.
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "printf '\\033[?2026hhalf drawn'; exit 0"],
    );
    let count = shared_wake_counter(session);

    let (ended, screen) = wait_for_exit(session);

    assert_eq!(ended.exit_code, 0);
    assert!(
        count.load(Ordering::Relaxed) > 0,
        "the exit was queued with nobody told to come for it",
    );
    assert!(
        screen.iter().any(|line| line.contains("half drawn")),
        "what the block held back never arrived; the screen said {screen:#?}",
    );

    unsafe { kt_session_free(session) };
}

/// A block the child opens and never closes would hold the screen for as long
/// as that child runs. Only the thread that waits can break it — the child
/// says nothing more, so no round comes round on its own. cf. `03-core.md` C5
#[test]
fn a_block_the_child_leaves_open_is_given_up_on() {
    // `exec` so the block stays open with nothing following it: what has to
    // reach the consumer is a wake no later output could have carried.
    let session = pty(
        24,
        4,
        &[
            "/bin/sh",
            "-c",
            "printf '\\033[?2026hheld open'; exec sleep 30",
        ],
    );
    let count = shared_wake_counter(session);

    let deadline = Instant::now() + PATIENCE;
    while count.load(Ordering::Relaxed) == 0 && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }

    assert!(
        count.load(Ordering::Relaxed) > 0,
        "the screen stayed frozen behind a block nobody closed",
    );

    unsafe { kt_session_free(session) };
}

/// Releasing a session is the user closing the window, and a child left behind
/// by that is a process nobody can talk to and nobody will collect.
#[test]
fn a_long_running_child_does_not_outlive_the_session() {
    // `exec` so the pid printed stays the pid that goes on running: what has
    // to be gone afterwards is the process knotty started, not a shell that
    // had already left of its own accord.
    let session = pty(
        24,
        4,
        &["/bin/sh", "-c", "printf 'pid[%s]' $$; exec sleep 30"],
    );
    let pid = pid_of_child(session);
    assert!(alive(pid), "the child was not there to begin with");

    unsafe { kt_session_free(session) };

    assert!(
        !alive(pid),
        "the child outlived the session that spawned it"
    );
}

/// Both calls reach past the I/O thread for what it owns: one would race the
/// engine it is parsing with, the other would take bytes it is already
/// handing to the child. cf. `03-core.md` C7
#[test]
fn a_pty_session_refuses_the_calls_that_are_only_for_a_detached_one() {
    let session = pty(4, 1, &["/bin/sh", "-c", "sleep 30"]);

    assert_eq!(
        unsafe { kt_session_feed(session, b"x".as_ptr(), 1) },
        KtStatus::NotDetached,
    );
    let mut queued = MaybeUninit::<KtBytes>::uninit();
    assert_eq!(
        unsafe { kt_session_take_writes(session, queued.as_mut_ptr()) },
        KtStatus::NotDetached,
    );

    unsafe { kt_session_free(session) };
}

/// A detached session has no child to hand them to, so its queue is where
/// they stop — beside what the terminal answered, since both are bound for
/// the same place.
#[test]
fn bytes_written_to_a_detached_session_wait_in_its_queue() {
    let session = detached(4, 1);

    write(session, b"typed");

    assert_eq!(writes(session), b"typed");

    unsafe { kt_session_free(session) };
}

#[test]
fn null_arguments_are_reported_rather_than_dereferenced() {
    assert_eq!(
        unsafe { kt_session_new_detached(4, 1, 0, ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_new_pty(4, 1, 0, ptr::null(), 0, ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    // A command of no words names nothing to run, which is the same missing
    // argument as a null one.
    let mut nothing_to_run = ptr::null_mut();
    assert_eq!(
        unsafe { kt_session_new_pty(4, 1, 0, ptr::null(), 0, &mut nothing_to_run) },
        KtStatus::NullArgument,
    );
    assert!(nothing_to_run.is_null());
    assert_eq!(
        unsafe { kt_session_feed(ptr::null_mut(), b"x".as_ptr(), 1) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_write(ptr::null_mut(), b"x".as_ptr(), 1) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_snapshot_view(ptr::null(), ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_take_writes(ptr::null_mut(), ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_take_events(ptr::null_mut(), ptr::null_mut()) },
        KtStatus::NullArgument,
    );
    assert_eq!(
        unsafe { kt_session_set_wake(ptr::null_mut(), None, ptr::null_mut()) },
        KtStatus::NullArgument,
    );

    // Freeing null is a no-op, not a crash.
    unsafe { kt_session_free(ptr::null_mut()) };
    unsafe { kt_snapshot_free(ptr::null_mut()) };
}
