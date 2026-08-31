//! Turning a mouse event into the bytes the terminal's modes make of it.
//!
//! The engine owns the encoding and every branch inside it — whether the
//! event reports at all, and which of the five formats it is written in. What
//! this adds is the two handles it wants held between calls and the geometry
//! it reads a position against. cf. `docs/adr/0017-semantic-input-events.md`

use std::ffi::c_void;
use std::ptr;

use libghostty_vt_sys as ffi;

use crate::mouse::{MouseAction, MouseButton, MouseEvent};
use crate::{Error, Result};

use super::check;

/// The engine's mouse encoder and the event handle it reads one out of.
///
/// Kept rather than made per event for the reason the key encoder's are: the
/// encoder is where the options an event is read against live, and it also
/// holds the last cell a motion was reported over, which is what keeps a
/// pointer crossing one cell from reporting a hundred times.
pub(super) struct Mouse {
    encoder: ffi::MouseEncoder,
    event: ffi::MouseEvent,
    /// Whether the encoder's options are behind the terminal's.
    ///
    /// Applying them is not free of consequence: every one of the setters
    /// forgets the last cell a motion was reported over, which is the state
    /// that keeps a drag across one cell from reporting a hundred times. So
    /// they are applied when something that could have moved them has
    /// happened rather than per event — which is a feed, since the modes are
    /// what a feed sets, and a resize, since the grid is what a position is
    /// read against.
    stale: bool,
}

impl Mouse {
    pub(super) fn new() -> Result<Self> {
        let mut mouse = Self {
            encoder: ptr::null_mut(),
            event: ptr::null_mut(),
            stale: true,
        };
        // SAFETY: a null allocator asks for the engine's own, and each out
        // parameter is handle-sized. Built into a value that already owns its
        // own `Drop`, so a failure on the second releases the first.
        check(unsafe { ffi::ghostty_mouse_encoder_new(ptr::null(), &raw mut mouse.encoder) })?;
        check(unsafe { ffi::ghostty_mouse_event_new(ptr::null(), &raw mut mouse.event) })?;

        // Motion deduplication by cell, which is what the encoder holds state
        // for. A trackpad reports a drag in pixels and a terminal is told
        // about it in cells, so without this every pixel of one cell is a
        // report.
        mouse.setopt(ffi::MouseEncoderOption::TRACK_LAST_CELL, &true);
        Ok(mouse)
    }

    /// Say that something happened which the encoder's options may be behind.
    pub(super) fn invalidate(&mut self) {
        self.stale = true;
    }

    /// Encode `event` as the modes `terminal` holds right now have it.
    ///
    /// The answer is empty for everything the terminal is meant to stay quiet
    /// about, which with reporting off is every mouse event there is.
    pub(super) fn encode(
        &mut self,
        terminal: ffi::Terminal,
        event: &MouseEvent,
        grid: (u16, u16),
    ) -> Result<Vec<u8>> {
        let button = match event.button {
            MouseButton::None => None,
            MouseButton::Left => Some(ffi::MouseButton::LEFT),
            MouseButton::Right => Some(ffi::MouseButton::RIGHT),
            MouseButton::Middle => Some(ffi::MouseButton::MIDDLE),
        };
        let action = match event.action {
            MouseAction::Press => ffi::MouseAction::PRESS,
            MouseAction::Release => ffi::MouseAction::RELEASE,
            MouseAction::Motion => ffi::MouseAction::MOTION,
        };
        self.encode_raw(
            terminal,
            action,
            button,
            event.mods,
            (event.x, event.y),
            grid,
        )
    }

    /// The same for a wheel turn, which reports as a press of one of the
    /// buttons a wheel is numbered as.
    pub(super) fn encode_wheel(
        &mut self,
        terminal: ffi::Terminal,
        button: ffi::MouseButton::Type,
        mods: u16,
        cell: (u16, u16),
        grid: (u16, u16),
    ) -> Result<Vec<u8>> {
        self.encode_raw(
            terminal,
            ffi::MouseAction::PRESS,
            Some(button),
            mods,
            cell,
            grid,
        )
    }

    /// One event, in the engine's own terms: `cell` is the column and row it
    /// happened over, `grid` how many of each there are.
    fn encode_raw(
        &mut self,
        terminal: ffi::Terminal,
        action: ffi::MouseAction::Type,
        button: Option<ffi::MouseButton::Type>,
        mods: u16,
        cell: (u16, u16),
        grid: (u16, u16),
    ) -> Result<Vec<u8>> {
        if self.stale {
            // The tracking mode and the format, taken off the terminal: the
            // sequence that turns reporting on is output, and a click
            // arriving right behind it has to be read against what that left.
            // SAFETY: both handles are live, and the call reads the terminal
            // without keeping it.
            unsafe { ffi::ghostty_mouse_encoder_setopt_from_terminal(self.encoder, terminal) };

            // A cell of one pixel, so that the cell coordinates the boundary
            // takes are already the surface-space position the engine
            // converts. The engine's own tests describe a grid this way.
            //
            // ponytail: SGR-Pixels reporting (mode 1016) then answers in
            // cells where it promises pixels. Carrying the sub-cell offset
            // across the boundary is what to do if something turns out to ask
            // for it.
            let size = ffi::MouseEncoderSize {
                size: size_of::<ffi::MouseEncoderSize>(),
                screen_width: u32::from(grid.0),
                screen_height: u32::from(grid.1),
                cell_width: 1,
                cell_height: 1,
                ..ffi::MouseEncoderSize::default()
            };
            self.setopt(ffi::MouseEncoderOption::SIZE, &size);
            self.stale = false;
        }
        // What the option is for is a motion that left the viewport: it is
        // reported only while something is held, which is how a drag out of
        // the window stays a drag. A press is one of those, which is what the
        // engine asks this to say. A release is always reported anyway.
        let pressed = action == ffi::MouseAction::PRESS
            || (action == ffi::MouseAction::MOTION && button.is_some());
        self.setopt(ffi::MouseEncoderOption::ANY_BUTTON_PRESSED, &pressed);

        // SAFETY: the event is live and each value is the setter's own type.
        unsafe {
            ffi::ghostty_mouse_event_set_action(self.event, action);
            match button {
                Some(button) => ffi::ghostty_mouse_event_set_button(self.event, button),
                None => ffi::ghostty_mouse_event_clear_button(self.event),
            }
            ffi::ghostty_mouse_event_set_mods(self.event, mods);
            ffi::ghostty_mouse_event_set_position(
                self.event,
                ffi::MousePosition {
                    x: f32::from(cell.0),
                    y: f32::from(cell.1),
                },
            );
        }
        self.encoded()
    }

    /// Ask how much room the sequence needs, then take it.
    ///
    /// Two calls for the reason the key encoder makes two: an event that
    /// reports nothing answers zero, and everything else answers its own
    /// length, so no length here is a guess.
    fn encoded(&mut self) -> Result<Vec<u8>> {
        let mut needed = 0;
        // SAFETY: both handles are live. A null buffer is how the call
        // documents the question, and it answers it in `needed`.
        let asked = unsafe {
            ffi::ghostty_mouse_encoder_encode(
                self.encoder,
                self.event,
                ptr::null_mut(),
                0,
                &raw mut needed,
            )
        };
        // Out of space is the answer to the question, not a failure: nothing
        // was written because nowhere was offered.
        if asked != ffi::Result::SUCCESS && asked != ffi::Result::OUT_OF_SPACE {
            return Err(Error::Engine);
        }

        let mut encoded = vec![0; needed];
        if needed == 0 {
            return Ok(encoded);
        }

        let mut written = 0;
        // SAFETY: as above, with a buffer of the length just asked for.
        check(unsafe {
            ffi::ghostty_mouse_encoder_encode(
                self.encoder,
                self.event,
                encoded.as_mut_ptr().cast(),
                encoded.len(),
                &raw mut written,
            )
        })?;
        encoded.truncate(written);
        Ok(encoded)
    }

    /// Hand the encoder one of its options, which it reads during the call.
    fn setopt<T>(&mut self, option: ffi::MouseEncoderOption::Type, value: &T) {
        // SAFETY: each caller passes the input type its option documents, and
        // the encoder copies it out of the borrow before returning.
        unsafe {
            ffi::ghostty_mouse_encoder_setopt(
                self.encoder,
                option,
                ptr::from_ref(value).cast::<c_void>(),
            );
        }
    }
}

impl Drop for Mouse {
    fn drop(&mut self) {
        // SAFETY: each handle was made by its matching constructor and is
        // freed once. Both calls document taking null, which a part-built
        // value leaves behind.
        unsafe {
            ffi::ghostty_mouse_event_free(self.event);
            ffi::ghostty_mouse_encoder_free(self.encoder);
        }
    }
}
