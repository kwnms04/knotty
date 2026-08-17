//! C3 — flattening the engine's render state into a snapshot.
//!
//! The one place the engine's grid is read, and the reason the facade is a
//! facade rather than a list of accessors: the contracts the C API states in
//! prose are kept here, once, instead of at every call site that would
//! otherwise have to remember them.
//!
//! - a grapheme buffer is sized by the length the engine reports, because the
//!   call that fills it is handed no length of its own
//! - a colour the cell does not carry comes back as a refusal, not a value
//! - the two layers of dirty state are cleared apart, because clearing one
//!   leaves the other standing

use std::mem::MaybeUninit;
use std::ptr;

use libghostty_vt_sys as ffi;

use super::{Terminal, check, read};
use crate::snapshot::{
    Attribute, Cell, Cursor, CursorShape, Dirty, Rgb, Row, RowFlag, ScreenState, Snapshot,
    Underline,
};
use crate::{Error, Result};

impl Terminal {
    /// Flatten the terminal's render state into a snapshot.
    ///
    /// Returns `Ok(None)` when nothing changed since the last capture, so a
    /// caller publishes at most once per unit of work and never for a frame
    /// that would be identical. `previous` is the screen state of the last
    /// capture: the engine's dirty tracking does not cover it, so a title or
    /// cursor move on an otherwise still screen would go unpublished without
    /// it.
    ///
    /// `even_if_unchanged` is for a caller with something to say that the
    /// screen cannot show, and takes that answer away: it always yields a
    /// frame.
    pub(crate) fn capture(
        &mut self,
        previous: &ScreenState,
        even_if_unchanged: bool,
    ) -> Result<Option<Snapshot>> {
        // SAFETY: both handles are ours and outlive the call, which is the one
        // point where reading the render state needs the terminal at all.
        check(unsafe { ffi::ghostty_render_state_update(self.render, self.raw) })?;

        let dirty = self.dirty()?;
        let screen = self.screen_state()?;
        if !even_if_unchanged && dirty == Dirty::Clean && screen == *previous {
            return Ok(None);
        }

        // SAFETY: each tag's documented output type.
        let cols: u16 = unsafe { self.render_get(ffi::RenderStateData::COLS) }?;
        let rows: u16 = unsafe { self.render_get(ffi::RenderStateData::ROWS) }?;
        let defaults = self.default_colors()?;

        // Any cell the iterators do not reach still has to honour the promise
        // that its colours are the terminal's defaults.
        let blank = Cell {
            foreground: defaults.foreground,
            background: defaults.background,
            ..Cell::default()
        };
        let mut cells = vec![blank; usize::from(cols) * usize::from(rows)];
        let mut row_state = vec![Row::default(); usize::from(rows)];
        let mut graphemes = Graphemes::default();

        // SAFETY: `rows` is our own iterator, which this points at the frame
        // just updated. Its data is good until the next update.
        check(unsafe {
            ffi::ghostty_render_state_get(
                self.render,
                ffi::RenderStateData::ROW_ITERATOR,
                ptr::from_mut(&mut self.rows).cast(),
            )
        })?;

        let mut y = 0usize;
        // SAFETY: the iterator is ours and positioned by this call.
        while unsafe { ffi::ghostty_render_state_row_iterator_next(self.rows) } {
            row_state[y] = self.row_state()?;
            // The engine tracks the two dirty layers separately, so clearing
            // the global one leaves these set. Clear it here, while we are on
            // the row.
            self.set_row_clean()?;

            // SAFETY: `cells` is our own iterator, pointed at the row the row
            // iterator is on.
            check(unsafe {
                ffi::ghostty_render_state_row_get(
                    self.rows,
                    ffi::RenderStateRowData::CELLS,
                    ptr::from_mut(&mut self.cells).cast(),
                )
            })?;

            let mut x = 0usize;
            // SAFETY: as the row iterator above.
            while unsafe { ffi::ghostty_render_state_row_cells_next(self.cells) } {
                cells[y * usize::from(cols) + x] = self.cell(defaults, &mut graphemes)?;
                x += 1;
            }
            y += 1;
        }

        // Consume the dirty state we just acted on, so an unchanged terminal
        // reports clean on the next capture.
        self.set_frame_clean()?;

        Ok(Some(Snapshot {
            cols,
            rows,
            dirty,
            // The caller fills this in: whether a selection exists is session
            // state, not something the render state can be asked.
            has_selection: false,
            screen,
            cells,
            row_state,
            graphemes: graphemes.table,
        }))
    }

    /// How much of the screen changed since the last capture.
    fn dirty(&self) -> Result<Dirty> {
        // SAFETY: the tag's documented output type.
        let dirty: ffi::RenderStateDirty::Type =
            unsafe { self.render_get(ffi::RenderStateData::DIRTY) }?;
        match dirty {
            ffi::RenderStateDirty::FALSE => Ok(Dirty::Clean),
            ffi::RenderStateDirty::PARTIAL => Ok(Dirty::Partial),
            ffi::RenderStateDirty::FULL => Ok(Dirty::Full),
            _ => Err(Error::Engine),
        }
    }

    /// Everything a snapshot says that is not the grid.
    fn screen_state(&self) -> Result<ScreenState> {
        Ok(ScreenState {
            cursor: self.cursor()?,
            title: without_control_characters(self.text(ffi::TerminalData::TITLE)?),
            pwd: without_control_characters(&path_of(self.text(ffi::TerminalData::PWD)?)),
        })
    }

    /// Where the cursor is and how it looks.
    fn cursor(&self) -> Result<Cursor> {
        // SAFETY: each tag's documented output type. The position is only
        // defined once this one says there is one.
        let in_viewport: bool =
            unsafe { self.render_get(ffi::RenderStateData::CURSOR_VIEWPORT_HAS_VALUE) }?;
        let (x, y) = if in_viewport {
            (
                unsafe { self.render_get(ffi::RenderStateData::CURSOR_VIEWPORT_X) }?,
                unsafe { self.render_get(ffi::RenderStateData::CURSOR_VIEWPORT_Y) }?,
            )
        } else {
            (0, 0)
        };
        let shown: bool = unsafe { self.render_get(ffi::RenderStateData::CURSOR_VISIBLE) }?;
        let shape: ffi::RenderStateCursorVisualStyle::Type =
            unsafe { self.render_get(ffi::RenderStateData::CURSOR_VISUAL_STYLE) }?;

        Ok(Cursor {
            x,
            y,
            // A cursor outside the viewport cannot be drawn either.
            visible: in_viewport && shown,
            shape: shape_of(shape),
        })
    }

    /// The colours a cell falls back to when it carries none of its own.
    fn default_colors(&self) -> Result<Defaults> {
        // Read one tag at a time rather than through the whole colour struct,
        // which carries a 256-entry palette knotty has no use for: the engine
        // has already resolved every cell's palette index by the time we see
        // it.
        //
        // SAFETY: each tag's documented output type.
        Ok(Defaults {
            foreground: rgb(unsafe { self.render_get(ffi::RenderStateData::COLOR_FOREGROUND) }?),
            background: rgb(unsafe { self.render_get(ffi::RenderStateData::COLOR_BACKGROUND) }?),
        })
    }

    /// What the snapshot says about the row the iterator is on.
    fn row_state(&self) -> Result<Row> {
        // SAFETY: the tag's documented output type.
        let dirty: bool = unsafe { self.row_get(ffi::RenderStateRowData::DIRTY) }?;
        // SAFETY: as above, and then the `bool` the wrap tag documents. The
        // raw row is an opaque value, queried in turn.
        let raw: ffi::Row = unsafe { self.row_get(ffi::RenderStateRowData::RAW) }?;
        let wrapped: bool = unsafe { of_row(raw, ffi::RowData::WRAP) }?;
        let selection = self.row_selection()?;

        let mut flags = 0;
        if dirty {
            flags |= RowFlag::Dirty as u8;
        }
        if wrapped {
            flags |= RowFlag::Wrapped as u8;
        }
        if selection.is_some() {
            flags |= RowFlag::Selected as u8;
        }

        Ok(Row {
            flags,
            selection_start: selection.map_or(0, |(start, _)| start),
            selection_end: selection.map_or(0, |(_, end)| end),
        })
    }

    /// The part of the selection that falls on this row, if any of it does.
    fn row_selection(&self) -> Result<Option<(u16, u16)>> {
        let mut selection = ffi::sized!(ffi::RenderStateRowSelection);
        // SAFETY: the sized out parameter the tag documents, with its size
        // filled in.
        let result = unsafe {
            ffi::ghostty_render_state_row_get(
                self.rows,
                ffi::RenderStateRowData::SELECTION,
                ptr::from_mut(&mut selection).cast(),
            )
        };
        match result {
            ffi::Result::SUCCESS => Ok(Some((selection.start_x, selection.end_x))),
            // The row does not meet the selection, which is not a failure.
            ffi::Result::NO_VALUE => Ok(None),
            _ => Err(Error::Engine),
        }
    }

    /// Take the dirty mark off the row the iterator is on.
    fn set_row_clean(&self) -> Result<()> {
        let clean = false;
        // SAFETY: the `bool` input the option documents, read during the call.
        check(unsafe {
            ffi::ghostty_render_state_row_set(
                self.rows,
                ffi::RenderStateRowOption::DIRTY,
                ptr::from_ref(&clean).cast(),
            )
        })
    }

    /// Take the dirty mark off the frame.
    fn set_frame_clean(&self) -> Result<()> {
        let clean = ffi::RenderStateDirty::FALSE;
        // SAFETY: the enum-sized input the option documents. It is a pointer
        // to the value itself and not to the reference holding it, which is
        // the mistake this spelling exists to avoid.
        check(unsafe {
            ffi::ghostty_render_state_set(
                self.render,
                ffi::RenderStateOption::DIRTY,
                ptr::from_ref(&clean).cast(),
            )
        })
    }

    /// Read the cell the iterator is on.
    fn cell(&self, defaults: Defaults, graphemes: &mut Graphemes) -> Result<Cell> {
        let mut style = ffi::sized!(ffi::Style);
        // SAFETY: the sized out parameter the tag documents.
        check(unsafe {
            ffi::ghostty_render_state_row_cells_get(
                self.cells,
                ffi::RenderStateRowCellsData::STYLE,
                ptr::from_mut(&mut style).cast(),
            )
        })?;

        // SAFETY: the tag's documented output type. The raw cell is an opaque
        // value, queried in turn below.
        let raw: ffi::Cell = unsafe { self.cell_get(ffi::RenderStateRowCellsData::RAW) }?;
        // SAFETY: each of the three is its tag's documented output type.
        let tag: ffi::CellContentTag::Type = unsafe { of_cell(raw, ffi::CellData::CONTENT_TAG) }?;
        let wide: ffi::CellWide::Type = unsafe { of_cell(raw, ffi::CellData::WIDE) }?;

        let (codepoint, overflow) = if tag == ffi::CellContentTag::CODEPOINT_GRAPHEME {
            (self.spill(graphemes)?, Attribute::Overflow as u16)
        } else {
            (unsafe { of_cell(raw, ffi::CellData::CODEPOINT) }?, 0)
        };

        Ok(Cell {
            codepoint,
            // The engine resolves palette indices for us; an unset colour
            // falls back to the terminal's current default.
            foreground: self
                .cell_color(ffi::RenderStateRowCellsData::FG_COLOR)?
                .unwrap_or(defaults.foreground),
            background: self
                .cell_color(ffi::RenderStateRowCellsData::BG_COLOR)?
                .unwrap_or(defaults.background),
            attributes: attributes_of(&style) | structure_of(wide) | overflow,
            underline: underline_of(style.underline),
        })
    }

    /// A colour the cell carries, or nothing when it carries none.
    fn cell_color(&self, tag: ffi::RenderStateRowCellsData::Type) -> Result<Option<Rgb>> {
        let mut color = MaybeUninit::<ffi::ColorRgb>::zeroed();
        // SAFETY: the tag's documented output type.
        let result = unsafe {
            ffi::ghostty_render_state_row_cells_get(self.cells, tag, color.as_mut_ptr().cast())
        };
        match result {
            // SAFETY: a successful call initializes the value.
            ffi::Result::SUCCESS => Ok(Some(rgb(unsafe { color.assume_init() }))),
            // The cell carries no colour of its own, which is the ordinary case
            // rather than a failure: the caller substitutes the terminal's
            // default. The header names only the first of these, and the second
            // is here because it is the same answer under the name the rest of
            // the C API uses for it — refusing it would narrow what a colour is
            // allowed to be absent by.
            ffi::Result::INVALID_VALUE | ffi::Result::NO_VALUE => Ok(None),
            _ => Err(Error::Engine),
        }
    }

    /// Append the cell's codepoints to the grapheme table, returning the index
    /// the cell should carry.
    fn spill(&self, graphemes: &mut Graphemes) -> Result<u32> {
        // Nothing bounds the table: a cell contributes its whole cluster, and
        // there is no ceiling on either the cluster length or the cell count.
        // The cell addresses the table with a u32, so refuse rather than
        // truncate.
        let index = u32::try_from(graphemes.table.len()).map_err(|_| Error::TooLarge)?;

        // SAFETY: the tag's documented output type.
        let len: u32 = unsafe { self.cell_get(ffi::RenderStateRowCellsData::GRAPHEMES_LEN) }?;
        graphemes.cluster.resize(len as usize, 0);
        if len > 0 {
            // The engine is handed a bare pointer and no length, so the buffer
            // being at least `len` long is this side's whole guarantee — which
            // is why the length is read first and the buffer sized from it.
            //
            // SAFETY: the scratch holds exactly the `len` codepoints the call
            // writes.
            check(unsafe {
                ffi::ghostty_render_state_row_cells_get(
                    self.cells,
                    ffi::RenderStateRowCellsData::GRAPHEMES_BUF,
                    graphemes.cluster.as_mut_ptr().cast(),
                )
            })?;
        }

        graphemes.table.push(len);
        graphemes.table.extend_from_slice(&graphemes.cluster);
        Ok(index)
    }

    /// Read a render state value into `T`.
    ///
    /// # Safety
    ///
    /// `T` must be the output type `data` documents.
    unsafe fn render_get<T>(&self, data: ffi::RenderStateData::Type) -> Result<T> {
        // SAFETY: the caller's, as declared.
        unsafe { read(|out| ffi::ghostty_render_state_get(self.render, data, out)) }
    }

    /// Read a value off the row the iterator is on.
    ///
    /// # Safety
    ///
    /// `T` must be the output type `data` documents.
    unsafe fn row_get<T>(&self, data: ffi::RenderStateRowData::Type) -> Result<T> {
        // SAFETY: the caller's, as declared.
        unsafe { read(|out| ffi::ghostty_render_state_row_get(self.rows, data, out)) }
    }

    /// Read a value off the cell the iterator is on.
    ///
    /// # Safety
    ///
    /// `T` must be the output type `data` documents.
    unsafe fn cell_get<T>(&self, data: ffi::RenderStateRowCellsData::Type) -> Result<T> {
        // SAFETY: the caller's, as declared.
        unsafe { read(|out| ffi::ghostty_render_state_row_cells_get(self.cells, data, out)) }
    }
}

/// Read a value off an opaque row, which is queried apart from the iterator
/// that produced it.
///
/// # Safety
///
/// `T` must be the output type `data` documents.
unsafe fn of_row<T>(row: ffi::Row, data: ffi::RowData::Type) -> Result<T> {
    // SAFETY: the caller's, as declared.
    unsafe { read(|out| ffi::ghostty_row_get(row, data, out)) }
}

/// The same for an opaque cell.
///
/// # Safety
///
/// `T` must be the output type `data` documents.
unsafe fn of_cell<T>(cell: ffi::Cell, data: ffi::CellData::Type) -> Result<T> {
    // SAFETY: the caller's, as declared.
    unsafe { read(|out| ffi::ghostty_cell_get(cell, data, out)) }
}

/// The snapshot's grapheme table and the scratch a cell is read into.
///
/// One value rather than two, because both are `Vec<u32>` and passing them
/// side by side is an argument order nothing would catch getting wrong.
#[derive(Default)]
struct Graphemes {
    /// What the snapshot carries away.
    table: Vec<u32>,
    /// Reused across cells so that spilling one does not allocate.
    cluster: Vec<u32>,
}

/// The terminal's own foreground and background, which every cell that carries
/// no colour of its own is drawn in.
#[derive(Clone, Copy)]
struct Defaults {
    foreground: Rgb,
    background: Rgb,
}

fn rgb(color: ffi::ColorRgb) -> Rgb {
    Rgb {
        r: color.r,
        g: color.g,
        b: color.b,
    }
}

fn shape_of(style: ffi::RenderStateCursorVisualStyle::Type) -> CursorShape {
    match style {
        ffi::RenderStateCursorVisualStyle::BLOCK => CursorShape::Block,
        ffi::RenderStateCursorVisualStyle::BAR => CursorShape::Bar,
        ffi::RenderStateCursorVisualStyle::UNDERLINE => CursorShape::Underline,
        ffi::RenderStateCursorVisualStyle::BLOCK_HOLLOW => CursorShape::BlockHollow,
        // Say so rather than picking a shape, so an upstream addition shows up
        // instead of hiding inside one of the shapes we do know.
        _ => CursorShape::Unknown,
    }
}

/// The engine declares the field signed and the values unsigned, so the cast
/// is the header's own inconsistency and not a narrowing: anything negative
/// lands on the unnameable kind either way.
#[expect(
    clippy::cast_sign_loss,
    reason = "the field is signed and its values are not"
)]
fn underline_of(underline: std::os::raw::c_int) -> Underline {
    match underline as ffi::SgrUnderline::Type {
        ffi::SgrUnderline::NONE => Underline::None,
        ffi::SgrUnderline::SINGLE => Underline::Single,
        ffi::SgrUnderline::DOUBLE => Underline::Double,
        ffi::SgrUnderline::CURLY => Underline::Curly,
        ffi::SgrUnderline::DOTTED => Underline::Dotted,
        ffi::SgrUnderline::DASHED => Underline::Dashed,
        // As with the cursor shape: still an underline, but its kind cannot be
        // named.
        _ => Underline::Unknown,
    }
}

fn structure_of(wide: ffi::CellWide::Type) -> u16 {
    match wide {
        ffi::CellWide::WIDE => Attribute::Wide as u16,
        ffi::CellWide::SPACER_TAIL => Attribute::WideTail as u16,
        // Narrow needs no flag, and SpacerHead is a soft-wrap artefact that
        // draws as nothing either way.
        _ => 0,
    }
}

fn attributes_of(style: &ffi::Style) -> u16 {
    let mut attributes = 0;
    for (present, attribute) in [
        (style.bold, Attribute::Bold),
        (style.faint, Attribute::Faint),
        (style.italic, Attribute::Italic),
        (style.blink, Attribute::Blink),
        (style.inverse, Attribute::Inverse),
        (style.invisible, Attribute::Invisible),
        (style.strikethrough, Attribute::Strikethrough),
        (style.overline, Attribute::Overline),
    ] {
        if present {
            attributes |= attribute as u16;
        }
    }
    attributes
}

/// Strip control characters, newlines included.
///
/// These values come from the program on the other end, which is untrusted,
/// and they end up in window titles and restore files.
fn without_control_characters(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).collect()
}

/// Reduce a working directory report to an absolute path.
///
/// The field promises an absolute path, so a report that does not yield one —
/// a relative directory, a URI with no path, an escape that does not spell
/// UTF-8 — publishes nothing rather than something a consumer would have to
/// second-guess.
fn path_of(reported: &str) -> String {
    decoded_path(reported)
        .filter(|path| path.starts_with('/'))
        .unwrap_or_default()
}

/// OSC 7 reports a `file://` URI; OSC 1337 reports a bare path, which needs
/// no undoing.
fn decoded_path(reported: &str) -> Option<String> {
    let Some(after_scheme) = reported.strip_prefix("file://") else {
        return Some(reported.to_owned());
    };
    // Drop the authority. knotty has no notion of which host it is on, so a
    // path reported by another one is taken at face value.
    let encoded = &after_scheme[after_scheme.find('/')?..];

    percent_decoded(encoded)
}

fn percent_decoded(encoded: &str) -> Option<String> {
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let escape = (bytes[index] == b'%')
            .then(|| bytes.get(index + 1..index + 3))
            .flatten()
            // from_str_radix takes a leading sign, so `%+A` would decode as
            // one. Only two hex digits are an escape.
            .filter(|digits| digits.iter().all(u8::is_ascii_hexdigit))
            .and_then(|digits| u8::from_str_radix(std::str::from_utf8(digits).ok()?, 16).ok());

        match escape {
            Some(byte) => {
                decoded.push(byte);
                index += 3;
            }
            None => {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
    }

    // A decoding that does not spell UTF-8 is not a path we can offer, and
    // handing back the still-encoded text would be a different lie.
    String::from_utf8(decoded).ok()
}
