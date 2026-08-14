/* Generated from knotty-ffi by cbindgen. Do not edit by hand. */

#ifndef KNOTTY_H
#define KNOTTY_H

#include <stdint.h>
#include <stddef.h>

/**
 * ABI version of this library.
 *
 * A caller reads the constant from the header it compiled against and
 * compares it with [`kt_abi_version`]. Mismatch means header and library
 * disagree about layouts, and the caller must not proceed.
 */
#define KT_ABI_VERSION 3

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
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtStatus KtStatus;
#else
typedef int32_t KtStatus;
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
 * Opaque handle to a session.
 */
typedef struct KtSession KtSession;

/**
 * Opaque handle to a snapshot.
 */
typedef struct KtSnapshot KtSnapshot;

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
   * Row-major grid of `rows * cols` cells.
   */
  const KtCell *cells;
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
 * Release a session. Null is a no-op.
 *
 * # Safety
 *
 * `session` must come from [`kt_session_new_detached`] and must not be used
 * afterwards.
 */
void kt_session_free(KtSession *session);

/**
 * Feed `len` bytes to a detached session.
 *
 * Processes the whole buffer on the calling thread before returning, and
 * publishes at most one snapshot.
 *
 * # Safety
 *
 * `session` must be a live handle, and `bytes` must point at `len` readable
 * bytes. `bytes` may be null only when `len` is 0.
 */
KtStatus kt_session_feed(KtSession *session, const uint8_t *bytes, size_t len);

/**
 * Take the latest snapshot, emptying the session's mailbox.
 *
 * Returns `KT_STATUS_NO_VALUE` when nothing has been published since the
 * last take. On success `out` receives an owned handle, to be released with
 * [`kt_snapshot_free`]; otherwise it receives null.
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
