//! What a consumer is handed: one immutable frame of terminal state.
//!
//! Types only. Filling them in is [`crate::vt`]'s, which is the one place the
//! engine is read. cf. `docs/adr/0005-flat-snapshot.md`

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
