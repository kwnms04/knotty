/* A C consumer of the generated header.
 *
 * Compiled for syntax only: its job is to prove the header is valid C, that
 * the frozen layout still holds, and that a cell read is plain indexing. */

#include <knotty.h>

/* The M0 layout is frozen; golden snapshot comparison depends on it. A change
 * here is an ABI change and must come with a version bump. */
_Static_assert(KT_ABI_VERSION == 1, "ABI version moved without updating this consumer");
_Static_assert(sizeof(KtCell) == 4, "KtCell grew or shrank");

/* The startup handshake a real consumer performs. */
int kt_consumer_abi_ok(void) {
    return kt_abi_version() == KT_ABI_VERSION;
}

/* Reading a cell costs an index, not a call across the boundary. */
uint32_t kt_consumer_codepoint_at(const KtSnapshotView *view, uint16_t row, uint16_t col) {
    return view->cells[(size_t)row * view->cols + col].codepoint;
}
