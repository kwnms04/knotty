//! Turning a key event into the bytes the terminal's modes make of it.
//!
//! The engine owns the encoding and every branch inside it. What this adds is
//! the two handles it wants held between calls and the one table that says
//! which of its keys ours is. cf. `docs/adr/0017-semantic-input-events.md`

use std::ptr;

use libghostty_vt_sys as ffi;

use crate::key::{Key, KeyAction, KeyEvent};
use crate::{Error, Result};

use super::check;

/// The engine's key encoder and the event handle it reads a key out of.
///
/// Both are kept rather than made per key: the encoder is where the options a
/// key is read against live, and the event is documented as reusable. Neither
/// is thread-safe, which is why they sit inside the terminal that is already
/// pinned to one thread.
pub(super) struct Keys {
    encoder: ffi::KeyEncoder,
    event: ffi::KeyEvent,
}

impl Keys {
    pub(super) fn new() -> Result<Self> {
        let mut keys = Self {
            encoder: ptr::null_mut(),
            event: ptr::null_mut(),
        };
        // SAFETY: a null allocator asks for the engine's own, and each out
        // parameter is handle-sized. Built into a value that already owns its
        // own `Drop`, so a failure on the second releases the first.
        check(unsafe { ffi::ghostty_key_encoder_new(ptr::null(), &raw mut keys.encoder) })?;
        check(unsafe { ffi::ghostty_key_event_new(ptr::null(), &raw mut keys.event) })?;
        Ok(keys)
    }

    /// Encode `event` as the modes `terminal` holds right now have it.
    ///
    /// The options are read off the terminal for every key rather than once,
    /// because what they answer is what the last feed left: the same arrow is
    /// `ESC [ A` or `ESC O A` by whether the child has asked for cursor key
    /// application mode since the last one was pressed.
    ///
    /// macOS Option-as-Meta is the one option a terminal cannot answer for,
    /// and this call resets it. It stays at the engine's default until there
    /// is a config pipeline to say otherwise. cf. `05-swift-app.md` § 7
    pub(super) fn encode(&mut self, terminal: ffi::Terminal, event: &KeyEvent) -> Result<Vec<u8>> {
        // SAFETY: both handles are live, and the call reads the terminal
        // without keeping it.
        unsafe { ffi::ghostty_key_encoder_setopt_from_terminal(self.encoder, terminal) };

        // Null rather than the address an empty `Vec` carries: the setter
        // documents null for no text, and an array of no length is not a
        // slice on either side of the boundary.
        let text = if event.text.is_empty() {
            ptr::null()
        } else {
            event.text.as_ptr()
        };

        // SAFETY: the event is live and each value is the setter's own type.
        // The text is borrowed rather than copied, so it is cleared below —
        // before the borrow it leaves behind could outlive `event`.
        unsafe {
            ffi::ghostty_key_event_set_action(self.event, action(event.action));
            ffi::ghostty_key_event_set_key(self.event, key(event.key));
            ffi::ghostty_key_event_set_mods(self.event, event.mods);
            ffi::ghostty_key_event_set_consumed_mods(self.event, event.consumed_mods);
            ffi::ghostty_key_event_set_composing(self.event, event.composing);
            ffi::ghostty_key_event_set_utf8(self.event, text.cast(), event.text.len());
        }

        let encoded = self.encoded();

        // SAFETY: as above. Null is what the setter documents for no text.
        unsafe { ffi::ghostty_key_event_set_utf8(self.event, ptr::null(), 0) };
        encoded
    }

    /// Ask how much room the sequence needs, then take it.
    ///
    /// Two calls rather than a buffer picked here: a key that encodes to
    /// nothing answers zero, and everything else answers its own length. So
    /// no length in this file is a guess, and the kitty protocol growing a
    /// longer sequence than a legacy one cannot truncate anything.
    fn encoded(&self) -> Result<Vec<u8>> {
        let mut needed = 0;
        // SAFETY: both handles are live. A null buffer is how the call
        // documents the question, and it answers it in `needed`.
        let asked = unsafe {
            ffi::ghostty_key_encoder_encode(
                self.encoder,
                self.event,
                ptr::null_mut(),
                0,
                &raw mut needed,
            )
        };
        // Out of space is the answer to the question, not a failure: nothing
        // was written because nowhere was offered.
        if asked != ffi::Result::SUCCESS && asked != ffi::Result::OUT_OF_SPACE {
            return Err(Error::Engine);
        }

        let mut encoded = vec![0; needed];
        if needed == 0 {
            return Ok(encoded);
        }

        let mut written = 0;
        // SAFETY: as above, with a buffer of the length just asked for.
        check(unsafe {
            ffi::ghostty_key_encoder_encode(
                self.encoder,
                self.event,
                encoded.as_mut_ptr().cast(),
                encoded.len(),
                &raw mut written,
            )
        })?;
        encoded.truncate(written);
        Ok(encoded)
    }
}

impl Drop for Keys {
    fn drop(&mut self) {
        // SAFETY: each handle was made by its matching constructor and is
        // freed once. Both calls document taking null, which a part-built
        // value leaves behind.
        unsafe {
            ffi::ghostty_key_event_free(self.event);
            ffi::ghostty_key_encoder_free(self.encoder);
        }
    }
}

fn action(action: KeyAction) -> ffi::KeyAction::Type {
    match action {
        KeyAction::Release => ffi::KeyAction::RELEASE,
        KeyAction::Press => ffi::KeyAction::PRESS,
        KeyAction::Repeat => ffi::KeyAction::REPEAT,
    }
}

/// Which of the engine's keys ours is.
///
/// Written out rather than cast, though the two lists agree value for value
/// today: what they agree on is an engine detail, and a version that inserts
/// a key would silently shift every key after it. The match is what makes
/// that a compile error instead.
fn key(key: Key) -> ffi::Key::Type {
    match key {
        Key::Unidentified => ffi::Key::UNIDENTIFIED,

        Key::Backquote => ffi::Key::BACKQUOTE,
        Key::Backslash => ffi::Key::BACKSLASH,
        Key::BracketLeft => ffi::Key::BRACKET_LEFT,
        Key::BracketRight => ffi::Key::BRACKET_RIGHT,
        Key::Comma => ffi::Key::COMMA,
        Key::Digit0 => ffi::Key::DIGIT_0,
        Key::Digit1 => ffi::Key::DIGIT_1,
        Key::Digit2 => ffi::Key::DIGIT_2,
        Key::Digit3 => ffi::Key::DIGIT_3,
        Key::Digit4 => ffi::Key::DIGIT_4,
        Key::Digit5 => ffi::Key::DIGIT_5,
        Key::Digit6 => ffi::Key::DIGIT_6,
        Key::Digit7 => ffi::Key::DIGIT_7,
        Key::Digit8 => ffi::Key::DIGIT_8,
        Key::Digit9 => ffi::Key::DIGIT_9,
        Key::Equal => ffi::Key::EQUAL,
        Key::IntlBackslash => ffi::Key::INTL_BACKSLASH,
        Key::IntlRo => ffi::Key::INTL_RO,
        Key::IntlYen => ffi::Key::INTL_YEN,
        Key::A => ffi::Key::A,
        Key::B => ffi::Key::B,
        Key::C => ffi::Key::C,
        Key::D => ffi::Key::D,
        Key::E => ffi::Key::E,
        Key::F => ffi::Key::F,
        Key::G => ffi::Key::G,
        Key::H => ffi::Key::H,
        Key::I => ffi::Key::I,
        Key::J => ffi::Key::J,
        Key::K => ffi::Key::K,
        Key::L => ffi::Key::L,
        Key::M => ffi::Key::M,
        Key::N => ffi::Key::N,
        Key::O => ffi::Key::O,
        Key::P => ffi::Key::P,
        Key::Q => ffi::Key::Q,
        Key::R => ffi::Key::R,
        Key::S => ffi::Key::S,
        Key::T => ffi::Key::T,
        Key::U => ffi::Key::U,
        Key::V => ffi::Key::V,
        Key::W => ffi::Key::W,
        Key::X => ffi::Key::X,
        Key::Y => ffi::Key::Y,
        Key::Z => ffi::Key::Z,
        Key::Minus => ffi::Key::MINUS,
        Key::Period => ffi::Key::PERIOD,
        Key::Quote => ffi::Key::QUOTE,
        Key::Semicolon => ffi::Key::SEMICOLON,
        Key::Slash => ffi::Key::SLASH,

        Key::AltLeft => ffi::Key::ALT_LEFT,
        Key::AltRight => ffi::Key::ALT_RIGHT,
        Key::Backspace => ffi::Key::BACKSPACE,
        Key::CapsLock => ffi::Key::CAPS_LOCK,
        Key::ContextMenu => ffi::Key::CONTEXT_MENU,
        Key::ControlLeft => ffi::Key::CONTROL_LEFT,
        Key::ControlRight => ffi::Key::CONTROL_RIGHT,
        Key::Enter => ffi::Key::ENTER,
        Key::MetaLeft => ffi::Key::META_LEFT,
        Key::MetaRight => ffi::Key::META_RIGHT,
        Key::ShiftLeft => ffi::Key::SHIFT_LEFT,
        Key::ShiftRight => ffi::Key::SHIFT_RIGHT,
        Key::Space => ffi::Key::SPACE,
        Key::Tab => ffi::Key::TAB,
        Key::Convert => ffi::Key::CONVERT,
        Key::KanaMode => ffi::Key::KANA_MODE,
        Key::NonConvert => ffi::Key::NON_CONVERT,

        Key::Delete => ffi::Key::DELETE,
        Key::End => ffi::Key::END,
        Key::Help => ffi::Key::HELP,
        Key::Home => ffi::Key::HOME,
        Key::Insert => ffi::Key::INSERT,
        Key::PageDown => ffi::Key::PAGE_DOWN,
        Key::PageUp => ffi::Key::PAGE_UP,

        Key::ArrowDown => ffi::Key::ARROW_DOWN,
        Key::ArrowLeft => ffi::Key::ARROW_LEFT,
        Key::ArrowRight => ffi::Key::ARROW_RIGHT,
        Key::ArrowUp => ffi::Key::ARROW_UP,

        Key::NumLock => ffi::Key::NUM_LOCK,
        Key::Numpad0 => ffi::Key::NUMPAD_0,
        Key::Numpad1 => ffi::Key::NUMPAD_1,
        Key::Numpad2 => ffi::Key::NUMPAD_2,
        Key::Numpad3 => ffi::Key::NUMPAD_3,
        Key::Numpad4 => ffi::Key::NUMPAD_4,
        Key::Numpad5 => ffi::Key::NUMPAD_5,
        Key::Numpad6 => ffi::Key::NUMPAD_6,
        Key::Numpad7 => ffi::Key::NUMPAD_7,
        Key::Numpad8 => ffi::Key::NUMPAD_8,
        Key::Numpad9 => ffi::Key::NUMPAD_9,
        Key::NumpadAdd => ffi::Key::NUMPAD_ADD,
        Key::NumpadBackspace => ffi::Key::NUMPAD_BACKSPACE,
        Key::NumpadClear => ffi::Key::NUMPAD_CLEAR,
        Key::NumpadClearEntry => ffi::Key::NUMPAD_CLEAR_ENTRY,
        Key::NumpadComma => ffi::Key::NUMPAD_COMMA,
        Key::NumpadDecimal => ffi::Key::NUMPAD_DECIMAL,
        Key::NumpadDivide => ffi::Key::NUMPAD_DIVIDE,
        Key::NumpadEnter => ffi::Key::NUMPAD_ENTER,
        Key::NumpadEqual => ffi::Key::NUMPAD_EQUAL,
        Key::NumpadMemoryAdd => ffi::Key::NUMPAD_MEMORY_ADD,
        Key::NumpadMemoryClear => ffi::Key::NUMPAD_MEMORY_CLEAR,
        Key::NumpadMemoryRecall => ffi::Key::NUMPAD_MEMORY_RECALL,
        Key::NumpadMemoryStore => ffi::Key::NUMPAD_MEMORY_STORE,
        Key::NumpadMemorySubtract => ffi::Key::NUMPAD_MEMORY_SUBTRACT,
        Key::NumpadMultiply => ffi::Key::NUMPAD_MULTIPLY,
        Key::NumpadParenLeft => ffi::Key::NUMPAD_PAREN_LEFT,
        Key::NumpadParenRight => ffi::Key::NUMPAD_PAREN_RIGHT,
        Key::NumpadSubtract => ffi::Key::NUMPAD_SUBTRACT,
        Key::NumpadSeparator => ffi::Key::NUMPAD_SEPARATOR,
        Key::NumpadUp => ffi::Key::NUMPAD_UP,
        Key::NumpadDown => ffi::Key::NUMPAD_DOWN,
        Key::NumpadRight => ffi::Key::NUMPAD_RIGHT,
        Key::NumpadLeft => ffi::Key::NUMPAD_LEFT,
        Key::NumpadBegin => ffi::Key::NUMPAD_BEGIN,
        Key::NumpadHome => ffi::Key::NUMPAD_HOME,
        Key::NumpadEnd => ffi::Key::NUMPAD_END,
        Key::NumpadInsert => ffi::Key::NUMPAD_INSERT,
        Key::NumpadDelete => ffi::Key::NUMPAD_DELETE,
        Key::NumpadPageUp => ffi::Key::NUMPAD_PAGE_UP,
        Key::NumpadPageDown => ffi::Key::NUMPAD_PAGE_DOWN,

        Key::Escape => ffi::Key::ESCAPE,
        Key::F1 => ffi::Key::F1,
        Key::F2 => ffi::Key::F2,
        Key::F3 => ffi::Key::F3,
        Key::F4 => ffi::Key::F4,
        Key::F5 => ffi::Key::F5,
        Key::F6 => ffi::Key::F6,
        Key::F7 => ffi::Key::F7,
        Key::F8 => ffi::Key::F8,
        Key::F9 => ffi::Key::F9,
        Key::F10 => ffi::Key::F10,
        Key::F11 => ffi::Key::F11,
        Key::F12 => ffi::Key::F12,
        Key::F13 => ffi::Key::F13,
        Key::F14 => ffi::Key::F14,
        Key::F15 => ffi::Key::F15,
        Key::F16 => ffi::Key::F16,
        Key::F17 => ffi::Key::F17,
        Key::F18 => ffi::Key::F18,
        Key::F19 => ffi::Key::F19,
        Key::F20 => ffi::Key::F20,
        Key::F21 => ffi::Key::F21,
        Key::F22 => ffi::Key::F22,
        Key::F23 => ffi::Key::F23,
        Key::F24 => ffi::Key::F24,
        Key::F25 => ffi::Key::F25,
        Key::Fn => ffi::Key::FN,
        Key::FnLock => ffi::Key::FN_LOCK,
        Key::PrintScreen => ffi::Key::PRINT_SCREEN,
        Key::ScrollLock => ffi::Key::SCROLL_LOCK,
        Key::Pause => ffi::Key::PAUSE,
    }
}
