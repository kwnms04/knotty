//! Session lifecycle and the publish path.

use libghostty_vt::selection::Selection;
use libghostty_vt::terminal::{Point, PointCoordinate};
use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use crate::mailbox::Mailbox;
use crate::snapshot::{self, SelectionRange, Snapshot};
use crate::{Error, Result};

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
    // The engine has no way to ask whether a selection is set, and a snapshot
    // has to say so even when no visible row falls inside it.
    has_selection: bool,
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
            has_selection: false,
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
        self.has_selection = range.is_some();

        self.publish()
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
        if let Some(mut snapshot) =
            snapshot::capture(&mut self.render, &self.terminal, self.has_selection)?
        {
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
