//! C3 — the single conversion point between VT engine types and kt types.
//!
//! Nothing outside this module names a VT engine type in a signature.

use libghostty_vt::render::{CellIteration, CellIterator, Dirty as VtDirty, RowIterator};
use libghostty_vt::screen::{CellContentTag, CellWide};
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

/// Cell attributes, OR-ed together into a cell's `attributes` field.
///
/// The low byte is SGR state, the high byte is structure. Underlining is in
/// neither: it has kinds rather than an on/off state, so it gets its own
/// field.
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
    /// The leading cell of a character two columns wide.
    Wide = 1 << 8,
    /// The trailing cell of a character two columns wide. It holds no text of
    /// its own; the leading cell carries the whole character.
    WideTail = 1 << 9,
    /// The cell's `codepoint` is an index into the snapshot's grapheme table
    /// rather than a codepoint.
    Overflow = 1 << 10,
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
    /// Underlined in a way this version of the engine knows and knotty does
    /// not. Still an underline, but its kind cannot be named.
    Unknown = 255,
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
            // The engine's enum is non-exhaustive. Say so rather than picking
            // a kind, so an upstream addition shows up instead of hiding
            // inside one of the kinds we do know.
            _ => Self::Unknown,
        }
    }
}

/// How much of the screen changed since the last snapshot was taken.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dirty {
    /// Nothing changed. A published snapshot never says this, because a
    /// capture that finds nothing to report is not published at all.
    Clean = 0,
    /// Some rows changed; the row flags say which.
    Partial = 1,
    /// Everything changed, as on a switch to or from the alternate screen.
    Full = 2,
}

impl From<VtDirty> for Dirty {
    fn from(dirty: VtDirty) -> Self {
        match dirty {
            VtDirty::Clean => Self::Clean,
            VtDirty::Partial => Self::Partial,
            VtDirty::Full => Self::Full,
        }
    }
}

/// Row state, OR-ed together into one entry of a snapshot's row flags.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowFlag {
    /// The row changed since the last snapshot.
    Dirty = 1 << 0,
    /// The row runs on into the next one. It ended because it ran out of
    /// columns, not at a newline.
    Wrapped = 1 << 1,
}

/// One terminal cell.
///
/// Fixed size and POD: the grid is a row-major flat array of these, so a
/// consumer indexes it without a function call per cell.
//
// The fields do not fill the struct, so it carries trailing padding whose
// contents are not defined. Comparing cells field by field is safe; comparing
// the grid as raw bytes is not, which the golden harness has to account for.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cell {
    /// The cell's codepoint, or 0 when it holds no text. When the cell has
    /// the overflow attribute this is an index into the snapshot's grapheme
    /// table instead.
    pub codepoint: u32,
    /// Foreground colour, with the terminal's default already substituted.
    pub foreground: Rgb,
    /// Background colour, with the terminal's default already substituted.
    pub background: Rgb,
    /// A bit set of `KtAttribute` values.
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
    /// How much of the screen changed since the last snapshot.
    pub dirty: Dirty,
    /// `rows * cols` cells in row-major order.
    pub cells: Vec<Cell>,
    /// One entry per row, each a bit set of [`RowFlag`] values.
    pub row_flags: Vec<u8>,
    /// Codepoints for the cells that did not fit in one, so that the cell
    /// stays a fixed size no matter how long its grapheme cluster is.
    ///
    /// A cell carrying the overflow attribute holds the index of its run's
    /// length; the codepoints follow, base first. The table is rebuilt every
    /// snapshot and never refers to an earlier one.
    pub graphemes: Vec<u32>,
}

impl Snapshot {
    /// Take on the change marks of a snapshot that was published but never
    /// consumed.
    ///
    /// The engine's marks are cleared as each snapshot is built, so they only
    /// describe what happened since the one before. When the mailbox drops an
    /// unconsumed snapshot, its marks would go with it and the consumer would
    /// be told less changed than really did. Cell contents need no such care:
    /// each snapshot already holds the whole screen.
    pub(crate) fn absorb_marks_of(&mut self, dropped: &Self) {
        if dropped.dirty == Dirty::Full {
            self.dirty = Dirty::Full;
        }
        for (flags, dropped) in self.row_flags.iter_mut().zip(&dropped.row_flags) {
            *flags |= dropped & RowFlag::Dirty as u8;
        }
    }
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
    let dirty = Dirty::from(frame.dirty()?);
    if dirty == Dirty::Clean {
        return Ok(None);
    }

    let cols = frame.cols()?;
    let rows = frame.rows()?;
    let defaults = frame.colors()?;
    // Any cell the iterators do not reach still has to honour the promise that
    // its colours are the terminal's defaults.
    let blank = Cell {
        foreground: defaults.foreground.into(),
        background: defaults.background.into(),
        ..Cell::default()
    };
    let mut cells = vec![blank; usize::from(cols) * usize::from(rows)];
    let mut row_flags = vec![0u8; usize::from(rows)];
    let mut graphemes = Vec::new();
    let mut cluster = Vec::new();

    let mut row_iter = RowIterator::new()?;
    let mut cell_iter = CellIterator::new()?;
    let mut rows_iteration = row_iter.update(&frame)?;
    let mut y = 0usize;
    while let Some(row) = rows_iteration.next() {
        row_flags[y] = row_flags_of(row.dirty()?, row.raw_row()?.is_wrapped()?);
        // The engine tracks the two dirty layers separately, so clearing the
        // global one leaves these set. Clear them here, while we have the row.
        row.set_dirty(false)?;

        let mut cells_iteration = cell_iter.update(row)?;
        let mut x = 0usize;
        while let Some(cell) = cells_iteration.next() {
            let style = cell.style()?;
            let raw = cell.raw_cell()?;

            let (codepoint, overflow) = if raw.content_tag()? == CellContentTag::CodepointGrapheme {
                (
                    spill(cell, &mut graphemes, &mut cluster)?,
                    Attribute::Overflow as u16,
                )
            } else {
                (raw.codepoint()?, 0)
            };

            cells[y * usize::from(cols) + x] = Cell {
                codepoint,
                // The engine resolves palette indices for us; an unset colour
                // falls back to the terminal's current default.
                foreground: cell.fg_color()?.unwrap_or(defaults.foreground).into(),
                background: cell.bg_color()?.unwrap_or(defaults.background).into(),
                attributes: attributes_of(&style) | structure_of(raw.wide()?) | overflow,
                underline: style.underline.into(),
            };
            x += 1;
        }
        y += 1;
    }

    // Consume the dirty state we just acted on, so an unchanged terminal
    // reports clean on the next capture.
    frame.set_dirty(VtDirty::Clean)?;

    Ok(Some(Snapshot {
        cols,
        rows,
        dirty,
        cells,
        row_flags,
        graphemes,
    }))
}

fn row_flags_of(dirty: bool, wrapped: bool) -> u8 {
    let mut flags = 0;
    if dirty {
        flags |= RowFlag::Dirty as u8;
    }
    if wrapped {
        flags |= RowFlag::Wrapped as u8;
    }
    flags
}

/// Append a cell's codepoints to the grapheme table, returning the index the
/// cell should carry.
///
/// `cluster` is scratch space the caller keeps across cells so that spilling
/// one does not allocate.
fn spill(
    cell: &CellIteration<'_, '_>,
    graphemes: &mut Vec<u32>,
    cluster: &mut Vec<char>,
) -> Result<u32> {
    // Nothing bounds the table: a cell contributes its whole cluster, and
    // there is no ceiling on either the cluster length or the cell count. The
    // cell addresses the table with a u32, so refuse rather than truncate.
    let index = u32::try_from(graphemes.len()).map_err(|_| Error::TooLarge)?;

    cluster.resize(cell.graphemes_len()?, '\0');
    cell.graphemes_buf(cluster)?;

    let len = u32::try_from(cluster.len()).map_err(|_| Error::TooLarge)?;
    graphemes.push(len);
    graphemes.extend(cluster.iter().map(|codepoint| *codepoint as u32));

    Ok(index)
}

fn structure_of(wide: CellWide) -> u16 {
    match wide {
        CellWide::Wide => Attribute::Wide as u16,
        CellWide::SpacerTail => Attribute::WideTail as u16,
        // Narrow needs no flag, and SpacerHead is a soft-wrap artefact that
        // draws as nothing either way.
        _ => 0,
    }
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
