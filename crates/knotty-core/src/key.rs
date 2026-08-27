//! What a key press is before anything has decided what bytes it becomes.
//!
//! The physical key travels rather than the character: the same key is `A` on
//! a US layout and `Ф` on a Russian one, which is what makes `⌃A` the same
//! place on the keyboard whatever the layout says. What the layout did make of
//! it travels beside it as text. cf. `docs/adr/0017-semantic-input-events.md`

/// Which way a key moved.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyAction {
    /// The key came back up.
    Release = 0,
    /// The key went down.
    #[default]
    Press = 1,
    /// The key is held down and the platform is repeating it.
    Repeat = 2,
}

/// Modifier state, OR-ed together into a key event's `mods` and
/// `consumed_mods` fields.
///
/// A side bit says which of a pair is held and means nothing unless its
/// modifier's own bit is set. Not every platform can tell the two apart, and
/// nothing here needs one to.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Modifier {
    /// Shift.
    Shift = 1 << 0,
    /// Control.
    Ctrl = 1 << 1,
    /// Alt, which is Option on macOS.
    Alt = 1 << 2,
    /// Super, which is Command on macOS.
    Super = 1 << 3,
    /// Caps lock is on.
    CapsLock = 1 << 4,
    /// Num lock is on.
    NumLock = 1 << 5,
    /// The shift held is the right-hand one.
    ShiftRight = 1 << 6,
    /// The control held is the right-hand one.
    CtrlRight = 1 << 7,
    /// The alt held is the right-hand one.
    AltRight = 1 << 8,
    /// The super held is the right-hand one.
    SuperRight = 1 << 9,
}

/// A physical key, named as the W3C `KeyboardEvent.code` standard names it.
///
/// Layout-independent by construction: the value says where on the keyboard
/// the key is, not what pressing it typed. The sections below are the
/// standard's own, and the media section (§ 3.6) is left out — macOS hands an
/// application no `keyDown` for those keys, so nothing could ever name one.
/// cf. <https://www.w3.org/TR/uievents-code>
#[repr(u32)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Key {
    /// No key this list names. A platform key that maps to nothing here
    /// arrives as this, which is a missing mapping rather than a key without
    /// a name.
    #[default]
    Unidentified = 0,

    // Writing System Keys (W3C § 3.1.1)
    Backquote,
    Backslash,
    BracketLeft,
    BracketRight,
    Comma,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Equal,
    IntlBackslash,
    IntlRo,
    IntlYen,
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Minus,
    Period,
    Quote,
    Semicolon,
    Slash,

    // Functional Keys (W3C § 3.1.2)
    AltLeft,
    AltRight,
    Backspace,
    CapsLock,
    ContextMenu,
    ControlLeft,
    ControlRight,
    Enter,
    MetaLeft,
    MetaRight,
    ShiftLeft,
    ShiftRight,
    Space,
    Tab,
    Convert,
    KanaMode,
    NonConvert,

    // Control Pad Section (W3C § 3.2)
    Delete,
    End,
    Help,
    Home,
    Insert,
    PageDown,
    PageUp,

    // Arrow Pad Section (W3C § 3.3)
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,

    // Numpad Section (W3C § 3.4)
    NumLock,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadBackspace,
    NumpadClear,
    NumpadClearEntry,
    NumpadComma,
    NumpadDecimal,
    NumpadDivide,
    NumpadEnter,
    NumpadEqual,
    NumpadMemoryAdd,
    NumpadMemoryClear,
    NumpadMemoryRecall,
    NumpadMemoryStore,
    NumpadMemorySubtract,
    NumpadMultiply,
    NumpadParenLeft,
    NumpadParenRight,
    NumpadSubtract,
    NumpadSeparator,
    NumpadUp,
    NumpadDown,
    NumpadRight,
    NumpadLeft,
    NumpadBegin,
    NumpadHome,
    NumpadEnd,
    NumpadInsert,
    NumpadDelete,
    NumpadPageUp,
    NumpadPageDown,

    // Function Section (W3C § 3.5)
    Escape,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    F13,
    F14,
    F15,
    F16,
    F17,
    F18,
    F19,
    F20,
    F21,
    F22,
    F23,
    F24,
    F25,
    Fn,
    FnLock,
    PrintScreen,
    ScrollLock,
    Pause,
}

/// One key event, as it crosses the boundary: what happened to which key,
/// what was held at the time, and what the layout made of it.
///
/// Undecided on purpose. Which bytes it comes to depends on modes the
/// terminal holds — cursor key application mode, keypad mode,
/// `modifyOtherKeys` — and reading those anywhere but beside the terminal
/// answers with the mode as of some earlier moment. cf.
/// `docs/adr/0017-semantic-input-events.md`
#[derive(Clone, Debug, Default)]
pub struct KeyEvent {
    /// Which way the key moved.
    pub action: KeyAction,
    /// Which key it was.
    pub key: Key,
    /// What was held down, as [`Modifier`] bits.
    pub mods: u16,
    /// Which of those the layout already spent on `text`, as [`Modifier`]
    /// bits. Option making `å` out of `⌥A` on macOS is one: the modifier was
    /// held, but it is not one the terminal should encode a second time.
    pub consumed_mods: u16,
    /// What the layout made of the key, as UTF-8, empty where it made
    /// nothing.
    ///
    /// Control characters do not belong here: the encoder derives those from
    /// the key and the modifiers, and a C0 byte arriving as text is one it
    /// would encode twice.
    pub text: Vec<u8>,
    /// Whether an input method is mid-composition. The engine holds keys back
    /// while it is, which is what keeps half a syllable out of the child.
    pub composing: bool,
}
