/* Generated from knotty-ffi by cbindgen. Do not edit by hand. */

#ifndef KNOTTY_H
#define KNOTTY_H

#include <stdint.h>
#include <stddef.h>
#include <stdbool.h>

/**
 * ABI version of this library.
 *
 * A caller reads the constant from the header it compiled against and
 * compares it with [`kt_abi_version`]. Mismatch means header and library
 * disagree about layouts, and the caller must not proceed.
 */
#define KT_ABI_VERSION 8

/**
 * Outcome of a call across the boundary.
 */
enum KtStatus
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : int32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * The call succeeded.
   */
  KT_STATUS_OK = 0,
  /**
   * Nothing was available. Not a failure.
   */
  KT_STATUS_NO_VALUE = 1,
  /**
   * A required pointer argument was null.
   */
  KT_STATUS_NULL_ARGUMENT = 2,
  /**
   * The VT engine rejected the operation.
   */
  KT_STATUS_ENGINE = 3,
  /**
   * The terminal's state is bigger than a snapshot can describe.
   */
  KT_STATUS_TOO_LARGE = 4,
  /**
   * A coordinate fell outside the terminal.
   */
  KT_STATUS_OUT_OF_RANGE = 5,
  /**
   * Something inside the core panicked. The call did nothing useful and
   * the session it was made on is now defunct.
   */
  KT_STATUS_PANICKED = 6,
  /**
   * The session already panicked. It keeps its last good snapshot but
   * takes no more input.
   */
  KT_STATUS_DEFUNCT = 7,
  /**
   * The call is only for a session with no PTY behind it. One with a PTY
   * has its own thread doing what the call would have done.
   */
  KT_STATUS_NOT_DETACHED = 8,
  /**
   * The queue of bytes bound for the child is at its cap, and what did not
   * fit was dropped. Reported once per overrun, so a later call succeeding
   * does not mean the dropped bytes came back.
   */
  KT_STATUS_WRITE_QUEUE_FULL = 9,
  /**
   * An operating system call failed — opening a terminal, starting a child,
   * or talking to one already started.
   */
  KT_STATUS_IO = 10,
  /**
   * A key event named no key. Nothing was queued for the child, and the
   * caller has a mapping to fill in rather than a key that has no bytes:
   * keys that encode to nothing are answered with `KT_STATUS_OK`.
   */
  KT_STATUS_UNIDENTIFIED_KEY = 11,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtStatus KtStatus;
#else
typedef int32_t KtStatus;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Which way a key moved.
 */
enum KtKeyAction
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * The key came back up.
   */
  KT_KEY_ACTION_RELEASE = 0,
  /**
   * The key went down.
   */
  KT_KEY_ACTION_PRESS = 1,
  /**
   * The key is held down and the platform is repeating it.
   */
  KT_KEY_ACTION_REPEAT = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtKeyAction KtKeyAction;
#else
typedef uint8_t KtKeyAction;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * A physical key, named as the W3C `KeyboardEvent.code` standard names it.
 *
 * Layout-independent by construction: the value says where on the keyboard
 * the key is, not what pressing it typed. The sections below are the
 * standard's own, and the media section (§ 3.6) is left out — macOS hands an
 * application no `keyDown` for those keys, so nothing could ever name one.
 * cf. <https://www.w3.org/TR/uievents-code>
 */
enum KtKey
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint32_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * No key this list names. A platform key that maps to nothing here
   * arrives as this, which is a missing mapping rather than a key without
   * a name.
   */
  KT_KEY_UNIDENTIFIED = 0,
  KT_KEY_BACKQUOTE,
  KT_KEY_BACKSLASH,
  KT_KEY_BRACKET_LEFT,
  KT_KEY_BRACKET_RIGHT,
  KT_KEY_COMMA,
  KT_KEY_DIGIT0,
  KT_KEY_DIGIT1,
  KT_KEY_DIGIT2,
  KT_KEY_DIGIT3,
  KT_KEY_DIGIT4,
  KT_KEY_DIGIT5,
  KT_KEY_DIGIT6,
  KT_KEY_DIGIT7,
  KT_KEY_DIGIT8,
  KT_KEY_DIGIT9,
  KT_KEY_EQUAL,
  KT_KEY_INTL_BACKSLASH,
  KT_KEY_INTL_RO,
  KT_KEY_INTL_YEN,
  KT_KEY_A,
  KT_KEY_B,
  KT_KEY_C,
  KT_KEY_D,
  KT_KEY_E,
  KT_KEY_F,
  KT_KEY_G,
  KT_KEY_H,
  KT_KEY_I,
  KT_KEY_J,
  KT_KEY_K,
  KT_KEY_L,
  KT_KEY_M,
  KT_KEY_N,
  KT_KEY_O,
  KT_KEY_P,
  KT_KEY_Q,
  KT_KEY_R,
  KT_KEY_S,
  KT_KEY_T,
  KT_KEY_U,
  KT_KEY_V,
  KT_KEY_W,
  KT_KEY_X,
  KT_KEY_Y,
  KT_KEY_Z,
  KT_KEY_MINUS,
  KT_KEY_PERIOD,
  KT_KEY_QUOTE,
  KT_KEY_SEMICOLON,
  KT_KEY_SLASH,
  KT_KEY_ALT_LEFT,
  KT_KEY_ALT_RIGHT,
  KT_KEY_BACKSPACE,
  KT_KEY_CAPS_LOCK,
  KT_KEY_CONTEXT_MENU,
  KT_KEY_CONTROL_LEFT,
  KT_KEY_CONTROL_RIGHT,
  KT_KEY_ENTER,
  KT_KEY_META_LEFT,
  KT_KEY_META_RIGHT,
  KT_KEY_SHIFT_LEFT,
  KT_KEY_SHIFT_RIGHT,
  KT_KEY_SPACE,
  KT_KEY_TAB,
  KT_KEY_CONVERT,
  KT_KEY_KANA_MODE,
  KT_KEY_NON_CONVERT,
  KT_KEY_DELETE,
  KT_KEY_END,
  KT_KEY_HELP,
  KT_KEY_HOME,
  KT_KEY_INSERT,
  KT_KEY_PAGE_DOWN,
  KT_KEY_PAGE_UP,
  KT_KEY_ARROW_DOWN,
  KT_KEY_ARROW_LEFT,
  KT_KEY_ARROW_RIGHT,
  KT_KEY_ARROW_UP,
  KT_KEY_NUM_LOCK,
  KT_KEY_NUMPAD0,
  KT_KEY_NUMPAD1,
  KT_KEY_NUMPAD2,
  KT_KEY_NUMPAD3,
  KT_KEY_NUMPAD4,
  KT_KEY_NUMPAD5,
  KT_KEY_NUMPAD6,
  KT_KEY_NUMPAD7,
  KT_KEY_NUMPAD8,
  KT_KEY_NUMPAD9,
  KT_KEY_NUMPAD_ADD,
  KT_KEY_NUMPAD_BACKSPACE,
  KT_KEY_NUMPAD_CLEAR,
  KT_KEY_NUMPAD_CLEAR_ENTRY,
  KT_KEY_NUMPAD_COMMA,
  KT_KEY_NUMPAD_DECIMAL,
  KT_KEY_NUMPAD_DIVIDE,
  KT_KEY_NUMPAD_ENTER,
  KT_KEY_NUMPAD_EQUAL,
  KT_KEY_NUMPAD_MEMORY_ADD,
  KT_KEY_NUMPAD_MEMORY_CLEAR,
  KT_KEY_NUMPAD_MEMORY_RECALL,
  KT_KEY_NUMPAD_MEMORY_STORE,
  KT_KEY_NUMPAD_MEMORY_SUBTRACT,
  KT_KEY_NUMPAD_MULTIPLY,
  KT_KEY_NUMPAD_PAREN_LEFT,
  KT_KEY_NUMPAD_PAREN_RIGHT,
  KT_KEY_NUMPAD_SUBTRACT,
  KT_KEY_NUMPAD_SEPARATOR,
  KT_KEY_NUMPAD_UP,
  KT_KEY_NUMPAD_DOWN,
  KT_KEY_NUMPAD_RIGHT,
  KT_KEY_NUMPAD_LEFT,
  KT_KEY_NUMPAD_BEGIN,
  KT_KEY_NUMPAD_HOME,
  KT_KEY_NUMPAD_END,
  KT_KEY_NUMPAD_INSERT,
  KT_KEY_NUMPAD_DELETE,
  KT_KEY_NUMPAD_PAGE_UP,
  KT_KEY_NUMPAD_PAGE_DOWN,
  KT_KEY_ESCAPE,
  KT_KEY_F1,
  KT_KEY_F2,
  KT_KEY_F3,
  KT_KEY_F4,
  KT_KEY_F5,
  KT_KEY_F6,
  KT_KEY_F7,
  KT_KEY_F8,
  KT_KEY_F9,
  KT_KEY_F10,
  KT_KEY_F11,
  KT_KEY_F12,
  KT_KEY_F13,
  KT_KEY_F14,
  KT_KEY_F15,
  KT_KEY_F16,
  KT_KEY_F17,
  KT_KEY_F18,
  KT_KEY_F19,
  KT_KEY_F20,
  KT_KEY_F21,
  KT_KEY_F22,
  KT_KEY_F23,
  KT_KEY_F24,
  KT_KEY_F25,
  KT_KEY_FN,
  KT_KEY_FN_LOCK,
  KT_KEY_PRINT_SCREEN,
  KT_KEY_SCROLL_LOCK,
  KT_KEY_PAUSE,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtKey KtKey;
#else
typedef uint32_t KtKey;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Which kind of event a [`KtEvent`] is, and so which of its fields carry
 * anything.
 */
enum KtEventKind
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * The child rang the bell.
   */
  KT_EVENT_KIND_BELL = 0,
  /**
   * The child asked for text to be put on a clipboard.
   */
  KT_EVENT_KIND_CLIPBOARD_WRITE = 1,
  /**
   * The child is gone.
   */
  KT_EVENT_KIND_CHILD_EXITED = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtEventKind KtEventKind;
#else
typedef uint8_t KtEventKind;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Which clipboard a write is bound for.
 *
 * The engine normalizes each protocol's own selectors onto these three
 * before a write reaches us, so they are all a write can name.
 */
enum KtClipboardTarget
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * The system clipboard.
   */
  KT_CLIPBOARD_TARGET_STANDARD = 0,
  /**
   * The selection clipboard.
   */
  KT_CLIPBOARD_TARGET_SELECTION = 1,
  /**
   * The primary selection.
   */
  KT_CLIPBOARD_TARGET_PRIMARY = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtClipboardTarget KtClipboardTarget;
#else
typedef uint8_t KtClipboardTarget;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * How much of the screen changed since the last snapshot was taken.
 *
 * The variants are ordered by how much they cover, so the larger of two is
 * the one that describes both.
 */
enum KtDirty
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * No row changed. A published snapshot can still say this: something
   * outside the grid, such as the title or the cursor, moved instead.
   */
  KT_DIRTY_CLEAN = 0,
  /**
   * Some rows changed; the row flags say which.
   */
  KT_DIRTY_PARTIAL = 1,
  /**
   * Everything changed, as on a switch to or from the alternate screen.
   */
  KT_DIRTY_FULL = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtDirty KtDirty;
#else
typedef uint8_t KtDirty;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * How a cell is underlined.
 */
enum KtUnderline
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * Not underlined.
   */
  KT_UNDERLINE_NONE = 0,
  /**
   * SGR 4.
   */
  KT_UNDERLINE_SINGLE = 1,
  /**
   * SGR 21.
   */
  KT_UNDERLINE_DOUBLE = 2,
  /**
   * SGR 4:3.
   */
  KT_UNDERLINE_CURLY = 3,
  /**
   * SGR 4:4.
   */
  KT_UNDERLINE_DOTTED = 4,
  /**
   * SGR 4:5.
   */
  KT_UNDERLINE_DASHED = 5,
  /**
   * Underlined in a way this version of the engine knows and knotty does
   * not. Still an underline, but its kind cannot be named.
   */
  KT_UNDERLINE_UNKNOWN = 255,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtUnderline KtUnderline;
#else
typedef uint8_t KtUnderline;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * What the cursor looks like.
 */
enum KtCursorShape
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * A filled block over the cell.
   */
  KT_CURSOR_SHAPE_BLOCK = 0,
  /**
   * A vertical bar before the cell.
   */
  KT_CURSOR_SHAPE_BAR = 1,
  /**
   * A line under the cell.
   */
  KT_CURSOR_SHAPE_UNDERLINE = 2,
  /**
   * An outlined block, drawn when the terminal is not focused.
   */
  KT_CURSOR_SHAPE_BLOCK_HOLLOW = 3,
  /**
   * A shape this version of the engine knows and knotty does not.
   */
  KT_CURSOR_SHAPE_UNKNOWN = 255,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtCursorShape KtCursorShape;
#else
typedef uint8_t KtCursorShape;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Whether a session has a child and what has become of it.
 *
 * Read apart from [`KtSessionState`]: the two are different facts, and a
 * session whose thread panicked with its child still running is a real
 * pairing. What decides whether closing the window needs a warning is this
 * one; what decides whether the window still takes input is the other.
 */
enum KtChildState
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * There is no child. A session with no PTY behind it is fed by its caller
   * and has none.
   */
  KT_CHILD_STATE_NONE = 0,
  /**
   * The child is still running.
   */
  KT_CHILD_STATE_RUNNING = 1,
  /**
   * The child is gone, and `child_exit_code` says what by.
   */
  KT_CHILD_STATE_EXITED = 2,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtChildState KtChildState;
#else
typedef uint8_t KtChildState;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Whether a session still works.
 */
enum KtSessionState
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * Working.
   */
  KT_SESSION_STATE_OK = 0,
  /**
   * Something inside it panicked. It keeps the last screen it published and
   * takes no more input, which comes back as `KT_STATUS_DEFUNCT`.
   */
  KT_SESSION_STATE_BROKEN = 1,
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtSessionState KtSessionState;
#else
typedef uint8_t KtSessionState;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Cell attributes, OR-ed together into a cell's `attributes` field.
 *
 * The low byte is SGR state, the high byte is structure. Underlining is in
 * neither: it has kinds rather than an on/off state, so it gets its own
 * field.
 */
enum KtAttribute
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint16_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * SGR 1.
   */
  KT_ATTRIBUTE_BOLD = (1 << 0),
  /**
   * SGR 2.
   */
  KT_ATTRIBUTE_FAINT = (1 << 1),
  /**
   * SGR 3.
   */
  KT_ATTRIBUTE_ITALIC = (1 << 2),
  /**
   * SGR 5.
   */
  KT_ATTRIBUTE_BLINK = (1 << 3),
  /**
   * SGR 7.
   */
  KT_ATTRIBUTE_INVERSE = (1 << 4),
  /**
   * SGR 8.
   */
  KT_ATTRIBUTE_INVISIBLE = (1 << 5),
  /**
   * SGR 9.
   */
  KT_ATTRIBUTE_STRIKETHROUGH = (1 << 6),
  /**
   * SGR 53.
   */
  KT_ATTRIBUTE_OVERLINE = (1 << 7),
  /**
   * The leading cell of a character two columns wide.
   */
  KT_ATTRIBUTE_WIDE = (1 << 8),
  /**
   * The trailing cell of a character two columns wide. It holds no text of
   * its own; the leading cell carries the whole character.
   */
  KT_ATTRIBUTE_WIDE_TAIL = (1 << 9),
  /**
   * The cell's `codepoint` is an index into the snapshot's grapheme table
   * rather than a codepoint.
   */
  KT_ATTRIBUTE_OVERFLOW = (1 << 10),
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtAttribute KtAttribute;
#else
typedef uint16_t KtAttribute;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Modifier state, OR-ed together into a key event's `mods` and
 * `consumed_mods` fields.
 *
 * A side bit says which of a pair is held and means nothing unless its
 * modifier's own bit is set. Not every platform can tell the two apart, and
 * nothing here needs one to.
 */
enum KtModifier
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint16_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * Shift.
   */
  KT_MODIFIER_SHIFT = (1 << 0),
  /**
   * Control.
   */
  KT_MODIFIER_CTRL = (1 << 1),
  /**
   * Alt, which is Option on macOS.
   */
  KT_MODIFIER_ALT = (1 << 2),
  /**
   * Super, which is Command on macOS.
   */
  KT_MODIFIER_SUPER = (1 << 3),
  /**
   * Caps lock is on.
   */
  KT_MODIFIER_CAPS_LOCK = (1 << 4),
  /**
   * Num lock is on.
   */
  KT_MODIFIER_NUM_LOCK = (1 << 5),
  /**
   * The shift held is the right-hand one.
   */
  KT_MODIFIER_SHIFT_RIGHT = (1 << 6),
  /**
   * The control held is the right-hand one.
   */
  KT_MODIFIER_CTRL_RIGHT = (1 << 7),
  /**
   * The alt held is the right-hand one.
   */
  KT_MODIFIER_ALT_RIGHT = (1 << 8),
  /**
   * The super held is the right-hand one.
   */
  KT_MODIFIER_SUPER_RIGHT = (1 << 9),
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtModifier KtModifier;
#else
typedef uint16_t KtModifier;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Row state, OR-ed together into a row's `flags` field.
 */
enum KtRowFlag
#if defined(__cplusplus) || __STDC_VERSION__ >= 202311L
  : uint8_t
#endif // defined(__cplusplus) || __STDC_VERSION__ >= 202311L
 {
  /**
   * The row changed since the last snapshot.
   */
  KT_ROW_FLAG_DIRTY = (1 << 0),
  /**
   * The row runs on into the next one. It ended because it ran out of
   * columns, not at a newline.
   */
  KT_ROW_FLAG_WRAPPED = (1 << 1),
  /**
   * Part of the row is selected, and the row's columns say which part.
   */
  KT_ROW_FLAG_SELECTED = (1 << 2),
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtRowFlag KtRowFlag;
#else
typedef uint8_t KtRowFlag;
#endif // __STDC_VERSION__ >= 202311L
#endif // __cplusplus

/**
 * Opaque handle to a session.
 */
typedef struct KtSession KtSession;

/**
 * Opaque handle to a snapshot.
 *
 * The screen is what the session published. The two states beside it are what
 * the session said of itself when the snapshot was taken, and they travel with
 * it so that a consumer draws one consistent answer rather than asking a
 * session that has moved on since.
 */
typedef struct KtSnapshot KtSnapshot;

/**
 * Borrowed UTF-8, valid for as long as whatever lent it stays put.
 *
 * Not null-terminated: read `len` bytes. What has been taken out of the text
 * is the lending field's to say: a snapshot's title and working directory
 * have had their control characters removed and so hold no interior nulls,
 * a clipboard payload is whatever the child asked to copy.
 */
typedef struct {
  /**
   * The bytes.
   */
  const uint8_t *bytes;
  /**
   * How many of them.
   */
  size_t len;
} KtText;

/**
 * A key event on its way in, before anything has decided what bytes it is.
 *
 * The physical key rather than the character: the same key is `A` on a US
 * layout and `Ф` on a Russian one, so `⌃A` is the same place on the keyboard
 * either way. What the layout made of it travels as `text` beside it.
 *
 * Which bytes it comes to is the core's to answer, because the modes it
 * depends on are the terminal's and reading them out here would read them as
 * of some earlier frame. cf. `docs/adr/0017-semantic-input-events.md`
 */
typedef struct {
  /**
   * Which way the key moved. Only a press or a repeat encodes anything.
   */
  KtKeyAction action;
  /**
   * Which key it was. `KT_KEY_UNIDENTIFIED` is refused rather than
   * encoded.
   */
  KtKey key;
  /**
   * What was held down, as `KtModifier` bits.
   */
  uint16_t mods;
  /**
   * Which of those the layout already spent on `text`, as `KtModifier`
   * bits. Option making `å` out of `⌥A` on macOS is one: the modifier was
   * held, but it is not one the terminal should encode a second time.
   */
  uint16_t consumed_mods;
  /**
   * Whether an input method is mid-composition. Keys are held back while
   * it is, which is what keeps half a syllable out of the child.
   */
  bool composing;
  /**
   * What the layout made of the key, as UTF-8, empty where it made
   * nothing. Borrowed for the length of the call.
   *
   * Neither control characters nor a platform's own function key codes
   * belong here — C0 and DEL, and on macOS the private use area
   * `U+F700`–`U+F8FF` that AppKit puts in `NSEvent.characters` for the
   * arrows and the F keys. The core derives all of those from the key and
   * the modifiers, and one arriving as text is one that would be encoded
   * twice. Leave it empty for them.
   */
  KtText text;
} KtKeyEvent;

/**
 * What a session calls when it has something new to be taken.
 *
 * `userdata` comes back exactly as it was handed to [`kt_session_set_wake`].
 *
 * The call is made on the thread that drove the session, from inside the call
 * that published — the caller's own thread for a detached session, the
 * session's I/O thread for one with a PTY behind it. **It may do nothing but
 * wake its own thread**: a call back across this boundary re-enters a session
 * the running call still holds.
 */
typedef void (*KtWake)(void *userdata);

/**
 * A selection's two endpoints, in viewport coordinates.
 *
 * Both ends are inclusive, and either may come first: the pair records which
 * way the selection was made, not which end is topmost.
 */
typedef struct {
  /**
   * Column of the first endpoint.
   */
  uint16_t start_x;
  /**
   * Row of the first endpoint.
   */
  uint16_t start_y;
  /**
   * Column of the second endpoint.
   */
  uint16_t end_x;
  /**
   * Row of the second endpoint.
   */
  uint16_t end_y;
  /**
   * Whether the endpoints are opposite corners of a block rather than the
   * ends of a run of text.
   */
  bool rectangle;
} KtSelectionRange;

/**
 * Borrowed bytes, valid until the call that lent them is made again.
 *
 * Not a string: these are whatever the terminal put on the wire, and nothing
 * promises they are text. Read `len` bytes.
 */
typedef struct {
  /**
   * The bytes.
   */
  const uint8_t *bytes;
  /**
   * How many of them.
   */
  size_t len;
} KtBytes;

/**
 * One thing that happened, whose happening is the whole of its meaning.
 */
typedef struct {
  /**
   * Which kind of event this is.
   */
  KtEventKind kind;
  /**
   * Which clipboard the text is bound for. Set only for a clipboard write.
   */
  KtClipboardTarget clipboard_target;
  /**
   * What to put on that clipboard, borrowed for as long as the run it came
   * in is. Empty for any other kind.
   *
   * Nothing has been taken out of it: it is what the child asked to copy,
   * control characters and all. Stripping those would eat the newlines out
   * of a copied paragraph, and untrusted bytes are made safe where they
   * re-enter — the paste path. cf. `docs/adr/0007-input-security.md`
   */
  KtText text;
  /**
   * What the child exited with, or 128 plus the signal that ended it — the
   * one number a shell reports either by. Set only for a child's exit, and
   * 0 for any other kind.
   */
  int32_t exit_code;
} KtEvent;

/**
 * Borrowed run of events, valid until the call that lent them is made again.
 */
typedef struct {
  /**
   * The events, oldest first.
   */
  const KtEvent *events;
  /**
   * How many of them.
   */
  size_t len;
  /**
   * How many events were dropped for want of room since the last take. A
   * dropped event never makes the screen wrong: everything that has to be
   * true is in the snapshot. The count empties with the queue, so one
   * overrun is reported once.
   */
  uint64_t dropped;
} KtEvents;

/**
 * A colour, already resolved out of the palette.
 */
typedef struct {
  /**
   * Red component.
   */
  uint8_t r;
  /**
   * Green component.
   */
  uint8_t g;
  /**
   * Blue component.
   */
  uint8_t b;
} KtRgb;

/**
 * One terminal cell.
 *
 * Fixed size and POD: the grid is a row-major flat array of these, so a
 * consumer indexes it without a function call per cell.
 */
typedef struct {
  /**
   * The cell's codepoint, or 0 when it holds no text. When the cell has
   * the overflow attribute this is an index into the snapshot's grapheme
   * table instead.
   */
  uint32_t codepoint;
  /**
   * Foreground colour, with the terminal's default already substituted.
   */
  KtRgb foreground;
  /**
   * Background colour, with the terminal's default already substituted.
   */
  KtRgb background;
  /**
   * A bit set of `KtAttribute` values.
   */
  uint16_t attributes;
  /**
   * Which underline the cell carries, if any.
   */
  KtUnderline underline;
} KtCell;

/**
 * What a snapshot says about one row.
 *
 * Selection lives here rather than in the cells. A renderer's line cache is
 * keyed on cell contents, so a selection inside a cell would throw the whole
 * cache away on every drag.
 */
typedef struct {
  /**
   * A bit set of `KtRowFlag` values.
   */
  uint8_t flags;
  /**
   * First selected column, inclusive. Only meaningful with the selected
   * flag set.
   */
  uint16_t selection_start;
  /**
   * Last selected column, inclusive. Only meaningful with the selected
   * flag set.
   */
  uint16_t selection_end;
} KtRow;

/**
 * Where the cursor is and how it looks.
 */
typedef struct {
  /**
   * Column, from the left of the viewport.
   */
  uint16_t x;
  /**
   * Row, from the top of the viewport.
   */
  uint16_t y;
  /**
   * Whether to draw it. False both when the terminal hid it and when it
   * sits outside the viewport, since neither is drawable.
   */
  bool visible;
  /**
   * Which shape to draw.
   */
  KtCursorShape shape;
} KtCursor;

/**
 * Borrowed view of a snapshot's contents.
 *
 * The pointers stay valid until the snapshot is freed.
 */
typedef struct {
  /**
   * Viewport width in cells.
   */
  uint16_t cols;
  /**
   * Viewport height in cells.
   */
  uint16_t rows;
  /**
   * How much of the grid changed since the last snapshot. Can be
   * `KT_DIRTY_CLEAN` when what changed was outside the grid.
   */
  KtDirty dirty;
  /**
   * Whether a selection exists. A selection scrolled out of the viewport
   * still exists, so this is not the same as no row being selected.
   */
  bool has_selection;
  /**
   * Row-major grid of `rows * cols` cells.
   */
  const KtCell *cells;
  /**
   * One entry per row: its flags and, where selected, its columns.
   */
  const KtRow *row_state;
  /**
   * Codepoints for cells whose cluster did not fit in one cell. A cell
   * carrying `KT_ATTRIBUTE_OVERFLOW` holds the index of its run's length
   * here; the codepoints follow, base first.
   */
  const uint32_t *graphemes;
  /**
   * Number of entries in `graphemes`, lengths included.
   */
  size_t grapheme_count;
  /**
   * Where the cursor is and how it looks.
   */
  KtCursor cursor;
  /**
   * Window title, control characters already removed.
   */
  KtText title;
  /**
   * Working directory as an absolute path, control characters already
   * removed.
   */
  KtText pwd;
  /**
   * Whether the session has a child and whether it is still running. This
   * is the truth about the child: the exit is an event as well, but events
   * can be dropped and this cannot.
   */
  KtChildState child_state;
  /**
   * Whether the session still works. A broken one keeps the screen it has
   * and refuses input.
   */
  KtSessionState session_state;
  /**
   * What the child exited with, or 128 plus the signal that ended it — the
   * one number a shell reports either by. Set only when `child_state` is
   * `KT_CHILD_STATE_EXITED`, and 0 otherwise.
   */
  int32_t child_exit_code;
} KtSnapshotView;

#ifdef __cplusplus
extern "C" {
#endif // __cplusplus

/**
 * Return the ABI version this library was built with.
 */
uint32_t kt_abi_version(void);

/**
 * Create a session with no PTY behind it.
 *
 * On success writes an owned handle to `out`, to be released with
 * [`kt_session_free`]. On failure `out` receives null.
 *
 * # Safety
 *
 * `out` must be a valid, writable pointer to a `KtSession *`.
 */
KtStatus kt_session_new_detached(uint16_t cols,
                                 uint16_t rows,
                                 size_t max_scrollback,
                                 KtSession **out);

/**
 * Create a session with a child process behind a pseudoterminal.
 *
 * `argv` is the command to run: `argv[0]` is the program, the rest its
 * arguments, and each is a run of bytes rather than a null-terminated string.
 * An empty `argv` names nothing to run and is reported as a missing argument.
 * The child starts knowing the size it was given here, so its first frame is
 * already the right shape.
 *
 * The session gets a thread of its own, which reads the terminal, feeds the
 * engine, publishes, and hands the child what [`kt_session_write`] queued. A
 * call that reaches past that thread to what it owns — [`kt_session_feed`],
 * [`kt_session_take_writes`] — is refused with `KT_STATUS_NOT_DETACHED`.
 *
 * On success writes an owned handle to `out`, to be released with
 * [`kt_session_free`]. On failure `out` receives null.
 *
 * # Safety
 *
 * `argv` must point at `argc` readable `KtText`s, each of which must point at
 * its own `len` readable bytes — null only where that length is 0. `out` must
 * be a valid, writable pointer to a `KtSession *`.
 */
KtStatus kt_session_new_pty(uint16_t cols,
                            uint16_t rows,
                            size_t max_scrollback,
                            const KtText *argv,
                            size_t argc,
                            KtSession **out);

/**
 * Release a session, stopping its I/O thread if it has one. Null is a no-op.
 *
 * # Safety
 *
 * `session` must come from [`kt_session_new_detached`] or
 * [`kt_session_new_pty`] and must not be used afterwards.
 */
void kt_session_free(KtSession *session);

/**
 * Feed `len` bytes to a detached session.
 *
 * Processes the whole buffer on the calling thread before returning, and
 * publishes at most one snapshot. A session with a PTY behind it takes its
 * input from that PTY, so this returns `KT_STATUS_NOT_DETACHED` for one.
 *
 * Returns `KT_STATUS_WRITE_QUEUE_FULL` when the terminal's answers to what
 * was fed did not fit in the writer queue. The snapshot is published either
 * way: what the child missed hearing does not make the frame wrong.
 *
 * # Safety
 *
 * `session` must be a live handle, and `bytes` must point at `len` readable
 * bytes. `bytes` may be null only when `len` is 0.
 */
KtStatus kt_session_feed(KtSession *session, const uint8_t *bytes, size_t len);

/**
 * Queue `len` bytes for the session's child.
 *
 * Returns as soon as they are queued rather than waiting on the child to
 * read them: a session with a PTY behind it hands them over on its own
 * thread, and a detached one has them collected by
 * [`kt_session_take_writes`] alongside what the terminal answered.
 *
 * Returns `KT_STATUS_WRITE_QUEUE_FULL` when they did not fit, in which case
 * none of them were queued — a prefix of what the user typed reaching the
 * child is worse than none of it.
 *
 * # Safety
 *
 * `session` must be a live handle, and `bytes` must point at `len` readable
 * bytes. `bytes` may be null only when `len` is 0.
 */
KtStatus kt_session_write(KtSession *session, const uint8_t *bytes, size_t len);

/**
 * Encode a key event and queue what it comes to for the session's child.
 *
 * The encoding is the core's, taken with the terminal's own modes in hand:
 * the same arrow key is `ESC [ A` at a prompt and `ESC O A` in an editor
 * that asked for cursor key application mode, and a caller never has to know
 * which. cf. `docs/adr/0017-semantic-input-events.md`
 *
 * A key that comes to nothing queues nothing and answers `KT_STATUS_OK` — a
 * bare modifier, a release, and every key at all while an input method is
 * composing. A key that names nothing answers
 * `KT_STATUS_UNIDENTIFIED_KEY` instead, so that a mapping missing from the
 * caller is heard about where it happens rather than found later in a key
 * that quietly does nothing.
 *
 * A detached session encodes on the calling thread, so it answers
 * `KT_STATUS_WRITE_QUEUE_FULL` when the bytes did not fit, as
 * [`kt_session_write`] does. A session with a PTY behind it encodes on its
 * own thread and is past answering by the time it finds out, the way
 * [`kt_session_set_selection`] is — and a queue that full on one of those is
 * the loop's own to shed, since the loop is the only thing that drains it.
 *
 * # Safety
 *
 * `session` must be a live handle, and `event` must point at a readable
 * `KtKeyEvent` whose text points at its own `len` readable bytes — null only
 * where that length is 0.
 */
KtStatus kt_session_key(KtSession *session, const KtKeyEvent *event);

/**
 * Register what a session calls when it has something new to be taken, or
 * clear it by passing null.
 *
 * Called once per publication that left something behind — a new snapshot, a
 * new event, or both. A feed that changed nothing calls nothing, so a
 * consumer that draws on this never draws a frame it did not need.
 *
 * Wakes coalesce, so on each one take the snapshot and drain the queues until
 * they are empty.
 *
 * While the child holds a synchronized output block open the call is held
 * back, and the close of the block makes it exactly once — a frame published
 * inside a block is a half-drawn screen, and the newest is the only one a
 * consumer would have got anyway.
 *
 * What fell due while no callback was registered stays owed, and registering
 * one pays it before this call returns — so a consumer that attaches late is
 * told there is something to take rather than having to know to look. A wake
 * a synchronized output block is holding back has not fallen due yet, and
 * goes out with the close of the block as it would have anyway.
 *
 * # Safety
 *
 * `session` must be a live handle. `userdata` is never read here, only handed
 * back, but whatever it points at must outlive the session or be cleared out
 * of it first.
 */
KtStatus kt_session_set_wake(KtSession *session, KtWake wake, void *userdata);

/**
 * Select a range of the viewport, or clear the selection by passing null.
 *
 * Publishes a snapshot, since the selection is part of what a consumer draws.
 *
 * A session with a PTY behind it applies this on its own thread, so the call
 * returns once the request is queued and an endpoint outside the viewport
 * comes back as a wake with nothing new selected rather than as
 * `KT_STATUS_OUT_OF_RANGE`.
 *
 * # Safety
 *
 * `session` must be a live handle, and `range` must be null or point at a
 * readable `KtSelectionRange`.
 */
KtStatus kt_session_set_selection(KtSession *session, const KtSelectionRange *range);

/**
 * Take the bytes a detached session has queued for its child, emptying the
 * queue.
 *
 * `out` receives a run borrowed from the session, valid until the next call
 * to this function on it or until the session is freed. A length of 0 means
 * nothing was queued, which is not a failure. A session with a PTY behind it
 * has its own reader draining the queue, so this returns
 * `KT_STATUS_NOT_DETACHED` for one.
 *
 * Works on a defunct session, for the same reason taking its snapshot does:
 * what it queued before it broke is still what it queued.
 *
 * # Safety
 *
 * `session` must be a live handle and `out` must be a valid, writable
 * pointer to a `KtBytes`.
 */
KtStatus kt_session_take_writes(KtSession *session, KtBytes *out);

/**
 * Take the events a session has queued for the app, emptying the queue.
 *
 * `out` receives a run borrowed from the session, valid until the next call
 * to this function on it or until the session is freed, along with the
 * number of events dropped for want of room since the last take. A length of
 * 0 means nothing was queued, which is not a failure.
 *
 * Unlike the writer queue this is not a detached-only drain: events are the
 * app's to consume, and a session with a PTY behind it has no one else to
 * consume them. Drain until the queue is empty on every wake.
 *
 * Works on a defunct session, for the same reason taking its snapshot does:
 * what it queued before it broke is still what it queued.
 *
 * # Safety
 *
 * `session` must be a live handle and `out` must be a valid, writable
 * pointer to a `KtEvents`.
 */
KtStatus kt_session_take_events(KtSession *session, KtEvents *out);

/**
 * Take the latest snapshot, emptying the session's mailbox.
 *
 * Returns `KT_STATUS_NO_VALUE` when nothing has been published since the
 * last take, or `KT_STATUS_DEFUNCT` when nothing has been published and the
 * session is past working — a broken session publishes no more, so a bare
 * "nothing new" would be the last thing a consumer ever heard from one. On
 * success `out` receives an owned handle, to be released with
 * [`kt_snapshot_free`]; otherwise it receives null.
 *
 * Works on a defunct session: what it holds is the last state that was
 * right, and handing that back is the whole point of keeping it. The snapshot
 * says so — a session that broke while its child went on running reports both
 * on the frame it hands over.
 *
 * # Safety
 *
 * `session` must be a live handle and `out` must be a valid, writable
 * pointer to a `KtSnapshot *`.
 */
KtStatus kt_session_take_snapshot(KtSession *session, KtSnapshot **out);

/**
 * Release a snapshot. Null is a no-op.
 *
 * # Safety
 *
 * `snapshot` must come from [`kt_session_take_snapshot`] and must not be
 * used afterwards.
 */
void kt_snapshot_free(KtSnapshot *snapshot);

/**
 * Fill `out` with a view of the snapshot's contents.
 *
 * # Safety
 *
 * `snapshot` must be a live handle and `out` must be a valid, writable
 * pointer to a `KtSnapshotView`.
 */
KtStatus kt_snapshot_view(const KtSnapshot *snapshot, KtSnapshotView *out);

#ifdef __cplusplus
}  // extern "C"
#endif  // __cplusplus

#endif  /* KNOTTY_H */
