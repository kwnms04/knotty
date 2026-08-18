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

use crate::session::{SelectionRange, Session};
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

/// How much comes off the terminal in one read.
const READ_CHUNK: usize = 64 * 1024;

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
                child: Kept { process: child },
            },
            Waker {
                sender,
                _receiver: nudge,
            },
        ))
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
    selection: &Receiver<Option<SelectionRange>>,
    writes: &WriteQueue,
    child_exit: &AtomicI32,
    stopping: &AtomicBool,
) -> Result<()> {
    let mut arrived = vec![0u8; READ_CHUNK];

    loop {
        // Nothing is left waiting on the answer: the call that asked for this
        // returned as soon as the request was queued.
        while let Ok(range) = selection.try_recv() {
            let _ = session.set_selection(range);
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
                    // The one collection that keeps our end open while it
                    // waits, and it may only because an end here means every
                    // handle on the far side is closed — a child holding
                    // output back cannot have got us this far. A reading that
                    // called an end any sooner would put this wait back where
                    // [`Kept`] says it must not be.
                    let code = terminal.child.kill_and_reap()?;
                    child_exit.store(code, Ordering::Relaxed);
                    return session.note_child_exit(code);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::Pty;

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
}
