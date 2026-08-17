//! Replay a recorded terminal stream and describe what it produced.
//!
//! The description is what a golden file holds, so it has to be a total
//! function of what a consumer gets: the screen, the bytes queued for the
//! child, the events queued for the app, and how many times the session said
//! there was something to take. Comparing two of them is comparing bytes.
//!
//! Everything here goes through the public C ABI. The harness is only worth
//! anything if the path it checks is the path an application takes.

use std::cell;
use std::ffi::c_void;
use std::fmt::Write as _;
use std::ptr;

use knotty_ffi::{
    Attribute, Cell, ClipboardTarget, CursorShape, Dirty, KtBytes, KtChildState, KtEvent,
    KtEventKind, KtEvents, KtSessionState, KtSnapshotView, KtStatus, KtText, RowFlag, Underline,
    kt_session_feed, kt_session_free, kt_session_new_detached, kt_session_set_wake,
    kt_session_take_events, kt_session_take_snapshot, kt_session_take_writes, kt_snapshot_free,
    kt_snapshot_view,
};

/// The format the goldens are written in. Bump it when the encoding changes,
/// so a stale golden fails loudly rather than diffing line by line.
const FORMAT: &str = "knotty-golden 3";

/// A recorded stream arrives from a PTY in pieces, not all at once, and an
/// escape sequence can straddle two of them. Replaying in chunks keeps the
/// harness honest about that.
const CHUNK: usize = 1024;

/// Count one wake into the counter `userdata` points at.
///
/// A detached session drives everything on the calling thread, so a `Cell` is
/// all the counter needs — nothing here crosses threads. It stays spelled out
/// because `Cell` is already the grid's.
extern "C" fn count_wake(userdata: *mut c_void) {
    let wakes = unsafe { &*userdata.cast::<cell::Cell<u32>>() };
    wakes.set(wakes.get() + 1);
}

/// Feed a recording to a fresh session and describe what it left behind.
///
/// # Errors
///
/// Returns the failing call's status when the boundary reports one.
pub fn replay(recording: &[u8], cols: u16, rows: u16, scrollback: usize) -> Result<String, String> {
    let mut session = ptr::null_mut();
    check("kt_session_new_detached", unsafe {
        kt_session_new_detached(cols, rows, scrollback, &mut session)
    })?;

    // The wake callback is handed a pointer to this, so it has to outlive the
    // session — which is freed at the end of this call, before it goes.
    let wakes = cell::Cell::new(0);

    let described = (|| {
        check("kt_session_set_wake", unsafe {
            kt_session_set_wake(
                session,
                Some(count_wake),
                ptr::from_ref(&wakes).cast_mut().cast(),
            )
        })?;

        for chunk in recording.chunks(CHUNK) {
            check("kt_session_feed", unsafe {
                kt_session_feed(session, chunk.as_ptr(), chunk.len())
            })?;
        }

        // Both runs are borrowed from the session and stay valid until the
        // same call is made again, which it is not.
        let mut writes = std::mem::MaybeUninit::<KtBytes>::uninit();
        check("kt_session_take_writes", unsafe {
            kt_session_take_writes(session, writes.as_mut_ptr())
        })?;
        let writes = unsafe { writes.assume_init() };

        let mut events = std::mem::MaybeUninit::<KtEvents>::uninit();
        check("kt_session_take_events", unsafe {
            kt_session_take_events(session, events.as_mut_ptr())
        })?;
        let events = unsafe { events.assume_init() };

        let mut snapshot = ptr::null_mut();
        check("kt_session_take_snapshot", unsafe {
            kt_session_take_snapshot(session, &mut snapshot)
        })?;

        let mut view = std::mem::MaybeUninit::<KtSnapshotView>::uninit();
        let status = unsafe { kt_snapshot_view(snapshot, view.as_mut_ptr()) };
        let described = check("kt_snapshot_view", status).map(|()| {
            describe(
                &unsafe { view.assume_init() },
                &writes,
                &events,
                wakes.get(),
            )
        });

        unsafe { kt_snapshot_free(snapshot) };
        described
    })();

    unsafe { kt_session_free(session) };
    described
}

fn check(call: &str, status: KtStatus) -> Result<(), String> {
    match status {
        KtStatus::Ok => Ok(()),
        other => Err(format!("{call} returned {other:?}")),
    }
}

/// Write out everything the session handed back.
fn describe(view: &KtSnapshotView, writes: &KtBytes, events: &KtEvents, wakes: u32) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "{FORMAT}");
    describe_outbound(&mut out, writes, events, wakes);
    let _ = writeln!(out, "size {} {}", view.cols, view.rows);
    // Constant for every recording — a detached session has no child and no
    // thread to lose. Written down anyway: what a replay must never say is
    // that some child of its own is running, and a line that never moves is
    // how that stays checked.
    let _ = writeln!(
        out,
        "child {}",
        child_name(view.child_state, view.child_exit_code)
    );
    let _ = writeln!(out, "session {}", session_name(view.session_state));
    let _ = writeln!(out, "dirty {}", dirty_name(view.dirty));
    let _ = writeln!(
        out,
        "selection {}",
        if view.has_selection {
            "present"
        } else {
            "none"
        }
    );
    let _ = writeln!(
        out,
        "cursor {} {} {} {}",
        view.cursor.x,
        view.cursor.y,
        if view.cursor.visible {
            "visible"
        } else {
            "hidden"
        },
        shape_name(view.cursor.shape),
    );
    let _ = writeln!(out, "title {}", quoted(text_of(view.title)));
    let _ = writeln!(out, "pwd {}", quoted(text_of(view.pwd)));
    let _ = writeln!(out, "graphemes {}", view.grapheme_count);

    for row in 0..view.rows {
        describe_row(&mut out, view, row);
    }
    out
}

/// Everything that left the session by a route other than the screen: what
/// was queued for the child, what was queued for the app, and how many times
/// the session said there was something to take.
fn describe_outbound(out: &mut String, writes: &KtBytes, events: &KtEvents, wakes: u32) {
    let _ = writeln!(out, "wakes {wakes}");

    let queued = if writes.len == 0 {
        &[][..]
    } else {
        unsafe { std::slice::from_raw_parts(writes.bytes, writes.len) }
    };
    let _ = writeln!(out, "writes {}", quoted_bytes(queued));

    let _ = writeln!(out, "events {} dropped {}", events.len, events.dropped);
    for index in 0..events.len {
        let event = unsafe { *events.events.add(index) };
        describe_event(out, index, &event);
    }
}

fn describe_event(out: &mut String, index: usize, event: &KtEvent) {
    let _ = match event.kind {
        KtEventKind::Bell => writeln!(out, "event {index} bell"),
        KtEventKind::ClipboardWrite => writeln!(
            out,
            "event {index} clipboard-write {} {}",
            clipboard_target_name(event.clipboard_target),
            quoted(text_of(event.text)),
        ),
        // No child stands behind a detached session, so nothing a recording
        // holds can produce one. The arm is here because the kinds are what a
        // consumer switches on, and one left out is one nobody notices.
        KtEventKind::ChildExited => {
            writeln!(out, "event {index} child-exited {}", event.exit_code)
        }
    };
}

fn describe_row(out: &mut String, view: &KtSnapshotView, row: u16) {
    let state = unsafe { *view.row_state.add(usize::from(row)) };

    let mut flags = Vec::new();
    if state.flags & RowFlag::Dirty as u8 != 0 {
        flags.push("dirty");
    }
    if state.flags & RowFlag::Wrapped as u8 != 0 {
        flags.push("wrapped");
    }
    let selection = if state.flags & RowFlag::Selected as u8 == 0 {
        "none".to_owned()
    } else {
        format!("{} {}", state.selection_start, state.selection_end)
    };
    let _ = writeln!(
        out,
        "row {row} flags {} selection {selection}",
        if flags.is_empty() {
            "-".to_owned()
        } else {
            flags.join(",")
        },
    );

    // The row's text, so that a human reading a diff sees what the screen
    // said before counting hex.
    let _ = writeln!(out, "text {}", quoted(row_text(view, row)));

    for col in 0..view.cols {
        let cell = cell_at(view, row, col);
        let codepoints: Vec<String> = codepoints_of(view, &cell)
            .iter()
            .map(|codepoint| format!("{codepoint:04X}"))
            .collect();
        let _ = writeln!(
            out,
            "cell {row} {col} {} {} {:04x} {} {}",
            rgb(cell.foreground.r, cell.foreground.g, cell.foreground.b),
            rgb(cell.background.r, cell.background.g, cell.background.b),
            cell.attributes,
            underline_name(cell.underline),
            codepoints.join(" "),
        );
    }
}

/// The grid is a flat row-major array, so a cell costs an index.
fn cell_at(view: &KtSnapshotView, row: u16, col: u16) -> Cell {
    unsafe {
        *view
            .cells
            .add(usize::from(row) * usize::from(view.cols) + usize::from(col))
    }
}

/// The characters of a row, with anything unprintable shown as a dot.
fn row_text(view: &KtSnapshotView, row: u16) -> String {
    (0..view.cols)
        .map(|col| {
            let cell = cell_at(view, row, col);
            match codepoints_of(view, &cell)
                .first()
                .and_then(|c| char::from_u32(*c))
            {
                Some('\0') | None => ' ',
                Some(character) if character.is_control() => '.',
                Some(character) => character,
            }
        })
        .collect()
}

/// A cell's text, resolved through the grapheme table when it does not fit.
fn codepoints_of(view: &KtSnapshotView, cell: &Cell) -> Vec<u32> {
    if cell.attributes & Attribute::Overflow as u16 == 0 {
        return vec![cell.codepoint];
    }

    let index = cell.codepoint as usize;
    assert!(index < view.grapheme_count, "grapheme index out of range");
    let len = unsafe { *view.graphemes.add(index) } as usize;
    assert!(
        index + 1 + len <= view.grapheme_count,
        "grapheme run runs off the table",
    );

    (0..len)
        .map(|offset| unsafe { *view.graphemes.add(index + 1 + offset) })
        .collect()
}

fn text_of(text: KtText) -> String {
    if text.len == 0 {
        return String::new();
    }
    let bytes = unsafe { std::slice::from_raw_parts(text.bytes, text.len) };
    std::str::from_utf8(bytes)
        .expect("the boundary promises UTF-8")
        .to_owned()
}

fn quoted(text: impl AsRef<str>) -> String {
    let escaped = text.as_ref().replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Bytes as themselves where they are printable ASCII and as `\xNN` where
/// they are not.
///
/// What the terminal answers a query with is mostly an escape sequence, so a
/// reader can see whether the answer carries what it should — which is the
/// whole point of writing the writer queue down. Hex throughout would hide
/// that behind arithmetic.
fn quoted_bytes(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for &byte in bytes {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'"' => out.push_str("\\\""),
            0x20..=0x7e => out.push(char::from(byte)),
            other => {
                let _ = write!(out, "\\x{other:02x}");
            }
        }
    }
    out.push('"');
    out
}

fn clipboard_target_name(target: ClipboardTarget) -> &'static str {
    match target {
        ClipboardTarget::Standard => "standard",
        ClipboardTarget::Selection => "selection",
        ClipboardTarget::Primary => "primary",
    }
}

fn rgb(r: u8, g: u8, b: u8) -> String {
    format!("{r:02x}{g:02x}{b:02x}")
}

fn child_name(child: KtChildState, exit_code: i32) -> String {
    match child {
        KtChildState::None => "none".to_owned(),
        KtChildState::Running => "running".to_owned(),
        KtChildState::Exited => format!("exited {exit_code}"),
    }
}

fn session_name(session: KtSessionState) -> &'static str {
    match session {
        KtSessionState::Ok => "ok",
        KtSessionState::Broken => "broken",
    }
}

fn dirty_name(dirty: Dirty) -> &'static str {
    match dirty {
        Dirty::Clean => "clean",
        Dirty::Partial => "partial",
        Dirty::Full => "full",
    }
}

fn shape_name(shape: CursorShape) -> &'static str {
    match shape {
        CursorShape::Block => "block",
        CursorShape::Bar => "bar",
        CursorShape::Underline => "underline",
        CursorShape::BlockHollow => "block-hollow",
        CursorShape::Unknown => "unknown",
    }
}

fn underline_name(underline: Underline) -> &'static str {
    match underline {
        Underline::None => "none",
        Underline::Single => "single",
        Underline::Double => "double",
        Underline::Curly => "curly",
        Underline::Dotted => "dotted",
        Underline::Dashed => "dashed",
        Underline::Unknown => "unknown",
    }
}

/// Say where two descriptions part company, in terms of the screen rather
/// than of byte offsets.
///
/// Every cell line names its own row and column, so quoting the lines that
/// differ is already the answer to "what changed where".
#[must_use]
pub fn diff(golden: &str, produced: &str) -> Option<String> {
    if golden == produced {
        return None;
    }

    let want: Vec<&str> = golden.lines().collect();
    let got: Vec<&str> = produced.lines().collect();

    // A golden written by an older encoding differs on every line, which says
    // nothing useful. Its first line says which encoding wrote it.
    if want.first() != got.first() {
        return Some(format!(
            "the golden was written in a different format\n  golden   {}\n  produced {}\n",
            want.first().unwrap_or(&"<empty>"),
            got.first().unwrap_or(&"<empty>"),
        ));
    }

    const SHOWN: usize = 12;
    let mut report = String::from("the screen does not match the golden\n");
    let mut differing = 0;

    for number in 0..want.len().max(got.len()) {
        let (want, got) = (want.get(number), got.get(number));
        if want == got {
            continue;
        }
        differing += 1;
        if differing <= SHOWN {
            let _ = writeln!(report, "  line {}:", number + 1);
            let _ = writeln!(report, "    golden   {}", want.unwrap_or(&"<missing>"));
            let _ = writeln!(report, "    produced {}", got.unwrap_or(&"<missing>"));
        }
    }

    if differing > SHOWN {
        let _ = writeln!(report, "  ... and {} more lines", differing - SHOWN);
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::{diff, quoted_bytes};

    #[test]
    fn queued_bytes_are_readable_where_they_can_be_and_escaped_where_they_cannot() {
        assert_eq!(quoted_bytes(b"\x1b]l\x1b\\"), r#""\x1b]l\x1b\\""#);
        assert_eq!(quoted_bytes("é".as_bytes()), r#""\xc3\xa9""#);
        assert_eq!(quoted_bytes(b"say \"hi\""), r#""say \"hi\"""#);
    }

    #[test]
    fn identical_descriptions_do_not_differ() {
        let same = "knotty-golden 1\nsize 2 1\ncell 0 0 x\n";
        assert!(diff(same, same).is_none());
    }

    #[test]
    fn a_report_names_the_line_and_shows_both_sides() {
        let report = diff(
            "knotty-golden 1\nsize 2 1\ncell 0 1 x\n",
            "knotty-golden 1\nsize 2 1\ncell 0 1 y\n",
        )
        .expect("the descriptions differ");

        assert!(report.contains("line 3:"), "{report}");
        assert!(report.contains("golden   cell 0 1 x"), "{report}");
        assert!(report.contains("produced cell 0 1 y"), "{report}");
    }

    #[test]
    fn a_report_shows_a_line_only_one_side_has() {
        let report = diff("same\nextra\n", "same\n").expect("the descriptions differ");

        assert!(report.contains("line 2:"), "{report}");
        assert!(report.contains("golden   extra"), "{report}");
        assert!(report.contains("produced <missing>"), "{report}");
    }

    #[test]
    fn a_golden_from_another_encoding_says_so_instead_of_diffing() {
        let report =
            diff("knotty-golden 0\nsize 2 1\n", "knotty-golden 1\nsize 2 1\n").expect("differ");

        assert!(report.contains("different format"), "{report}");
        assert!(!report.contains("line 2"), "{report}");
    }

    #[test]
    fn a_report_stops_listing_and_says_how_many_are_left() {
        let head = "knotty-golden 1\n";
        let golden: String =
            head.to_owned() + &(0..40).map(|l| format!("line {l}\n")).collect::<String>();
        let produced: String =
            head.to_owned() + &(0..40).map(|l| format!("other {l}\n")).collect::<String>();

        let report = diff(&golden, &produced).expect("the descriptions differ");
        assert!(report.contains("and 28 more lines"), "{report}");
    }
}
