import Carbon.HIToolbox
import Testing

import KnottySession

/// A session to type into. Small on purpose: what these read is the bytes that
/// left for the child, and a screen nobody typed onto is only there to be
/// complete — the same reason the Rust scripts run on a grid this size.
private func typist() throws -> Session {
    try Session(cols: 20, rows: 2, scrollback: 0)
}

/// One key pressed, answering what it queued for the child.
private func press(
    _ session: Session,
    _ macOSKeyCode: Int,
    mods: Modifiers = [],
    text: String = "",
    action: KeyAction = .press
) throws -> [UInt8] {
    try session.key(
        KeyEvent(macOSKeyCode: UInt16(macOSKeyCode), action: action, mods: mods, text: text)
    )
    return try session.takeWrites()
}

/// The codes `Carbon` assigns nothing to. No keyboard sends one, and a table
/// answering for one would be answering for a key that is not there.
private let unassigned: Set<UInt16> = [0x34, 0x42, 0x44, 0x46, 0x4d, 0x6c, 0x70]

/// The codes it does assign that the physical keys still have no name for: the
/// three media keys, which the W3C standard holds in § 3.6 and the list leaves
/// out because macOS hands an application no `keyDown` for them, and JIS 英数,
/// which neither the standard's functional keys nor the engine's own list
/// names at all.
private let unnameable: Set<UInt16> = [
    UInt16(kVK_VolumeUp), UInt16(kVK_VolumeDown), UInt16(kVK_Mute), UInt16(kVK_JIS_Eisu),
]

/// The mapping is filled in or a key falls out of the window silently, which
/// is the whole reason the boundary answers a key that named nothing
/// differently from one that encoded to nothing.
///
/// Read as a set rather than key by key, so a hole says which code is missing
/// and a code answering that should not says the same.
@Test func everyKeyCodeMacOSAssignsNamesAPhysicalKey() throws {
    let session = try typist()
    var nameless: Set<UInt16> = []

    for code in UInt16(0)...0x7e {
        do {
            try session.key(KeyEvent(macOSKeyCode: code))
            // Emptied as it goes: the queue is not what is being measured, and
            // a full one would refuse the rest of the keyboard.
            _ = try session.takeWrites()
        } catch let error as SessionError where error.status == SessionError.unidentifiedKey {
            nameless.insert(code)
        }
    }

    #expect(nameless == unassigned.union(unnameable))
}

/// The keys a shell cannot be used without, and the bytes a terminal has
/// always sent for them.
///
/// Which bytes a key comes to is the Rust harness's to pin, recording by
/// recording. What is here is the other half of that: a code carrying the
/// wrong key would leave every one of those goldens passing.
@Test func theKeysAShellNeedsComeToTheBytesItReads() throws {
    let session = try typist()

    #expect(try press(session, kVK_ANSI_A, text: "a") == [0x61])
    #expect(try press(session, kVK_Return) == [0x0d])
    #expect(try press(session, kVK_Delete) == [0x7f])
    #expect(try press(session, kVK_Tab) == [0x09])
    #expect(try press(session, kVK_Escape) == [0x1b])
    #expect(try press(session, kVK_ANSI_C, mods: .ctrl) == [0x03])
    #expect(try press(session, kVK_ANSI_D, mods: .ctrl) == [0x04])
    #expect(try press(session, kVK_UpArrow) == [UInt8](("\u{1b}[A").utf8))
    #expect(try press(session, kVK_DownArrow) == [UInt8](("\u{1b}[B").utf8))
}

/// A table that put two codes under one key would still answer for every code,
/// so what says the keys are told apart is that no two of them say the same
/// thing.
///
/// These are the keys with no character of their own, which is where such a
/// slip hides: a letter under the wrong letter is a typo anyone sees, and F7
/// under F8 is not.
@Test func theKeysWithoutCharactersAreToldApart() throws {
    let session = try typist()
    let codes = [
        kVK_F1, kVK_F2, kVK_F3, kVK_F4, kVK_F5, kVK_F6, kVK_F7, kVK_F8, kVK_F9, kVK_F10,
        kVK_F11, kVK_F12, kVK_Home, kVK_End, kVK_PageUp, kVK_PageDown, kVK_ForwardDelete,
        kVK_LeftArrow, kVK_RightArrow, kVK_UpArrow, kVK_DownArrow,
    ]

    var byWhatTheySay: [[UInt8]: Int] = [:]
    for code in codes {
        let bytes = try press(session, code)
        #expect(!bytes.isEmpty, "key code \(code) came to nothing")
        #expect(
            byWhatTheySay.updateValue(code, forKey: bytes) == nil,
            "key code \(code) came to what another key already had"
        )
    }
}

/// What AppKit puts on a key event is not always text.
///
/// `⌃A` carries the control character the core derives from the key and the
/// modifier anyway, and an arrow carries a private use codepoint that names
/// the key a second time. Either one passed on is one the child hears twice.
@Test func whatIsNotTextDoesNotTravelAsText() throws {
    let session = try typist()

    #expect(try press(session, kVK_ANSI_A, mods: .ctrl, text: "\u{01}") == [0x01])
    #expect(try press(session, kVK_UpArrow, text: "\u{f700}") == [UInt8](("\u{1b}[A").utf8))
}

/// A key held down is the platform repeating it, and a repeat types what the
/// press did. The release beside it is what types nothing.
@Test func aHeldKeyRepeatsWhatItTyped() throws {
    let session = try typist()

    #expect(try press(session, kVK_ANSI_A, text: "a", action: .repeat) == [0x61])
    #expect(try press(session, kVK_ANSI_A, text: "a", action: .release) == [])
}
