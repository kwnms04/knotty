//! The event loop, the pseudoterminal, and the only knowledge of file
//! descriptors in the crate.
//!
//! What sits above asks for three things and is handed no platform type back:
//! bytes arrived, write this much, the child is gone. That boundary is what
//! leaves the detached path untouched, and what a port to another operating
//! system would have to rewrite — this module and nothing else. cf.
//! `docs/adr/0001-portable-core.md`

use std::ffi::OsStr;
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::mpsc::Receiver;
use std::time::Duration;

use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::fs::{Mode, OFlags};
use rustix::io::{Errno, ioctl_fioclex, ioctl_fionbio};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

use crate::session::{Request, Session};
use crate::writer::WriteQueue;
use crate::{Error, Result};

impl From<Errno> for Error {
    fn from(_: Errno) -> Self {
        Self::Io
    }
}

impl From<std::io::Error> for Error {
    fn from(_: std::io::Error) -> Self {
        Self::Io
    }
}

/// What a terminal knotty drives calls itself.
///
/// A child told nothing draws nothing: terminfo is how a program finds out
/// which sequences to send, and this string is the key it looks itself up
/// under. It names a widely installed entry rather than one of knotty's own,
/// which would have to be shipped and installed before it was worth anything.
const TERM: &str = "xterm-256color";

/// That every colour a child names is drawn as named.
///
/// `TERM` promises 256 where knotty draws 24 bits, so a child reading `TERM`
/// alone underestimates it. This is the variable that says otherwise, and it
/// is half of what tmux's `terminal-features` autodetection reads. cf.
/// `06-integration.md`
const COLORTERM: &str = "truecolor";

/// What knotty calls itself to a child that asks which terminal it is in.
///
/// The other half tmux reads. Knotty's own name and not a better-known
/// terminal's: passing off as `Apple_Terminal` would buy `OSC 7` out of
/// `/etc/zshrc` for free and lie to every other program that tells terminals
/// apart, which the M4 spec turned down. cf. `06-integration.md`
const TERM_PROGRAM: &str = "knotty";

/// How much comes off the terminal in one read.
const READ_CHUNK: usize = 64 * 1024;

/// What a wait found ready.
pub struct Ready {
    /// The child wrote something.
    pub readable: bool,
    /// The terminal has room for more.
    pub writable: bool,
    /// The child and everything it started have let go of the lifeline, so
    /// nothing more will ever be written to the terminal.
    pub ended: bool,
}

/// What one read off the terminal produced.
pub enum Chunk {
    /// This many bytes went into the buffer.
    Bytes(usize),
    /// Nothing was there after all.
    Empty,
    /// The child is gone and nothing more will arrive.
    Ended,
}

/// A pseudoterminal with a child process on the far side of it.
pub struct Pty {
    /// Our end. Non-blocking, so that a child which has stopped reading can
    /// never hold the loop inside a write while its own output goes untaken.
    terminal: OwnedFd,
    /// The child's end, held open for as long as the session lasts.
    ///
    /// Not the child's copy — that one is its three standard streams. This is
    /// ours, and holding it is what keeps the child's last output readable.
    /// macOS tears the terminal down when the session leader exits: it waits
    /// about half a second for our end to drain what is queued and then
    /// throws the rest away, so a child that prints once and stops loses its
    /// only line to a thread that was slow to be scheduled. A far end that is
    /// still open is a terminal that is not torn down, and the queue stays
    /// where it is until we come for it. cf. `kwnms04/knotty#43`
    ///
    /// It costs the end-of-file that used to be how the child's exit
    /// arrived — hence the lifeline below.
    ///
    /// It is also the end the size is set on, for the reason [`set_size`]
    /// gives.
    ///
    /// [`set_size`]: Pty::set_size
    far: OwnedFd,
    /// Read end of the nudge pipe, waited on beside the terminal so that
    /// input from another thread cuts the wait short.
    nudge: Arc<OwnedFd>,
    /// Read end of a pipe whose only other end is the child's.
    ///
    /// Nothing is ever written down it. What it is for is closing: the write
    /// end is opened in the child between fork and exec and inherited by
    /// everything the child starts, so the read end reports a hang-up exactly
    /// when the last of them is gone — which is the same moment the terminal
    /// used to report one, and the moment after which no more output can
    /// arrive.
    lifeline: OwnedFd,
    /// Held so the child stays ours to wait on, and to collect on the way out.
    ///
    /// **Declared last on purpose.** Fields are dropped in declaration order,
    /// so this one is collected with the terminal already closed. See
    /// [`Kept`] for what that ordering is worth.
    child: Kept,
}

/// The child, held so that letting go of it puts it down and collects it.
///
/// A type of its own because a `Drop` on [`Pty`] would run *before* any of
/// [`Pty`]'s fields, our end of the terminal included — and there is no order
/// to be had from within it. Carried on a field instead, the collecting runs
/// after every field declared ahead of it, which is how it comes to happen
/// with the terminal already closed.
///
/// That order is the whole of the matter. A child killed while the terminal
/// still holds output nobody has taken cannot finish exiting until that
/// output is drained or the terminal goes away, and the thread that would
/// drain it is the very one waiting here — so waiting with our end still open
/// is each side waiting for the other. Letting go first leaves the kernel
/// nothing to wait for. cf. `03-core.md` C6
///
/// A [`Foreground`] is a second handle on our end, so it has to be let go of
/// before the wait that gets here — which is what `PtySession::drop` does,
/// and why this stays the last handle closing.
struct Kept {
    /// Ours to wait on for as long as this lives.
    process: Child,
}

impl Kept {
    /// Put the child down and collect it, answering with what it ended by.
    ///
    /// The signal is for the child that let go of its terminal and kept
    /// running: knotty can no longer see it or talk to it, so waiting on one
    /// would be waiting forever and leaving it would be leaving it for good.
    /// A child that ended of its own accord — every ordinary one — is only
    /// waiting to be collected by the time this is called, and a signal
    /// changes nothing about what it ended with.
    fn kill_and_reap(&mut self) -> Result<i32> {
        let _ = self.process.kill();
        Ok(exit_code(self.process.wait()?))
    }
}

impl Drop for Kept {
    /// Collect the child, so that letting go of a session leaves nothing of it
    /// behind.
    ///
    /// This is the path for a session released while its child is still
    /// running: closing the terminal alone is a hangup the child is free to
    /// ignore, and an ignored one leaves a process the app can no longer
    /// reach. A child already collected on the way out of the loop makes this
    /// an error nobody has anything to do with.
    fn drop(&mut self) {
        let _ = self.kill_and_reap();
    }
}

/// The one number a child's end is reported by.
///
/// A death by signal has no exit code of its own, so it is reported as the
/// shells report one: 128 plus the signal. A consumer showing "exited 139"
/// says as much as one that would have had to know what a signal is.
fn exit_code(status: ExitStatus) -> i32 {
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// A standing question about a [`Pty`]: is anything running in front of the
/// shell?
///
/// Handed to whoever closes windows, which is never the thread sitting in the
/// wait — the same reason [`Waker`] is a type of its own. It carries a handle
/// of its own on the terminal so that asking never has to reach across to the
/// thread that owns one.
///
/// Asked rather than published, and that is the whole of why this is not a
/// field on a snapshot. A frame is only taken when one was published, and a
/// job that prints nothing between starting and being closed on — `make`
/// with its output redirected, a `sleep` — publishes none: the last frame
/// anybody holds was captured before the job started and says so. What the
/// window needs is the answer at the moment it is closing, and nothing but
/// asking then gives that. Asking then is also what keeps it off the idle
/// path — nothing here runs until somebody tries to close a window. cf.
/// `07-definition-of-done.md` B7
///
/// **Let go of before a session waits on its thread.** The handle is a second
/// copy of our end of the terminal, and [`Kept`] leans on the release inside
/// that thread being the last copy closing. `PtySession::drop` drops this
/// first for that reason.
pub struct Foreground {
    /// Our own end of the terminal, duplicated from the one the I/O thread
    /// took. Read-only as far as this is concerned.
    terminal: OwnedFd,
    /// The process group the child leads, which is the number a foreground
    /// group is only interesting for differing from.
    shell: i32,
}

impl Foreground {
    /// Whether something other than the shell itself holds the terminal's
    /// foreground.
    ///
    /// The child is the shell, and a shell puts every job it runs in the
    /// foreground into a process group of its own — so a foreground group
    /// that is not the shell's is a program running, and one that is is a
    /// prompt waiting. Asking what the child state says instead would answer
    /// "running" for as long as the shell lives, which is every window there
    /// has ever been. cf. `05-swift-app.md` 8
    ///
    /// Read on our end of the terminal and not the far one, which is the
    /// opposite of what [`set_size`] does: this ioctl is answered for the
    /// terminal the asking process controls, and the far end is the
    /// *child's* controlling terminal rather than ours — asking there is
    /// `ENOTTY`. A pseudoterminal's own end answers it for the far end's
    /// foreground group, which is the number wanted here.
    ///
    /// A terminal with no foreground group at all — the moment between the
    /// fork and the child's `setsid`, and a terminal already torn down —
    /// reads as quiet. Nothing is running that closing the window would take
    /// down, and a warning nobody can act on is the one that teaches people
    /// to click through the one that matters.
    ///
    /// [`set_size`]: Pty::set_size
    #[must_use]
    pub fn busy(&self) -> bool {
        let Ok(foreground) = rustix::termios::tcgetpgrp(&self.terminal) else {
            return false;
        };
        foreground.as_raw_nonzero().get() != self.shell
    }
}

/// The sending end of a [`Pty`]'s nudge pipe.
///
/// Handed to whoever has input for the child, which is never the thread
/// sitting in the wait.
pub struct Waker {
    sender: OwnedFd,
    /// The reading end, held for no reason but to outlive the sender.
    ///
    /// Writing down a pipe nobody is reading raises `SIGPIPE`, which by
    /// default ends the process — and a library may not go changing what its
    /// host does with a signal. The terminal's own thread lets go of its copy
    /// as soon as the child ends, so this is what keeps a nudge sent after
    /// that from taking the app with it.
    _receiver: Arc<OwnedFd>,
}

impl Waker {
    /// Cut the wait short.
    pub fn nudge(&self) {
        // A pipe that already holds a byte is a wait already about to end, and
        // a full one is a great many of them — so a write that does not fit
        // has nothing left to say.
        let _ = rustix::io::write(&self.sender, b"\0");
    }
}

impl Pty {
    /// Open a terminal of `cols` by `rows`, and start `program` on the far
    /// side of it.
    pub fn spawn(program: &[u8], args: &[Vec<u8>], cols: u16, rows: u16) -> Result<(Self, Waker)> {
        // Opened before the terminal so that no child of ours can inherit it,
        // and marked close-on-exec so that no other thread's can either.
        let (nudge, sender) = rustix::pipe::pipe()?;
        ioctl_fioclex(&nudge)?;
        ioctl_fioclex(&sender)?;
        // Neither end may ever block: a nudge is sent from the thread that
        // must not wait, and drained by the one that must not either.
        ioctl_fionbio(&nudge, true)?;
        ioctl_fionbio(&sender, true)?;
        let nudge = Arc::new(nudge);

        let terminal = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
        grantpt(&terminal)?;
        unlockpt(&terminal)?;
        ioctl_fioclex(&terminal)?;
        ioctl_fionbio(&terminal, true)?;

        let far = rustix::fs::open(
            ptsname(&terminal, Vec::new())?,
            OFlags::RDWR | OFlags::NOCTTY,
            Mode::empty(),
        )?;
        // The child's end of the lifeline is opened between fork and exec, so
        // the parent's copy stays close-on-exec and no other thread's spawn
        // can inherit it and hold it open past our own child's end.
        let (lifeline, held_by_child) = rustix::pipe::pipe()?;
        ioctl_fioclex(&lifeline)?;
        ioctl_fioclex(&held_by_child)?;
        ioctl_fionbio(&lifeline, true)?;
        let held_raw = held_by_child.as_raw_fd();

        let far_raw = far.as_raw_fd();
        // The child gets the far end as its three standard streams and wants
        // it as its controlling terminal, neither of which needs this fourth
        // handle on it to survive the exec.
        ioctl_fioclex(&far)?;
        // Sized before the child exists, so its first frame is drawn to the
        // real width rather than to a default it has to be told to leave.
        //
        // In cells only: how many pixels one of them is belongs to the display
        // the window came up on, which nothing here has been told. The app
        // fills it in with its first resize. cf. `02-ffi.md`
        tcsetwinsize(&far, winsize(cols, rows, 0, 0))?;

        let mut command = Command::new(OsStr::from_bytes(program));
        command
            .args(args.iter().map(|arg| OsStr::from_bytes(arg)))
            .stdin(Stdio::from(far.try_clone()?))
            .stdout(Stdio::from(far.try_clone()?))
            .stderr(Stdio::from(far.try_clone()?))
            .env("TERM", TERM)
            .env("COLORTERM", COLORTERM)
            .env("TERM_PROGRAM", TERM_PROGRAM);
        // The crate's other exception to the ban on `unsafe`, and the smaller
        // one: there is no safe way to ask for work between fork and exec.
        //
        // SAFETY: both calls are async-signal-safe, which is the whole of what
        // a child between fork and exec may make.
        #[allow(unsafe_code, reason = "no safe spelling of a pre-exec hook")]
        unsafe {
            command.pre_exec(move || {
                // A session of its own, and then the terminal as its
                // controlling one. Without the pair there is no job control:
                // Ctrl-C reaches nobody and the child is never told the window
                // went away.
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(far_raw))?;
                // Duplicated rather than inherited: the parent's copy is
                // close-on-exec and a duplicate is not, so this is the one
                // handle on the lifeline that survives into the program.
                // Never closed here — outliving this call is the whole job,
                // and the exec that follows leaves it exactly where it is.
                std::mem::forget(rustix::io::dup(BorrowedFd::borrow_raw(held_raw))?);
                Ok(())
            });
        }
        let child = command.spawn()?;
        // The child has its own copy now, and ours must go or the read end
        // never reports the hang-up that says the child is gone.
        drop(held_by_child);

        Ok((
            Self {
                terminal,
                far,
                nudge: Arc::clone(&nudge),
                lifeline,
                child: Kept { process: child },
            },
            Waker {
                sender,
                _receiver: nudge,
            },
        ))
    }

    /// Take a handle for asking, later and from another thread, whether
    /// anything is running in front of the shell.
    ///
    /// Its own duplicate of the terminal rather than a borrow of this one:
    /// what asks outlives neither more nor less than the session, and this
    /// [`Pty`] belongs to a thread the asking side may not touch.
    ///
    /// # Errors
    ///
    /// [`Error::Io`] when the terminal could not be duplicated.
    pub fn foreground(&self) -> Result<Foreground> {
        Ok(Foreground {
            terminal: self.terminal.try_clone()?,
            // `setsid` made the child the leader of a session and of a process
            // group at once, so the group it leads is numbered by its own
            // process id and no second reading is needed to learn it.
            //
            // Said rather than defended against: a process id is an `i32` on
            // the way in and cannot fail to be one on the way back out. A
            // fallback here would be a number that is nobody's process group,
            // which is every window warning for ever.
            shell: i32::try_from(self.child.process.id()).expect("a process id is an i32"),
        })
    }

    /// Block until the terminal has something, has room, a nudge arrives, or
    /// `no_longer_than` runs out.
    ///
    /// `want_room` asks about room only when there is something to put in it:
    /// a terminal with nothing queued for it is writable at all times, and
    /// waiting on that would be a loop that never sleeps.
    ///
    /// A wait that ran out answers with nothing ready, which is the truth: it
    /// is the caller's own deadline coming round, not the terminal's news.
    pub fn wait(&self, want_room: bool, no_longer_than: Option<Duration>) -> Result<Ready> {
        let mut wanted = PollFlags::IN;
        if want_room {
            wanted |= PollFlags::OUT;
        }
        let mut waiting = [
            PollFd::new(&self.terminal, wanted),
            PollFd::new(&self.nudge, PollFlags::IN),
            // No events asked for: a hang-up is reported whether or not it
            // was, and a hang-up is the only thing this pipe ever says.
            PollFd::new(&self.lifeline, PollFlags::empty()),
        ];
        let deadline = no_longer_than.map(|left| Timespec {
            tv_sec: left.as_secs() as _,
            tv_nsec: left.subsec_nanos() as _,
        });
        loop {
            match poll(&mut waiting, deadline.as_ref()) {
                Ok(_) => break,
                Err(Errno::INTR) => continue,
                Err(error) => return Err(error.into()),
            }
        }

        if waiting[1].revents().contains(PollFlags::IN) {
            // A nudge says nothing but "come round", so what it carried is of
            // no interest — only that the pipe is empty for the next one.
            let mut spent = [0u8; 64];
            while matches!(rustix::io::read(&self.nudge, &mut spent[..]), Ok(1..)) {}
        }

        let ready = waiting[0].revents();
        Ok(Ready {
            readable: ready.intersects(PollFlags::IN | PollFlags::HUP),
            writable: ready.contains(PollFlags::OUT),
            ended: waiting[2]
                .revents()
                .intersects(PollFlags::HUP | PollFlags::ERR),
        })
    }

    /// Tell the terminal how big it now is, which is what raises `SIGWINCH`
    /// in the child.
    ///
    /// Set on the far end rather than ours: macOS answers this ioctl on our
    /// end with `ENOTTY`, and both ends of a pseudoterminal share the one
    /// size anyway.
    ///
    /// The pixel size goes with the counts because a program that asks in
    /// pixels asks the terminal, and a zero there is knotty saying it does
    /// not know.
    pub fn set_size(&self, cols: u16, rows: u16, cell_width: u32, cell_height: u32) -> Result<()> {
        Ok(tcsetwinsize(
            &self.far,
            winsize(cols, rows, cell_width, cell_height),
        )?)
    }

    /// Take what the child wrote into `buffer`.
    pub fn read(&self, buffer: &mut [u8]) -> Result<Chunk> {
        match rustix::io::read(&self.terminal, buffer) {
            Ok(0) => Ok(Chunk::Ended),
            Ok(read) => Ok(Chunk::Bytes(read)),
            // The far end closing is how a child's exit reaches us, and on a
            // terminal that is an error rather than an end of file.
            Err(Errno::IO) => Ok(Chunk::Ended),
            Err(Errno::AGAIN | Errno::INTR) => Ok(Chunk::Empty),
            Err(error) => Err(error.into()),
        }
    }

    /// Hand the child as much of `bytes` as the terminal will take, returning
    /// how much that was.
    pub fn write(&self, bytes: &[u8]) -> Result<usize> {
        match rustix::io::write(&self.terminal, bytes) {
            Ok(written) => Ok(written),
            Err(Errno::AGAIN | Errno::INTR) => Ok(0),
            // A child that is gone did not hear this, and saying so is not
            // this side's job — the read side is what reports an end.
            Err(Errno::IO | Errno::PIPE) => Ok(0),
            Err(error) => Err(error.into()),
        }
    }
}

/// The size a terminal is set to, in the two units it is measured in.
///
/// The pixel fields are the whole grid rather than one cell, and they are
/// 16 bits wide — so a grid that does not fit is reported as the largest one
/// that does. Nothing is looking at a terminal that wide, and a number that
/// wrapped would be a smaller lie than a saturated one.
fn winsize(cols: u16, rows: u16, cell_width: u32, cell_height: u32) -> Winsize {
    Winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: u16::try_from(u32::from(cols) * cell_width).unwrap_or(u16::MAX),
        ws_ypixel: u16::try_from(u32::from(rows) * cell_height).unwrap_or(u16::MAX),
    }
}

/// Drive `session` against `terminal` until the child ends or `stopping` is
/// set.
///
/// One thread reads the terminal and feeds the engine, so the engine is driven
/// from here and nowhere else. What the app has for the child does not come
/// through here at all: it goes straight into the queue below, which is where
/// the two are serialized. cf. `03-core.md` C1
///
/// `writes` is the queue the app and the terminal both put bytes in. This loop
/// is the only thing that takes them out of it, and it hands them over inside
/// the queue's own lock — so what the app queues mid-hand-over lands behind
/// what is being handed over rather than in front of it.
///
/// `child_exit` is where the child's code is left for the app's side to read.
/// It is written before the exit is committed, so that a consumer coming for
/// that telling finds the code already there. What the negative it starts out
/// as means is the reader's, in `session`.
pub(crate) fn run(
    session: &mut Session,
    terminal: &mut Pty,
    requests: &Receiver<Request>,
    writes: &WriteQueue,
    child_exit: &AtomicI32,
    stopping: &AtomicBool,
) -> Result<()> {
    let mut arrived = vec![0u8; READ_CHUNK];

    loop {
        // The call that asked for one of these returned as soon as it was
        // queued, and reads its answer off the frame that follows. The one
        // exception is a copy, whose answer is not the screen and which is
        // waiting on the channel it came in on.
        while let Ok(request) = requests.try_recv() {
            let _ = match request {
                Request::Select(range) => session.set_selection(range),
                Request::Gesture {
                    anchor,
                    cell,
                    unit,
                    rectangle,
                } => session.select(anchor, cell, unit, rectangle),
                // The one request with somebody waiting on it. Nothing is
                // sent for an engine that refused: the caller reads the
                // dropped answer as the refusal it is.
                Request::Copy(answering) => session.copy_selection().map(|text| {
                    let _ = answering.send(text);
                }),
                Request::Scroll { lines } => session.scroll_viewport(lines),
                Request::Key(event) => session.key(&event),
                Request::Paste(bytes) => session.paste(&bytes),
                Request::Mouse(event) => session.mouse(&event),
                Request::Wheel(event) => session.wheel(&event),
                Request::Focus { gained } => session.focus(gained),
                // The engine first: what the child draws when it hears of the
                // new size arrives back here, and it has to meet a grid that
                // is already that size.
                Request::Resize {
                    cols,
                    rows,
                    cell_width,
                    cell_height,
                } => session
                    .resize(cols, rows, cell_width, cell_height)
                    .and_then(|()| terminal.set_size(cols, rows, cell_width, cell_height)),
                Request::Theme(theme) => session.set_theme(&theme),
            };
        }

        // A held-back block is waited no longer than it has left, so that the
        // wait itself is what gives up on one the child never closes. The
        // clock is asked here because this is the only thread that waits: a
        // detached session goes back to its caller between rounds and never
        // sits still long enough for one to run out. cf. `03-core.md` C5
        let ready = terminal.wait(writes.waiting() != 0, session.wait_out_sync_block())?;
        if stopping.load(Ordering::Relaxed) {
            return Ok(());
        }

        if ready.writable {
            writes.drain_with(|bytes| terminal.write(bytes))?;
        }

        let mut ended = ready.ended;
        if ready.readable {
            match terminal.read(&mut arrived)? {
                Chunk::Bytes(read) => match session.feed(&arrived[..read]) {
                    // This loop is the only drain a PTY session's writer queue
                    // has, so a full one is ours to shed rather than anyone's
                    // to be told about.
                    Ok(()) | Err(Error::WriteQueueFull) => {}
                    Err(error) => return Err(error),
                },
                Chunk::Empty => {}
                Chunk::Ended => ended = true,
            }
        }

        // The end of the child is seen here rather than watched for
        // elsewhere — so the exit is committed after the last byte was fed
        // and published, with no second mechanism to order the two. cf.
        // `03-core.md` C6
        //
        // What says so is the lifeline rather than the terminal: the far end
        // is ours and stays open, which is what keeps a short-lived child's
        // output from being swept away before this thread gets to it. So the
        // terminal is drained dry here instead. Nothing can arrive behind
        // this — every writer is gone, which is what the hang-up meant — and
        // nothing has been discarded, which is what holding the far end
        // bought. cf. `kwnms04/knotty#43`
        if ended {
            while let Chunk::Bytes(read) = terminal.read(&mut arrived)? {
                match session.feed(&arrived[..read]) {
                    Ok(()) | Err(Error::WriteQueueFull) => {}
                    Err(error) => return Err(error),
                }
            }
            // A reap that fails leaves no code to write down, and the round it
            // fails in ends the session as broken — which is the state an app
            // acts on ahead of anything the child did.
            let code = terminal.child.kill_and_reap()?;
            child_exit.store(code, Ordering::Relaxed);
            return session.note_child_exit(code);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use rustix::termios::tcgetwinsize;

    use super::{Chunk, Pty};

    /// How long a release is given before it counts as one that never ends.
    const PATIENCE: Duration = Duration::from_secs(10);

    /// One of the two places in this crate that start a real child, because
    /// the deadlock it guards lives below the parser: it is between a child
    /// closing its terminal and whoever was meant to be draining that
    /// terminal, and the loop above reaches it only by luck. Driving a
    /// session from outside cannot hold the drain still long enough to see
    /// it — the loop's own reads are what keep letting the child go.
    #[test]
    fn letting_go_of_a_terminal_nobody_drained_still_collects_the_child() {
        // Outliving the test on purpose: the release under test is the one
        // that has a live child to put down, and a child that ended on its own
        // would be collected before the terminal ever mattered.
        let (terminal, _waker) =
            Pty::spawn(b"/bin/sh", &[b"-c".to_vec(), b"sleep 300".to_vec()], 4, 1)
                .expect("a terminal with a child on it");

        // Never read back, so the echo of it stays in the terminal's output
        // queue — which is what a child on its way out waits to see drained.
        //
        // The volume is not the obvious way round. A terminal whose input
        // queue is over its limit throws that queue away instead of refusing
        // it, echoing little of what it dropped — so our end goes on accepting
        // for as long as anyone cares to write, and only a fraction of this
        // ever reaches the queue the test needs left full. There is no way to
        // read back how full it is without the reading that would empty it,
        // so the count is settled the only way left: a single chunk let the
        // test pass against the very bug it guards, and this many does not.
        let chunk = vec![b'x'; 64 * 1024];
        for _ in 0..512 {
            terminal.write(&chunk).expect("a write");
        }

        // Released on a thread of its own, so that a release which never ends
        // is a test that fails rather than a suite that hangs.
        let (released, waiting) = mpsc::channel();
        thread::spawn(move || {
            drop(terminal);
            let _ = released.send(());
        });

        waiting
            .recv_timeout(PATIENCE)
            .expect("the child was put down and collected");
    }

    /// What a program asking the terminal how big it is in pixels reads.
    ///
    /// The counts are what a resize is mostly about, but the pixel pair is the
    /// half that was nailed to zero and the half nothing else here would
    /// catch: it reaches neither the engine nor the screen. Read back off the
    /// terminal's far end, which is the very field a child's `TIOCGWINSZ`
    /// answers from.
    #[test]
    fn a_resize_fills_in_the_size_in_pixels() {
        let (terminal, _waker) =
            Pty::spawn(b"/bin/echo", &[b"knotty".to_vec()], 80, 24).expect("a terminal");

        terminal.set_size(100, 30, 8, 16).expect("a resize");

        let size = tcgetwinsize(&terminal.far).expect("the terminal's own size");
        assert_eq!((size.ws_col, size.ws_row), (100, 30));
        assert_eq!((size.ws_xpixel, size.ws_ypixel), (800, 480));
    }

    /// Long enough to be on the far side of the window macOS leaves before it
    /// throws a dead child's output away — and the very stall a loaded runner
    /// puts on a thread that has only just been started.
    const LATER_THAN_THE_TEARDOWN: Duration = Duration::from_secs(2);

    /// A child that prints once and stops keeps what it printed until it is
    /// read.
    ///
    /// The other place in this crate that starts a real child, because what
    /// it guards is below the parser too: macOS tears a terminal down once
    /// the last far-end handle is closed, waiting about six-tenths of a
    /// second for our end to drain what is queued and then throwing the rest
    /// away. A thread slow to be scheduled used to come back to an empty
    /// terminal and no way of telling that from a child which printed
    /// nothing. cf. `kwnms04/knotty#43`
    #[test]
    fn a_child_that_prints_once_and_stops_keeps_its_output() {
        let (terminal, _waker) = Pty::spawn(b"/bin/echo", &[b"knotty".to_vec()], 80, 24)
            .expect("a terminal with a child on it");

        thread::sleep(LATER_THAN_THE_TEARDOWN);

        let mut arrived = [0u8; 64];
        let mut seen = Vec::new();
        let deadline = Instant::now() + PATIENCE;
        while !seen.contains(&b'\n') && Instant::now() < deadline {
            match terminal.read(&mut arrived).expect("a read") {
                Chunk::Bytes(read) => seen.extend_from_slice(&arrived[..read]),
                Chunk::Empty => thread::sleep(Duration::from_millis(10)),
                Chunk::Ended => break,
            }
        }

        assert_eq!(
            String::from_utf8_lossy(&seen).trim_end(),
            "knotty",
            "the child's only line went missing"
        );
    }
}
