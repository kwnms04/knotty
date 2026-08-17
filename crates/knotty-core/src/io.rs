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
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering};
use std::sync::mpsc::Receiver;

use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::{Mode, OFlags};
use rustix::io::{Errno, ioctl_fioclex, ioctl_fionbio};
use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
use rustix::termios::{Winsize, tcsetwinsize};

use crate::session::{SelectionRange, Session};
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

/// How much comes off the terminal in one read.
const READ_CHUNK: usize = 64 * 1024;

/// What the app has for a session, waiting for the one thread allowed to
/// touch it.
pub enum Input {
    /// Bytes bound for the child.
    Write(Vec<u8>),
    /// A range of the viewport to select, or `None` to clear the selection.
    Selection(Option<SelectionRange>),
}

/// What a wait found ready.
pub struct Ready {
    /// The child wrote something, or let go of the terminal.
    pub readable: bool,
    /// The terminal has room for more.
    pub writable: bool,
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
    /// Read end of the nudge pipe, waited on beside the terminal so that
    /// input from another thread cuts the wait short.
    nudge: Arc<OwnedFd>,
    /// Held so the child stays ours to wait on, and to collect on the way out.
    child: Child,
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
        let far_raw = far.as_raw_fd();
        // The child gets the far end as its three standard streams and wants
        // it as its controlling terminal, neither of which needs this fourth
        // handle on it to survive the exec.
        ioctl_fioclex(&far)?;
        // Sized before the child exists, so its first frame is drawn to the
        // real width rather than to a default it has to be told to leave.
        //
        // Set on the far end rather than ours: macOS answers this ioctl on our
        // end with `ENOTTY`, and both ends of a pseudoterminal share the one
        // size anyway.
        tcsetwinsize(
            &far,
            Winsize {
                ws_row: rows,
                ws_col: cols,
                ws_xpixel: 0,
                ws_ypixel: 0,
            },
        )?;

        let mut command = Command::new(OsStr::from_bytes(program));
        command
            .args(args.iter().map(|arg| OsStr::from_bytes(arg)))
            .stdin(Stdio::from(far.try_clone()?))
            .stdout(Stdio::from(far.try_clone()?))
            .stderr(Stdio::from(far.try_clone()?))
            .env("TERM", TERM);
        // SAFETY: both calls are async-signal-safe, which is the whole of what
        // a child between fork and exec may make.
        unsafe {
            command.pre_exec(move || {
                // A session of its own, and then the terminal as its
                // controlling one. Without the pair there is no job control:
                // Ctrl-C reaches nobody and the child is never told the window
                // went away.
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(far_raw))?;
                Ok(())
            });
        }
        let child = command.spawn()?;
        // The far end must stop being ours, or the terminal never reaches its
        // end when the child lets go — and an end nobody sees is an exit
        // nobody hears about.
        drop(far);

        Ok((
            Self {
                terminal,
                nudge: Arc::clone(&nudge),
                child,
            },
            Waker {
                sender,
                _receiver: nudge,
            },
        ))
    }

    /// Block until the terminal has something, has room, or a nudge arrives.
    ///
    /// `want_room` asks about room only when there is something to put in it:
    /// a terminal with nothing queued for it is writable at all times, and
    /// waiting on that would be a loop that never sleeps.
    pub fn wait(&self, want_room: bool) -> Result<Ready> {
        let mut wanted = PollFlags::IN;
        if want_room {
            wanted |= PollFlags::OUT;
        }
        let mut waiting = [
            PollFd::new(&self.terminal, wanted),
            PollFd::new(&self.nudge, PollFlags::IN),
        ];
        loop {
            match poll(&mut waiting, None) {
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
            // A hang-up is the child's exit arriving as readiness. It counts
            // as readable because the read that follows is what turns it into
            // an end.
            readable: ready.intersects(PollFlags::IN | PollFlags::HUP),
            writable: ready.contains(PollFlags::OUT),
        })
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

    /// Put the child down and collect it, answering with what it ended by.
    ///
    /// The signal is for the child that let go of its terminal and kept
    /// running: knotty can no longer see it or talk to it, so waiting on one
    /// would be waiting forever and leaving it would be leaving it for good.
    /// A child that ended of its own accord — every ordinary one — is only
    /// waiting to be collected by the time this is called, and a signal
    /// changes nothing about what it ended with.
    pub fn kill_and_reap(&mut self) -> Result<i32> {
        let _ = self.child.kill();
        Ok(exit_code(self.child.wait()?))
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

impl Drop for Pty {
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

/// Drive `session` against `terminal` until the child ends or `stopping` is
/// set.
///
/// Reading and writing are the same thread, which is what makes the engine's
/// answers and the app's input serialize with no lock between them. cf.
/// `03-core.md` C1
///
/// `backlog` counts every byte still waiting for the child, whoever queued it,
/// and is what the app's own writes are refused against — so it is added to
/// here as the engine answers and taken from as the terminal accepts.
///
/// `child_exit` is where the child's code is left for the app's side to read.
/// It is written before the exit is committed, so that a consumer coming for
/// that telling finds the code already there. What the negative it starts out
/// as means is the reader's, in `session`.
pub fn run(
    session: &mut Session,
    terminal: &mut Pty,
    input: &Receiver<Input>,
    backlog: &AtomicUsize,
    child_exit: &AtomicI32,
    stopping: &AtomicBool,
) -> Result<()> {
    let mut arrived = vec![0u8; READ_CHUNK];
    // Bounded by whoever queues into it: the app's share is what `backlog`
    // counts and the cap refuses past, and the session's own writer queue caps
    // the engine's answers before they ever get here.
    let mut bound_for_child = Vec::new();

    loop {
        while let Ok(one) = input.try_recv() {
            match one {
                Input::Write(bytes) => bound_for_child.extend_from_slice(&bytes),
                // Nothing is left waiting on the answer: the call that asked
                // for this returned as soon as the request was queued.
                Input::Selection(range) => {
                    let _ = session.set_selection(range);
                }
            }
        }
        let answers = session.take_writes();
        backlog.fetch_add(answers.len(), Ordering::Relaxed);
        bound_for_child.extend_from_slice(answers);

        let ready = terminal.wait(!bound_for_child.is_empty())?;
        if stopping.load(Ordering::Relaxed) {
            return Ok(());
        }

        if ready.writable {
            let written = terminal.write(&bound_for_child)?;
            backlog.fetch_sub(written, Ordering::Relaxed);
            bound_for_child.drain(..written);
        }
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
                // The end of the terminal is the end of the child, and it is
                // seen here rather than watched for elsewhere — so the exit is
                // committed after the last byte was fed and published, with no
                // second mechanism to order the two. cf. `03-core.md` C6
                // A reap that fails leaves no code to write down, and the
                // round it fails in ends the session as broken — which is the
                // state an app acts on ahead of anything the child did.
                Chunk::Ended => {
                    let code = terminal.kill_and_reap()?;
                    child_exit.store(code, Ordering::Relaxed);
                    return session.note_child_exit(code);
                }
            }
        }
    }
}
