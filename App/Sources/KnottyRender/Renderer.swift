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
public struct CellMetrics: Sendable {
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

    /// Measure the grid the system's fixed-pitch font makes at this size.
    ///
    /// The primary font decides the cell alone, so this is the whole of where
    /// the numbers come from. cf. 04-renderer R4.
    public static func system(pointSize: Double, scale: Double) -> CellMetrics {
        let pixelSize = pointSize * scale
        let font = systemFont(pixelSize: pixelSize)

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

    /// The one font this milestone draws with. Configuration is M4's, so the
    /// face is a constant and the size is the only dial.
    static func systemFont(pixelSize: Double) -> CTFont {
        guard let font = CTFontCreateUIFontForLanguage(.userFixedPitch, CGFloat(pixelSize), nil)
        else {
            preconditionFailure("the system has no fixed-pitch font")
        }
        return font
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

/// One atlas quad, always exactly its cell — which is why nothing here says
/// how big it is.
public struct GlyphInstance {
    /// The cell's origin, in device pixels.
    public let x: Int32
    public let y: Int32
    /// Where the glyph was baked on the page, in device pixels.
    public let atlasX: Int32
    public let atlasY: Int32
    /// What the shader tints the coverage with.
    public let color: Rgb
}

/// One glyph, newly baked, and where on the page it goes.
public struct AtlasUpdate {
    /// What was baked. The uploader has no use for it; a reader of a frame
    /// does, and it is the only name a quad's coverage has.
    public let codepoint: UInt32
    public let x: Int32
    public let y: Int32
    /// One cell of coverage, row-major and tightly packed.
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
    private let atlas: Atlas
    /// How heavy the cursor's stroke is, in device pixels. One number for
    /// the bar and the underline both: a stroke weighs the same whichever
    /// way it runs.
    private let cursorStroke: Int32

    /// A page side, in device pixels. What an atlas coordinate is measured
    /// against, and so what the uploader needs to make a texture the size of
    /// the page the renderer is packing into.
    public var atlasSide: Int32 { Atlas.side }

    public init(metrics: CellMetrics) {
        self.metrics = metrics
        atlas = Atlas(metrics: metrics)
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

                // An overflowed cluster holds a table index rather than a
                // codepoint, and an index can look like a letter. Resolving
                // one is M3's; here it draws as its background, as a
                // concealed cell does. What the cell asks of the *font* —
                // bold, italic — is not read at all: one face, coloured to
                // match. cf. 04-renderer R3.
                guard !cell.isOverflow, !cell.isInvisible,
                    let slot = atlas.slot(for: cell.codepoint, updates: &atlasUpdates)
                else { continue }
                glyphs.append(
                    GlyphInstance(
                        x: x, y: y, atlasX: slot.x, atlasY: slot.y,
                        color: index == hidden ? colors.background : colors.foreground
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
