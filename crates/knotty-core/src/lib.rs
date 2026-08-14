//! Terminal session state, snapshot conversion, and the snapshot mailbox.
//!
//! The VT engine lives behind this crate: its types appear only in
//! [`snapshot`], the single conversion point (C3).

pub mod mailbox;
pub mod session;
pub mod snapshot;

pub use session::Session;
pub use snapshot::{Attribute, Cell, Rgb, Snapshot, Underline};

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
}

/// Result alias for fallible core operations.
pub type Result<T> = std::result::Result<T, Error>;
