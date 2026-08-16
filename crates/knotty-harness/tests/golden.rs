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

fn directory(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(name)
}

fn check(name: &str) {
    let recording = directory("recordings").join(format!("{name}.vt"));
    let golden = directory("goldens").join(format!("{name}.golden"));

    let bytes = std::fs::read(&recording)
        .unwrap_or_else(|error| panic!("read {}: {error}", recording.display()));
    let produced = knotty_harness::replay(&bytes, COLS, ROWS, SCROLLBACK)
        .unwrap_or_else(|error| panic!("replaying {name}: {error}"));

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

/// vim over a file of CJK, combining marks and ZWJ emoji. The three
/// application recordings above are all ASCII, so without this the wide-cell
/// flags and the grapheme table — the parts of the snapshot most likely to
/// move when the engine does — would go unwatched.
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
