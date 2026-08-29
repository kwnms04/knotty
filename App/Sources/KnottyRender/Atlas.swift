import CoreGraphics
import CoreText

/// The one A8 page glyphs are baked into, and the only state the renderer
/// keeps between frames.
///
/// A slot is a whole number of cells wide and one cell tall. That is what
/// keeps the packer a cursor walking shelves — shelf packing is nothing more
/// when the heights are equal, and a monospace grid makes them equal — and it
/// is also what keeps the raster out of the layout: where a glyph lands on
/// screen follows from the metrics and from the glyph's own bounds, not from
/// where the packer happened to put it. cf. 04-renderer R5, R6.
///
/// Why a slot is not always one cell: a ligature is drawn by one of the cells
/// it spans and its ink reaches across the others — three cells of it, in the
/// worst face measured. A cell-sized slot would keep the third of it that
/// lands on its own cell. cf. adr/0016.
///
/// A glyph and not a codepoint is what a slot is found by, because shaping is
/// what decides which glyph a cell draws and two cells holding the same
/// codepoint need not draw the same one. Growing the page and emptying it are
/// M4's; this one fills up and then answers nothing. cf. 04-renderer R7.
final class Atlas {
    /// A page side, in device pixels.
    static let side: Int32 = 1024

    /// Where a glyph sits on the page and how the quad that shows it stands
    /// against its cell.
    struct Slot {
        let x: Int32
        let y: Int32
        /// The slot's width, in device pixels. Its height is one cell.
        let width: Int32
        /// How far left of the cell's origin the quad starts. Zero or less,
        /// because a glyph's ink can begin before the cell that draws it.
        let offsetX: Int32
    }

    private let metrics: CellMetrics
    private let face: FontFace
    /// The baseline, measured up from the bottom of the cell as Core Graphics
    /// measures everything.
    private let baseline: CGFloat
    /// One slot's worth of grayscale, cleared and redrawn for each bake. Wide
    /// enough for the overhang the face's own GSUB said to expect, and made
    /// again when a glyph wants more than that — which the hint being a hint
    /// and not a bound is what allows. cf. adr/0016.
    private var scratch: CGContext

    private var slots: [CGGlyph: Slot] = [:]
    private var nextX: Int32 = 0
    private var nextY: Int32 = 0

    init(metrics: CellMetrics, face: FontFace) {
        self.metrics = metrics
        self.face = face
        baseline = CTFontGetDescent(face.font).rounded(.up)

        // The face's own two hints, whichever is larger: how far its ink
        // leaves a cell, and how many cells one of its rules can fold into a
        // mark. Neither is a bound — a glyph past both makes the scratch
        // again. cf. adr/0016.
        let ligatures = face.ligatures
        let hint = max(
            1 + Int32(ligatures.leftOverhang.rounded(.up))
                + Int32(ligatures.rightOverhang.rounded(.up)),
            Int32(ligatures.input)
        )
        scratch = Self.context(width: metrics.width * hint, height: metrics.height)
    }

    /// Grayscale rather than alpha-only: white on black is coverage all the
    /// same, and it is the shape Core Graphics is happiest drawing text into.
    /// Let it choose the row stride — a bake copies out row by row either way.
    private static func context(width: Int32, height: Int32) -> CGContext {
        guard
            let context = CGContext(
                data: nil,
                width: Int(width),
                height: Int(height),
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue
            )
        else {
            preconditionFailure("no grayscale context for a \(width)x\(height) slot")
        }
        // Grayscale antialiasing and one raster per glyph: macOS dropped
        // subpixel antialiasing, and subpixel positions are what integer cell
        // origins exist to avoid. cf. 04-renderer R5.
        context.setShouldAntialias(true)
        context.setShouldSmoothFonts(false)
        context.setShouldSubpixelPositionFonts(false)
        context.setShouldSubpixelQuantizeFonts(false)
        return context
    }

    /// Where `glyph` sits on the page, baking it if this is the first ask and
    /// appending what was baked to `updates`.
    ///
    /// Answers nil when the page has no room left, which is the caller's cue
    /// to leave the cell to its background.
    func slot(for glyph: CGGlyph, updates: inout [AtlasUpdate]) -> Slot? {
        if let slot = slots[glyph] { return slot }

        // The width is looked at before the shelf is, so that a glyph too
        // wide for the page cannot cost a shelf every frame it is asked for.
        let (offsetX, width) = span(of: glyph)
        guard width <= Self.side else { return nil }
        if nextX + width > Self.side {
            nextX = 0
            nextY += metrics.height
        }
        guard nextY + metrics.height <= Self.side else { return nil }

        let slot = Slot(x: nextX, y: nextY, width: width, offsetX: offsetX)
        nextX += width
        slots[glyph] = slot
        updates.append(
            AtlasUpdate(x: slot.x, y: slot.y, width: width, coverage: bake(glyph, slot: slot))
        )
        return slot
    }

    /// The whole cells a glyph's ink needs: where they start against its own
    /// cell, and how wide they are.
    private func span(of glyph: CGGlyph) -> (offsetX: Int32, width: Int32) {
        var one = glyph
        var box = CGRect.zero
        CTFontGetBoundingRectsForGlyphs(face.font, .horizontal, &one, &box, 1)
        guard !box.isNull, !box.isEmpty else { return (0, metrics.width) }

        let cell = CGFloat(metrics.width)
        let first = min(0, Int32((box.minX / cell).rounded(.down)))
        let last = max(1, Int32((box.maxX / cell).rounded(.up)))
        return (first * metrics.width, (last - first) * metrics.width)
    }

    /// One slot's worth of coverage, row-major and tightly packed.
    private func bake(_ glyph: CGGlyph, slot: Slot) -> [UInt8] {
        let width = Int(slot.width)
        let height = Int(metrics.height)
        if scratch.width < width {
            scratch = Self.context(width: slot.width, height: metrics.height)
        }

        scratch.setFillColor(gray: 0, alpha: 1)
        scratch.fill(CGRect(x: 0, y: 0, width: scratch.width, height: height))
        scratch.setFillColor(gray: 1, alpha: 1)

        var one = glyph
        var origin = CGPoint(x: CGFloat(-slot.offsetX), y: baseline)
        CTFontDrawGlyphs(face.font, &one, &origin, 1, scratch)

        guard let data = scratch.data else {
            preconditionFailure("the scratch context lost its backing store")
        }
        let stride = scratch.bytesPerRow
        var coverage = [UInt8](repeating: 0, count: width * height)
        coverage.withUnsafeMutableBufferPointer { out in
            // A bitmap context is stored top-down however its coordinates
            // run, so the copy is a copy: only the row stride differs.
            for row in 0..<height {
                let source = data.advanced(by: row * stride)
                out.baseAddress!.advanced(by: row * width)
                    .update(from: source.assumingMemoryBound(to: UInt8.self), count: width)
            }
        }
        return coverage
    }
}
