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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    /// A button went down.
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
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    /// No button — which only a motion can be.
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
#[derive(Clone, Copy, Debug)]
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

/// One turn of the wheel, in lines, over one cell.
///
/// Lines rather than pixels: a trackpad reports its inertia a few pixels at
/// a time and reports a great many of them, and turning those into lines
/// wants the height a line is drawn at — which is known where the display
/// is. Up and right are positive.
///
/// Undecided like the rest. What a turn comes to is a mouse code, a run of
/// cursor keys or the viewport moving, and which of the three is a question
/// only the terminal can answer. cf. `docs/adr/0017-semantic-input-events.md`
#[derive(Clone, Copy, Debug)]
pub struct WheelEvent {
    /// How many lines sideways, right positive.
    pub delta_x: i32,
    /// How many lines up or down, up positive.
    pub delta_y: i32,
    /// What was held down, as [`crate::key::Modifier`] bits.
    pub mods: u16,
    /// The column the pointer was over, counted from the left of the
    /// viewport.
    pub x: u16,
    /// The row the pointer was over, counted from the top of the viewport.
    pub y: u16,
}
