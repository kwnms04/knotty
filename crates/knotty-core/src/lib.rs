//! Terminal session state, snapshot conversion, and the snapshot mailbox.
//!
//! The VT engine lives behind this crate. Reading it into a snapshot happens
//! in [`snapshot`] and nowhere else, which is the conversion the boundary
//! depends on; [`session`] also names engine types where it drives the engine.
//! No engine type appears in any signature outside those two modules.

pub mod mailbox;
pub mod session;
pub mod snapshot;

pub use session::{SelectionRange, Session};
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
}

/// Result alias for fallible core operations.
pub type Result<T> = std::result::Result<T, Error>;
