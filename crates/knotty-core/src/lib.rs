//! Terminal session state, snapshot conversion, and the snapshot mailbox.
//!
//! The VT engine is walled off in the private `vt` module, which is the whole
//! of what knows it exists: it drives the engine, it flattens the engine's
//! render state into a [`snapshot::Snapshot`], and no engine type appears in a
//! signature outside it. cf. `docs/adr/0004-hide-vt-engine-types.md`
//!
//! The operating system is walled off the same way. [`io`] is the only module
//! that knows a file descriptor, and no platform type appears in a signature
//! outside it. cf. `docs/adr/0001-portable-core.md`
//!
//! The two walls are also where the crate's `unsafe` is. Everywhere else it is
//! denied outright, which is what makes the pair of exceptions checkable. cf.
//! `docs/adr/0012-own-the-binding-layer.md`

#![deny(unsafe_code)]

pub mod io;
pub mod mailbox;
pub mod queue;
pub mod session;
pub mod snapshot;
mod vt;

pub use queue::{ClipboardTarget, Event};
pub use session::{ChildState, PtySession, SelectionRange, Session, Wake};
pub use snapshot::{
    Attribute, Cell, Cursor, CursorShape, Dirty, Rgb, Row, RowFlag, ScreenState, Snapshot,
    Underline,
};

/// Why a core operation failed.
///
/// Variants are distinguishable so the FFI layer can map them onto status
/// codes a caller can classify.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    /// The VT engine rejected an operation.
    Engine,
    /// The terminal's state is bigger than a snapshot can describe.
    TooLarge,
    /// A coordinate fell outside the terminal.
    OutOfRange,
    /// The queue of bytes bound for the child is at its cap, and what did not
    /// fit was dropped.
    WriteQueueFull,
    /// An operating system call failed — opening a terminal, starting a child,
    /// or talking to one already started.
    Io,
}

/// Result alias for fallible core operations.
pub type Result<T> = std::result::Result<T, Error>;
