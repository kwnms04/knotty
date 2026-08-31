//! What a mouse did before anything has decided what bytes it becomes.
//!
//! The cell rather than the pixel: converting one to the other wants the
//! metrics, and those live where the display is. Everything after that —
//! whether the child hears about it at all, which of the five reporting
//! formats it is written in, whether a wheel is a mouse code or a cursor key
//! or the viewport moving — is read against modes the terminal holds and so
//! is decided beside it. cf. `docs/adr/0017-semantic-input-events.md`

/// Which way a mouse moved.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
    #[default]
    Press = 0,
    /// A button came back up.
    Release = 1,
    /// The pointer moved.
    Motion = 2,
}

/// Which button a mouse event is about.
///
/// The three a terminal has ever been told about, and the absence that a
/// motion with nothing held is. The engine names eight more; nothing on this
/// side has a way to press one, so nothing here names them.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MouseButton {
    /// No button — which only a motion can be.
    #[default]
    None = 0,
    /// The left button.
    Left = 1,
    /// The right button.
    Right = 2,
    /// The middle button.
    Middle = 3,
}

/// One mouse event, as it crosses the boundary: what happened to which
/// button, what was held at the time, and which cell it happened over.
///
/// Undecided on purpose. A click with reporting off comes to no bytes at all,
/// and which of the reporting formats a click that does report is written in
/// is the terminal's to say — reading either anywhere but beside the terminal
/// reads it as of some earlier frame. cf.
/// `docs/adr/0017-semantic-input-events.md`
#[derive(Clone, Copy, Debug, Default)]
pub struct MouseEvent {
    /// Which way the mouse moved.
    pub action: MouseAction,
    /// Which button it was about.
    pub button: MouseButton,
    /// What was held down, as [`crate::key::Modifier`] bits.
    pub mods: u16,
    /// The column the pointer was over, counted from the left of the
    /// viewport.
    pub x: u16,
    /// The row the pointer was over, counted from the top of the viewport.
    pub y: u16,
}
