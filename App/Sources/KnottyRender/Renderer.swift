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
    /// What the shader tints the coverage with.
    public let color: Rgb
    /// Which path chose this glyph, and where the cell sits in what that path
    /// looked at: the cell's index into the run that was shaped, and how many
    /// cells that run was. Both are one for the fast path, which looks at a
    /// cell alone.
    public let path: GlyphPath
    public let cellIndex: Int32
    public let cluster: Int32
}

/// One glyph, newly baked, and where on the page it goes.
public struct AtlasUpdate {
    public let x: Int32
    public let y: Int32
    /// The slot's width in device pixels; its height is one cell.
    public let width: Int32
    /// The slot's coverage, row-major and tightly packed.
    public let coverage: [UInt8]
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

/// Snapshot and metrics in, instance buffers and atlas updates out.
///
/// No AppKit, no window, no GPU device: the only state is the atlas, and what
/// this answers is a function of the frame it was given. That is what lets the
/// goldens below run with neither a window nor a GPU, and what will make
/// moving this to a render thread a line of dispatch. cf. 04-renderer R9.
public final class Renderer {
    public let metrics: CellMetrics
    /// The one face this milestone draws with, and what its GSUB said about
    /// its ligatures. Four of them is ticket 10's; a second one for the
    /// cascade is 09's.
    public let face: FontFace
    private let atlas: Atlas
    /// How heavy the cursor's stroke is, in device pixels. One number for
    /// the bar and the underline both: a stroke weighs the same whichever
    /// way it runs.
    private let cursorStroke: Int32

    /// A page side, in device pixels. What an atlas coordinate is measured
    /// against, and so what an uploader needs to make a texture the size of
    /// the page the renderer packs into. Of the type and not of an instance:
    /// a page is the same size whichever renderer is filling one.
    public static var atlasSide: Int32 { Atlas.side }

    public init(metrics: CellMetrics, face: FontFace? = nil) {
        self.metrics = metrics
        self.face = face ?? FontFace(metrics: metrics)
        atlas = Atlas(metrics: metrics, face: self.face)
        cursorStroke = max(1, metrics.width / 8)
    }

    /// Draw a snapshot.
    ///
    /// `cursorColor` is what the theme asked for; without one the cursor
    /// takes the colour of the text it stands on. cf. 04-renderer R1.
    ///
    /// The whole grid comes out every time. The line cache R2 asks for is not
    /// here yet: what it would hold is the placement of a glyph, and on the
    /// ASCII path that placement is a lookup cheaper than the one that would
    /// find it. It arrives with the shaping it exists to save.
    /// cf. 04-renderer R2.
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
                color: cursorColor ?? resolved(snapshot.cells[cell]).foreground
            )
        }
        let hidden = cursorRectangle.flatMap { rectangle in
            rectangle.width == metrics.width && rectangle.height == metrics.height
                ? cursorCell : nil
        }

        for row in 0..<rows {
            let placed = place(row: row, of: snapshot)
            for col in 0..<cols {
                let index = row * cols + col
                let cell = snapshot.cells[index]
                let colors = resolved(cell)
                let x = Int32(col) * metrics.width
                let y = Int32(row) * metrics.height

                backgrounds.append(
                    BackgroundInstance(
                        x: x, y: y, width: metrics.width, height: metrics.height,
                        color: colors.background
                    )
                )

                guard let placed = placed[col],
                    let slot = atlas.slot(for: placed.glyph, updates: &atlasUpdates)
                else { continue }
                glyphs.append(
                    GlyphInstance(
                        x: x, y: y, atlasX: slot.x, atlasY: slot.y,
                        offsetX: slot.offsetX, width: slot.width,
                        color: index == hidden ? colors.background : colors.foreground,
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
        let glyph: CGGlyph
        let path: GlyphPath
        let cellIndex: Int32
        let cluster: Int32
    }

    /// Choose a glyph for every cell of one row, or nothing where the cell
    /// has nothing to draw.
    ///
    /// The fast path answers every cell first, because it answers at least
    /// 89% of them in every font that was measured and is a dictionary lookup
    /// when it does. Then the cells whose glyph the font's own GSUB says a
    /// rule can replace are shaped in runs, and only those.
    /// cf. 04-renderer R3, adr/0016.
    private func place(row: Int, of snapshot: Snapshot) -> [Placed?] {
        let cols = Int(snapshot.cols)
        var placed = [Placed?](repeating: nil, count: cols)
        var participates = [Bool](repeating: false, count: cols)
        let text = read(row: row, of: snapshot)

        for col in 0..<cols {
            // A cell the row does not read as a character is one no path here
            // can draw: it holds a table index rather than a codepoint, or a
            // character of two UTF-16 units. Both are the slow path's, which
            // is 09's, and until then they draw as their background — as a
            // concealed cell does. What the cell asks of the *font* — bold,
            // italic — is not read at all: one face, coloured to match.
            // cf. 04-renderer R3.
            let cell = snapshot.cells[row * cols + col]
            guard text[col] != nil, !cell.isInvisible,
                let glyph = face.glyph(for: cell.codepoint)
            else { continue }
            placed[col] = Placed(glyph: glyph, path: .fast, cellIndex: 0, cluster: 1)
            participates[col] = face.participates(glyph)
        }

        guard face.ligatures.enabled else { return placed }
        var col = 0
        while col < cols {
            guard participates[col] else {
                col += 1
                continue
            }
            var end = col + 1
            while end < cols, participates[end] { end += 1 }
            shape(col..<end, text: text, into: &placed)
            col = end
        }
        return placed
    }

    /// One row as a shaper would read it: a character per cell, and nothing
    /// where a cell cannot be one.
    ///
    /// An empty cell reads as the space it draws as, so that a window is the
    /// line as it reads rather than the line with holes in it. An overflowed
    /// cluster holds a table index rather than a codepoint, and a cell outside
    /// the basic plane is two UTF-16 units — a window holding either would put
    /// the cells and the glyphs out of step, so neither reads as anything and
    /// neither can be in a run.
    private func read(row: Int, of snapshot: Snapshot) -> [Unicode.Scalar?] {
        let cols = Int(snapshot.cols)
        return (0..<cols).map { col in
            let cell = snapshot.cells[row * cols + col]
            guard !cell.isOverflow else { return nil }
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
    private func shape(_ run: Range<Int>, text: [Unicode.Scalar?], into placed: inout [Placed?]) {
        var window = run.compactMap { text[$0] }
        guard window.count == run.count else { return }

        var from = run.lowerBound
        while run.lowerBound - from < face.ligatures.backtrack, from > 0,
            let before = text[from - 1]
        {
            window.insert(before, at: 0)
            from -= 1
        }
        var to = run.upperBound
        while to - run.upperBound < face.ligatures.lookahead, to < text.count, let after = text[to] {
            window.append(after)
            to += 1
        }

        let shaped = String(String.UnicodeScalarView(window))
        guard let glyphs = face.shape(shaped), glyphs.count == to - from else { return }
        for col in run {
            placed[col] = Placed(
                glyph: glyphs[col - from],
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
    private func resolved(_ cell: Cell) -> (foreground: Rgb, background: Rgb) {
        cell.isInverse ? (cell.background, cell.foreground) : (cell.foreground, cell.background)
    }
}
