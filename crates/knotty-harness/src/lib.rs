//! Replay a recorded terminal stream and describe the screen it produced.
//!
//! The description is what a golden file holds, so it has to be a total
//! function of the snapshot: everything a consumer can read is in it, and
//! nothing else is. Comparing two of them is comparing bytes.
//!
//! Everything here goes through the public C ABI. The harness is only worth
//! anything if the path it checks is the path an application takes.

use std::fmt::Write as _;
use std::ptr;

use knotty_ffi::{
    Attribute, Cell, CursorShape, Dirty, KtSnapshotView, KtStatus, KtText, RowFlag, Underline,
    kt_session_feed, kt_session_free, kt_session_new_detached, kt_session_take_snapshot,
    kt_snapshot_free, kt_snapshot_view,
};

/// The format the goldens are written in. Bump it when the encoding changes,
/// so a stale golden fails loudly rather than diffing line by line.
const FORMAT: &str = "knotty-golden 1";

/// A recorded stream arrives from a PTY in pieces, not all at once, and an
/// escape sequence can straddle two of them. Replaying in chunks keeps the
/// harness honest about that.
const CHUNK: usize = 1024;

/// Feed a recording to a fresh session and describe the screen it left.
///
/// # Errors
///
/// Returns the failing call's status when the boundary reports one.
pub fn replay(recording: &[u8], cols: u16, rows: u16, scrollback: usize) -> Result<String, String> {
    let mut session = ptr::null_mut();
    check("kt_session_new_detached", unsafe {
        kt_session_new_detached(cols, rows, scrollback, &mut session)
    })?;

    let described = (|| {
        for chunk in recording.chunks(CHUNK) {
            check("kt_session_feed", unsafe {
                kt_session_feed(session, chunk.as_ptr(), chunk.len())
            })?;
        }

        let mut snapshot = ptr::null_mut();
        check("kt_session_take_snapshot", unsafe {
            kt_session_take_snapshot(session, &mut snapshot)
        })?;

        let mut view = std::mem::MaybeUninit::<KtSnapshotView>::uninit();
        let status = unsafe { kt_snapshot_view(snapshot, view.as_mut_ptr()) };
        let described =
            check("kt_snapshot_view", status).map(|()| describe(&unsafe { view.assume_init() }));

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

/// Write out everything the snapshot says.
#[must_use]
pub fn describe(view: &KtSnapshotView) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "{FORMAT}");
    let _ = writeln!(out, "size {} {}", view.cols, view.rows);
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
        let cell = unsafe {
            *view
                .cells
                .add(usize::from(row) * usize::from(view.cols) + usize::from(col))
        };
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

/// The characters of a row, with anything unprintable shown as a dot.
fn row_text(view: &KtSnapshotView, row: u16) -> String {
    (0..view.cols)
        .map(|col| {
            let cell = unsafe {
                *view
                    .cells
                    .add(usize::from(row) * usize::from(view.cols) + usize::from(col))
            };
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

fn text_of(text: KtText) -> &'static str {
    if text.len == 0 {
        return "";
    }
    let bytes = unsafe { std::slice::from_raw_parts(text.bytes, text.len) };
    std::str::from_utf8(bytes).expect("the boundary promises UTF-8")
}

fn quoted(text: impl AsRef<str>) -> String {
    let escaped = text.as_ref().replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn rgb(r: u8, g: u8, b: u8) -> String {
    format!("{r:02x}{g:02x}{b:02x}")
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

    const SHOWN: usize = 12;
    let mut report = String::from("the screen does not match the golden\n");
    let mut differing = 0;

    for (number, (want, got)) in golden.lines().zip(produced.lines()).enumerate() {
        if want == got {
            continue;
        }
        differing += 1;
        if differing <= SHOWN {
            let _ = writeln!(report, "  line {}:", number + 1);
            let _ = writeln!(report, "    golden   {want}");
            let _ = writeln!(report, "    produced {got}");
        }
    }

    if differing > SHOWN {
        let _ = writeln!(report, "  ... and {} more lines", differing - SHOWN);
    }

    let (want, got) = (golden.lines().count(), produced.lines().count());
    if want != got {
        let _ = writeln!(report, "  golden has {want} lines, produced has {got}");
    }
    Some(report)
}

#[cfg(test)]
mod tests {
    use super::diff;

    #[test]
    fn identical_descriptions_do_not_differ() {
        assert!(diff("size 2 1\ncell 0 0 x\n", "size 2 1\ncell 0 0 x\n").is_none());
    }

    #[test]
    fn a_report_names_the_line_and_shows_both_sides() {
        let report = diff("size 2 1\ncell 0 1 x\n", "size 2 1\ncell 0 1 y\n")
            .expect("the descriptions differ");

        assert!(report.contains("line 2:"), "{report}");
        assert!(report.contains("golden   cell 0 1 x"), "{report}");
        assert!(report.contains("produced cell 0 1 y"), "{report}");
    }

    #[test]
    fn a_report_says_when_one_side_is_longer() {
        let report = diff("a\nb\n", "a\n").expect("the descriptions differ");
        assert!(
            report.contains("golden has 2 lines, produced has 1"),
            "{report}"
        );
    }

    #[test]
    fn a_report_stops_listing_and_says_how_many_are_left() {
        let golden: String = (0..40).map(|line| format!("line {line}\n")).collect();
        let produced: String = (0..40).map(|line| format!("other {line}\n")).collect();

        let report = diff(&golden, &produced).expect("the descriptions differ");
        assert!(report.contains("and 28 more lines"), "{report}");
    }
}
