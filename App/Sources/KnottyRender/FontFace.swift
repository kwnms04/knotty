import CoreGraphics
import CoreText
import Foundation

/// What a face's GSUB says its ligatures need, and whether they are used.
///
/// Every number here is derived at load rather than written down, because the
/// six fonts that were measured agreed on none of them — the participating
/// set ran from 13 codepoints to 271 and did not even overlap. The measured
/// worst case is an allocation hint and nothing else; a font past it costs a
/// reallocation, not a defect. cf. adr/0016.
public struct Ligatures: Sendable {
    /// The glyphs a lookup can replace. A cell holding one of these is the
    /// only kind of cell the ligature path has to look at; every other cell
    /// is an atlas lookup and nothing more.
    public let participating: Set<CGGlyph>
    /// The longest input a rule matches, in cells.
    public let input: Int
    /// How far back a rule looks before deciding, in cells.
    public let backtrack: Int
    /// How far ahead a rule looks before deciding, in cells.
    public let lookahead: Int
    /// How far a produced glyph's ink leaves its own cell, in cells. Which
    /// way it goes is the font's technique: a face that draws the ligature on
    /// the last cell overhangs left, one that draws it on the first overhangs
    /// right.
    public let leftOverhang: Double
    public let rightOverhang: Double
    /// Whether shaping the probe string returned one glyph per character.
    ///
    /// A font that folds cells breaks the grid rather than a glyph — columns
    /// stop lining up and every coordinate the terminal keeps goes with them
    /// — so the answer to one is to draw it without ligatures.
    public let preservesGrid: Bool

    /// Whether the ligature path runs for this face at all.
    public var enabled: Bool { preservesGrid && !participating.isEmpty }

    /// What no ligature at all looks like: the shape a font with neither
    /// `liga` nor `calt` answers with.
    static func none(preservesGrid: Bool) -> Ligatures {
        Ligatures(
            participating: [], input: 1, backtrack: 0, lookahead: 0,
            leftOverhang: 0, rightOverhang: 0, preservesGrid: preservesGrid
        )
    }
}

/// One loaded face: the font, what its GSUB says, and the two lookups the
/// renderer asks of it a cell at a time.
///
/// Loading walks the font's GSUB once. On a ten-megabyte font that is a
/// perceptible cost, and it is paid once per face rather than once per frame.
public final class FontFace {
    /// The face this milestone draws with. Configuration is M4's, so this is
    /// a constant — but not the system's fixed-pitch font, which carries no
    /// ligature feature at all and so could never draw one.
    public static let preferredName = "JetBrains Mono"

    /// The two features a ligature comes out of, enabled together and read
    /// together. Both, because neither alone is enough: Monaspace's ligatures
    /// do not fire under `calt` on its own and JetBrains Mono's do not fire
    /// under `liga` on its own. cf. adr/0016.
    static let features = ["liga", "calt"]

    /// What the grid is asserted with: the sequences a programming font
    /// ligates, spaced apart so that only the intended ones can join.
    ///
    /// One string, so one shaping call. What it does not touch it does not
    /// check — the fonts this guards against are ones that break a
    /// convention, not ones that hide from a probe.
    public static let gridProbe = "!= => -> <=> === // /// |> :: /* <!-- || !! ;; ... www"

    let font: CTFont
    public let ligatures: Ligatures
    /// One glyph per codepoint, looked up once. The cascade the misses need
    /// is the slow path's, which is not here yet.
    private var glyphs: [UInt32: CGGlyph] = [:]

    /// Load a face at this size, with ``features`` on and what they cover
    /// read off the same font's GSUB.
    public init(metrics: CellMetrics, name: String? = preferredName, probe: String = gridProbe) {
        font = Self.base(pixelSize: metrics.fontPixelSize, name: name)

        // The probe is shaped whether or not there was a table to read,
        // because what it asks about is the face's grid and not its
        // ligatures. A face that has none has nothing to turn off, and the
        // answer is still the truth about it.
        let grid = Self.shape(probe, with: font) != nil
        let table = CTFontCopyTable(font, CTFontTableTag(kCTFontTableGSUB), [])
            .map { [UInt8](Data(referencing: $0)) }
        guard let table, let rules = readGsub(table, tags: Set(Self.features)) else {
            ligatures = .none(preservesGrid: grid)
            return
        }
        let (left, right) = Self.overhang(font, rules.produced, cell: Double(metrics.width))
        ligatures = Ligatures(
            participating: rules.substitutable,
            input: rules.input,
            backtrack: rules.backtrack,
            lookahead: rules.lookahead,
            leftOverhang: left,
            rightOverhang: right,
            preservesGrid: grid
        )
    }

    /// The font itself, before anything is asked of its table.
    ///
    /// A name that names nothing resolves to a font that is not it, so what
    /// came back is checked against what was asked for and the system's own
    /// fixed-pitch face is the answer when they differ.
    static func base(pixelSize: Double, name: String? = preferredName) -> CTFont {
        if let name {
            let font = CTFontCreateWithName(name as CFString, CGFloat(pixelSize), nil)
            if CTFontCopyFamilyName(font) as String == name {
                return withLigatures(font, pixelSize: pixelSize)
            }
        }
        guard let font = CTFontCreateUIFontForLanguage(.userFixedPitch, CGFloat(pixelSize), nil)
        else {
            preconditionFailure("the system has no fixed-pitch font")
        }
        return withLigatures(font, pixelSize: pixelSize)
    }

    private static func withLigatures(_ font: CTFont, pixelSize: Double) -> CTFont {
        let settings = features.map { tag in
            [
                kCTFontOpenTypeFeatureTag as String: tag,
                kCTFontOpenTypeFeatureValue as String: 1,
            ]
        }
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(
            CTFontCopyFontDescriptor(font),
            [kCTFontFeatureSettingsAttribute: settings] as CFDictionary
        )
        return CTFontCreateWithFontDescriptor(descriptor, CGFloat(pixelSize), nil)
    }

    /// How far the ink of the glyphs a lookup can produce leaves its cell,
    /// left and right, in cells.
    private static func overhang(
        _ font: CTFont, _ produced: Set<CGGlyph>, cell: Double
    ) -> (Double, Double) {
        guard !produced.isEmpty, cell > 0 else { return (0, 0) }
        var glyphs = Array(produced)
        var bounds = [CGRect](repeating: .zero, count: glyphs.count)
        CTFontGetBoundingRectsForGlyphs(font, .horizontal, &glyphs, &bounds, glyphs.count)
        var left = 0.0
        var right = 0.0
        for box in bounds where !box.isNull && !box.isEmpty {
            left = max(left, -box.minX / cell)
            right = max(right, (box.maxX - cell) / cell)
        }
        return (left, right)
    }

    /// The glyph this codepoint draws as on its own, or nil when there is
    /// nothing to draw: a blank, or a codepoint this font has no glyph for.
    ///
    /// Space is left out along with the control characters — it rasters to
    /// nothing and a terminal screen is mostly spaces.
    public func glyph(for codepoint: UInt32) -> CGGlyph? {
        guard codepoint > 0x20, !(0x7F...0x9F).contains(codepoint) else { return nil }
        if let glyph = glyphs[codepoint] { return glyph == 0 ? nil : glyph }

        var glyph = CGGlyph(0)
        if let scalar = Unicode.Scalar(codepoint) {
            var units = Array(String(scalar).utf16)
            var found = [CGGlyph](repeating: 0, count: units.count)
            // A surrogate pair maps into the first slot and leaves the second
            // zero, so taking the first is right for either length.
            CTFontGetGlyphsForCharacters(font, &units, &found, units.count)
            glyph = found[0]
        }
        glyphs[codepoint] = glyph
        return glyph == 0 ? nil : glyph
    }

    /// Whether a cell holding this glyph has to be shaped rather than looked
    /// up.
    func participates(_ glyph: CGGlyph) -> Bool {
        ligatures.participating.contains(glyph)
    }

    /// The same question asked in codepoints. The set itself is glyph ids and
    /// an id moves with the font's version, so a caller holding a character
    /// rather than a glyph asks here rather than resolving one for itself.
    public func participates(codepoint: UInt32) -> Bool {
        glyph(for: codepoint).map(participates) ?? false
    }

    /// Shape a bounded window, answering the glyph each of its characters
    /// drew as.
    ///
    /// Nil when the grid did not survive it — a glyph count that is not the
    /// character count, or a character this face did not draw itself. Either
    /// is a window the ligature path cannot place on cells, and the caller's
    /// cue to leave those cells the glyphs they already had.
    func shape(_ text: String) -> [CGGlyph]? { Self.shape(text, with: font) }

    private static func shape(_ text: String, with font: CTFont) -> [CGGlyph]? {
        let units = text.utf16.count
        let line = CTLineCreateWithAttributedString(
            NSAttributedString(
                string: text, attributes: [kCTFontAttributeName as NSAttributedString.Key: font]
            )
        )
        guard CTLineGetGlyphCount(line) == units else { return nil }

        var placed = [CGGlyph?](repeating: nil, count: units)
        for run in CTLineGetGlyphRuns(line) as? [CTRun] ?? [] {
            let attributes = CTRunGetAttributes(run) as? [CFString: Any]
            // A run in another font is the cascade having stepped in, which
            // means these glyphs are not ours to bake. Core Text puts a font
            // there and nothing else, but the dictionary is untyped and a
            // bridged cast to a Core Foundation type always succeeds — so the
            // type is asked for outright rather than assumed.
            guard let drew = attributes?[kCTFontAttributeName] as CFTypeRef?,
                CFGetTypeID(drew) == CTFontGetTypeID(),
                CTFontCopyPostScriptName(drew as! CTFont) == CTFontCopyPostScriptName(font)
            else { return nil }
            let count = CTRunGetGlyphCount(run)
            var glyphs = [CGGlyph](repeating: 0, count: count)
            var indices = [CFIndex](repeating: 0, count: count)
            CTRunGetGlyphs(run, CFRange(location: 0, length: count), &glyphs)
            CTRunGetStringIndices(run, CFRange(location: 0, length: count), &indices)
            for i in 0..<count {
                guard indices[i] >= 0, indices[i] < units else { return nil }
                placed[indices[i]] = glyphs[i]
            }
        }
        return placed.contains(nil) ? nil : placed.map { $0! }
    }
}
