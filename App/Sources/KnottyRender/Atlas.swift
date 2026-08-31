import CoreGraphics
import CoreText

/// Which page a slot was baked into.
///
/// Two, because colour costs four times what coverage does and a screen is
/// mostly coverage: mixing them would make every letter pay what an emoji
/// costs. cf. 04-renderer R6.
public enum AtlasPage: Int, Sendable {
    /// Grayscale. One byte a pixel, tinted with the cell's foreground.
    case coverage
    /// Colour. Four bytes a pixel, premultiplied, drawn as it was baked.
    case color

    /// How many bytes one of this page's pixels is.
    public var bytesPerPixel: Int { self == .coverage ? 1 : 4 }
}

/// The pages glyphs are baked into, and the only state the renderer keeps
/// between frames.
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
/// A glyph and not a codepoint is what a fast-path slot is found by, because
/// shaping is what decides which glyph a cell draws and two cells holding the
/// same codepoint need not draw the same one. A slow-path slot is found by
/// the cluster instead, which is the same key the shaping cache takes and for
/// the same reason: what the cascade and the mark positions come to is a
/// property of the whole cluster and of nothing smaller. cf. 04-renderer R3.
///
/// Growing the pages and emptying them are M4's; these fill up and then
/// answer nothing. cf. 04-renderer R7.
final class Atlas {
    /// A page side, in device pixels.
    static let side: Int32 = 1024

    /// What a cell asks the pages for.
    ///
    /// The style is part of every key. A glyph id means nothing without the
    /// face it came from — two faces of a family number their glyphs
    /// differently — and a cluster the cascade laid out in the bold face is
    /// not the raster the regular one would have given. This is the axis M2
    /// deferred, and the shaping cache is keyed the same way for the same
    /// reason. cf. 04-renderer R3.
    enum Request: Hashable {
        /// One glyph the fast or ligature path already chose, in the face it
        /// chose it from.
        case glyph(CGGlyph, FontStyle)
        /// A whole cluster, laid out by Core Text across the cells it owns.
        case cluster(String, cells: Int32, FontStyle)
    }

    /// Where a glyph sits on the pages and how the quad that shows it stands
    /// against its cell.
    struct Slot {
        let x: Int32
        let y: Int32
        /// The slot's width, in device pixels. Its height is one cell.
        let width: Int32
        /// How far left of the cell's origin the quad starts. Zero or less,
        /// because a glyph's ink can begin before the cell that draws it.
        let offsetX: Int32
        /// Which page it was baked into.
        let page: AtlasPage
    }

    /// Where the next slot goes on one page. One of these per page: a page is
    /// a texture of its own, so a shelf on one says nothing about the other.
    private struct Shelf {
        var x: Int32 = 0
        var y: Int32 = 0
    }

    private let metrics: CellMetrics
    private let faces: Faces
    /// The baseline, measured up from the bottom of the cell as Core Graphics
    /// measures everything.
    private let baseline: CGFloat
    /// One slot's worth of grayscale, cleared and redrawn for each bake. Wide
    /// enough for the overhang the face's own GSUB said to expect, and made
    /// again when a glyph wants more than that — which the hint being a hint
    /// and not a bound is what allows. cf. adr/0016.
    private var scratch: CGContext

    private var slots: [Request: Slot] = [:]
    /// One shelf per page, in the order ``AtlasPage`` numbers them.
    private var shelves = [Shelf(), Shelf()]

    init(metrics: CellMetrics, faces: Faces) {
        self.metrics = metrics
        self.faces = faces
        // The primary face's, for every face: a slot is one cell tall and the
        // rows of a screen share one baseline, so a bold face with a deeper
        // descent draws on the grid the regular one settled rather than on a
        // grid of its own. cf. 04-renderer R4.
        baseline = CTFontGetDescent(faces[.regular].font).rounded(.up)

        // Each face's own two hints, whichever of the eight is largest: how
        // far its ink leaves a cell, and how many cells one of its rules can
        // fold into a mark. Neither is a bound — a glyph past both makes the
        // scratch again. cf. adr/0016.
        let hint = FontStyle.allCases.map { style -> Int32 in
            let ligatures = faces[style].ligatures
            return max(
                1 + Int32(ligatures.leftOverhang.rounded(.up))
                    + Int32(ligatures.rightOverhang.rounded(.up)),
                Int32(ligatures.input)
            )
        }.max() ?? 1
        scratch = Self.context(width: metrics.width * hint, height: metrics.height, page: .coverage)
    }

    /// A slot-sized context of the page's own shape.
    ///
    /// Grayscale rather than alpha-only for a coverage page: white on black is
    /// coverage all the same, and it is the shape Core Graphics is happiest
    /// drawing text into. Premultiplied RGBA for a colour one, which is the
    /// only shape it draws colour glyphs into at all. Let it choose the row
    /// stride either way — a bake copies out row by row.
    private static func context(width: Int32, height: Int32, page: AtlasPage) -> CGContext {
        let space = page == .coverage ? CGColorSpaceCreateDeviceGray() : CGColorSpaceCreateDeviceRGB()
        let alpha =
            page == .coverage
            ? CGImageAlphaInfo.none.rawValue : CGImageAlphaInfo.premultipliedLast.rawValue
        guard
            let context = CGContext(
                data: nil,
                width: Int(width),
                height: Int(height),
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: space,
                bitmapInfo: alpha
            )
        else {
            preconditionFailure("no \(page) context for a \(width)x\(height) slot")
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

    /// Where what a cell asked for sits on the pages, baking it if this is the
    /// first ask and appending what was baked to `updates`.
    ///
    /// Answers nil when the page has no room left, which is the caller's cue
    /// to leave the cell to its background.
    func slot(for request: Request, updates: inout [AtlasUpdate]) -> Slot? {
        if let slot = slots[request] { return slot }

        // Where it goes is settled before it is drawn, so that a page with no
        // room left costs a measurement rather than a raster on every frame
        // the cell is on screen. Both arms measure without rastering: a
        // glyph's ink from its own bounds, a cluster's page from which fonts
        // the cascade answered its line with.
        switch request {
        case .glyph(let glyph, let style):
            let (offsetX, width) = span(of: glyph, in: style)
            guard let slot = place(width: width, offsetX: offsetX, page: .coverage) else {
                return nil
            }
            let pixels = bake(glyph, width: width, offsetX: offsetX, in: style)
            return keep(slot, for: request, pixels: pixels, into: &updates)
        case .cluster(let text, let cells, let style):
            let line = faces[style].line(for: text)
            let page: AtlasPage = FontFace.drawsInColor(line) ? .color : .coverage
            let width = cells * metrics.width
            guard let slot = place(width: width, offsetX: 0, page: page) else { return nil }
            let pixels = bake(line, width: width, page: page)
            return keep(slot, for: request, pixels: pixels, into: &updates)
        }
    }

    /// Walk the page's shelf far enough to fit `width`, or answer nil when it
    /// has run out — of this shelf and of the ones below it, or of the page
    /// altogether for something wider than one.
    private func place(width: Int32, offsetX: Int32, page: AtlasPage) -> Slot? {
        guard width <= Self.side else { return nil }
        var shelf = shelves[page.rawValue]
        if shelf.x + width > Self.side {
            shelf.x = 0
            shelf.y += metrics.height
        }
        guard shelf.y + metrics.height <= Self.side else { return nil }

        let slot = Slot(x: shelf.x, y: shelf.y, width: width, offsetX: offsetX, page: page)
        shelf.x += width
        shelves[page.rawValue] = shelf
        return slot
    }

    /// Remember a slot, and say what has to reach its page before it is drawn.
    private func keep(
        _ slot: Slot, for request: Request, pixels: [UInt8], into updates: inout [AtlasUpdate]
    ) -> Slot {
        slots[request] = slot
        updates.append(
            AtlasUpdate(
                x: slot.x, y: slot.y, width: slot.width, page: slot.page, pixels: pixels
            )
        )
        return slot
    }

    /// The whole cells a glyph's ink needs: where they start against its own
    /// cell, and how wide they are.
    private func span(of glyph: CGGlyph, in style: FontStyle) -> (offsetX: Int32, width: Int32) {
        var one = glyph
        var box = CGRect.zero
        CTFontGetBoundingRectsForGlyphs(faces[style].font, .horizontal, &one, &box, 1)
        guard !box.isNull, !box.isEmpty else { return (0, metrics.width) }

        let cell = CGFloat(metrics.width)
        let first = min(0, Int32((box.minX / cell).rounded(.down)))
        let last = max(1, Int32((box.maxX / cell).rounded(.up)))
        return (first * metrics.width, (last - first) * metrics.width)
    }

    /// One slot's worth of coverage for a single glyph of the primary face.
    ///
    /// The pen starts `offsetX` to the right of the slot, which is to say as
    /// far right as the glyph's ink reaches left of its own cell: a slot holds
    /// the whole mark, and a face that draws its ligature on the last cell
    /// reaches back over the ones before it. cf. adr/0016.
    private func bake(
        _ glyph: CGGlyph, width: Int32, offsetX: Int32, in style: FontStyle
    ) -> [UInt8] {
        if scratch.width < Int(width) {
            scratch = Self.context(width: width, height: metrics.height, page: .coverage)
        }
        clear(scratch)

        var one = glyph
        var origin = CGPoint(x: CGFloat(-offsetX), y: baseline)
        CTFontDrawGlyphs(faces[style].font, &one, &origin, 1, scratch)
        return copy(scratch, width: Int(width))
    }

    /// One slot's worth of a whole cluster, centred across the cells it owns.
    ///
    /// The cell is not negotiable and the glyph is: the primary font settles
    /// the grid alone, so a cluster the cascade drew in another font is
    /// centred in the cells the engine gave it and scaled down when its ink
    /// wants more than that. Scaled rather than clipped — a clipped emoji is
    /// a defect and a smaller one is not. cf. 04-renderer R4.
    private func bake(_ line: CTLine, width: Int32, page: AtlasPage) -> [UInt8] {
        let context = Self.context(width: width, height: metrics.height, page: page)
        clear(context)

        // Image bounds rather than the typographic ones, because what is being
        // centred is the ink. A colour emoji is a bitmap with no outline to
        // measure, and the typographic bounds are what is left for one.
        var bounds = CTLineGetImageBounds(line, context)
        if bounds.isNull || bounds.isEmpty {
            bounds = CTLineGetBoundsWithOptions(line, .excludeTypographicLeading)
        }
        guard !bounds.isNull, bounds.width > 0, bounds.height > 0 else {
            return copy(context, width: Int(width))
        }

        let slot = CGSize(width: CGFloat(width), height: CGFloat(metrics.height))
        var scale = min(1, slot.width / bounds.width)
        // The baseline is kept where the rest of the row keeps it, and given
        // up only when the ink would leave the cell — a slot is one cell tall,
        // so ink that does not fit vertically is ink that would be cut off.
        var y = baseline
        if baseline + scale * bounds.maxY > slot.height || baseline + scale * bounds.minY < 0 {
            scale = min(scale, slot.height / bounds.height)
            y = (slot.height - scale * bounds.height) / 2 - scale * bounds.minY
        }
        let x = (slot.width - scale * bounds.width) / 2 - scale * bounds.minX

        context.translateBy(x: x, y: y)
        context.scaleBy(x: scale, y: scale)
        context.textPosition = .zero
        CTLineDraw(line, context)
        return copy(context, width: Int(width))
    }

    /// Empty the slot and leave the pen white, which is what a coverage page
    /// reads as ink and what a colour page's own colours ignore.
    ///
    /// A coverage page has no alpha to clear, so it is painted black instead:
    /// on it, black is the absence of a letter rather than a colour. Which of
    /// the two a context is, it says itself.
    private func clear(_ context: CGContext) {
        let slot = CGRect(x: 0, y: 0, width: context.width, height: context.height)
        if context.alphaInfo == .none {
            context.setFillColor(gray: 0, alpha: 1)
            context.fill(slot)
        } else {
            context.clear(slot)
        }
        context.setFillColor(gray: 1, alpha: 1)
    }

    /// One slot's worth of pixels, row-major and tightly packed.
    private func copy(_ context: CGContext, width: Int) -> [UInt8] {
        guard let data = context.data else {
            preconditionFailure("the scratch context lost its backing store")
        }
        let height = Int(metrics.height)
        let row = width * context.bitsPerPixel / 8
        let stride = context.bytesPerRow
        var pixels = [UInt8](repeating: 0, count: row * height)
        pixels.withUnsafeMutableBufferPointer { out in
            // A bitmap context is stored top-down however its coordinates
            // run, so the copy is a copy: only the row stride differs.
            for line in 0..<height {
                let source = data.advanced(by: line * stride)
                out.baseAddress!.advanced(by: line * row)
                    .update(from: source.assumingMemoryBound(to: UInt8.self), count: row)
            }
        }
        return pixels
    }
}
