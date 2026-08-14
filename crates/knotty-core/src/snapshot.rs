//! C3 — the single conversion point between VT engine types and kt types.
//!
//! Nothing outside this module names a VT engine type in a signature.

use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::{RenderState, Terminal};

use crate::{Error, Result};

/// One terminal cell.
///
/// Fixed size and POD: the grid is a row-major flat array of these, so a
/// consumer indexes it without a function call per cell.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme's base codepoint, or 0 when the cell holds no text.
    pub codepoint: u32,
}

/// One immutable frame of terminal state.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    /// Viewport width in cells.
    pub cols: u16,
    /// Viewport height in cells.
    pub rows: u16,
    /// `rows * cols` cells in row-major order.
    pub cells: Vec<Cell>,
}

/// Flatten the terminal's render state into a snapshot.
///
/// Returns `Ok(None)` when nothing changed since the last capture, so a
/// caller publishes at most once per unit of work and never for a frame that
/// would be identical.
pub(crate) fn capture(
    render: &mut RenderState<'static>,
    terminal: &Terminal<'static, 'static>,
) -> Result<Option<Snapshot>> {
    let frame = render.update(terminal).map_err(|_| Error::Engine)?;
    if frame.dirty().map_err(|_| Error::Engine)? == Dirty::Clean {
        return Ok(None);
    }

    let cols = frame.cols().map_err(|_| Error::Engine)?;
    let rows = frame.rows().map_err(|_| Error::Engine)?;
    let mut cells = vec![Cell::default(); usize::from(cols) * usize::from(rows)];

    let mut row_iter = RowIterator::new().map_err(|_| Error::Engine)?;
    let mut cell_iter = CellIterator::new().map_err(|_| Error::Engine)?;
    let mut rows_iteration = row_iter.update(&frame).map_err(|_| Error::Engine)?;
    let mut y = 0usize;
    while let Some(row) = rows_iteration.next() {
        let mut cells_iteration = cell_iter.update(row).map_err(|_| Error::Engine)?;
        let mut x = 0usize;
        while let Some(cell) = cells_iteration.next() {
            let raw = cell.raw_cell().map_err(|_| Error::Engine)?;
            cells[y * usize::from(cols) + x] = Cell {
                codepoint: raw.codepoint().map_err(|_| Error::Engine)?,
            };
            x += 1;
        }
        y += 1;
    }

    // Consume the dirty state we just acted on, so an unchanged terminal
    // reports clean on the next capture.
    frame.set_dirty(Dirty::Clean).map_err(|_| Error::Engine)?;

    Ok(Some(Snapshot { cols, rows, cells }))
}
