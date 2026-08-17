//! B4 and B5: what a session costs its app while the child floods it.
//!
//! Run by hand in the reference environment the Definition of Done names, and
//! never in CI. cf. `docs/07-definition-of-done.md`, section B
//!
//! No benchmark framework, because neither number is a function's cost: both
//! are tail latencies of one thread while another is under load, so what has
//! to be built is the load and the clock, not a loop around a call. The exit
//! status is the verdict — zero when both gates were met.

use std::hint;
use std::process;
use std::thread;
use std::time::{Duration, Instant};

use knotty_core::PtySession;

/// The width of the viewport the Definition of Done measures in.
const COLS: u16 = 200;

/// Its height.
const ROWS: u16 = 60;

/// Scrollback deep enough that the flood is really scrolling into it, which
/// is where a runaway child's cost is. Not the hundred thousand of B9 — that
/// gate is about resizing, and this one must not be measuring it.
const SCROLLBACK: usize = 10_000;

/// How often a consumer comes for a frame: one refresh of a 120Hz display.
const FRAME: Duration = Duration::from_micros(8_333);

/// How many rounds each scenario measures — ten seconds of frames.
///
/// Enough that a p99 rests on a dozen samples rather than on one.
const ROUNDS: usize = 1_200;

/// How long the child is given to get up to speed before the clock starts.
const WARMUP: Duration = Duration::from_millis(500);

/// How long a keystroke is given before it counts as one that never arrived.
const PATIENCE: Duration = Duration::from_secs(1);

/// How many rounds may find no new frame before the run stops counting as a
/// runaway one.
///
/// One in a hundred, and generous at that: a child flooding a terminal leaves
/// a frame between any two refreshes of a 120Hz display. Enough of them and
/// the child stopped flooding, which makes every number of that run a session
/// at rest wearing a gate's name.
const STALLED: usize = ROUNDS / 100;

/// B4: taking the newest frame, while the child floods.
const SNAPSHOT_GATE: Duration = Duration::from_millis(1);

/// B5: a keystroke reaching the terminal, while the child floods.
const KEY_GATE: Duration = Duration::from_millis(2);

/// A child that floods its terminal and reads what is typed at it.
///
/// The reading half earns its place: a child that never read would let the
/// terminal's own input queue fill up, and what B5 timed after that would be
/// that queue refusing rather than knotty writing. So the flood goes to the
/// background and a reader stands in the foreground, which is also the shape
/// of every real child — a shell reads what is typed at it.
fn flooding() -> PtySession {
    let line = "x".repeat(usize::from(COLS) - 1);
    let child = format!("yes {line} & exec cat >/dev/null");
    let session = PtySession::new(
        b"/bin/sh",
        &[b"-c".to_vec(), child.into_bytes()],
        COLS,
        ROWS,
        SCROLLBACK,
    )
    .expect("a session with a flooding child");

    thread::sleep(WARMUP);
    session
}

/// B4 — how long the app waits to be handed the newest frame.
///
/// The take is what is timed and nothing else. Letting go of the frame
/// afterwards is the app's own housekeeping and falls outside the clock, and
/// so does the sleep that spaces the rounds out: the gate is about the
/// handover, not about waiting for one to be there.
///
/// Answers with the times of the rounds that were handed a frame, and how
/// many were handed none.
fn snapshot_takes(session: &PtySession) -> (Vec<Duration>, usize) {
    let mut taken = Vec::with_capacity(ROUNDS);
    let mut empty = 0;

    for _ in 0..ROUNDS {
        thread::sleep(FRAME);
        let start = Instant::now();
        let snapshot = session.take_snapshot();
        let handed_over = start.elapsed();

        // An empty mailbox is not a handover, and timing one measures how
        // fast the slot can be found empty. Kept out of the samples rather
        // than in them: a dozen of those would sit under everything else and
        // push the real tail out past the p99, which is the one number this
        // is for. Counted instead, because enough of them is a flood that
        // stopped.
        match snapshot {
            Some(_) => taken.push(handed_over),
            None => empty += 1,
        }
    }

    (taken, empty)
}

/// B5 — how long a keystroke waits before the terminal has it.
///
/// The end of the wait is the backlog falling back to zero, which is the I/O
/// thread having handed every queued byte to the terminal. It is spun on
/// rather than slept on: the shortest sleep the scheduler gives back is
/// longer than the whole of what is being measured.
///
/// The key carries its newline, because the child reads a line at a time —
/// which every child does until it asks not to. Without one nothing it typed
/// would ever be read, the terminal's own input queue would fill, and the
/// last rounds would be timing that queue overflowing rather than knotty
/// writing.
fn key_writes(session: &PtySession) -> Vec<Duration> {
    let mut waited = Vec::with_capacity(ROUNDS);

    for _ in 0..ROUNDS {
        thread::sleep(FRAME);
        // The backlog holds the terminal's answers as well as the app's
        // writes, so it is only the key's own latency while nothing else is
        // in it. This load asks no questions and gets no answers, and the
        // round before left the queue empty — asserted rather than assumed,
        // because a load changed later would quietly make this another
        // number.
        assert_eq!(
            session.backlog(),
            0,
            "something other than the key was waiting for the child",
        );

        let start = Instant::now();
        session.write(b"k\n").expect("the key to be queued");
        while session.backlog() != 0 {
            assert!(
                start.elapsed() < PATIENCE,
                "the key never reached the terminal",
            );
            hint::spin_loop();
        }
        waited.push(start.elapsed());
    }

    // A flood still flooding always has a frame waiting to be taken. One that
    // stopped is no load at all, and would have let every round above pass on
    // a session at rest.
    assert!(
        session.take_snapshot().is_some(),
        "the child was not flooding",
    );

    waited
}

/// The value at `p` percent of `sorted`, by nearest rank.
///
/// Nearest rank rather than an interpolation between neighbours: the number
/// reported is then one that was really measured, and a gate is easier to
/// argue with when its value happened.
fn percentile(sorted: &[Duration], p: usize) -> Duration {
    let rank = (sorted.len() * p).div_ceil(100).max(1);
    sorted[rank - 1]
}

/// Check the ranking against a list short enough to count by hand.
///
/// This bench reports no test result and is run by the person deciding
/// whether a milestone is finished, so an off-by-one here would be a gate
/// passed on the wrong sample with nothing to catch it. Cheap enough to run
/// every time rather than to leave somewhere it is not run at all.
fn check_percentile() {
    let hundred: Vec<Duration> = (1..=100).map(Duration::from_micros).collect();

    assert_eq!(percentile(&hundred, 50), Duration::from_micros(50));
    assert_eq!(percentile(&hundred, 99), Duration::from_micros(99));
    assert_eq!(percentile(&hundred, 100), Duration::from_micros(100));
    assert_eq!(percentile(&hundred[..1], 99), Duration::from_micros(1));
}

/// Print what was measured, and answer whether it met the gate.
fn report(gate_name: &str, what: &str, mut samples: Vec<Duration>, gate: Duration) -> bool {
    samples.sort_unstable();
    let p99 = percentile(&samples, 99);
    let met = p99 < gate;

    println!("{gate_name}  {what}");
    println!(
        "  n {}  p50 {:?}  p90 {:?}  p99 {:?}  max {:?}",
        samples.len(),
        percentile(&samples, 50),
        percentile(&samples, 90),
        p99,
        samples.last().expect("a sample"),
    );
    println!("  p99 < {gate:?} — {}", if met { "met" } else { "MISSED" },);

    met
}

fn main() {
    check_percentile();

    // A session apiece, because each scenario is measured against the flood
    // and nothing else: the keys B5 types come back as output B4 would have
    // had to draw. The first is let go of as the statement measuring it ends,
    // so the second never runs beside a flood of its own.
    //
    // Each scenario answers for its own load before anything is reported: a
    // run that measured a session at rest must not print a gate as met on the
    // way to saying so.
    let (takes, empty) = snapshot_takes(&flooding());
    assert!(
        empty <= STALLED,
        "the child was not flooding: {empty} of {ROUNDS} rounds had nothing to take",
    );
    let keys = key_writes(&flooding());

    let mut met = report(
        "B4",
        "snapshot taken under runaway output",
        takes,
        SNAPSHOT_GATE,
    );
    println!("  {empty} of {ROUNDS} rounds found no new frame");
    met &= report(
        "B5",
        "key written to the terminal under runaway output",
        keys,
        KEY_GATE,
    );

    process::exit(i32::from(!met));
}
