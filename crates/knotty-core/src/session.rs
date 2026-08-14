//! Session lifecycle and the publish path.

use libghostty_vt::{RenderState, Terminal, TerminalOptions};

use crate::Result;
use crate::mailbox::Mailbox;
use crate::snapshot::{self, Snapshot};

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
        })
    }

    /// Process `bytes` to completion on the calling thread, publishing at most
    /// one snapshot.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<()> {
        self.terminal.vt_write(bytes);
        if let Some(snapshot) = snapshot::capture(&mut self.render, &self.terminal)? {
            self.mailbox.publish(snapshot);
        }
        Ok(())
    }

    /// Take the latest snapshot, emptying the mailbox.
    ///
    /// Returns `None` when nothing has been published since the last take.
    pub fn take_snapshot(&self) -> Option<Snapshot> {
        self.mailbox.take()
    }
}
