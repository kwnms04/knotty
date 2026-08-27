// PROTOTYPE — throwaway. Does a JetBrains Mono ligature change the cell count?
import CoreText
import Foundation

let url = URL(fileURLWithPath: NSString(string: "~/Library/Fonts/JetBrainsMono-Regular.ttf").expandingTildeInPath)
let descs = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as! [CTFontDescriptor]
let font = CTFontCreateWithFontDescriptor(descs[0], 16, nil)

func shape(_ s: String) {
    let attrs = [kCTFontAttributeName as NSAttributedString.Key: font]
    let line = CTLineCreateWithAttributedString(NSAttributedString(string: s, attributes: attrs))
    var glyphs: [CGGlyph] = []
    var advances: [CGFloat] = []
    for run in CTLineGetGlyphRuns(line) as! [CTRun] {
        let n = CTRunGetGlyphCount(run)
        var g = [CGGlyph](repeating: 0, count: n)
        var a = [CGSize](repeating: .zero, count: n)
        CTRunGetGlyphs(run, CFRange(location: 0, length: n), &g)
        CTRunGetAdvances(run, CFRange(location: 0, length: n), &a)
        glyphs += g
        advances += a.map { $0.width }
    }
    let names = glyphs.map { (CTFontCopyNameForGlyph(font, $0) as String?) ?? "?" }
    print("  \(s.debugDescription.padding(toLength: 12, withPad: " ", startingAt: 0))"
        + " chars=\(s.count) glyphs=\(glyphs.count) advances=\(advances.map { Int($0) })")
    print("      \(names.joined(separator: " "))")
}

print("ligature candidates:")
for s in ["!=", "=>", "->", "<=>", "===", "|>", "::", "/*", "www"] { shape(s) }
print("guarded (should NOT ligate):")
for s in ["a:b", "1:2", "http://x"] { shape(s) }
print("sub-run vs whole line (does context past 4/5 cells matter?):")
for s in ["x!=y", "!=y", "x!=", "!="] { shape(s) }

print("ink extents:")
for name in ["m", "SPC", "exclam_equal.liga", "less_equal_greater.liga", "equal_equal_equal.liga"] {
    let g = CTFontGetGlyphWithName(font, name as CFString)
    var gg = g
    var rect = CGRect.zero
    CTFontGetBoundingRectsForGlyphs(font, .horizontal, &gg, &rect, 1)
    var adv = CGSize.zero
    CTFontGetAdvancesForGlyphs(font, .horizontal, &gg, &adv, 1)
    print(String(format: "  %-26s bbox x=%.1f w=%.1f   advance=%.1f", (name as NSString).utf8String!, rect.origin.x, rect.width, adv.width))
}
