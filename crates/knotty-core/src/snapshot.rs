//! C3 — the single conversion point between VT engine types and kt types.
//!
//! Nothing outside this module names a VT engine type in a signature.

use libghostty_vt::render::{
    CellIteration, CellIterator, CursorVisualStyle, Dirty as VtDirty, RowIterator, RowSelection,
    Snapshot as Frame,
};
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
///
/// The variants are ordered by how much they cover, so the larger of two is
/// the one that describes both.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Dirty {
    /// No row changed. A published snapshot can still say this: something
    /// outside the grid, such as the title or the cursor, moved instead.
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

/// Row state, OR-ed together into a row's `flags` field.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowFlag {
    /// The row changed since the last snapshot.
    Dirty = 1 << 0,
    /// The row runs on into the next one. It ended because it ran out of
    /// columns, not at a newline.
    Wrapped = 1 << 1,
    /// Part of the row is selected, and the row's columns say which part.
    Selected = 1 << 2,
}

/// What the cursor looks like.
#[repr(u8)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorShape {
    /// A filled block over the cell.
    #[default]
    Block = 0,
    /// A vertical bar before the cell.
    Bar = 1,
    /// A line under the cell.
    Underline = 2,
    /// An outlined block, drawn when the terminal is not focused.
    BlockHollow = 3,
    /// A shape this version of the engine knows and knotty does not.
    Unknown = 255,
}

impl From<CursorVisualStyle> for CursorShape {
    fn from(style: CursorVisualStyle) -> Self {
        match style {
            CursorVisualStyle::Block => Self::Block,
            CursorVisualStyle::Bar => Self::Bar,
            CursorVisualStyle::Underline => Self::Underline,
            CursorVisualStyle::BlockHollow => Self::BlockHollow,
            // The engine's enum is non-exhaustive; say so rather than picking
            // a shape, so an upstream addition shows up.
            _ => Self::Unknown,
        }
    }
}

/// Where the cursor is and how it looks.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Cursor {
    /// Column, from the left of the viewport.
    pub x: u16,
    /// Row, from the top of the viewport.
    pub y: u16,
    /// Whether to draw it. False both when the terminal hid it and when it
    /// sits outside the viewport, since neither is drawable.
    pub visible: bool,
    /// Which shape to draw.
    pub shape: CursorShape,
}

/// Screen state that is not part of the grid.
///
/// The engine's dirty tracking does not cover any of this, so a capture
/// compares it against the previous one to decide whether a frame that left
/// the grid alone is still worth publishing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenState {
    /// Where the cursor is and how it looks.
    pub cursor: Cursor,
    /// Window title, with control characters removed.
    pub title: String,
    /// Working directory as an absolute path, with control characters
    /// removed.
    pub pwd: String,
}

/// What a snapshot says about one row.
///
/// Selection lives here rather than in the cells. A renderer's line cache is
/// keyed on cell contents, so a selection inside a cell would throw the whole
/// cache away on every drag.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Row {
    /// A bit set of `KtRowFlag` values.
    pub flags: u8,
    /// First selected column, inclusive. Only meaningful with the selected
    /// flag set.
    pub selection_start: u16,
    /// Last selected column, inclusive. Only meaningful with the selected
    /// flag set.
    pub selection_end: u16,
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
    /// Whether a selection exists at all.
    ///
    /// This is not the same as no row being selected: a selection scrolled
    /// out of the viewport still exists, and the two states are told apart
    /// here rather than by looking at the rows.
    pub has_selection: bool,
    /// Cursor, title and working directory.
    pub screen: ScreenState,
    /// `rows * cols` cells in row-major order.
    pub cells: Vec<Cell>,
    /// One entry per row.
    pub row_state: Vec<Row>,
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
        // Both layers have to move together. Taking on rows while leaving the
        // global level clean would tell a consumer that reads only that level
        // to skip a redraw those rows are asking for.
        self.dirty = self.dirty.max(dropped.dirty);
        for (row, dropped) in self.row_state.iter_mut().zip(&dropped.row_state) {
            row.flags |= dropped.flags & RowFlag::Dirty as u8;
        }
    }
}

/// Flatten the terminal's render state into a snapshot.
///
/// Returns `Ok(None)` when nothing changed since the last capture, so a
/// caller publishes at most once per unit of work and never for a frame that
/// would be identical. `previous` is the screen state of the last capture:
/// the engine's dirty tracking does not cover it, so a title or cursor move
/// on an otherwise still screen would go unpublished without it.
pub(crate) fn capture(
    render: &mut RenderState<'static>,
    terminal: &Terminal<'static, 'static>,
    previous: &ScreenState,
) -> Result<Option<Snapshot>> {
    let frame = render.update(terminal)?;
    let dirty = Dirty::from(frame.dirty()?);
    let screen = screen_state_of(&frame, terminal)?;
    if dirty == Dirty::Clean && screen == *previous {
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
    let mut row_state = vec![Row::default(); usize::from(rows)];
    let mut graphemes = Vec::new();
    let mut cluster = Vec::new();

    let mut row_iter = RowIterator::new()?;
    let mut cell_iter = CellIterator::new()?;
    let mut rows_iteration = row_iter.update(&frame)?;
    let mut y = 0usize;
    while let Some(row) = rows_iteration.next() {
        row_state[y] = row_state_of(row.dirty()?, row.raw_row()?.is_wrapped()?, row.selection()?);
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
        // The caller fills this in: whether a selection exists is session
        // state, not something the render state can be asked.
        has_selection: false,
        screen,
        cells,
        row_state,
        graphemes,
    }))
}

fn row_state_of(dirty: bool, wrapped: bool, selection: Option<RowSelection>) -> Row {
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

    Row {
        flags,
        selection_start: selection.map_or(0, |range| range.start_x),
        selection_end: selection.map_or(0, |range| range.end_x),
    }
}

fn screen_state_of(
    frame: &Frame<'_, '_>,
    terminal: &Terminal<'static, 'static>,
) -> Result<ScreenState> {
    let position = frame.cursor_viewport()?;
    Ok(ScreenState {
        cursor: Cursor {
            x: position.map_or(0, |at| at.x),
            y: position.map_or(0, |at| at.y),
            // A cursor outside the viewport cannot be drawn either.
            visible: position.is_some() && frame.cursor_visible()?,
            shape: frame.cursor_visual_style()?.into(),
        },
        title: without_control_characters(terminal.title()?),
        pwd: without_control_characters(&path_of(terminal.pwd()?)),
    })
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
