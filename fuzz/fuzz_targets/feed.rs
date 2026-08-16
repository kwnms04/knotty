//! Drive a detached session through its whole cycle on arbitrary bytes.
//!
//! The target is `knotty-core` rather than the C ABI the golden harness uses.
//! The boundary catches panics and turns them into a status, so fuzzing
//! through it would report every panic as a clean refusal and find nothing.
//!
//! What is under test is not one call but the round a session lives in: feed,
//! take the screen, drain both queues, feed again. A crash is the failure —
//! whatever the child's broken escape sequence draws is the engine's business,
//! and the golden harness is what watches that.

#![no_main]

use knotty_core::Session;
use libfuzzer_sys::fuzz_target;

/// The size the goldens are recorded at. Fuzzing another one would be no more
/// arbitrary and would stop a crashing input from being replayable there.
const COLS: u16 = 80;
const ROWS: u16 = 24;
const SCROLLBACK: usize = 1000;

/// How much of the input goes in per feed.
///
/// Small enough that an input of any interesting length rounds the cycle
/// several times, which is the point of the target. A sequence straddling the
/// boundary comes out of the fuzzer shifting bytes ahead of it, so the split
/// does not have to be part of the input.
const CHUNK: usize = 64;

fuzz_target!(|stream: &[u8]| {
    let Ok(mut session) = Session::new_detached(COLS, ROWS, SCROLLBACK) else {
        return;
    };
    // A session with nobody to wake keeps owing the wake it never pays, so
    // clearing the debt and making the call are reached only with one set.
    session.set_wake(Some(Box::new(|| {})));

    for (round, chunk) in stream.chunks(CHUNK).enumerate() {
        // Errors are outcomes, not failures: a full writer queue is what the
        // cap is for. Only a panic or a crash fails this target.
        let _ = session.feed(chunk);
        // Every other round, so that the round in between leaves a snapshot
        // unconsumed and the next publication has to carry its change marks
        // across rather than drop them.
        if round % 2 == 0 {
            let _ = session.take_snapshot();
        }
        session.take_writes();
        session.take_events();
    }
});
