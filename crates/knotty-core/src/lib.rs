//! Terminal session state, snapshot conversion, and the snapshot mailbox.
//!
//! The VT engine lives behind this crate. Reading it into a snapshot happens
//! in [`snapshot`] and nowhere else, which is the conversion the boundary
//! depends on; [`session`] also names engine types where it drives the engine.
//! No engine type appears in any signature outside those two modules.
//!
//! The operating system is walled off the same way. [`io`] is the only module
//! that knows a file descriptor, and no platform type appears in a signature
//! outside it. cf. `docs/adr/0001-portable-core.md`

pub mod io;
pub mod mailbox;
pub mod queue;
pub mod session;
pub mod snapshot;

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
