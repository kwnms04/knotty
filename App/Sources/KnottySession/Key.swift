import CKnotty

/// Which way a key moved.
public enum KeyAction: Sendable {
    /// The key came back up.
    case release
    /// The key went down.
    case press
    /// The key is held down and the platform is repeating it.
    case `repeat`

    fileprivate var raw: UInt8 {
        let action =
            switch self {
            case .release: KT_KEY_ACTION_RELEASE
            case .press: KT_KEY_ACTION_PRESS
            case .repeat: KT_KEY_ACTION_REPEAT
            }
        return UInt8(action.rawValue)
    }
}

/// What was held down when a key moved.
///
/// The four a terminal encodes, and the lock macOS reports beside them. The
/// side bits the header also holds — which of a pair is down — wait for the
/// thing that would read them: Option-as-Meta is settable per side and its
/// configuration pipeline is M4's. cf. 05-swift-app 7.
public struct Modifiers: OptionSet, Sendable {
    public let rawValue: UInt16

    public init(rawValue: UInt16) { self.rawValue = rawValue }

    /// Shift.
    public static let shift = Modifiers(rawValue: UInt16(KT_MODIFIER_SHIFT.rawValue))
    /// Control.
    public static let ctrl = Modifiers(rawValue: UInt16(KT_MODIFIER_CTRL.rawValue))
    /// Alt, which is Option on macOS.
    public static let alt = Modifiers(rawValue: UInt16(KT_MODIFIER_ALT.rawValue))
    /// Super, which is Command on macOS.
    public static let `super` = Modifiers(rawValue: UInt16(KT_MODIFIER_SUPER.rawValue))
    /// Caps lock is on.
    public static let capsLock = Modifiers(rawValue: UInt16(KT_MODIFIER_CAPS_LOCK.rawValue))
}

/// One key on its way to the terminal, before anything has decided what bytes
/// it becomes.
///
/// What crosses is the physical key rather than the character: the same key is
/// `A` on a US layout and `Ф` on a Russian one, which is what makes `⌃A` the
/// same place on the keyboard whatever the layout says. What the layout made
/// of it travels beside it as text.
///
/// Which bytes it comes to is the core's to answer, because the modes that
/// decide belong to the terminal — cursor key mode, keypad mode,
/// Option-as-Meta, `modifyOtherKeys`. cf. adr/0017.
public struct KeyEvent: Sendable {
    private let action: KeyAction
    private let key: UInt32
    private let mods: Modifiers
    private let consumedMods: Modifiers
    private let text: [UInt8]
    private let composing: Bool

    /// Read a key off what macOS handed the app.
    ///
    /// `macOSKeyCode` is `NSEvent.keyCode`: the virtual key code, which says
    /// where on the keyboard the key is rather than what pressing it typed. A
    /// code naming no key of ours is refused by ``Session/key(_:)`` with
    /// ``SessionError/unidentifiedKey``, so a hole in the table below is heard
    /// about where it happens.
    ///
    /// `consumedMods` is what the layout already spent on `text` — Option
    /// making `å` out of `⌥A` is one, and the terminal encoding Meta on top of
    /// it would be that modifier counted twice.
    ///
    /// `text` is what the layout made of the key. What is not text is dropped
    /// rather than carried: the core derives a control character from the key
    /// and the modifiers, and AppKit's own private use codepoints for the
    /// arrows and the function keys name keys it already has.
    public init(
        macOSKeyCode: UInt16,
        action: KeyAction = .press,
        mods: Modifiers = [],
        consumedMods: Modifiers = [],
        text: String = "",
        composing: Bool = false
    ) {
        self.action = action
        key = physicalKey(macOSKeyCode: macOSKeyCode)
        self.mods = mods
        self.consumedMods = consumedMods
        var kept = String.UnicodeScalarView()
        kept.append(contentsOf: text.unicodeScalars.filter(isText))
        self.text = Array(String(kept).utf8)
        self.composing = composing
    }

    /// Lend the event to `body` as the boundary takes one.
    ///
    /// The text is borrowed for the length of the call, which is the only span
    /// the boundary reads it for.
    func withRaw<Value>(_ body: (KtKeyEvent) throws -> Value) rethrows -> Value {
        try text.withUnsafeBufferPointer { text in
            try body(
                KtKeyEvent(
                    action: action.raw,
                    key: key,
                    mods: mods.rawValue,
                    consumed_mods: consumedMods.rawValue,
                    composing: composing,
                    // Null only where the length is 0, which is what the
                    // boundary allows and what a key that typed nothing is.
                    text: KtText(bytes: text.baseAddress, len: text.count)
                )
            )
        }
    }
}

/// Whether a codepoint AppKit put on a key event is really text.
///
/// C0 and DEL are not: the core derives those from the key and the modifiers,
/// and one arriving as text is one it would encode twice. Neither is the
/// private use area AppKit answers with for the arrows and the function keys —
/// those are key codes wearing a character's clothes, and the key itself has
/// already said which they are.
private func isText(_ scalar: Unicode.Scalar) -> Bool {
    scalar.value >= 0x20 && scalar.value != 0x7f && !(0xf700...0xf8ff).contains(scalar.value)
}

/// Which physical key sits at a macOS virtual key code.
///
/// Every code `Carbon.HIToolbox` assigns is here except four, and each of the
/// four is a key this list has no name for rather than one left out: the three
/// media keys, which the W3C standard holds in § 3.6 and this enum leaves out
/// because macOS hands an application no `keyDown` for them, and JIS 英数,
/// which neither the standard's functional keys nor the engine's own list
/// names.
///
/// Written out as the numbers rather than against `Carbon`, so that the test
/// beside it — which does name the constants — is checking the table and not
/// repeating it.
private func physicalKey(macOSKeyCode: UInt16) -> UInt32 {
    let key =
        switch macOSKeyCode {
        // Letters, which sit where the original Apple keyboard put them
        // rather than in any order.
        case 0x00: KT_KEY_A
        case 0x0b: KT_KEY_B
        case 0x08: KT_KEY_C
        case 0x02: KT_KEY_D
        case 0x0e: KT_KEY_E
        case 0x03: KT_KEY_F
        case 0x05: KT_KEY_G
        case 0x04: KT_KEY_H
        case 0x22: KT_KEY_I
        case 0x26: KT_KEY_J
        case 0x28: KT_KEY_K
        case 0x25: KT_KEY_L
        case 0x2e: KT_KEY_M
        case 0x2d: KT_KEY_N
        case 0x1f: KT_KEY_O
        case 0x23: KT_KEY_P
        case 0x0c: KT_KEY_Q
        case 0x0f: KT_KEY_R
        case 0x01: KT_KEY_S
        case 0x11: KT_KEY_T
        case 0x20: KT_KEY_U
        case 0x09: KT_KEY_V
        case 0x0d: KT_KEY_W
        case 0x07: KT_KEY_X
        case 0x10: KT_KEY_Y
        case 0x06: KT_KEY_Z

        // The digit row, and the punctuation around it.
        case 0x1d: KT_KEY_DIGIT0
        case 0x12: KT_KEY_DIGIT1
        case 0x13: KT_KEY_DIGIT2
        case 0x14: KT_KEY_DIGIT3
        case 0x15: KT_KEY_DIGIT4
        case 0x17: KT_KEY_DIGIT5
        case 0x16: KT_KEY_DIGIT6
        case 0x1a: KT_KEY_DIGIT7
        case 0x1c: KT_KEY_DIGIT8
        case 0x19: KT_KEY_DIGIT9
        case 0x32: KT_KEY_BACKQUOTE
        case 0x2a: KT_KEY_BACKSLASH
        case 0x21: KT_KEY_BRACKET_LEFT
        case 0x1e: KT_KEY_BRACKET_RIGHT
        case 0x2b: KT_KEY_COMMA
        case 0x18: KT_KEY_EQUAL
        case 0x1b: KT_KEY_MINUS
        case 0x2f: KT_KEY_PERIOD
        case 0x27: KT_KEY_QUOTE
        case 0x29: KT_KEY_SEMICOLON
        case 0x2c: KT_KEY_SLASH

        // The keys only some keyboards have: the ISO one beside the left
        // shift, and the three a JIS board adds.
        case 0x0a: KT_KEY_INTL_BACKSLASH
        case 0x5e: KT_KEY_INTL_RO
        case 0x5d: KT_KEY_INTL_YEN
        case 0x68: KT_KEY_KANA_MODE

        // The functional keys, including the modifiers — which encode to
        // nothing on their own, and are here because what a key event carries
        // is which key it was and not whether it will come to anything.
        case 0x3a: KT_KEY_ALT_LEFT
        case 0x3d: KT_KEY_ALT_RIGHT
        case 0x33: KT_KEY_BACKSPACE
        case 0x39: KT_KEY_CAPS_LOCK
        case 0x6e: KT_KEY_CONTEXT_MENU
        case 0x3b: KT_KEY_CONTROL_LEFT
        case 0x3e: KT_KEY_CONTROL_RIGHT
        case 0x24: KT_KEY_ENTER
        case 0x37: KT_KEY_META_LEFT
        case 0x36: KT_KEY_META_RIGHT
        case 0x38: KT_KEY_SHIFT_LEFT
        case 0x3c: KT_KEY_SHIFT_RIGHT
        case 0x31: KT_KEY_SPACE
        case 0x30: KT_KEY_TAB
        case 0x3f: KT_KEY_FN

        // The control pad. Backspace is `kVK_Delete` above; this is the other
        // one, which deletes forwards.
        case 0x75: KT_KEY_DELETE
        case 0x77: KT_KEY_END
        case 0x72: KT_KEY_HELP
        case 0x73: KT_KEY_HOME
        case 0x79: KT_KEY_PAGE_DOWN
        case 0x74: KT_KEY_PAGE_UP

        case 0x7d: KT_KEY_ARROW_DOWN
        case 0x7b: KT_KEY_ARROW_LEFT
        case 0x7c: KT_KEY_ARROW_RIGHT
        case 0x7e: KT_KEY_ARROW_UP

        // The numeric keypad. What a PC board has num lock on, an Apple one
        // has Clear on, and this list has a name for the key that is really
        // there.
        case 0x52: KT_KEY_NUMPAD0
        case 0x53: KT_KEY_NUMPAD1
        case 0x54: KT_KEY_NUMPAD2
        case 0x55: KT_KEY_NUMPAD3
        case 0x56: KT_KEY_NUMPAD4
        case 0x57: KT_KEY_NUMPAD5
        case 0x58: KT_KEY_NUMPAD6
        case 0x59: KT_KEY_NUMPAD7
        case 0x5b: KT_KEY_NUMPAD8
        case 0x5c: KT_KEY_NUMPAD9
        case 0x45: KT_KEY_NUMPAD_ADD
        case 0x47: KT_KEY_NUMPAD_CLEAR
        case 0x5f: KT_KEY_NUMPAD_COMMA
        case 0x41: KT_KEY_NUMPAD_DECIMAL
        case 0x4b: KT_KEY_NUMPAD_DIVIDE
        case 0x4c: KT_KEY_NUMPAD_ENTER
        case 0x51: KT_KEY_NUMPAD_EQUAL
        case 0x43: KT_KEY_NUMPAD_MULTIPLY
        case 0x4e: KT_KEY_NUMPAD_SUBTRACT

        case 0x35: KT_KEY_ESCAPE
        case 0x7a: KT_KEY_F1
        case 0x78: KT_KEY_F2
        case 0x63: KT_KEY_F3
        case 0x76: KT_KEY_F4
        case 0x60: KT_KEY_F5
        case 0x61: KT_KEY_F6
        case 0x62: KT_KEY_F7
        case 0x64: KT_KEY_F8
        case 0x65: KT_KEY_F9
        case 0x6d: KT_KEY_F10
        case 0x67: KT_KEY_F11
        case 0x6f: KT_KEY_F12
        case 0x69: KT_KEY_F13
        case 0x6b: KT_KEY_F14
        case 0x71: KT_KEY_F15
        case 0x6a: KT_KEY_F16
        case 0x40: KT_KEY_F17
        case 0x4f: KT_KEY_F18
        case 0x50: KT_KEY_F19
        case 0x5a: KT_KEY_F20

        default: KT_KEY_UNIDENTIFIED
        }
    return UInt32(key.rawValue)
}
