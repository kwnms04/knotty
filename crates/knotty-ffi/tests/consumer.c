/* A C consumer of the generated header.
 *
 * Compiled for syntax only: its job is to prove the header is valid C, that
 * the frozen layout still holds, and that a cell read is plain indexing. */

#include <knotty.h>
#include <stddef.h>
#include <string.h>

/* Golden snapshot comparison depends on these layouts, so a change here is an
 * ABI change and must come with a version bump. */
_Static_assert(KT_ABI_VERSION == 8, "ABI version moved without updating this consumer");

_Static_assert(sizeof(KtCell) == 16, "KtCell grew or shrank");
_Static_assert(offsetof(KtCell, codepoint) == 0, "KtCell fields moved");
_Static_assert(offsetof(KtCell, foreground) == 4, "KtCell fields moved");
_Static_assert(offsetof(KtCell, background) == 7, "KtCell fields moved");
_Static_assert(offsetof(KtCell, attributes) == 10, "KtCell fields moved");
_Static_assert(offsetof(KtCell, underline) == 12, "KtCell fields moved");

_Static_assert(sizeof(KtRow) == 6, "KtRow grew or shrank");
_Static_assert(offsetof(KtRow, flags) == 0, "KtRow fields moved");
_Static_assert(offsetof(KtRow, selection_start) == 2, "KtRow fields moved");
_Static_assert(offsetof(KtRow, selection_end) == 4, "KtRow fields moved");

_Static_assert(sizeof(KtCursor) == 6, "KtCursor grew or shrank");
_Static_assert(offsetof(KtCursor, x) == 0, "KtCursor fields moved");
_Static_assert(offsetof(KtCursor, y) == 2, "KtCursor fields moved");
_Static_assert(offsetof(KtCursor, visible) == 4, "KtCursor fields moved");
_Static_assert(offsetof(KtCursor, shape) == 5, "KtCursor fields moved");

_Static_assert(sizeof(KtSnapshotView) == 88, "KtSnapshotView grew or shrank");
_Static_assert(offsetof(KtSnapshotView, cols) == 0, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, rows) == 2, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, dirty) == 4, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, has_selection) == 5, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, cells) == 8, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, row_state) == 16, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, graphemes) == 24, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, grapheme_count) == 32, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, cursor) == 40, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, title) == 48, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, pwd) == 64, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, child_state) == 80, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, session_state) == 81, "KtSnapshotView fields moved");
_Static_assert(offsetof(KtSnapshotView, child_exit_code) == 84, "KtSnapshotView fields moved");

_Static_assert(sizeof(KtKeyEvent) == 32, "KtKeyEvent grew or shrank");
_Static_assert(offsetof(KtKeyEvent, action) == 0, "KtKeyEvent fields moved");
_Static_assert(offsetof(KtKeyEvent, key) == 4, "KtKeyEvent fields moved");
_Static_assert(offsetof(KtKeyEvent, mods) == 8, "KtKeyEvent fields moved");
_Static_assert(offsetof(KtKeyEvent, consumed_mods) == 10, "KtKeyEvent fields moved");
_Static_assert(offsetof(KtKeyEvent, composing) == 12, "KtKeyEvent fields moved");
_Static_assert(offsetof(KtKeyEvent, text) == 16, "KtKeyEvent fields moved");

_Static_assert(sizeof(KtEvent) == 32, "KtEvent grew or shrank");
_Static_assert(offsetof(KtEvent, kind) == 0, "KtEvent fields moved");
_Static_assert(offsetof(KtEvent, clipboard_target) == 1, "KtEvent fields moved");
_Static_assert(offsetof(KtEvent, text) == 8, "KtEvent fields moved");
_Static_assert(offsetof(KtEvent, exit_code) == 24, "KtEvent fields moved");

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
           (view->row_state[row].flags & KT_ROW_FLAG_DIRTY) != 0;
}

/* A row that runs on joins with the next; one that ended at a newline does
 * not. */
int kt_consumer_joins_next_row(const KtSnapshotView *view, uint16_t row) {
    return (view->row_state[row].flags & KT_ROW_FLAG_WRAPPED) != 0;
}

/* Text is a run of bytes with a length, not a null-terminated string. */
int kt_consumer_title_is(const KtSnapshotView *view, const char *expected) {
    size_t len = strlen(expected);
    return view->title.len == len && memcmp(view->title.bytes, expected, len) == 0;
}

/* Selection is read beside the cells, never out of them, so highlighting a
 * drag leaves every cell value untouched. */
int kt_consumer_is_selected(const KtSnapshotView *view, uint16_t row, uint16_t col) {
    const KtRow *state = &view->row_state[row];
    return (state->flags & KT_ROW_FLAG_SELECTED) != 0 && col >= state->selection_start &&
           col <= state->selection_end;
}

/* The two states are read apart, because the app does two different things
 * with them: a child still running is what makes closing the window worth a
 * warning, and a session that broke is what makes the window stop taking
 * input. Either can be true without the other. */
int kt_consumer_warns_before_closing(const KtSnapshotView *view) {
    return view->child_state == KT_CHILD_STATE_RUNNING;
}

int kt_consumer_takes_typing(const KtSnapshotView *view) {
    return view->session_state == KT_SESSION_STATE_OK;
}

/* What the child ended by, for the window that stays up saying so. */
int kt_consumer_exit_code(const KtSnapshotView *view, int *out_code) {
    if (view->child_state != KT_CHILD_STATE_EXITED) {
        return 0;
    }
    *out_code = view->child_exit_code;
    return 1;
}

/* A session with a PTY behind it drains its own writer queue and takes its
 * input from its child, so the two calls that would reach past its thread are
 * refused rather than answered wrongly. */
_Static_assert(KT_STATUS_NOT_DETACHED == 8, "the PTY refusal moved");

/* Starting a shell: the command is words of borrowed text, program first, and
 * the size the child is born knowing comes with it. */
KtStatus kt_consumer_open_a_shell(KtSession **out) {
    KtText argv[2] = {{(const uint8_t *)"/bin/sh", 7}, {(const uint8_t *)"-l", 2}};
    return kt_session_new_pty(80, 24, 1000, argv, 2, out);
}

/* Typing: queued for the child, never waited on. */
KtStatus kt_consumer_type(KtSession *session, const char *keys, size_t len) {
    return kt_session_write(session, (const uint8_t *)keys, len);
}

/* A key: the physical key and what the layout made of it, with which bytes it
 * comes to left to the core. Nothing here says what mode the terminal is in,
 * which is the whole point — ctrl+c is the same call inside an editor and at a
 * prompt. */
KtStatus kt_consumer_press_ctrl_c(KtSession *session) {
    KtKeyEvent event = {
        .action = KT_KEY_ACTION_PRESS,
        .key = KT_KEY_C,
        .mods = KT_MODIFIER_CTRL,
        .consumed_mods = 0,
        .composing = false,
        .text = {NULL, 0},
    };
    return kt_session_key(session, &event);
}

/* A wake runs on the core's thread and may do nothing but flag the consumer's
 * own, which is why it takes no lock and reads nothing back. */
static void kt_consumer_on_wake(void *userdata) { *(int *)userdata = 1; }

KtStatus kt_consumer_draw_when_told(KtSession *session, int *needs_frame) {
    return kt_session_set_wake(session, kt_consumer_on_wake, needs_frame);
}
