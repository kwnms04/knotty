import CoreGraphics
import CoreText

/// The one A8 page glyphs are baked into, and the only state the renderer
/// keeps between frames.
///
/// Every slot is a whole cell. That is what makes the packer a cursor walking
/// shelves — shelf packing is nothing more when the heights are equal, and a
/// monospace grid makes them equal — and it is also what keeps the raster out
/// of the layout: a glyph quad is always its cell, so where a glyph lands on
/// screen follows from the metrics alone and not from what this machine's
/// font rasterizer measured. cf. 04-renderer R5, R6.
///
/// ASCII is the whole of what goes in. Ninety-five slots cannot fill a page
/// of this size, so there is no path here that grows one or empties one.
/// cf. 04-renderer R6, R7.
final class Atlas {
    /// A page side, in device pixels.
    static let side: Int32 = 1024

    /// The first and last codepoints that get baked. Space is left out along
    /// with the control characters: it rasters to nothing, and a terminal
    /// screen is mostly spaces.
    static let bakeable: ClosedRange<UInt32> = 0x21...0x7E

    private let metrics: CellMetrics
    private let font: CTFont
    /// The baseline, measured up from the bottom of the cell as Core Graphics
    /// measures everything.
    private let baseline: CGFloat
    /// One cell's worth of grayscale, cleared and redrawn for each bake.
    private let scratch: CGContext

    private var slots: [UInt32: (x: Int32, y: Int32)] = [:]
    private var nextX: Int32 = 0
    private var nextY: Int32 = 0

    init(metrics: CellMetrics) {
        self.metrics = metrics
        font = CellMetrics.systemFont(pixelSize: metrics.fontPixelSize)
        baseline = CTFontGetDescent(font).rounded(.up)

        // Grayscale rather than alpha-only: white on black is coverage all
        // the same, and it is the shape Core Graphics is happiest drawing
        // text into. Let it choose the row stride — a bake copies out row by
        // row either way.
        guard
            let scratch = CGContext(
                data: nil,
                width: Int(metrics.width),
                height: Int(metrics.height),
                bitsPerComponent: 8,
                bytesPerRow: 0,
                space: CGColorSpaceCreateDeviceGray(),
                bitmapInfo: CGImageAlphaInfo.none.rawValue
            )
        else {
            preconditionFailure("no grayscale context for a \(metrics.width)x\(metrics.height) cell")
        }
        // Grayscale antialiasing and one raster per glyph: macOS dropped
        // subpixel antialiasing, and subpixel positions are what integer cell
        // origins exist to avoid. cf. 04-renderer R5.
        scratch.setShouldAntialias(true)
        scratch.setShouldSmoothFonts(false)
        scratch.setShouldSubpixelPositionFonts(false)
        scratch.setShouldSubpixelQuantizeFonts(false)
        self.scratch = scratch
    }

    /// Where `codepoint` sits on the page, baking it if this is the first ask
    /// and appending what was baked to `updates`.
    ///
    /// Answers nil for anything outside what this milestone draws, which is
    /// the caller's cue to leave the cell to its background.
    func slot(for codepoint: UInt32, updates: inout [AtlasUpdate]) -> (x: Int32, y: Int32)? {
        guard Self.bakeable.contains(codepoint) else { return nil }
        if let slot = slots[codepoint] { return slot }

        if nextX + metrics.width > Self.side {
            nextX = 0
            nextY += metrics.height
        }
        // ASCII cannot reach this, but a page that has run out is still a
        // page that cannot answer.
        guard nextY + metrics.height <= Self.side else { return nil }

        let slot = (x: nextX, y: nextY)
        nextX += metrics.width
        slots[codepoint] = slot
        updates.append(
            AtlasUpdate(codepoint: codepoint, x: slot.x, y: slot.y, coverage: bake(codepoint))
        )
        return slot
    }

    /// One cell's worth of coverage, row-major and tightly packed.
    private func bake(_ codepoint: UInt32) -> [UInt8] {
        let width = Int(metrics.width)
        let height = Int(metrics.height)

        let cell = CGRect(x: 0, y: 0, width: width, height: height)
        scratch.setFillColor(gray: 0, alpha: 1)
        scratch.fill(cell)
        scratch.setFillColor(gray: 1, alpha: 1)

        var glyph = CGGlyph()
        // A codepoint this range holds is one UTF-16 unit, and a font with no
        // glyph for it answers with the missing-glyph one, which draws as
        // nothing here.
        var unit = UniChar(codepoint)
        if CTFontGetGlyphsForCharacters(font, &unit, &glyph, 1) {
            var origin = CGPoint(x: 0, y: baseline)
            CTFontDrawGlyphs(font, &glyph, &origin, 1, scratch)
        }

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
