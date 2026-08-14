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
#define KT_ABI_VERSION 1

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
};
#ifndef __cplusplus
#if __STDC_VERSION__ >= 202311L
typedef enum KtStatus KtStatus;
#else
typedef int32_t KtStatus;
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
 * One terminal cell.
 *
 * Fixed size and POD: the grid is a row-major flat array of these, so a
 * consumer indexes it without a function call per cell.
 */
typedef struct {
  /**
   * The grapheme's base codepoint, or 0 when the cell holds no text.
   */
  uint32_t codepoint;
} KtCell;

/**
 * Borrowed view of a snapshot's contents.
 *
 * `cells` points at `rows * cols` cells in row-major order and stays valid
 * until the snapshot is freed.
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
 * [`kt_session_free`].
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
 * Returns [`KtStatus::NoValue`] and writes null to `out` when nothing has
 * been published since the last take. On success `out` receives an owned
 * handle, to be released with [`kt_snapshot_free`].
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
