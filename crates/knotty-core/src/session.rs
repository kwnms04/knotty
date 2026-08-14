//! Session lifecycle and the publish path.

use libghostty_vt::screen::Screen;
use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::{Point, PointCoordinate};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use crate::mailbox::Mailbox;
use crate::snapshot::{self, ScreenState, Snapshot};
use crate::{Error, Result};

/// A selection's two endpoints, in viewport coordinates.
///
/// Both ends are inclusive, and either may come first: the pair records which
/// way the selection was made, not which end is topmost.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SelectionRange {
    /// Column of the first endpoint.
    pub start_x: u16,
    /// Row of the first endpoint.
    pub start_y: u16,
    /// Column of the second endpoint.
    pub end_x: u16,
    /// Row of the second endpoint.
    pub end_y: u16,
    /// Whether the endpoints are opposite corners of a block rather than the
    /// ends of a run of text.
    pub rectangle: bool,
}

/// A terminal session.
///
/// A detached session owns no thread and no child process: [`feed`] runs the
/// VT engine on the calling thread. Everything past the parser — conversion
/// and mailbox publication — is the path a PTY session will take too.
///
/// [`feed`]: Session::feed
pub struct Session {
    terminal: Terminal<'static, 'static>,
    render: RenderState<'static>,
    mailbox: Mailbox<Snapshot>,
    // Which screen the selection was made on, or None when there is none.
    //
    // A snapshot has to say whether a selection exists even when no visible
    // row falls inside one, and the engine's own answer is out of reach: the
    // C API has it, but the pinned safe wrapper keeps the raw handle private.
    // So knotty keeps its own record. See `Session::has_selection` for what
    // that costs.
    selection_screen: Option<Screen>,
    // What the last capture said about the screen outside the grid, so that a
    // title or cursor change on an otherwise still screen still publishes.
    last_screen: ScreenState,
}

impl Session {
    /// Create a session with no PTY behind it.
    pub fn new_detached(cols: u16, rows: u16, max_scrollback: usize) -> Result<Self> {
        let terminal = Terminal::new(TerminalOptions {
            cols,
            rows,
            max_scrollback,
        })?;
        let render = RenderState::new()?;

        Ok(Self {
            terminal,
            render,
            mailbox: Mailbox::new(),
            selection_screen: None,
            last_screen: ScreenState::default(),
        })
    }

    /// Select a range of the viewport, or clear the selection with `None`.
    ///
    /// Publishes a snapshot: the selection is part of what a consumer draws.
    pub fn set_selection(&mut self, range: Option<SelectionRange>) -> Result<()> {
        match range {
            Some(range) => {
                let at = |x, y| Point::Viewport(PointCoordinate { x, y: u32::from(y) });
                let start = self
                    .terminal
                    .grid_ref(at(range.start_x, range.start_y))
                    .map_err(|_| Error::OutOfRange)?;
                let end = self
                    .terminal
                    .grid_ref(at(range.end_x, range.end_y))
                    .map_err(|_| Error::OutOfRange)?;
                self.terminal
                    .set_selection(Some(&Selection::new(start, end, range.rectangle)))?;
            }
            None => {
                self.terminal.set_selection(None)?;
            }
        }
        self.selection_screen = match range {
            Some(_) => Some(self.terminal.active_screen()?),
            None => None,
        };

        self.publish()
    }

    /// Whether a selection exists.
    ///
    /// The engine's selection belongs to the active screen and is dropped when
    /// that changes, so knotty's record only holds while the screen does. A
    /// sequence that resets the terminal outright also drops the selection and
    /// is not detectable here, so this can read true for a while after such a
    /// reset. Exposing the engine's own answer needs the wrapper to hand out
    /// the raw handle.
    fn has_selection(&self) -> Result<bool> {
        Ok(match self.selection_screen {
            Some(screen) => screen == self.terminal.active_screen()?,
            None => false,
        })
    }

    /// Process `bytes` to completion on the calling thread, publishing at most
    /// one snapshot.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.terminal.vt_write(bytes);
        self.publish()
    }

    /// Take the latest snapshot, emptying the mailbox.
    ///
    /// Returns `None` when nothing has been published since the last take.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.mailbox.take()
    }

    /// Capture the terminal and publish it, unless nothing changed.
    fn publish(&mut self) -> Result<()> {
        let has_selection = self.has_selection()?;
        if let Some(mut snapshot) =
            snapshot::capture(&mut self.render, &self.terminal, &self.last_screen)?
        {
            snapshot.has_selection = has_selection;
            self.last_screen = snapshot.screen.clone();
            // The mailbox keeps only the newest snapshot, so publishing over
            // an unconsumed one drops it. Carry its change marks across, or a
            // consumer that misses a frame is told less changed than did.
            if let Some(dropped) = self.mailbox.take() {
                snapshot.absorb_marks_of(&dropped);
            }
            self.mailbox.publish(snapshot);
        }
        Ok(())
    }
}
