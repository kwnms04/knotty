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
    Attribute, Cell, ClipboardTarget, CursorShape, Dirty, Key, KeyAction, KtBytes, KtChildState,
    KtEvent, KtEventKind, KtEvents, KtKeyEvent, KtSessionState, KtSnapshotView, KtStatus, KtText,
    Modifier, MouseAction, MouseButton, RowFlag, SelectionUnit, Underline,
    kt_session_copy_selection, kt_session_feed, kt_session_focus, kt_session_free, kt_session_key,
    kt_session_mouse, kt_session_new_detached, kt_session_resize, kt_session_scroll_viewport,
    kt_session_select, kt_session_set_wake, kt_session_take_events, kt_session_take_snapshot,
    kt_session_take_writes, kt_session_wheel, kt_snapshot_free, kt_snapshot_view,
};

/// The format the goldens are written in. Bump it when the encoding changes,
/// so a stale golden fails loudly rather than diffing line by line.
const FORMAT: &str = "knotty-golden 4";

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

/// One step of a recording: bytes the child sent, or an event the app made.
///
/// A recording of nothing but the first kind is what a `.vt` file holds, and
/// replaying one is unchanged by this. The second is what lets a golden hold
/// an encoding that turns on a mode: the sequence that sets the mode and the
/// key that is read against it have to be in one file, in order, or the
/// branch cannot be reproduced at all.
enum Step {
    /// Bytes from the child, fed to the engine.
    Out(Vec<u8>),
    /// A key event from the app.
    Key(KeyStep),
    /// A mouse event from the app, over one cell.
    Mouse {
        action: MouseAction,
        button: MouseButton,
        mods: u16,
        x: u16,
        y: u16,
    },
    /// A wheel turn from the app, in lines, over one cell.
    Wheel {
        delta_x: i32,
        delta_y: i32,
        x: u16,
        y: u16,
        mods: u16,
    },
    /// The window gaining or losing focus.
    Focus { gained: bool },
    /// A resize from the app: the new grid, and how big one cell now is.
    Resize {
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    },
    /// A selection gesture: where it began, where the pointer is now, and
    /// what it measures in.
    Select {
        anchor: (u16, u16),
        cell: (u16, u16),
        unit: SelectionUnit,
        rectangle: bool,
    },
    /// Take the selection as plain text, which the description carries.
    Copy,
    /// The viewport moving of the app's own accord, which is what a drag out
    /// of the window asks for. Up positive.
    Scroll { lines: i32 },
}

/// A key event and the text it carries, which the event borrows.
struct KeyStep {
    action: KeyAction,
    key: Key,
    mods: u16,
    consumed_mods: u16,
    composing: bool,
    text: Vec<u8>,
}

impl KeyStep {
    /// The event as the boundary takes it, borrowing this step's text.
    fn event(&self) -> KtKeyEvent {
        KtKeyEvent {
            action: self.action,
            key: self.key,
            mods: self.mods,
            consumed_mods: self.consumed_mods,
            composing: self.composing,
            text: KtText {
                bytes: self.text.as_ptr(),
                len: self.text.len(),
            },
        }
    }
}

/// Read a script into the steps it names.
///
/// One step per line, blank lines and `#` comments ignored:
///
/// ```text
/// out "\x1b[?1h"
/// key ArrowUp
/// key A ctrl
/// key A alt consumed=alt "å"
/// key Enter composing
/// resize 10 4 8 16
/// mouse press left 3 1
/// wheel 0 -2 3 1
/// focus gained
/// select 0 0 5 0 word
/// scroll -2
/// copy
/// ```
///
/// `out` takes a quoted run of bytes, written the way the golden writes one.
/// `key` takes a key's name, then any of `ctrl`, `shift`, `alt`, `super`,
/// `release`, `repeat`, `composing` and `consumed=<mods>`, and last of all a
/// quoted run for what the layout made of the key. A key is a press with
/// nothing held unless a word says otherwise. `resize` takes the new grid in
/// cells and then one cell in pixels, in that order.
///
/// `mouse` takes an action — `press`, `release` or `motion` — then a button
/// — `left`, `right`, `middle` or `none` — then the cell, then any modifiers.
/// `wheel` takes the two deltas in lines, up and right positive, then the
/// cell, then any modifiers. `focus` takes `gained` or `lost`.
///
/// `select` takes the anchor cell, then the cell the pointer is over, then a
/// unit — `cell`, `word` or `line` — and then `rectangle` if the two ends are
/// opposite corners of a block. `scroll` takes a count of lines, up positive.
/// `copy` takes nothing: what it took comes out in the description.
fn parse(script: &str) -> Result<Vec<Step>, String> {
    script
        .lines()
        .enumerate()
        .filter(|(_, line)| {
            let line = line.trim_start();
            !line.is_empty() && !line.starts_with('#')
        })
        .map(|(number, line)| step(line).map_err(|error| format!("line {}: {error}", number + 1)))
        .collect()
}

fn step(line: &str) -> Result<Step, String> {
    // A quoted run is the last thing on a line, so everything before the
    // first quote is words and everything from it is bytes.
    let (words, quoted) = match line.find('"') {
        Some(at) => (&line[..at], Some(unquote(line[at..].trim_end())?)),
        None => (line, None),
    };
    let mut words = words.split_whitespace();

    match words.next() {
        Some("out") => {
            if let Some(word) = words.next() {
                return Err(format!("out takes bytes and nothing else, not {word}"));
            }
            Ok(Step::Out(quoted.ok_or("out says nothing to feed")?))
        }
        Some("key") => key_step(words, quoted.unwrap_or_default()).map(Step::Key),
        Some("resize") => {
            if quoted.is_some() {
                return Err("resize takes numbers and nothing else".to_owned());
            }
            resize_step(words)
        }
        Some("mouse") => mouse_step(words),
        Some("wheel") => wheel_step(words),
        Some("select") => select_step(words),
        Some("copy") => match words.next() {
            None => Ok(Step::Copy),
            Some(word) => Err(format!("copy takes nothing, not {word}")),
        },
        Some("scroll") => Ok(Step::Scroll {
            lines: number(words.next().ok_or("scroll names no count of lines")?)?,
        }),
        Some("focus") => match words.next() {
            Some("gained") => Ok(Step::Focus { gained: true }),
            Some("lost") => Ok(Step::Focus { gained: false }),
            other => Err(format!(
                "focus is gained or lost, not {}",
                other.unwrap_or("nothing")
            )),
        },
        Some(other) => Err(format!("{other} is not a directive this format knows")),
        None => Err("a line with only a quoted run says nothing to do with it".to_owned()),
    }
}

/// A resize: the grid in cells, then one cell in pixels.
///
/// All four are named rather than defaulted. The pixel pair is the half of a
/// resize that is easiest to leave out and hardest to notice missing, so a
/// script that leaves it out is a script that says so.
fn resize_step<'a>(words: impl Iterator<Item = &'a str>) -> Result<Step, String> {
    let numbers: Vec<&str> = words.collect();
    let [cols, rows, cell_width, cell_height] = numbers[..] else {
        return Err(format!(
            "resize takes cols, rows, cell width and cell height, not {} words",
            numbers.len()
        ));
    };
    Ok(Step::Resize {
        cols: number(cols)?,
        rows: number(rows)?,
        cell_width: number(cell_width)?,
        cell_height: number(cell_height)?,
    })
}

/// A mouse event: what happened to which button, over which cell, under
/// whatever was held.
fn mouse_step<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<Step, String> {
    let action = match words.next() {
        Some("press") => MouseAction::Press,
        Some("release") => MouseAction::Release,
        Some("motion") => MouseAction::Motion,
        other => {
            return Err(format!(
                "a mouse action is press, release or motion, not {}",
                other.unwrap_or("nothing")
            ));
        }
    };
    let button = match words.next() {
        Some("none") => MouseButton::None,
        Some("left") => MouseButton::Left,
        Some("right") => MouseButton::Right,
        Some("middle") => MouseButton::Middle,
        other => {
            return Err(format!(
                "a mouse button is none, left, right or middle, not {}",
                other.unwrap_or("nothing")
            ));
        }
    };
    let x = number(words.next().ok_or("mouse names no column")?)?;
    let y = number(words.next().ok_or("mouse names no row")?)?;
    Ok(Step::Mouse {
        action,
        button,
        mods: held(words)?,
        x,
        y,
    })
}

/// A wheel turn: both deltas in lines, then the cell it happened over.
///
/// Both are named even where one is zero. A wheel that only ever turns one
/// way in a script is a script that says which way the other one would be.
fn wheel_step<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<Step, String> {
    let delta_x = number(words.next().ok_or("wheel names no sideways delta")?)?;
    let delta_y = number(words.next().ok_or("wheel names no delta")?)?;
    let x = number(words.next().ok_or("wheel names no column")?)?;
    let y = number(words.next().ok_or("wheel names no row")?)?;
    Ok(Step::Wheel {
        delta_x,
        delta_y,
        x,
        y,
        mods: held(words)?,
    })
}

/// A selection gesture: the anchor cell, the cell the pointer is over, the
/// unit, and whether the two ends are corners of a block.
///
/// Both cells are named because both travel: a word or a line is widened from
/// each end, and a script naming one would be describing a call the boundary
/// does not have.
fn select_step<'a>(mut words: impl Iterator<Item = &'a str>) -> Result<Step, String> {
    let anchor_x = number(words.next().ok_or("select names no anchor column")?)?;
    let anchor_y = number(words.next().ok_or("select names no anchor row")?)?;
    let x = number(words.next().ok_or("select names no column")?)?;
    let y = number(words.next().ok_or("select names no row")?)?;
    let unit = match words.next() {
        Some("cell") => SelectionUnit::Cell,
        Some("word") => SelectionUnit::Word,
        Some("line") => SelectionUnit::Line,
        other => {
            return Err(format!(
                "a selection unit is cell, word or line, not {}",
                other.unwrap_or("nothing")
            ));
        }
    };
    let rectangle = match words.next() {
        None => false,
        Some("rectangle") => true,
        Some(word) => return Err(format!("{word} is not a word select knows")),
    };
    Ok(Step::Select {
        anchor: (anchor_x, anchor_y),
        cell: (x, y),
        unit,
        rectangle,
    })
}

/// Whatever modifier words are left, OR-ed together.
fn held<'a>(words: impl Iterator<Item = &'a str>) -> Result<u16, String> {
    words
        .map(modifiers)
        .try_fold(0, |held, bits| Ok(held | bits?))
}

fn number<T: std::str::FromStr>(word: &str) -> Result<T, String> {
    word.parse()
        .map_err(|_| format!("{word} is not a number this format can use"))
}

fn key_step<'a>(
    mut words: impl Iterator<Item = &'a str>,
    text: Vec<u8>,
) -> Result<KeyStep, String> {
    let name = words.next().ok_or("key names no key")?;
    let mut step = KeyStep {
        action: KeyAction::Press,
        key: named_key(name).ok_or_else(|| format!("{name} is not a key this format knows"))?,
        mods: 0,
        consumed_mods: 0,
        composing: false,
        text,
    };

    for word in words {
        match word {
            "release" => step.action = KeyAction::Release,
            "repeat" => step.action = KeyAction::Repeat,
            "composing" => step.composing = true,
            _ => match word.strip_prefix("consumed=") {
                Some(consumed) => step.consumed_mods = modifiers(consumed)?,
                None => step.mods |= modifiers(word)?,
            },
        }
    }
    Ok(step)
}

fn modifiers(list: &str) -> Result<u16, String> {
    list.split(',').try_fold(0, |held, name| {
        let bit = match name {
            "shift" => Modifier::Shift,
            "ctrl" => Modifier::Ctrl,
            "alt" => Modifier::Alt,
            "super" => Modifier::Super,
            other => return Err(format!("{other} is not a word this format knows")),
        };
        Ok(held | bit as u16)
    })
}

/// The keys the scripts name.
///
/// Not every key the header holds: a name this does not know fails the script
/// saying so, which is a line to add here rather than some other key pressed
/// by accident.
fn named_key(name: &str) -> Option<Key> {
    Some(match name {
        "A" => Key::A,
        "ArrowUp" => Key::ArrowUp,
        "Enter" => Key::Enter,
        _ => return None,
    })
}

/// Read back what [`quoted_bytes`] writes, and a character written out as
/// itself besides — a script saying `"å"` reads better than the two bytes it
/// stands for, and nothing here has to round-trip.
fn unquote(quoted: &str) -> Result<Vec<u8>, String> {
    let body = quoted
        .strip_prefix('"')
        .and_then(|body| body.strip_suffix('"'))
        .ok_or_else(|| format!("{quoted} is not a quoted run of bytes"))?;

    let mut bytes = Vec::new();
    let mut rest = body.chars();
    while let Some(character) = rest.next() {
        if character != '\\' {
            let mut encoded = [0; 4];
            bytes.extend_from_slice(character.encode_utf8(&mut encoded).as_bytes());
            continue;
        }
        match rest.next() {
            Some('\\') => bytes.push(b'\\'),
            Some('"') => bytes.push(b'"'),
            Some('x') => {
                let digits: String = rest.by_ref().take(2).collect();
                let byte = u8::from_str_radix(&digits, 16)
                    .map_err(|_| format!("\\x{digits} is not a byte"))?;
                bytes.push(byte);
            }
            Some(other) => return Err(format!("\\{other} is not an escape this format knows")),
            None => return Err("a backslash at the end escapes nothing".to_owned()),
        }
    }
    Ok(bytes)
}

/// Feed a recording to a fresh session and describe what it left behind.
///
/// # Errors
///
/// Returns the failing call's status when the boundary reports one.
pub fn replay(recording: &[u8], cols: u16, rows: u16, scrollback: usize) -> Result<String, String> {
    run(&[Step::Out(recording.to_vec())], cols, rows, scrollback)
}

/// The same for a script, which says what the child sent and what the app did
/// in the order they happened.
///
/// # Errors
///
/// Returns what the script says that this format does not know, or the
/// failing call's status when the boundary reports one.
pub fn replay_script(
    script: &str,
    cols: u16,
    rows: u16,
    scrollback: usize,
) -> Result<String, String> {
    run(&parse(script)?, cols, rows, scrollback)
}

fn run(steps: &[Step], cols: u16, rows: u16, scrollback: usize) -> Result<String, String> {
    // What every copy in the script took, in order, so that a golden holds
    // the text and not only the fact that a copy happened.
    let mut copies: Vec<Option<Vec<u8>>> = Vec::new();
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

        for step in steps {
            match step {
                Step::Out(bytes) => {
                    for chunk in bytes.chunks(CHUNK) {
                        check("kt_session_feed", unsafe {
                            kt_session_feed(session, chunk.as_ptr(), chunk.len())
                        })?;
                    }
                }
                Step::Key(key) => check("kt_session_key", unsafe {
                    kt_session_key(session, &key.event())
                })?,
                Step::Resize {
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                } => check("kt_session_resize", unsafe {
                    kt_session_resize(session, *cols, *rows, *cell_width, *cell_height)
                })?,
                Step::Mouse {
                    action,
                    button,
                    mods,
                    x,
                    y,
                } => check("kt_session_mouse", unsafe {
                    kt_session_mouse(session, *action, *button, *mods, *x, *y)
                })?,
                Step::Wheel {
                    delta_x,
                    delta_y,
                    x,
                    y,
                    mods,
                } => check("kt_session_wheel", unsafe {
                    kt_session_wheel(session, *delta_x, *delta_y, *x, *y, *mods)
                })?,
                Step::Focus { gained } => check("kt_session_focus", unsafe {
                    kt_session_focus(session, *gained)
                })?,
                Step::Select {
                    anchor,
                    cell,
                    unit,
                    rectangle,
                } => check("kt_session_select", unsafe {
                    kt_session_select(
                        session, anchor.0, anchor.1, cell.0, cell.1, *unit, *rectangle,
                    )
                })?,
                Step::Copy => {
                    let mut text = std::mem::MaybeUninit::<KtBytes>::uninit();
                    // Copied out rather than kept: the run is the session's
                    // until the next copy, and a script may make more than one.
                    match unsafe { kt_session_copy_selection(session, text.as_mut_ptr()) } {
                        KtStatus::Ok => {
                            let text = unsafe { text.assume_init() };
                            copies.push(Some(unsafe { borrowed(&text) }.to_vec()));
                        }
                        // Nothing was selected, which is an answer and not a
                        // failure — and one the golden says out loud.
                        KtStatus::NoValue => copies.push(None),
                        other => {
                            return Err(format!("kt_session_copy_selection returned {other:?}"));
                        }
                    }
                }
                Step::Scroll { lines } => check("kt_session_scroll_viewport", unsafe {
                    kt_session_scroll_viewport(session, *lines)
                })?,
            }
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
                &copies,
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

/// The run a `KtBytes` lends.
///
/// # Safety
///
/// The bytes must be ones the boundary lends for at least `'a`, which is
/// until the call that lent them is made again.
unsafe fn borrowed<'a>(bytes: &KtBytes) -> &'a [u8] {
    if bytes.len == 0 {
        // A null run is not a slice at any length, empty included, and an
        // empty answer is spelled with one.
        return &[];
    }
    unsafe { std::slice::from_raw_parts(bytes.bytes, bytes.len) }
}

fn check(call: &str, status: KtStatus) -> Result<(), String> {
    match status {
        KtStatus::Ok => Ok(()),
        other => Err(format!("{call} returned {other:?}")),
    }
}

/// Write out everything the session handed back.
fn describe(
    view: &KtSnapshotView,
    writes: &KtBytes,
    copies: &[Option<Vec<u8>>],
    events: &KtEvents,
    wakes: u32,
) -> String {
    let mut out = String::new();

    let _ = writeln!(out, "{FORMAT}");
    describe_outbound(&mut out, writes, copies, events, wakes);
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
fn describe_outbound(
    out: &mut String,
    writes: &KtBytes,
    copies: &[Option<Vec<u8>>],
    events: &KtEvents,
    wakes: u32,
) {
    let _ = writeln!(out, "wakes {wakes}");

    let queued = unsafe { borrowed(writes) };
    let _ = writeln!(out, "writes {}", quoted_bytes(queued));

    // The clipboard the app would have written, in the order the script asked
    // for it. `none` is a copy with nothing selected, which is an answer.
    let _ = writeln!(out, "copies {}", copies.len());
    for (index, copied) in copies.iter().enumerate() {
        let _ = match copied {
            Some(text) => writeln!(out, "copy {index} {}", quoted_bytes(text)),
            None => writeln!(out, "copy {index} none"),
        };
    }

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
