/* A C consumer of the generated header.
 *
 * Compiled for syntax only: its job is to prove the header is valid C, that
 * the frozen layout still holds, and that a cell read is plain indexing. */

#include <knotty.h>
#include <stddef.h>

/* Golden snapshot comparison depends on these layouts, so a change here is an
 * ABI change and must come with a version bump. */
_Static_assert(KT_ABI_VERSION == 4, "ABI version moved without updating this consumer");

_Static_assert(sizeof(KtCell) == 16, "KtCell grew or shrank");
_Static_assert(offsetof(KtCell, codepoint) == 0, "KtCell fields moved");
_Static_assert(offsetof(KtCell, foreground) == 4, "KtCell fields moved");
_Static_assert(offsetof(KtCell, background) == 7, "KtCell fields moved");
_Static_assert(offsetof(KtCell, attributes) == 10, "KtCell fields moved");
_Static_assert(offsetof(KtCell, underline) == 12, "KtCell fields moved");

_Static_assert(sizeof(KtSnapshotView) == 40, "KtSnapshotView grew or shrank");
_Static_assert(offsetof(KtSnapshotView, cols) == 0, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, rows) == 2, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, dirty) == 4, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, cells) == 8, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, row_flags) == 16, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, graphemes) == 24, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, grapheme_count) == 32, "KtSnapshotView fields moved");

/* The startup handshake a real consumer performs. */
int kt_consumer_abi_ok(void) {
    return kt_abi_version() == KT_ABI_VERSION;
}

/* Reading a cell costs an index, not a call across the boundary. */
uint32_t kt_consumer_codepoint_at(const KtSnapshotView *view, uint16_t row, uint16_t col) {
    return view->cells[(size_t)row * view->cols + col].codepoint;
}

/* Attributes are a bit set; underline is an enum beside it. */
int kt_consumer_is_bold_and_curly(const KtCell *cell) {
    return (cell->attributes & KT_ATTRIBUTE_BOLD) != 0 && cell->underline == KT_UNDERLINE_CURLY;
}

/* A cell too small for its cluster points into the grapheme table: the entry
 * it names is the run length, and the codepoints follow. */
const uint32_t *kt_consumer_text_of(const KtSnapshotView *view, const KtCell *cell,
                                    uint32_t *out_len) {
    if ((cell->attributes & KT_ATTRIBUTE_OVERFLOW) == 0) {
        *out_len = 1;
        return &cell->codepoint;
    }
    const uint32_t *run = &view->graphemes[cell->codepoint];
    *out_len = run[0];
    return run + 1;
}

/* The two halves of a wide character are told apart by their flags. */
int kt_consumer_is_wide_tail(const KtCell *cell) {
    return (cell->attributes & KT_ATTRIBUTE_WIDE_TAIL) != 0;
}

/* Redrawing the least: everything on a full frame, only the marked rows on a
 * partial one. */
int kt_consumer_needs_redraw(const KtSnapshotView *view, uint16_t row) {
    return view->dirty == KT_DIRTY_FULL ||
           (view->row_flags[row] & KT_ROW_FLAG_DIRTY) != 0;
}

/* A row that runs on joins with the next; one that ended at a newline does
 * not. */
int kt_consumer_joins_next_row(const KtSnapshotView *view, uint16_t row) {
    return (view->row_flags[row] & KT_ROW_FLAG_WRAPPED) != 0;
}
