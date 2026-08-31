import CoreGraphics
import CoreText

import KnottySession

/// What the grid measures, in device pixels.
///
/// Integers, because the cell origin and the cell width are snapped to device
/// pixels and a type that cannot hold a fraction is the shortest way to say
/// so. The snapping happens once, where the font's fractional advance meets
/// the grid — which is ``system(pointSize:scale:)`` — and never again.
/// cf. 04-renderer R5.
public struct CellMetrics: Sendable, Equatable {
    /// Cell width in device pixels.
    public let width: Int32
    /// Cell height in device pixels.
    public let height: Int32
    /// The size glyphs are baked at, in device pixels.
    public let fontPixelSize: Double

    public init(width: Int32, height: Int32, fontPixelSize: Double) {
        self.width = width
        self.height = height
        self.fontPixelSize = fontPixelSize
    }

    /// Measure the grid the primary font makes at this size.
    ///
    /// The primary font decides the cell alone, so this is the whole of where
    /// the numbers come from. cf. 04-renderer R4.
    public static func system(
        pointSize: Double, scale: Double, name: String? = FontFace.preferredName
    ) -> CellMetrics {
        let pixelSize = pointSize * scale
        let font = FontFace.base(pixelSize: pixelSize, name: name)

        // Monospace: every glyph advances the same, so one stands for all.
        var glyph = CGGlyph()
        var unit = UniChar(UInt8(ascii: "M"))
        var advance = CGSize.zero
        if CTFontGetGlyphsForCharacters(font, &unit, &glyph, 1) {
            CTFontGetAdvancesForGlyphs(font, .horizontal, &glyph, &advance, 1)
        }
        let lineHeight = CTFontGetAscent(font) + CTFontGetDescent(font) + CTFontGetLeading(font)

        return CellMetrics(
            width: Int32(advance.width.rounded(.up)),
            height: Int32(lineHeight.rounded(.up)),
            fontPixelSize: pixelSize
        )
    }
}

/// A solid rectangle: one per cell, then one for the cursor.
///
/// Device pixels throughout, with the origin at the top left of the grid.
public struct BackgroundInstance {
    public let x: Int32
    public let y: Int32
    public let width: Int32
    public let height: Int32
    public let color: Rgb
}

/// Which of the three paths chose a cell's glyph.
///
/// A judgement about the screen and not about the font, which is what makes
/// it something a golden can hold: a glyph id moves with the font's version,
/// and the path taken does not. cf. 04-renderer R3.
public enum GlyphPath: String, Sendable {
    /// The cell's own codepoint, looked up in the atlas and nothing more.
    case fast
    /// A run of cells shaped together, so that the glyph a cell draws is one
    /// its neighbours had a say in.
    case ligature
    /// A cluster Core Text laid out whole, because no arrangement of glyphs
    /// on cells would have said what it says: a mark on its base, a sequence
    /// joined into one emoji, a character the primary face does not have.
    case slow
}

/// One atlas quad: a cell, and the glyph that cell draws.
///
/// The quad is not the cell. A ligature is drawn by one of the cells it spans
/// and reaches across the others, so a quad starts at ``offsetX`` from the
/// cell's origin and is ``width`` wide — one cell tall either way, which is
/// why nothing here says a height.
public struct GlyphInstance {
    /// The cell's origin, in device pixels.
    public let x: Int32
    public let y: Int32
    /// Where the glyph was baked on the page, in device pixels.
    public let atlasX: Int32
    public let atlasY: Int32
    /// The quad against the cell: where it starts, and how wide it is.
    public let offsetX: Int32
    public let width: Int32
    /// What the shader tints the coverage with. A colour glyph brings its own
    /// colours and this is not applied to it.
    public let color: Rgb
    /// Which page the slot is on.
    public let page: AtlasPage
    /// Which of the four faces drew it. A judgement about the cell's
    /// attributes and not about the font, so a golden can hold it.
    public let style: FontStyle
    /// Which path chose this glyph, and where the cell sits in what that path
    /// looked at: the cell's index into the run that was shaped, and how many
    /// cells that run was. Both are one for the fast path, which looks at a
    /// cell alone.
    public let path: GlyphPath
    public let cellIndex: Int32
    public let cluster: Int32
}

/// One glyph, newly baked, and where on which page it goes.
public struct AtlasUpdate {
    public let x: Int32
    public let y: Int32
    /// The slot's width in device pixels; its height is one cell.
    public let width: Int32
    /// Which page the bytes belong to, and so how many of them a pixel is.
    public let page: AtlasPage
    /// The slot's pixels, row-major and tightly packed.
    public let pixels: [UInt8]
}

/// What one snapshot draws as.
public struct Frame {
    /// The background pass, in draw order: every cell, then the cursor over
    /// whichever cell it sits on. One buffer and so one draw call, which is
    /// what keeps a cursor from costing a pass of its own.
    public let backgrounds: [BackgroundInstance]
    /// The cursor's rectangle — the last of ``backgrounds``, named, so that
    /// a reader of a frame does not have to know it is last. Nil when there
    /// is no cursor to draw.
    public let cursor: BackgroundInstance?
    /// The glyph pass, in row-major order over the cells that have a glyph.
    public let glyphs: [GlyphInstance]
    /// What has to reach the page before this frame is drawn. Empty once the
    /// screen has stopped showing letters it has not shown before.
    public let atlasUpdates: [AtlasUpdate]
}

/// How often the line cache answered, since the renderer was made.
///
/// **The unit is a row**, not a run and not a cluster: a hit is a row that
/// reached the screen without being shaped at all, and a miss is a row that
/// had to be placed. That is the number the line cache exists to move — what
/// a terminal redraws is mostly rows it has already drawn — and it is what
/// R3's rate is read off here.
///
/// A run that appears on two rows of different content is shaped once per
/// row, because there is one cache and its key is the row. A second cache
/// keyed on the run would catch that; nothing has shown it is worth keeping
/// two, and M5 measures against this one before anything is added.
///
/// The rate B3 asks for is measured in M5; what M3 owes is the counter it
/// would be read off. cf. 04-renderer R3.
public struct CacheStats: Sendable {
    /// Rows whose placement was already known.
    public let hits: Int
    /// Rows that had to be placed, and so shaped.
    public let misses: Int

    /// Hits over rows asked for, and one when nothing has been asked yet.
    public var hitRate: Double {
        hits + misses == 0 ? 1 : Double(hits) / Double(hits + misses)
    }
}

/// Snapshot and metrics in, instance buffers and atlas updates out.
///
/// No AppKit, no window, no GPU device: the only state is the atlas, and what
/// this answers is a function of the frame it was given. That is what lets the
/// goldens below run with neither a window nor a GPU, and what will make
/// moving this to a render thread a line of dispatch. cf. 04-renderer R9.
public final class Renderer {
    public let metrics: CellMetrics
    /// The four faces a cell can be drawn in, each with what its own GSUB
    /// said about its ligatures. The fallback needs no face of its own: the
    /// cascade is Core Text's, and the slow path asks for it by handing one
    /// of these a cluster it cannot draw. cf. 04-renderer R4.
    private let faces: Faces
    private let atlas: Atlas
    /// What every row placed so far came to, found by what the row holds.
    ///
    /// The key is the content and not the row number, which is the whole of
    /// what makes a scrolled row a hit: the same line one row up is the same
    /// line. cf. 04-renderer R2.
    private var lines: [UInt64: [Placed?]] = [:]
    private var hits = 0
    private var misses = 0

    /// How often the line cache has answered. cf. 04-renderer R3.
    public var lineCache: CacheStats { CacheStats(hits: hits, misses: misses) }

    /// ponytail: how many rows the cache keeps before it is emptied whole.
    /// No LRU — saturation is rare and an LRU is a cost on every frame, which
    /// is the trade R7 already made for the atlas. Enough for several screens
    /// and their scrollback; a screen that scrolls past it pays one frame of
    /// placement, not a defect. Per-row eviction if a profile ever says the
    /// reset frame shows.
    private static let cachedRows = 512
    /// How heavy the cursor's stroke is, in device pixels. One number for
    /// the bar and the underline both: a stroke weighs the same whichever
    /// way it runs.
    private let cursorStroke: Int32

    /// A page side, in device pixels. What an atlas coordinate is measured
    /// against, and so what an uploader needs to make a texture the size of
    /// the page the renderer packs into. Of the type and not of an instance:
    /// a page is the same size whichever renderer is filling one.
    public static var atlasSide: Int32 { Atlas.side }

    public init(metrics: CellMetrics, faces: Faces? = nil) {
        self.metrics = metrics
        self.faces = faces ?? Faces(metrics: metrics)
        atlas = Atlas(metrics: metrics, faces: self.faces)
        cursorStroke = max(1, metrics.width / 8)
    }

    /// Draw a snapshot.
    ///
    /// `cursorColor` is what the theme asked for; without one the cursor
    /// takes the colour of the text it stands on. cf. 04-renderer R1.
    ///
    /// The whole grid comes out every time — there is no partial present, and
    /// what a row that did not change saves is the shaping rather than the
    /// draw. cf. 04-renderer R2.
    public func frame(for snapshot: Snapshot, cursorColor: Rgb? = nil) -> Frame {
        let cols = Int(snapshot.cols)
        let rows = Int(snapshot.rows)

        var backgrounds: [BackgroundInstance] = []
        backgrounds.reserveCapacity(cols * rows + 1)
        var glyphs: [GlyphInstance] = []
        glyphs.reserveCapacity(cols * rows)
        var atlasUpdates: [AtlasUpdate] = []

        // The cursor is settled before the grid is walked, because the letter
        // under one that covers its whole cell would be hidden by it and so
        // is drawn in the colour the cell would have been. A bar and an
        // underline cover nothing and leave that letter alone — which is the
        // rectangle's size saying it, and not a second reading of the shape.
        // cf. 04-renderer R1.
        let cursorCell = cursorIndex(in: snapshot)
        let cursorRectangle = cursorCell.map { cell in
            self.cursorRectangle(
                for: snapshot.cursor,
                // Selected like any other cell: a block cursor takes the
                // colour of the text under it, and under a highlight that
                // text is drawn the other way round.
                color: cursorColor
                    ?? resolved(
                        snapshot.cells[cell],
                        selected: selectedColumns(row: cell / cols, of: snapshot)
                            .contains(cell % cols)
                    ).foreground
            )
        }
        let hidden = cursorRectangle.flatMap { rectangle in
            rectangle.width == metrics.width && rectangle.height == metrics.height
                ? cursorCell : nil
        }

        for row in 0..<rows {
            let placed = placement(row: row, of: snapshot)
            let selected = selectedColumns(row: row, of: snapshot)
            for col in 0..<cols {
                let index = row * cols + col
                let cell = snapshot.cells[index]
                let colors = resolved(cell, selected: selected.contains(col))
                let x = Int32(col) * metrics.width
                let y = Int32(row) * metrics.height

                backgrounds.append(
                    BackgroundInstance(
                        x: x, y: y, width: metrics.width, height: metrics.height,
                        color: colors.background
                    )
                )

                guard let placed = placed[col],
                    let slot = atlas.slot(for: placed.request, updates: &atlasUpdates)
                else { continue }
                glyphs.append(
                    GlyphInstance(
                        x: x, y: y, atlasX: slot.x, atlasY: slot.y,
                        offsetX: slot.offsetX, width: slot.width,
                        color: index == hidden ? colors.background : colors.foreground,
                        page: slot.page, style: placed.style,
                        path: placed.path, cellIndex: placed.cellIndex, cluster: placed.cluster
                    )
                )
            }
        }

        if let cursorRectangle {
            backgrounds.append(cursorRectangle)
        }

        return Frame(
            backgrounds: backgrounds, cursor: cursorRectangle, glyphs: glyphs,
            atlasUpdates: atlasUpdates
        )
    }

    /// What one cell draws, and which path said so.
    private struct Placed {
        let request: Atlas.Request
        let style: FontStyle
        let path: GlyphPath
        let cellIndex: Int32
        let cluster: Int32
    }

    /// What one row places as, from the cache when the row has been seen
    /// before and from ``place(row:of:)`` when it has not.
    ///
    /// This is where the shaping is skipped. What comes back holds the atlas
    /// requests and not the slots, so a cached row still asks the pages —
    /// which is a dictionary lookup, and is also what keeps a row that
    /// scrolled back into view from missing its bake. cf. 04-renderer R2.
    private func placement(row: Int, of snapshot: Snapshot) -> [Placed?] {
        let key = key(row: row, of: snapshot)
        // The length is checked and not assumed: a hash names a row and two
        // rows can collide, and a row of the wrong width is the one collision
        // that would be read past the end of rather than drawn wrongly.
        if let cached = lines[key], cached.count == Int(snapshot.cols) {
            hits += 1
            return cached
        }
        misses += 1
        let placed = place(row: row, of: snapshot)
        if lines.count >= Self.cachedRows { lines.removeAll(keepingCapacity: true) }
        lines[key] = placed
        return placed
    }

    /// What one row's placement depends on, hashed.
    ///
    /// The cells and nothing beside them. **A selection is not in here**, and
    /// that is why it is carried on the row rather than in the cell: a
    /// selection inside a cell would empty this cache on every mouse move.
    /// Neither are the row's coordinates, which is what makes a line that
    /// scrolled the line it was. cf. 02-ffi, 04-renderer R2.
    ///
    /// The colours are left out for a different reason: they are applied to
    /// the quad after the placement rather than by it, so a row that changed
    /// only its palette places the same way.
    ///
    /// An overflowed cell contributes the codepoints of its cluster rather
    /// than the index it holds. An index is a place in this snapshot's
    /// grapheme table and means something else in the next one's.
    ///
    /// The column count is in the key because the cache outlives a resize,
    /// and a row of the wrong length is not a row this frame can read.
    ///
    /// ponytail: a 64-bit hash, so two rows that collide draw as one — the
    /// width is checked on the way out, so the worst of one is a wrong row
    /// and never a read past its end. The alternative is keeping every row's
    /// cells to compare against, which is the grid again in the renderer.
    private func key(row: Int, of snapshot: Snapshot) -> UInt64 {
        let cols = Int(snapshot.cols)
        var hasher = Hasher()
        hasher.combine(cols)
        for col in 0..<cols {
            let cell = snapshot.cells[row * cols + col]
            hasher.combine(cell.attributes)
            if cell.isOverflow {
                hasher.combine(snapshot.codepoints(of: cell))
            } else {
                hasher.combine(cell.codepoint)
            }
        }
        return UInt64(bitPattern: Int64(hasher.finalize()))
    }

    /// Choose what every cell of one row draws, or nothing where the cell has
    /// nothing to draw.
    ///
    /// The fast path answers every cell first, because it answers at least
    /// 89% of them in every font that was measured and is a dictionary lookup
    /// when it does. What it cannot answer goes to the slow path a cell at a
    /// time. Then the cells whose glyph the font's own GSUB says a rule can
    /// replace are shaped in runs, and only those.
    /// cf. 04-renderer R3, adr/0016.
    private func place(row: Int, of snapshot: Snapshot) -> [Placed?] {
        let cols = Int(snapshot.cols)
        var placed = [Placed?](repeating: nil, count: cols)
        var participates = [Bool](repeating: false, count: cols)
        var styles = [FontStyle](repeating: .regular, count: cols)
        let text = read(row: row, of: snapshot)

        for col in 0..<cols {
            let cell = snapshot.cells[row * cols + col]
            // Which of the four faces the cell asked for. Read before the
            // invisible cell is passed over, because a cell that draws
            // nothing still ends a run: a run is shaped by one face, and the
            // cell beside it is not part of it if it wanted another.
            let style = FontStyle(bold: cell.isBold, italic: cell.isItalic)
            styles[col] = style
            guard !cell.isInvisible else { continue }

            // The fast path is the cell that reads as one character its own
            // face has a glyph for and that owns one column. Every other cell
            // that holds text is the slow path's: an overflowed cluster, a
            // character of two UTF-16 units, a wide one, or one the cascade
            // has to answer. cf. 04-renderer R3.
            let face = faces[style]
            if let scalar = text[col], let glyph = face.glyph(for: scalar.value) {
                placed[col] = Placed(
                    request: .glyph(glyph, style), style: style, path: .fast,
                    cellIndex: 0, cluster: 1
                )
                participates[col] = face.ligatures.enabled && face.participates(glyph)
                continue
            }
            guard let cluster = snapshot.text(of: cell) else { continue }
            // The cells the engine gave it, which is what the quad covers: the
            // grid is the primary font's and a fallback does not widen it.
            // cf. 04-renderer R4.
            let cells: Int32 = cell.isWide ? 2 : 1
            placed[col] = Placed(
                request: .cluster(cluster, cells: cells, style), style: style, path: .slow,
                cellIndex: 0, cluster: cells
            )
        }

        var col = 0
        while col < cols {
            guard participates[col] else {
                col += 1
                continue
            }
            var end = col + 1
            while end < cols, participates[end], styles[end] == styles[col] { end += 1 }
            shape(col..<end, text: text, styles: styles, into: &placed)
            col = end
        }
        return placed
    }

    /// One row as a shaper would read it: a character per cell, and nothing
    /// where a cell cannot be one.
    ///
    /// An empty cell reads as the space it draws as, so that a window is the
    /// line as it reads rather than the line with holes in it. An overflowed
    /// cluster holds a table index rather than a codepoint, a cell outside the
    /// basic plane is two UTF-16 units, and a wide character is one character
    /// over two cells — a window holding any of them would put the cells and
    /// the glyphs out of step, so none reads as anything and none can be in a
    /// run. Which is also what sends them to the slow path.
    private func read(row: Int, of snapshot: Snapshot) -> [Unicode.Scalar?] {
        let cols = Int(snapshot.cols)
        return (0..<cols).map { col in
            let cell = snapshot.cells[row * cols + col]
            guard !cell.isOverflow, !cell.isWide, !cell.isWideTail else { return nil }
            guard cell.codepoint != 0 else { return " " }
            return cell.codepoint <= 0xFFFF ? Unicode.Scalar(cell.codepoint) : nil
        }
    }

    /// Shape one run of cells and give each of them the glyph that came back.
    ///
    /// The window either side is the font's own — what its rules look back and
    /// ahead at before they decide — and a sub-run carrying that much context
    /// shapes identically to the whole line, which is a bound and not a rate.
    /// A window that did not come back one glyph per cell is one the grid
    /// would not survive, so those cells keep the glyphs the fast path already
    /// gave them. cf. 04-renderer R3.
    private func shape(
        _ run: Range<Int>, text: [Unicode.Scalar?], styles: [FontStyle],
        into placed: inout [Placed?]
    ) {
        let style = styles[run.lowerBound]
        let face = faces[style]
        var window = run.compactMap { text[$0] }
        guard window.count == run.count else { return }

        // The context either side is the run's own face's. A neighbour drawn
        // bold is a cell another face will shape, and putting it in this
        // window would let one face's rules decide on the other's characters.
        var from = run.lowerBound
        while run.lowerBound - from < face.ligatures.backtrack, from > 0,
            styles[from - 1] == style, let before = text[from - 1]
        {
            window.insert(before, at: 0)
            from -= 1
        }
        var to = run.upperBound
        while to - run.upperBound < face.ligatures.lookahead, to < text.count,
            styles[to] == style, let after = text[to]
        {
            window.append(after)
            to += 1
        }

        let shaped = String(String.UnicodeScalarView(window))
        guard let glyphs = face.shape(shaped), glyphs.count == to - from else { return }
        for col in run {
            placed[col] = Placed(
                request: .glyph(glyphs[col - from], style),
                style: style,
                path: .ligature,
                cellIndex: Int32(col - run.lowerBound),
                cluster: Int32(run.count)
            )
        }
    }

    /// The cell the cursor stands on, or nil when there is none to draw.
    private func cursorIndex(in snapshot: Snapshot) -> Int? {
        let cursor = snapshot.cursor
        guard cursor.visible, cursor.x < snapshot.cols, cursor.y < snapshot.rows else {
            return nil
        }
        return Int(cursor.y) * Int(snapshot.cols) + Int(cursor.x)
    }

    /// The one rectangle that is the cursor. The shapes differ in its size
    /// and in nothing else — an unfocused block has no outline that one
    /// rectangle can draw, and focus is M3's, so it draws as the block it is.
    private func cursorRectangle(for cursor: Cursor, color: Rgb) -> BackgroundInstance {
        let x = Int32(cursor.x) * metrics.width
        let y = Int32(cursor.y) * metrics.height

        switch cursor.drawnShape {
        case .bar:
            return BackgroundInstance(
                x: x, y: y, width: cursorStroke, height: metrics.height, color: color
            )
        case .underline:
            return BackgroundInstance(
                x: x, y: y + metrics.height - cursorStroke,
                width: metrics.width, height: cursorStroke, color: color
            )
        case .block, .blockHollow, .unknown:
            return BackgroundInstance(
                x: x, y: y, width: metrics.width, height: metrics.height, color: color
            )
        }
    }

    /// A cell's colours as they are drawn, which is the pair the snapshot
    /// carries unless the cell asked for them the other way round. The
    /// palette is already resolved; this is the one thing left to apply.
    ///
    /// A selected cell asks for the same swap, which is what makes the
    /// highlight the background pass draws — no colour of its own, so nothing
    /// here waits on a theme, and a cell already inverse comes back the right
    /// way up. cf. 04-renderer R1.
    private func resolved(_ cell: Cell, selected: Bool) -> (
        foreground: Rgb, background: Rgb
    ) {
        cell.isInverse != selected
            ? (cell.background, cell.foreground) : (cell.foreground, cell.background)
    }

    /// Which of a row's columns the selection covers, and none where it
    /// covers none.
    ///
    /// Read off the row rather than out of the cells, which is the whole
    /// reason the boundary carries it there: a selection inside a cell would
    /// be in the line cache's key, and every mouse move would empty the
    /// cache. cf. 02-ffi, 04-renderer R2.
    ///
    /// Half-open, and empty where the row is not selected — a range that
    /// carries its own emptiness is what keeps the caller from asking twice.
    /// The columns are inclusive on the boundary, hence the one added here.
    private func selectedColumns(row: Int, of snapshot: Snapshot) -> Range<Int> {
        let state = snapshot.rowStates[row]
        let (first, last) = (Int(state.selection_start), Int(state.selection_end))
        // The order is checked and not assumed. A row whose columns came back
        // the wrong way round is a boundary that broke its own contract, and
        // the answer to one is a row drawn unselected rather than a range that
        // cannot exist.
        guard state.isSelected, first <= last else { return 0..<0 }
        return first..<(last + 1)
    }
}
