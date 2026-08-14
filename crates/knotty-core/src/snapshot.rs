//! C3 — the single conversion point between VT engine types and kt types.
//!
//! Nothing outside this module names a VT engine type in a signature.

use libghostty_vt::render::{CellIterator, Dirty, RowIterator};
use libghostty_vt::{RenderState, Terminal};

use crate::{Error, Result};

impl From<libghostty_vt::Error> for Error {
    fn from(_: libghostty_vt::Error) -> Self {
        Self::Engine
    }
}

/// A colour, already resolved out of the palette.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rgb {
    /// Red component.
    pub r: u8,
    /// Green component.
    pub g: u8,
    /// Blue component.
    pub b: u8,
}

impl From<libghostty_vt::style::RgbColor> for Rgb {
    fn from(color: libghostty_vt::style::RgbColor) -> Self {
        Self {
            r: color.r,
            g: color.g,
            b: color.b,
        }
    }
}

/// SGR attributes, OR-ed together into [`Cell::attributes`].
///
/// Underlining is not here: it has kinds rather than an on/off state, so it
/// gets its own field.
#[repr(u16)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Attribute {
    /// SGR 1.
    Bold = 1 << 0,
    /// SGR 2.
    Faint = 1 << 1,
    /// SGR 3.
    Italic = 1 << 2,
    /// SGR 5.
    Blink = 1 << 3,
    /// SGR 7.
    Inverse = 1 << 4,
    /// SGR 8.
    Invisible = 1 << 5,
    /// SGR 9.
    Strikethrough = 1 << 6,
    /// SGR 53.
    Overline = 1 << 7,
}

/// How a cell is underlined.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Underline {
    /// Not underlined.
    #[default]
    None = 0,
    /// SGR 4.
    Single = 1,
    /// SGR 21.
    Double = 2,
    /// SGR 4:3.
    Curly = 3,
    /// SGR 4:4.
    Dotted = 4,
    /// SGR 4:5.
    Dashed = 5,
}

impl From<libghostty_vt::style::Underline> for Underline {
    fn from(underline: libghostty_vt::style::Underline) -> Self {
        use libghostty_vt::style::Underline as Vt;

        match underline {
            Vt::None => Self::None,
            Vt::Single => Self::Single,
            Vt::Double => Self::Double,
            Vt::Curly => Self::Curly,
            Vt::Dotted => Self::Dotted,
            Vt::Dashed => Self::Dashed,
            // The engine's enum is non-exhaustive. A kind we don't know yet is
            // still an underline, so report the plain one rather than none.
            _ => Self::Single,
        }
    }
}

/// One terminal cell.
///
/// Fixed size and POD: the grid is a row-major flat array of these, so a
/// consumer indexes it without a function call per cell.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The grapheme's base codepoint, or 0 when the cell holds no text.
    pub codepoint: u32,
    /// Foreground colour, with the terminal's default already substituted.
    pub foreground: Rgb,
    /// Background colour, with the terminal's default already substituted.
    pub background: Rgb,
    /// [`Attribute`] bits.
    pub attributes: u16,
    /// Which underline the cell carries, if any.
    pub underline: Underline,
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
    let frame = render.update(terminal)?;
    if frame.dirty()? == Dirty::Clean {
        return Ok(None);
    }

    let cols = frame.cols()?;
    let rows = frame.rows()?;
    let defaults = frame.colors()?;
    let mut cells = vec![Cell::default(); usize::from(cols) * usize::from(rows)];

    let mut row_iter = RowIterator::new()?;
    let mut cell_iter = CellIterator::new()?;
    let mut rows_iteration = row_iter.update(&frame)?;
    let mut y = 0usize;
    while let Some(row) = rows_iteration.next() {
        let mut cells_iteration = cell_iter.update(row)?;
        let mut x = 0usize;
        while let Some(cell) = cells_iteration.next() {
            let style = cell.style()?;
            cells[y * usize::from(cols) + x] = Cell {
                codepoint: cell.raw_cell()?.codepoint()?,
                // The engine resolves palette indices for us; an unset colour
                // falls back to the terminal's current default.
                foreground: cell.fg_color()?.unwrap_or(defaults.foreground).into(),
                background: cell.bg_color()?.unwrap_or(defaults.background).into(),
                attributes: attributes_of(&style),
                underline: style.underline.into(),
            };
            x += 1;
        }
        y += 1;
    }

    // Consume the dirty state we just acted on, so an unchanged terminal
    // reports clean on the next capture.
    frame.set_dirty(Dirty::Clean)?;

    Ok(Some(Snapshot { cols, rows, cells }))
}

fn attributes_of(style: &libghostty_vt::style::Style) -> u16 {
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
