/* A C consumer of the generated header.
 *
 * Compiled for syntax only: its job is to prove the header is valid C, that
 * the frozen layout still holds, and that a cell read is plain indexing. */

#include <knotty.h>
#include <stddef.h>

/* The M0 layout is frozen; golden snapshot comparison depends on it. A change
 * here is an ABI change and must come with a version bump. */
_Static_assert(KT_ABI_VERSION == 2, "ABI version moved without updating this consumer");
_Static_assert(sizeof(KtCell) == 16, "KtCell grew or shrank");
_Static_assert(offsetof(KtCell, codepoint) == 0, "KtCell fields moved");
_Static_assert(offsetof(KtCell, foreground) == 4, "KtCell fields moved");
_Static_assert(offsetof(KtCell, background) == 7, "KtCell fields moved");
_Static_assert(offsetof(KtCell, attributes) == 10, "KtCell fields moved");
_Static_assert(offsetof(KtCell, underline) == 12, "KtCell fields moved");

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
