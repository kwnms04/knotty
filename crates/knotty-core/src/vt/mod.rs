//! The safe boundary over the VT engine.
//!
//! knotty owns this layer rather than taking a third-party one: the surface it
//! reaches is a fraction of a general-purpose wrapper's, and every raw pointer
//! it turns into a Rust value is one knotty has audited. cf.
//! `docs/adr/0012-own-the-binding-layer.md`
//!
//! It is not a renaming layer. The flattening loop that reads the engine's
//! render state into a [`Snapshot`] lives in [`capture`], which is what keeps
//! contracts the engine states in prose — a grapheme buffer sized by the
//! length the engine reports, a clipboard array that is null when empty —
//! enforceable in one place instead of at every call site. No engine type
//! appears in a signature outside this module. cf.
//! `docs/adr/0004-hide-vt-engine-types.md`
//!
//! The crate denies `unsafe` and lifts it here. This is the larger of the two
//! places it is lifted — the other is a single pre-exec hook in [`crate::io`],
//! which has no safe spelling. What the C API asks of a caller and what this
//! module does about it is written at each call.

// The one place the crate's ban is lifted. cf. `lib.rs`
#![allow(unsafe_code)]

mod capture;
mod key;
mod mouse;

use std::ffi::c_void;
use std::mem::MaybeUninit;
use std::ptr;

use libghostty_vt_sys as ffi;

use crate::key::{Key, KeyAction, KeyEvent};
use crate::listener::{ClipboardRefusal, Listener, Representation};
use crate::mouse::{MouseEvent, WheelEvent};
use crate::queue::ClipboardTarget;
use crate::session::{SelectionRange, SelectionUnit};
use crate::{Error, Result};

/// DEC mode 2026, synchronized output.
///
/// Named by number because the engine takes a number: a DEC mode is its value,
/// and only ANSI modes carry a bit alongside it.
const SYNC_OUTPUT: ffi::Mode = 2026;

/// DEC mode 2027, grapheme clustering.
///
/// Held on from the moment a terminal exists rather than waiting to be asked
/// for. With it off, a cell is a codepoint and its width is `wcwidth`'s: a
/// flag arrives as two cells and a family emoji as three, so what is one
/// character to the person reading the screen is several to everything that
/// draws it. With it on, a cell is a grapheme cluster, and the cluster is what
/// the snapshot's grapheme table carries.
///
/// The cost is that an application computing its own widths the old way
/// disagrees about where its columns are. That is the trade the mode exists to
/// name, and it is the one ghostty's own app takes by default. cf.
/// `docs/adr/0019-grapheme-clustering-on.md`.
const GRAPHEME_CLUSTERING: ffi::Mode = 2027;

/// DEC mode 1004, focus reporting.
///
/// The gate on whether the window gaining or losing focus is told to the
/// child at all. The engine encodes the report but holds no opinion about
/// when one is wanted, so the mode is read here.
const FOCUS_REPORTING: ffi::Mode = 1004;

/// DEC mode 2004, bracketed paste.
///
/// What says whether the child asked for pasted text to arrive wrapped, so
/// that it can tell a paste from typing. The engine's encoder takes the answer
/// as an argument and holds no opinion about where it comes from, so the mode
/// is read here.
const BRACKETED_PASTE: ffi::Mode = 2004;

/// DEC mode 1007, alternate scroll.
///
/// On by default, and what makes the wheel a cursor key on the alternate
/// screen: a pager that never asked for mouse reporting still scrolls,
/// because what it gets is the arrows it already reads.
const ALTERNATE_SCROLL: ffi::Mode = 1007;

/// The most lines one turn of the wheel is told to the child as.
///
/// A turn becomes one report or one arrow per line, and how many lines it was
/// crosses the boundary as a plain number — so this is what stands between a
/// caller whose arithmetic went wrong and an allocation of gigabytes. Far
/// more lines than the tallest screen anyone flicks across.
const MAX_WHEEL_LINES: u32 = 1024;

/// What knotty answers a device attributes query with.
///
/// A VT220 with color, which is what the engine implements. DA2's firmware
/// field is a version number and stays 0 while knotty is 0.x — it is set by
/// hand, not from the crate version. DA3's unit id is meaningless for an
/// emulator.
//
// The one piece of knotty's answering policy that lives in here rather than in
// `listener`: the engine's own struct is the only way to spell it.
const DEVICE_ATTRIBUTES: ffi::DeviceAttributes = ffi::DeviceAttributes {
    primary: ffi::DeviceAttributesPrimary {
        conformance_level: ffi::DA_CONFORMANCE_VT220,
        features: {
            let mut features = [0; 64];
            features[0] = ffi::DA_FEATURE_ANSI_COLOR;
            features
        },
        num_features: 1,
    },
    secondary: ffi::DeviceAttributesSecondary {
        device_type: ffi::DA_DEVICE_TYPE_VT220,
        firmware_version: 0,
        rom_cartridge: 0,
    },
    tertiary: ffi::DeviceAttributesTertiary { unit_id: 0 },
};

/// What one turn of the wheel came to.
///
/// The three branches come to two answers: something for the child, or a
/// screen that moved. Told apart here rather than inferred from an empty run
/// of bytes, since a turn the modes had nothing to say about is empty too.
pub enum Wheel {
    /// Bytes for the child — a mouse code, or the cursor keys.
    Bytes(Vec<u8>),
    /// The viewport moved into the scrollback, which is a frame to publish
    /// rather than anything to send.
    Scrolled,
}

/// A terminal, its render state, and the iterators a capture walks it with.
///
/// All four handles are the engine's, and none of them is thread-safe: the
/// thread that built a terminal is the only one that may drive it. cf.
/// `docs/adr/0003-snapshot-mailbox.md`
pub struct Terminal {
    raw: ffi::Terminal,
    render: ffi::RenderState,
    rows: ffi::RenderStateRowIterator,
    cells: ffi::RenderStateRowCells,
    /// The key encoder and its event, which are read against the terminal
    /// above every time a key arrives.
    keys: key::Keys,
    /// The same for the mouse, which has an encoder of its own.
    mouse: mouse::Mouse,
    /// Owned outright rather than held in a `Box`, so that the pointer the
    /// engine keeps stays valid while the terminal around it is borrowed.
    listener: *mut Listener,
}

impl Terminal {
    /// Build a terminal of `cols` by `rows` with `max_scrollback` lines behind
    /// it, and wire `listener` to it.
    pub fn new(cols: u16, rows: u16, max_scrollback: usize, listener: Listener) -> Result<Self> {
        // Made before the terminal, so that a failure here releases itself
        // and leaves nothing else to release.
        let keys = key::Keys::new()?;
        let mouse = mouse::Mouse::new()?;
        let options = ffi::TerminalOptions {
            cols,
            rows,
            max_scrollback,
        };
        let mut raw: ffi::Terminal = ptr::null_mut();
        // SAFETY: a null allocator asks for the engine's own, and `raw` is a
        // handle-sized out parameter.
        check(unsafe { ffi::ghostty_terminal_new(ptr::null(), &raw mut raw, options) })?;

        // Built one field at a time so that a failure part-way through is
        // released by this value's own `Drop` rather than by a second copy of
        // it written out here.
        let mut terminal = Self {
            raw,
            render: ptr::null_mut(),
            rows: ptr::null_mut(),
            cells: ptr::null_mut(),
            keys,
            mouse,
            listener: Box::into_raw(Box::new(listener)),
        };
        // SAFETY: as above, for each of the three handles a capture needs.
        check(unsafe { ffi::ghostty_render_state_new(ptr::null(), &raw mut terminal.render) })?;
        check(unsafe {
            ffi::ghostty_render_state_row_iterator_new(ptr::null(), &raw mut terminal.rows)
        })?;
        check(unsafe {
            ffi::ghostty_render_state_row_cells_new(ptr::null(), &raw mut terminal.cells)
        })?;

        terminal.listen()?;
        // SAFETY: the terminal is ours, and the call takes nothing but a mode
        // number and a flag alongside it.
        check(unsafe { ffi::ghostty_terminal_mode_set(raw, GRAPHEME_CLUSTERING, true) })?;
        Ok(terminal)
    }

    /// Give the engine the callbacks and the userdata they are handed back
    /// with.
    fn listen(&self) -> Result<()> {
        // The engine takes a callback's address rather than a pointer to one.
        // Each goes past a function pointer type on the way, because a
        // function *item* is a zero-sized value whose address says nothing.
        self.set(ffi::TerminalOption::USERDATA, self.listener.cast())?;
        self.set(
            ffi::TerminalOption::WRITE_PTY,
            on_pty_write as PtyWriteFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::BELL,
            on_bell as BellFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::ENQUIRY,
            on_enquiry as AnswerFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::XTVERSION,
            on_xtversion as AnswerFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::DEVICE_ATTRIBUTES,
            on_device_attributes as DeviceAttributesFn as *const c_void,
        )?;
        self.set(
            ffi::TerminalOption::CLIPBOARD_WRITE,
            on_clipboard_write as ClipboardWriteFn as *const c_void,
        )?;

        // A clipboard read gets no callback because the engine drops the
        // request without telling us: there is nothing to answer, so nothing
        // goes out. The color scheme query gets none because that one is an
        // answer the core cannot know. The size query is no longer one of
        // those — a resize tells the terminal how big a cell is — but it
        // stays unanswered all the same: what asks in pixels today asks the
        // pseudoterminal, and answering the escape sequence as well is a
        // reflection path to weigh rather than one to add in passing. cf.
        // `docs/adr/0007-input-security.md`
        Ok(())
    }

    /// Process `bytes` to completion.
    ///
    /// The engine's callbacks all fire inside this call, on this thread. It
    /// cannot fail: malformed input is the case it exists for.
    pub fn feed(&mut self, bytes: &[u8]) {
        // SAFETY: the engine reads `len` bytes and keeps none of them.
        unsafe { ffi::ghostty_terminal_vt_write(self.raw, bytes.as_ptr(), bytes.len()) }
        // What a feed can have set is the mouse reporting mode and, by way of
        // DECCOLM, the width a position is read against.
        self.mouse.invalidate();
    }

    /// Resize the grid, and say how big one cell now is in pixels.
    ///
    /// The primary screen reflows; the alternate screen does not, which is
    /// the engine's own rule and the right one — a full-screen program redraws
    /// itself for the new size rather than having its old screen folded.
    ///
    /// The pixel size travels with the counts because this is the one moment
    /// the engine can be told it: it is what an in-band size report carries,
    /// and a cell is only ever measured where the display is.
    ///
    /// # Errors
    ///
    /// [`Error::Engine`] when the engine refused the size, which a grid of no
    /// columns or no rows comes back as.
    pub fn resize(
        &mut self,
        cols: u16,
        rows: u16,
        cell_width: u32,
        cell_height: u32,
    ) -> Result<()> {
        // SAFETY: the terminal is ours, and the call takes nothing but
        // numbers alongside it.
        check(unsafe {
            ffi::ghostty_terminal_resize(self.raw, cols, rows, cell_width, cell_height)
        })?;
        self.mouse.invalidate();
        Ok(())
    }

    /// Encode a key event as the modes this terminal holds make of it.
    ///
    /// The answer is empty for every key that comes to nothing — a bare
    /// modifier, a release, anything at all while an input method is
    /// composing — which is not a failure.
    pub fn encode_key(&mut self, event: &KeyEvent) -> Result<Vec<u8>> {
        self.keys.encode(self.raw, event)
    }

    /// Encode a mouse event as the modes this terminal holds make of it.
    ///
    /// Empty for everything the terminal is meant to stay quiet about, which
    /// with reporting off is every mouse event there is — a click that goes
    /// nowhere is the mode working, not a failure.
    pub fn encode_mouse(&mut self, event: &MouseEvent) -> Result<Vec<u8>> {
        let grid = self.grid()?;
        self.mouse.encode(self.raw, event, grid)
    }

    /// Turn a wheel over the cell at `x`, `y` and answer what the child is to
    /// hear of it.
    ///
    /// Three things it can be, and the terminal is what says which. With
    /// reporting on it is a mouse code, because a program that asked to hear
    /// about the mouse asked about this too. On the alternate screen with
    /// alternate scroll left on it is cursor keys, which is how a pager that
    /// never asked for the mouse still scrolls. Otherwise it is nobody's but
    /// ours: the viewport moves, which is the scrollback being read.
    ///
    /// Both deltas are in lines, and up and right are positive. Coalescing
    /// pixels into lines belongs to whoever knows how tall a line is drawn,
    /// which is not this side. cf. `docs/adr/0017-semantic-input-events.md`
    pub fn wheel(&mut self, event: &WheelEvent) -> Result<Wheel> {
        let cell = (event.x, event.y);
        if self.mouse_tracking()? {
            let grid = self.grid()?;
            let mut encoded = Vec::new();
            // One report per line, which is what a wheel is: the protocol has
            // no count, so three lines are three turns of it. Which is also
            // what the cap on the count is for.
            for (delta, buttons) in [
                (
                    event.delta_y,
                    (ffi::MouseButton::FOUR, ffi::MouseButton::FIVE),
                ),
                (
                    event.delta_x,
                    (ffi::MouseButton::SIX, ffi::MouseButton::SEVEN),
                ),
            ] {
                let button = if delta > 0 { buttons.0 } else { buttons.1 };
                for _ in 0..capped_lines(delta) {
                    encoded.extend(
                        self.mouse
                            .encode_wheel(self.raw, button, event.mods, cell, grid)?,
                    );
                }
            }
            return Ok(Wheel::Bytes(encoded));
        }

        if event.delta_y == 0 {
            // Nothing below this reads a sideways turn: the alternate screen
            // has no cursor key for one and the viewport does not move that
            // way.
            return Ok(Wheel::Bytes(Vec::new()));
        }

        if self.alternate_screen()? && self.mode(ALTERNATE_SCROLL)? {
            // Encoded rather than written out, so that the arrow is the same
            // arrow the keyboard sends — cursor key application mode included,
            // which is the mode a full-screen program most likely left on.
            let key = if event.delta_y > 0 {
                Key::ArrowUp
            } else {
                Key::ArrowDown
            };
            let pressed = KeyEvent {
                action: KeyAction::Press,
                key,
                ..KeyEvent::default()
            };
            let one = self.keys.encode(self.raw, &pressed)?;
            return Ok(Wheel::Bytes(
                one.repeat(capped_lines(event.delta_y) as usize),
            ));
        }

        // The viewport counts down where the wheel counts up, and by the
        // capped count — the engine clamps against the history it has, which
        // is a different question from how big a number arrived.
        let delta = isize::try_from(capped_lines(event.delta_y)).map_err(|_| Error::OutOfRange)?;
        self.scroll(ffi::TerminalScrollViewport {
            tag: ffi::TerminalScrollViewportTag::DELTA,
            value: ffi::TerminalScrollViewportValue {
                delta: if event.delta_y > 0 { -delta } else { delta },
            },
        });
        Ok(Wheel::Scrolled)
    }

    /// Bring the viewport back to the active area, and say whether it had to
    /// move.
    ///
    /// What typing does in every terminal: a screen left up in the history is
    /// one the next command would run off the bottom of. The answer is what
    /// says whether a frame has to be published for it, and asking costs a
    /// flag rather than a capture.
    pub fn snap_to_active(&mut self) -> Result<bool> {
        // SAFETY: the tag's documented output type.
        let active: bool = unsafe { self.get(ffi::TerminalData::VIEWPORT_ACTIVE) }?;
        if active {
            return Ok(false);
        }
        self.scroll(ffi::TerminalScrollViewport {
            tag: ffi::TerminalScrollViewportTag::BOTTOM,
            value: ffi::TerminalScrollViewportValue::default(),
        });
        Ok(true)
    }

    /// Encode the window gaining or losing focus, or nothing when the child
    /// has not asked to hear about it.
    ///
    /// The gate is here rather than above the boundary because the mode is
    /// the terminal's: vim's `autoread` lives down this path, and whether it
    /// is listening is something only the last feed knows.
    pub fn encode_focus(&self, gained: bool) -> Result<Vec<u8>> {
        if !self.mode(FOCUS_REPORTING)? {
            return Ok(Vec::new());
        }

        let event = if gained {
            ffi::FocusEvent::GAINED
        } else {
            ffi::FocusEvent::LOST
        };
        // `CSI I` and `CSI O` are three bytes; the room is there so that an
        // engine that ever answers with more is a refusal rather than a
        // truncation.
        let mut buffer = [0; 8];
        let mut written = 0;
        // SAFETY: the buffer is ours and its length is told truthfully, and
        // `written` is the out parameter the call documents.
        check(unsafe {
            ffi::ghostty_focus_encode(
                event,
                buffer.as_mut_ptr().cast(),
                buffer.len(),
                &raw mut written,
            )
        })?;
        Ok(buffer[..written].to_vec())
    }

    /// Sanitize `bytes` and wrap them the way this terminal's modes ask.
    ///
    /// The whole of what makes untrusted text safe to put in the input
    /// stream, and the engine's: the control bytes that would be read as
    /// commands become spaces, and what is left is wrapped in the bracketed
    /// paste sequences when the child asked for them or has its newlines
    /// turned into carriage returns when it did not. Nothing here decides any
    /// of it. cf. `docs/adr/0007-input-security.md`
    ///
    /// **There is no way past this on the way to the child.** A caller that
    /// wants a warning first asks [`paste_is_safe`] before calling; a caller
    /// that skips the warning still arrives here, because the sanitizing is
    /// inside the paste rather than beside it.
    pub fn encode_paste(&self, bytes: &[u8]) -> Result<Vec<u8>> {
        let bracketed = self.mode(BRACKETED_PASTE)?;
        // The engine sanitizes in place, so what it works over is a copy —
        // the caller's run is borrowed and is not ours to rewrite.
        let mut data = bytes.to_vec();

        let mut needed = 0;
        // SAFETY: the copy is ours and its length is told truthfully. A null
        // buffer is how the call documents the question, and it answers it in
        // `needed`.
        let asked = unsafe {
            ffi::ghostty_paste_encode(
                data.as_mut_ptr().cast(),
                data.len(),
                bracketed,
                ptr::null_mut(),
                0,
                &raw mut needed,
            )
        };
        match asked {
            // The answer to the question: nothing was written because nowhere
            // was offered.
            ffi::Result::OUT_OF_SPACE => {}
            // A paste that comes to nothing at all, which the call has room
            // for without a buffer — an empty run with no wrapping to add.
            ffi::Result::SUCCESS => return Ok(Vec::new()),
            _ => return Err(Error::Engine),
        }

        // The probe rewrote the copy where it found a control byte, and it is
        // handed on as it stands: what the call puts there is a space, which
        // is not itself a byte it takes out, and it is one byte for one — so
        // measuring neither changed what a second pass makes of it nor how
        // much room that needs.
        let mut encoded = vec![0; needed];
        let mut written = 0;
        // SAFETY: as above, with a buffer of the length just asked for.
        check(unsafe {
            ffi::ghostty_paste_encode(
                data.as_mut_ptr().cast(),
                data.len(),
                bracketed,
                encoded.as_mut_ptr().cast(),
                encoded.len(),
                &raw mut written,
            )
        })?;
        encoded.truncate(written);
        Ok(encoded)
    }

    /// Move the viewport the way `behavior` says.
    ///
    /// Clamped by the engine at either end, so asking to go past the top of
    /// the history or the bottom of the active area does nothing.
    fn scroll(&mut self, behavior: ffi::TerminalScrollViewport) {
        // SAFETY: the terminal is ours, and the tagged union is filled in for
        // the tag it carries.
        unsafe { ffi::ghostty_terminal_scroll_viewport(self.raw, behavior) };
    }

    /// Whether any of the mouse tracking modes is on.
    fn mouse_tracking(&self) -> Result<bool> {
        // SAFETY: the tag's documented output type.
        unsafe { self.get(ffi::TerminalData::MOUSE_TRACKING) }
    }

    /// Whether the alternate screen is the active one.
    fn alternate_screen(&self) -> Result<bool> {
        // SAFETY: the tag's documented output type.
        let screen: ffi::TerminalScreen::Type =
            unsafe { self.get(ffi::TerminalData::ACTIVE_SCREEN) }?;
        Ok(screen == ffi::TerminalScreen::ALTERNATE)
    }

    /// How many cells the grid is, which is the geometry a mouse position is
    /// read against.
    fn grid(&self) -> Result<(u16, u16)> {
        // SAFETY: each tag's documented output type.
        Ok(unsafe {
            (
                self.get(ffi::TerminalData::COLS)?,
                self.get(ffi::TerminalData::ROWS)?,
            )
        })
    }

    /// Whether a synchronized output block is open as of now.
    pub fn sync_output_open(&self) -> Result<bool> {
        self.mode(SYNC_OUTPUT)
    }

    /// Whether a DEC mode is set as of now.
    fn mode(&self, mode: ffi::Mode) -> Result<bool> {
        let mut set = false;
        // SAFETY: `set` is the `bool` out parameter the call documents.
        check(unsafe { ffi::ghostty_terminal_mode_get(self.raw, mode, &raw mut set) })?;
        Ok(set)
    }

    /// Whether the active screen has a selection.
    ///
    /// Only that there is one: the snapshot carries where it falls per row,
    /// and the engine's answer is borrowed grid references that go stale on
    /// the next feed.
    pub fn has_selection(&self) -> Result<bool> {
        // Not `get`: its reader zeroes the value, which leaves the sized
        // struct's own size unfilled, and `check` would turn the answer this
        // asks for into an error.
        let mut selection = ffi::sized!(ffi::Selection);
        // SAFETY: the tag's documented output type, sized as its ABI asks.
        let code = unsafe {
            ffi::ghostty_terminal_get(
                self.raw,
                ffi::TerminalData::SELECTION,
                (&raw mut selection).cast(),
            )
        };
        match code {
            ffi::Result::SUCCESS => Ok(true),
            ffi::Result::NO_VALUE => Ok(false),
            _ => Err(Error::Engine),
        }
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// # Errors
    ///
    /// [`Error::OutOfRange`] when either endpoint is outside the viewport.
    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> Result<()> {
        let Some(range) = range else {
            return self.set(ffi::TerminalOption::SELECTION, ptr::null());
        };

        let selection = ffi::Selection {
            start: self.grid_ref(range.start_x, range.start_y)?,
            end: self.grid_ref(range.end_x, range.end_y)?,
            rectangle: range.rectangle,
            ..ffi::sized!(ffi::Selection)
        };
        // The engine copies the selection and converts it to tracked state
        // during the call, so neither this value nor the references in it have
        // to outlive the call.
        self.set(
            ffi::TerminalOption::SELECTION,
            ptr::from_ref(&selection).cast(),
        )
    }

    /// Select from the cell a gesture began on out to the one it is over now,
    /// measured in `unit`.
    ///
    /// **Both ends, not one.** A drag that named only the cell under the
    /// pointer would have nothing to widen a word or a line from, and the
    /// selection would collapse the moment the pointer crossed a space —
    /// which is where the engine's own between-two-refs search comes in: each
    /// end asks for the nearest unit looking toward the other, and what is
    /// installed spans both answers. The pair also records which way the drag
    /// went, so an endpoint moved back over the anchor reverses rather than
    /// emptying.
    ///
    /// A gesture over content that holds no unit at all — a drag across blank
    /// screen — falls back to the two cells themselves. Nothing is lost by
    /// it: there is no word there to have found.
    ///
    /// **Nothing is selected while the child has asked to hear about the
    /// mouse**, which is what the answer says: a drag inside an editor is the
    /// editor's, and painting a highlight of our own over its own selection
    /// would be two answers to one drag. The mode is read here for the reason
    /// a click's is — the sequence that turns reporting on is output, and a
    /// drag arriving right behind it has to be read against what that left.
    /// cf. `docs/adr/0017-semantic-input-events.md`
    ///
    /// ponytail: no override, so a selection cannot be made over a program
    /// that took the mouse. Holding shift is what every other terminal makes
    /// that override, and it is a key to spend and so a setting — which is
    /// M4's pipeline, not a constant to plant here.
    ///
    /// Coordinates are clamped to the grid, as a mouse event's are: a drag
    /// out of the window is a pointer past the edge, and the edge is what it
    /// means.
    ///
    /// # Errors
    ///
    /// [`Error::Engine`] when the engine refused a coordinate the grid says
    /// is inside it.
    pub fn select(
        &mut self,
        anchor: (u16, u16),
        cell: (u16, u16),
        unit: SelectionUnit,
        rectangle: bool,
    ) -> Result<bool> {
        if self.mouse_tracking()? {
            return Ok(false);
        }
        let grid = self.grid()?;
        let anchor = clamped(anchor, grid);
        let cell = clamped(cell, grid);

        let anchor_ref = self.grid_ref(anchor.0, anchor.1)?;
        let cell_ref = self.grid_ref(cell.0, cell.1)?;
        let plain = ffi::Selection {
            start: anchor_ref,
            end: cell_ref,
            rectangle,
            ..ffi::sized!(ffi::Selection)
        };

        let selection = match unit {
            SelectionUnit::Cell => plain,
            // Each end's unit, asked for from that end looking at the other.
            // Either coming back empty leaves the cells themselves, which is
            // what a gesture over blank screen is.
            _ => match (
                self.unit_at(unit, anchor_ref, cell_ref)?,
                self.unit_at(unit, cell_ref, anchor_ref)?,
            ) {
                (Some(from_anchor), Some(from_cell)) => {
                    let from_anchor = self.ordered(&from_anchor)?;
                    let from_cell = self.ordered(&from_cell)?;
                    // Ordered top-left first, so which end of each to take is
                    // the direction of the drag. The anchor's end comes
                    // first either way: that is what keeps the selection
                    // hanging off the cell the gesture began on.
                    let (start, end) = if (anchor.1, anchor.0) <= (cell.1, cell.0) {
                        (from_anchor.start, from_cell.end)
                    } else {
                        (from_anchor.end, from_cell.start)
                    };
                    ffi::Selection {
                        start,
                        end,
                        rectangle,
                        ..ffi::sized!(ffi::Selection)
                    }
                }
                _ => plain,
            },
        };

        self.set(
            ffi::TerminalOption::SELECTION,
            ptr::from_ref(&selection).cast(),
        )?;
        Ok(true)
    }

    /// The word or the line `at` falls in, searched from there toward
    /// `toward`, or `None` where there is none to find.
    ///
    /// The direction is what makes a drag over a run of spaces hold still:
    /// the engine walks from one ref to the other and answers with the first
    /// unit it meets, so a pointer between two words picks up the one on the
    /// far side rather than nothing at all.
    fn unit_at(
        &self,
        unit: SelectionUnit,
        at: ffi::GridRef,
        toward: ffi::GridRef,
    ) -> Result<Option<ffi::Selection>> {
        let mut selection = ffi::sized!(ffi::Selection);
        let code = match unit {
            // Never asked for: a cell is its own unit and the caller above
            // takes the refs it already has.
            SelectionUnit::Cell => return Ok(None),
            SelectionUnit::Word => {
                let options = ffi::TerminalSelectWordBetweenOptions {
                    start: at,
                    end: toward,
                    // Null asks for the engine's own word boundaries, which
                    // are the Unicode rules knotty is here not to re-derive.
                    boundary_codepoints: ptr::null(),
                    boundary_codepoints_len: 0,
                    ..ffi::sized!(ffi::TerminalSelectWordBetweenOptions)
                };
                // SAFETY: the terminal is ours, the options are filled in for
                // their own size, and `selection` is the sized out parameter
                // the call documents.
                unsafe {
                    ffi::ghostty_terminal_select_word_between(
                        self.raw,
                        &raw const options,
                        &raw mut selection,
                    )
                }
            }
            SelectionUnit::Line => {
                let options = ffi::TerminalSelectLineOptions {
                    ref_: at,
                    whitespace: ptr::null(),
                    whitespace_len: 0,
                    // OSC 133 is not in v1, so nothing ever marks a prompt
                    // for this to bound a line at.
                    semantic_prompt_boundary: false,
                    ..ffi::sized!(ffi::TerminalSelectLineOptions)
                };
                // SAFETY: as above.
                unsafe {
                    ffi::ghostty_terminal_select_line(
                        self.raw,
                        &raw const options,
                        &raw mut selection,
                    )
                }
            }
        };
        match code {
            ffi::Result::SUCCESS => Ok(Some(selection)),
            // No word under the pointer and none between it and the anchor,
            // which is a drag across blank screen rather than a failure.
            ffi::Result::NO_VALUE => Ok(None),
            _ => Err(Error::Engine),
        }
    }

    /// The same selection with its endpoints put in reading order.
    ///
    /// Which end of a unit to take depends on which way the drag went, and
    /// the engine hands one back in whatever order it found it.
    fn ordered(&self, selection: &ffi::Selection) -> Result<ffi::Selection> {
        let mut ordered = ffi::sized!(ffi::Selection);
        // SAFETY: both are ours, and the refs in `selection` came from this
        // terminal with nothing fed to it since.
        check(unsafe {
            ffi::ghostty_terminal_selection_ordered(
                self.raw,
                ptr::from_ref(selection),
                ffi::SelectionOrder::FORWARD,
                &raw mut ordered,
            )
        })?;
        Ok(ordered)
    }

    /// The selection as plain text, or `None` when there is none.
    ///
    /// Soft wraps are unwrapped and trailing blanks trimmed, which is what
    /// makes a folded line paste back as the one line it was typed as. The
    /// engine can write VT and HTML too; the clipboard knotty puts this on
    /// carries `text/plain` and nothing else.
    pub fn selection_text(&self) -> Result<Option<Vec<u8>>> {
        let options = ffi::TerminalSelectionFormatOptions {
            emit: ffi::FormatterFormat::PLAIN,
            unwrap: true,
            trim: true,
            // Null asks for the terminal's own active selection, which is the
            // one the gestures installed. Nothing here holds a snapshot of it
            // that a feed could have staled.
            selection: ptr::null(),
            ..ffi::sized!(ffi::TerminalSelectionFormatOptions)
        };

        let mut needed = 0;
        // SAFETY: the terminal is ours and the options are sized. A null
        // buffer is how the call documents the question, and it answers it in
        // `needed`.
        let asked = unsafe {
            ffi::ghostty_terminal_selection_format_buf(
                self.raw,
                options,
                ptr::null_mut(),
                0,
                &raw mut needed,
            )
        };
        match asked {
            // Nothing is selected, which is not a failure — it is the answer
            // a copy with no selection gets.
            ffi::Result::NO_VALUE => return Ok(None),
            // The answer to the question: nothing was written because
            // nowhere was offered.
            ffi::Result::OUT_OF_SPACE => {}
            // A selection that formats to nothing at all, which the call has
            // room for without a buffer.
            ffi::Result::SUCCESS => return Ok(Some(Vec::new())),
            _ => return Err(Error::Engine),
        }

        let mut text = vec![0; needed];
        let mut written = 0;
        // SAFETY: as above, with a buffer of the length just asked for.
        check(unsafe {
            ffi::ghostty_terminal_selection_format_buf(
                self.raw,
                options,
                text.as_mut_ptr(),
                text.len(),
                &raw mut written,
            )
        })?;
        text.truncate(written);
        Ok(Some(text))
    }

    /// Move the viewport `lines` lines, up positive.
    ///
    /// What a drag out of the window asks for: the pointer stands still and
    /// the screen has to keep coming, so the app's own timer is what calls
    /// this. Clamped by the engine at either end.
    pub fn scroll_viewport(&mut self, lines: i32) {
        // Capped the way a wheel turn is, and for the same reason: the count
        // crosses the boundary as a plain number.
        let capped = capped_lines(lines) as isize;
        self.scroll(ffi::TerminalScrollViewport {
            tag: ffi::TerminalScrollViewportTag::DELTA,
            value: ffi::TerminalScrollViewportValue {
                // The viewport counts down where the wheel counts up.
                delta: if lines > 0 { -capped } else { capped },
            },
        });
    }

    /// Resolve a viewport coordinate to a reference the engine can hold on to.
    ///
    /// The result is good until the next thing that moves the grid, which is
    /// why it never leaves the call that asked for it.
    fn grid_ref(&self, x: u16, y: u16) -> Result<ffi::GridRef> {
        let point = ffi::Point {
            tag: ffi::PointTag::VIEWPORT,
            value: ffi::PointValue {
                coordinate: ffi::PointCoordinate { x, y: u32::from(y) },
            },
        };
        let mut resolved = ffi::sized!(ffi::GridRef);
        // SAFETY: `resolved` is the sized out parameter the call documents,
        // with its size filled in.
        let result = unsafe { ffi::ghostty_terminal_grid_ref(self.raw, point, &raw mut resolved) };
        if result == ffi::Result::SUCCESS {
            return Ok(resolved);
        }
        // The one way this fails that a caller can do anything about, and the
        // only one an app can cause: a point outside the coordinate space.
        Err(Error::OutOfRange)
    }

    /// Read a borrowed string out of the terminal.
    ///
    /// The engine lends it until the next feed, which is what the borrow of
    /// `self` stands for. Nothing promises it is UTF-8 in the type, so it is
    /// checked here rather than assumed — the whole reason this layer exists.
    fn text(&self, data: ffi::TerminalData::Type) -> Result<&str> {
        // SAFETY: the tag's documented output type.
        let string: ffi::String = unsafe { self.get(data) }?;
        if string.ptr.is_null() {
            // The C header declares the pointer non-optional and the engine
            // returns a static sentinel for the empty string. Checked anyway:
            // a null slice is undefined behaviour even at length zero, and
            // this is the boundary that is supposed to catch that.
            return Ok("");
        }
        // SAFETY: non-null, and lent for as long as the terminal is not fed.
        let bytes = unsafe { std::slice::from_raw_parts(string.ptr, string.len) };
        std::str::from_utf8(bytes).map_err(|_| Error::Engine)
    }

    /// Read a terminal value into `T`.
    ///
    /// # Safety
    ///
    /// `T` must be the output type `data` documents.
    unsafe fn get<T>(&self, data: ffi::TerminalData::Type) -> Result<T> {
        // SAFETY: the caller's, as declared.
        unsafe { read(|out| ffi::ghostty_terminal_get(self.raw, data, out)) }
    }

    /// Set a terminal option to whatever `value` points at, or to nothing.
    fn set(&self, option: ffi::TerminalOption::Type, value: *const c_void) -> Result<()> {
        // SAFETY: each caller passes the input type its option documents, and
        // the engine reads it during the call.
        check(unsafe { ffi::ghostty_terminal_set(self.raw, option, value) })
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // SAFETY: each handle was made by its matching constructor and is
        // freed once. A part-built terminal leaves the rest null, which is why
        // each is checked — only some of these calls document taking null.
        unsafe {
            if !self.cells.is_null() {
                ffi::ghostty_render_state_row_cells_free(self.cells);
            }
            if !self.rows.is_null() {
                ffi::ghostty_render_state_row_iterator_free(self.rows);
            }
            if !self.render.is_null() {
                ffi::ghostty_render_state_free(self.render);
            }
            // Before the listener, which the engine may still call from here.
            ffi::ghostty_terminal_free(self.raw);
            drop(Box::from_raw(self.listener));
        }
    }
}

/// A cell brought inside a grid of `cols` by `rows`.
///
/// A grid always has a cell, so the subtraction never wraps past what the
/// saturation leaves.
fn clamped(cell: (u16, u16), grid: (u16, u16)) -> (u16, u16) {
    (
        cell.0.min(grid.0.saturating_sub(1)),
        cell.1.min(grid.1.saturating_sub(1)),
    )
}

/// Whether `bytes` can go to the child without asking the user first.
///
/// Unsafe means a newline, which a shell runs the moment it arrives, or the
/// bracketed paste terminator `ESC [ 201 ~`, which would end the wrapping
/// early and leave the rest of the run being read as commands. The engine's
/// judgement, and a conservative one: it does not look at what modes the
/// terminal is in.
///
/// **A pre-check and nothing more.** It takes no terminal because it needs
/// none, which is what lets a warning be shown before anything is pasted. It
/// gates the warning, never the sanitizing — [`Terminal::encode_paste`] does
/// that whichever way the answer went. cf. `docs/adr/0007-input-security.md`
pub fn paste_is_safe(bytes: &[u8]) -> bool {
    // SAFETY: the run is borrowed for the whole call, and its length is told
    // truthfully. An empty slice lends a dangling but aligned pointer, which
    // is what a length of 0 says not to read.
    unsafe { ffi::ghostty_paste_is_safe(bytes.as_ptr().cast(), bytes.len()) }
}

/// How many lines a delta is worth, capped at [`MAX_WHEEL_LINES`].
fn capped_lines(delta: i32) -> u32 {
    delta.unsigned_abs().min(MAX_WHEEL_LINES)
}

/// Turn a checked result code into knotty's.
///
/// Every failure the engine reports is the same news up here: the engine
/// refused, and the caller's own error tells what it was doing at the time.
fn check(code: ffi::Result::Type) -> Result<()> {
    if code == ffi::Result::SUCCESS {
        return Ok(());
    }
    Err(Error::Engine)
}

/// Hand `get` somewhere to put a `T` and take back what it put there.
///
/// The engine's readers are all this shape — a tag, a pointer to write
/// through, and a result code — and the delicate half is the same every time.
/// It lives here once rather than at each of the five tag namespaces, which
/// keep their own entry points so that a tag cannot be used against the handle
/// it does not belong to.
///
/// # Safety
///
/// `T` must be the output type the tag `get` passes documents.
unsafe fn read<T>(get: impl FnOnce(*mut c_void) -> ffi::Result::Type) -> Result<T> {
    let mut value = MaybeUninit::<T>::zeroed();
    check(get(value.as_mut_ptr().cast()))?;
    // SAFETY: a successful call initializes the value, and the caller promised
    // it is a `T`.
    Ok(unsafe { value.assume_init() })
}

/// The engine's callback types with the null it also allows taken off, so that
/// the functions below can be coerced to pointers.
type PtyWriteFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void, *const u8, usize);
type BellFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void);
type AnswerFn = unsafe extern "C" fn(ffi::Terminal, *mut c_void) -> ffi::String;
type DeviceAttributesFn =
    unsafe extern "C" fn(ffi::Terminal, *mut c_void, *mut ffi::DeviceAttributes) -> bool;
type ClipboardWriteFn = unsafe extern "C" fn(
    ffi::Terminal,
    *mut c_void,
    *const ffi::ClipboardWrite,
) -> ffi::ClipboardWriteResult::Type;

/// Borrow an engine string as bytes, whatever it holds.
///
/// # Safety
///
/// The bytes must be ones the engine lends for at least `'a`.
unsafe fn bytes_of<'a>(ptr: *const u8, len: usize) -> &'a [u8] {
    if ptr.is_null() {
        // A null array is not a slice at any length, empty included.
        return &[];
    }
    // SAFETY: non-null, and lent for `'a` by the caller's promise.
    unsafe { std::slice::from_raw_parts(ptr, len) }
}

/// Borrow the listener the engine was given.
///
/// # Safety
///
/// `userdata` must be the pointer a live [`Terminal`] registered, and no other
/// borrow of it may be live. The engine calls back only from inside a feed, so
/// the terminal that owns it is alive and nothing else is looking at it.
unsafe fn listener_of<'a>(userdata: *mut c_void) -> &'a mut Listener {
    // SAFETY: the caller's, as declared.
    unsafe { &mut *userdata.cast::<Listener>() }
}

unsafe extern "C" fn on_pty_write(
    _terminal: ffi::Terminal,
    userdata: *mut c_void,
    ptr: *const u8,
    len: usize,
) {
    // SAFETY: called from inside a feed on the terminal that registered it.
    let listener = unsafe { listener_of(userdata) };
    // SAFETY: the engine lends `len` bytes for the length of this call.
    let bytes = unsafe { bytes_of(ptr, len) };
    (listener.pty_write)(bytes);
}

unsafe extern "C" fn on_bell(_terminal: ffi::Terminal, userdata: *mut c_void) {
    // SAFETY: as `on_pty_write`.
    let listener = unsafe { listener_of(userdata) };
    (listener.bell)();
}

unsafe extern "C" fn on_enquiry(_terminal: ffi::Terminal, userdata: *mut c_void) -> ffi::String {
    // SAFETY: as `on_pty_write`. The answer is a `'static` string of knotty's,
    // so the engine may read it whenever it likes.
    let listener = unsafe { listener_of(userdata) };
    ffi::String::from(listener.answerback)
}

unsafe extern "C" fn on_xtversion(_terminal: ffi::Terminal, userdata: *mut c_void) -> ffi::String {
    // SAFETY: as `on_enquiry`.
    let listener = unsafe { listener_of(userdata) };
    ffi::String::from(listener.version)
}

unsafe extern "C" fn on_device_attributes(
    _terminal: ffi::Terminal,
    _userdata: *mut c_void,
    out: *mut ffi::DeviceAttributes,
) -> bool {
    if out.is_null() {
        return false;
    }
    // SAFETY: non-null storage for one of these, handed over to be filled in.
    unsafe { *out = DEVICE_ATTRIBUTES };
    true
}

unsafe extern "C" fn on_clipboard_write(
    _terminal: ffi::Terminal,
    userdata: *mut c_void,
    write: *const ffi::ClipboardWrite,
) -> ffi::ClipboardWriteResult::Type {
    // SAFETY: as `on_pty_write`.
    let listener = unsafe { listener_of(userdata) };
    // SAFETY: the engine lends one of these for the length of this call.
    // `as_ref` is the checked spelling: a null write is refused below rather
    // than dereferenced.
    let Some(write) = (unsafe { write.as_ref() }) else {
        return ffi::ClipboardWriteResult::UNSUPPORTED;
    };

    // A write carrying no representations is the C API asking for the
    // clipboard to be cleared, and it says so with a null array rather than an
    // empty one — which is not a slice, at any length. This check is one of
    // the two the facade was built for. cf. ADR 0012
    let contents = if write.contents.is_null() {
        &[][..]
    } else {
        // SAFETY: the engine lends the array for the length of this call.
        unsafe { std::slice::from_raw_parts(write.contents, write.contents_len) }
    };
    let target = match write.location {
        ffi::ClipboardLocation::SELECTION => ClipboardTarget::Selection,
        ffi::ClipboardLocation::PRIMARY => ClipboardTarget::Primary,
        // Standard, and anything a later engine adds: the system clipboard is
        // the one every protocol means by default.
        _ => ClipboardTarget::Standard,
    };

    // Handed on as knotty's own values, so that the table deciding what
    // becomes of them never sees an engine type — nor needs a terminal to be
    // called. cf. `03-core.md` C3
    let offered: Vec<Representation<'_>> = contents
        .iter()
        // SAFETY: both strings are borrowed from the engine for the length of
        // this call, which outlives the borrow below.
        .map(|content| unsafe {
            Representation {
                mime: bytes_of(content.mime.ptr, content.mime.len),
                data: bytes_of(content.data.ptr, content.data.len),
            }
        })
        .collect();

    match (listener.clipboard_write)(target, &offered) {
        Ok(()) => ffi::ClipboardWriteResult::SUCCESS,
        Err(ClipboardRefusal::Denied) => ffi::ClipboardWriteResult::DENIED,
        Err(ClipboardRefusal::InvalidData) => ffi::ClipboardWriteResult::INVALID_DATA,
        Err(ClipboardRefusal::Unsupported) => ffi::ClipboardWriteResult::UNSUPPORTED,
    }
}
