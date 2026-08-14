//! The public C ABI is the seam: everything here goes through the same
//! entry points a real consumer calls.

use std::mem::MaybeUninit;
use std::ptr;

use knotty_ffi::{
    Attribute, Cell, Cursor, CursorShape, Dirty, KtSession, KtSnapshot, KtSnapshotView, KtStatus,
    KtText, Rgb, Row, RowFlag, SelectionRange, Underline, kt_abi_version, kt_session_feed,
    kt_session_free, kt_session_new_detached, kt_session_set_selection, kt_session_take_snapshot,
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
