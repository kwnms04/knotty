//! Replay each recording and hold the result against its golden.
//!
//! A failure here is either knotty changing what it reports or the VT engine
//! changing what it renders. Which of the two it is shows in the diff.
//!
//! Run with `KNOTTY_UPDATE_GOLDENS=1` to write the goldens instead of
//! checking them. A plain run never writes.

use std::path::PathBuf;

/// The size the recordings were made at. Replaying at another one would show
/// a screen no application ever drew.
const COLS: u16 = 80;
const ROWS: u16 = 24;
const SCROLLBACK: usize = 1000;

/// The size the scripts run at. They are about the bytes that leave for the
/// child rather than about the screen, so the grid is only as big as it takes
/// to keep the golden readable beside them.
const SCRIPT_COLS: u16 = 20;
const SCRIPT_ROWS: u16 = 2;

fn directory(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn check(name: &str) {
    let recording = directory("recordings").join(format!("{name}.vt"));
    let bytes = std::fs::read(&recording)
        .unwrap_or_else(|error| panic!("read {}: {error}", recording.display()));

    hold_against_golden(
        name,
        knotty_harness::replay(&bytes, COLS, ROWS, SCROLLBACK)
            .unwrap_or_else(|error| panic!("replaying {name}: {error}")),
    );
}

/// The same for a script, which says what the child sent and what the app did
/// in the order they happened.
fn check_script(name: &str) {
    let script = directory("recordings").join(format!("{name}.vts"));
    let text = std::fs::read_to_string(&script)
        .unwrap_or_else(|error| panic!("read {}: {error}", script.display()));

    hold_against_golden(
        name,
        knotty_harness::replay_script(&text, SCRIPT_COLS, SCRIPT_ROWS, SCROLLBACK)
            .unwrap_or_else(|error| panic!("replaying {name}: {error}")),
    );
}

fn hold_against_golden(name: &str, produced: String) {
    let golden = directory("goldens").join(format!("{name}.golden"));

    if std::env::var_os("KNOTTY_UPDATE_GOLDENS").is_some() {
        std::fs::create_dir_all(golden.parent().unwrap()).expect("create goldens directory");
        std::fs::write(&golden, &produced)
            .unwrap_or_else(|error| panic!("write {}: {error}", golden.display()));
        return;
    }

    let expected = std::fs::read_to_string(&golden).unwrap_or_else(|error| {
        panic!(
            "read {}: {error}\nrun with KNOTTY_UPDATE_GOLDENS=1 to create it",
            golden.display(),
        )
    });

    if let Some(report) = knotty_harness::diff(&expected, &produced) {
        panic!("{name}: {report}\nif this change is meant, rerun with KNOTTY_UPDATE_GOLDENS=1",);
    }
}

#[test]
fn vim() {
    check("vim");
}

#[test]
fn tmux() {
    check("tmux");
}

#[test]
fn htop() {
    check("htop");
}

/// vim over a file of CJK, combining marks and emoji. The three application
/// recordings above are all ASCII, so without this the wide-cell flags and the
/// grapheme table — the parts of the snapshot most likely to move when the
/// engine does — would go unwatched.
///
/// The file's ZWJ sequences reach the terminal as the literal `<200d>` vim
/// draws an unprintable character as, so what a joined cluster comes to is not
/// something a capture of an editor can show. The skin-tone modifier is: it is
/// printable, so vim prints it, and the cell it lands on carries both.
#[test]
fn unicode() {
    check("unicode");
}

/// Not a capture: no application was going to ring the bell, copy to the
/// clipboard, ask for the title back and open a synchronized output block in
/// the same run, and the four recordings above touch none of them. So the
/// stream is written by hand, in that order, with the block spanning a chunk
/// boundary — a block that fits in one feed says nothing about suppression.
///
/// The enquiry and tertiary attributes queries are here for the same reason.
/// tmux asks the other three the terminal answers; nothing asks these two.
#[test]
fn synthetic() {
    check("synthetic");
}

/// The same arrow key, before and after the child asks for cursor key
/// application mode. Two keystrokes that differ only in what arrived between
/// them, which is the whole of what the script format was added for.
#[test]
fn cursor_keys() {
    check_script("cursor-keys");
}

/// One key under each of the four modifiers. What each comes to is the
/// engine's answer and not knotty's, and pinning it here is what keeps an
/// engine that changes its mind about one from doing so quietly.
#[test]
fn modifiers() {
    check_script("modifiers");
}

/// Keys arriving while an input method is composing, which the terminal is
/// meant to stay silent about until the composition ends.
#[test]
fn composing() {
    check_script("composing");
}

/// A line as wide as the grid, and then a narrower grid. What the reflow made
/// of it is the screen this pins, and the in-band size report beside it is
/// what the child was told the new size was.
#[test]
fn reflow() {
    check_script("reflow");
}

/// The same line, folded and then unfolded by a second resize. What this pins
/// is the widening — and, because it is a round trip, that the fold before it
/// kept every cell it was given.
#[test]
fn unfold() {
    check_script("unfold");
}

/// Clicks on either side of the sequence that asks to hear about them. The
/// one right behind it is the point: the mode that decides arrives as output,
/// so a branch taken anywhere but beside the terminal would still be reading
/// the old one.
#[test]
fn mouse() {
    check_script("mouse");
}

/// The wheel with nobody having asked for it, which is the viewport walking
/// back into the scrollback. The screen is what this pins, since that branch
/// says nothing to the child at all.
#[test]
fn wheel_scrollback() {
    check_script("wheel-scrollback");
}

/// The wheel with mouse reporting on, which is what an editor turns on: it
/// becomes a mouse code like any other button.
#[test]
fn wheel_report() {
    check_script("wheel-report");
}

/// The wheel on the alternate screen with alternate scroll left on, which is
/// where a pager starts: it becomes the cursor keys the program already
/// reads.
#[test]
fn wheel_alt_scroll() {
    check_script("wheel-alt-scroll");
}

/// The window coming and going, on either side of the mode that decides
/// whether the child hears about it. vim's `autoread` lives down this path.
#[test]
fn focus() {
    check_script("focus");
}

/// A drag over cells and the same two corners as a block, each copied. What
/// this pins is that both ends of the gesture travel on every call: nothing
/// above the boundary ever computes a range.
#[test]
fn selection_drag() {
    check_script("selection-drag");
}

/// A double-click and then a drag, over a space and back past the anchor.
/// The pointer standing on nothing is the case the two-ended call exists for,
/// and a golden is what says the selection held rather than blinked.
#[test]
fn selection_word() {
    check_script("selection-word");
}

/// A triple-click on a line the terminal folded. The copy is the point: the
/// line comes back as the one line it was typed as.
#[test]
fn selection_line() {
    check_script("selection-line");
}

/// A selection output pushed into the scrollback. The engine tracks it, so
/// the copy is over the same text — which is why the app keeps no
/// coordinates of its own.
#[test]
fn selection_scrollback() {
    check_script("selection-scrollback");
}

/// The clipboard on its way to the child, on either side of the mode that
/// asks for the wrapping. What the golden's `writes` holds is the sanitizing:
/// the control bytes that would have been read as commands are spaces, the
/// end sequence in the content did not break out of the wrapping, and the
/// newlines are text inside it and carriage returns without it. Nothing on
/// the way in could have skipped any of it.
#[test]
fn paste() {
    check_script("paste");
}
