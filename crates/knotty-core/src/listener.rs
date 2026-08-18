//! C4 — what knotty does about what the child asked for.
//!
//! The engine's callbacks are where the event table is actually implemented,
//! and this is that table: which requests reach the app, which are refused,
//! which are answered from a constant of ours, and what an answer on its way
//! back to the child has taken out of it.
//!
//! No engine type reaches here. `vt` hands over what the child offered as
//! knotty's own values, which is what lets each rule below be called — and
//! tested — without a terminal to feed bytes to. cf. `03-core.md` C4

use std::sync::{Arc, Mutex};

use crate::queue::{ClipboardTarget, Event, EventQueue};
use crate::writer::WriteQueue;

/// The largest clipboard payload knotty will carry.
///
/// A write past it is refused whole rather than cut short: nothing tells the
/// user their copy arrived as its first megabyte, so a truncated clipboard is
/// worse than one that did not change. With the queue's cap this is also what
/// bounds what undrained events can hold.
const CLIPBOARD_TEXT_CAP: usize = 1024 * 1024;

/// The one representation knotty takes off a clipboard write.
///
/// The engine normalizes OSC 52 and iTerm2's copy sequence into the same
/// shape, and both carry this. v1 has no rich clipboard to put anything else
/// on, so a write offering no plain text has nothing for us.
const CLIPBOARD_MIME: &[u8] = b"text/plain";

/// What an ENQ is answered with: knotty's name.
///
/// An answerback reaches the child as if it were typed, so it stays a fixed
/// string of ours — nothing that reaches the screen can steer what is sent.
const ANSWERBACK: &str = "knotty";

/// Name and version, the payload of the XTVERSION answer.
///
/// Programs pick a code path by this, so it has to be knotty's rather than the
/// `libghostty` the engine falls back to.
const VERSION_REPORT: &str = concat!("knotty ", env!("CARGO_PKG_VERSION"));

/// How the engine answers a title query: `OSC l <title> ST`.
const TITLE_REPORT_PREFIX: &[u8] = b"\x1b]l";

/// The same answer carrying no title.
const EMPTY_TITLE_REPORT: &[u8] = b"\x1b]l\x1b\\";

/// Why a clipboard write was refused.
///
/// Only the reasons knotty gives. The engine defines more, and none of them is
/// something this core can conclude.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardRefusal {
    /// Refused by policy — the payload is beyond what knotty will carry.
    Denied,
    /// The payload is not what the representation it came under promises.
    InvalidData,
    /// Nothing in the write is a representation knotty can act on.
    Unsupported,
}

/// One of the forms a clipboard write offers its payload in.
///
/// Borrowed for the length of the call that carried it and no longer. Nothing
/// promises the payload is text: `OSC 52` carries base64 of whatever the child
/// chose to send, and the C API documents the field as binary-safe — so what
/// the bytes mean under their MIME type is decided below.
pub struct Representation<'a> {
    /// What the child says the payload is.
    pub mime: &'a [u8],
    /// The payload as it arrived.
    pub data: &'a [u8],
}

/// What the terminal's answer to a query is given to.
pub type PtyWrite = Box<dyn FnMut(&[u8])>;

/// What a bell is given to.
pub type Bell = Box<dyn FnMut()>;

/// What decides what becomes of a clipboard write.
pub type ClipboardWriter =
    Box<dyn FnMut(ClipboardTarget, &[Representation<'_>]) -> Result<(), ClipboardRefusal>>;

/// What the engine hands back while it parses.
///
/// One value rather than a registration apiece: the C API keeps a single
/// userdata pointer, so everything the engine calls arrives through this.
///
/// The three queries knotty answers from a constant are not here. A callback
/// that would return the same bytes every time is a value, and the engine is
/// given it as one.
pub struct Listener {
    /// What an ENQ is answered with.
    pub answerback: &'static str,
    /// What an XTVERSION query is answered with.
    pub version: &'static str,
    /// Bytes the terminal wants sent to the child, which is every answer it
    /// makes.
    pub pty_write: PtyWrite,
    /// The child rang the bell.
    pub bell: Bell,
    /// The child asked for something to be put on a clipboard.
    pub clipboard_write: ClipboardWriter,
}

impl Listener {
    /// Wire the table to the two queues a session hands its news to.
    ///
    /// What is not here matters as much as what is. The pixel size is the
    /// renderer's to answer and the color scheme the app's, so those queries
    /// get no handler and the engine stays silent on them — knotty does not
    /// invent a value it cannot know. One handler covers all three size
    /// reports, so the size in cells goes silent with the two in pixels rather
    /// than being answered alongside made-up pixels. And a clipboard read is
    /// never forwarded at all, so there is nothing to answer and nothing goes
    /// out: answering would hand the child whatever the user last copied. cf.
    /// `docs/adr/0007-input-security.md`
    pub fn new(writes: Arc<WriteQueue>, events: Arc<Mutex<EventQueue>>) -> Self {
        Self {
            answerback: ANSWERBACK,
            version: VERSION_REPORT,
            pty_write: Box::new(move |bytes| writes.push(sanitized_answer(bytes))),
            bell: Box::new({
                let events = Arc::clone(&events);
                move || events.lock().expect("event queue lock").push(Event::Bell)
            }),
            clipboard_write: Box::new(move |target, offered| {
                // A refusal is how this says no. OSC 52 carries no
                // acknowledgement, so nothing reaches the child either way —
                // what it buys is that the app is never handed a payload it
                // should not act on.
                let Some(text) = clipboard_text(offered)? else {
                    return Ok(());
                };

                events
                    .lock()
                    .expect("event queue lock")
                    .push(Event::ClipboardWrite {
                        target,
                        text: text.to_owned(),
                    });
                Ok(())
            }),
        }
    }
}

/// What of a clipboard write reaches the app, if anything.
///
/// `Ok(None)` is a representation that is explicitly empty, which the engine's
/// contract says does not clear the clipboard. An app that wrote it faithfully
/// would wipe what the user last copied, so it stops here rather than going
/// out as a copy of nothing.
///
/// A write carrying no representation of ours at all is how the engine asks
/// for the clipboard to be cleared, and refusing it is the answer we want for
/// the same reason: acting on it faithfully would wipe what the user last
/// copied on the say-so of the child.
///
/// The payload is base64 of whatever the child chose to send, so it is bytes
/// and nothing promises they are text. What is not UTF-8 is malformed as
/// `text/plain`, and the event carries `KtText`, which promises UTF-8 to the
/// app. So it is refused here rather than decoded lossily. cf. ADR 0012
fn clipboard_text<'a>(offered: &[Representation<'a>]) -> Result<Option<&'a str>, ClipboardRefusal> {
    let Some(data) = offered
        .iter()
        .find(|representation| representation.mime == CLIPBOARD_MIME)
        .map(|representation| representation.data)
    else {
        return Err(ClipboardRefusal::Unsupported);
    };

    if data.is_empty() {
        return Ok(None);
    }
    if data.len() > CLIPBOARD_TEXT_CAP {
        return Err(ClipboardRefusal::Denied);
    }
    str::from_utf8(data)
        .map(Some)
        .map_err(|_| ClipboardRefusal::InvalidData)
}

/// Empty out an answer that would carry writable state back to the child.
///
/// A title query (`CSI 21 t`) is answered with the title, and a PTY write is
/// keyboard input as far as the shell can tell — so output nobody trusts can
/// set the title to a command, query it back, and have it typed. The engine
/// emits this one answer with no callback to fill in, which is why it is
/// caught here on the wire rather than refused earlier. The answer still goes
/// out: a program waiting on one must not hang. cf.
/// `docs/adr/0007-input-security.md`.
///
/// The icon label query is the other half of the pair the ADR names. Its
/// report is not written here because nothing emits one: an answer the filter
/// never sees costs a prefix to match and proves nothing when it does.
fn sanitized_answer(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(TITLE_REPORT_PREFIX) {
        return EMPTY_TITLE_REPORT;
    }
    bytes
}

#[cfg(test)]
mod tests {
    use super::{
        CLIPBOARD_TEXT_CAP, ClipboardRefusal, EMPTY_TITLE_REPORT, Representation, clipboard_text,
        sanitized_answer,
    };

    /// One `text/plain` representation, as the engine normalizes both copy
    /// sequences into.
    fn plain(data: &[u8]) -> [Representation<'_>; 1] {
        [Representation {
            mime: b"text/plain",
            data,
        }]
    }

    #[test]
    fn text_reaches_the_app() {
        assert_eq!(clipboard_text(&plain(b"hi")), Ok(Some("hi")));
    }

    /// The fuzzer cannot stand in for this: the bytes crash nothing once the
    /// binding layer stops building invalid values out of them, so what is
    /// under test is a refusal, not a survival. cf. ADR 0012
    #[test]
    fn a_payload_that_is_not_utf8_is_refused() {
        // FF FE, which no encoding of a `text/plain` payload produces.
        assert_eq!(
            clipboard_text(&plain(b"\xff\xfe")),
            Err(ClipboardRefusal::InvalidData),
        );
    }

    #[test]
    fn a_payload_past_the_cap_is_refused_rather_than_cut_short() {
        let payload = vec![b'x'; CLIPBOARD_TEXT_CAP + 1];

        assert_eq!(
            clipboard_text(&plain(&payload)),
            Err(ClipboardRefusal::Denied),
        );
        assert!(
            clipboard_text(&plain(&payload[1..])).is_ok(),
            "the cap itself was refused",
        );
    }

    /// The C API's way of asking for the clipboard to be cleared. Refused, so
    /// that the child cannot wipe what the user last copied.
    #[test]
    fn a_write_offering_nothing_is_refused() {
        assert_eq!(clipboard_text(&[]), Err(ClipboardRefusal::Unsupported));
    }

    /// Rich clipboard formats are not something v1 has anywhere to put.
    #[test]
    fn a_write_offering_no_plain_text_is_refused() {
        let offered = [Representation {
            mime: b"image/png",
            data: b"\x89PNG",
        }];

        assert_eq!(clipboard_text(&offered), Err(ClipboardRefusal::Unsupported));
    }

    /// An explicitly empty representation is not a request to clear, so it is
    /// taken and carries nothing.
    #[test]
    fn an_empty_payload_leaves_the_clipboard_alone() {
        assert_eq!(clipboard_text(&plain(b"")), Ok(None));
    }

    /// The one answer that would carry state the child set back to the child.
    #[test]
    fn a_title_report_goes_out_empty() {
        assert_eq!(
            sanitized_answer(b"\x1b]l; rm -rf /\x1b\\"),
            EMPTY_TITLE_REPORT,
        );
    }

    #[test]
    fn every_other_answer_goes_out_as_it_is() {
        // A primary device attributes answer, which carries nothing of the
        // child's.
        let answer = b"\x1b[?62;22c";

        assert_eq!(sanitized_answer(answer), answer);
    }
}
