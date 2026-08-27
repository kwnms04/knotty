// PROTOTYPE — throwaway.  Ground truth for Q4: can a ligature ever make
// CoreText return fewer glyphs than characters?  A GSUB table can hold a
// cell-collapsing type-4 rule that shaping never reaches, or that a later
// expansion undoes — only a real run settles it, under every feature
// combination the renderer might choose.
//
//   swift prototype/gsub-probe/shape.swift
import CoreText
import Foundation

let dir = URL(fileURLWithPath: "prototype/gsub-probe/fonts")

func load(_ path: String, _ features: [(String, Int)]) -> CTFont? {
    guard let ds = CTFontManagerCreateFontDescriptorsFromURL(
            URL(fileURLWithPath: path) as CFURL) as? [CTFontDescriptor],
          let base = ds.first else { return nil }
    if features.isEmpty { return CTFontCreateWithFontDescriptor(base, 16, nil) }
    let settings = features.map { (tag, val) -> [String: Any] in
        [kCTFontOpenTypeFeatureTag as String: tag,
         kCTFontOpenTypeFeatureValue as String: val]
    }
    let d = CTFontDescriptorCreateCopyWithAttributes(
        base, [kCTFontFeatureSettingsAttribute: settings] as CFDictionary)
    return CTFontCreateWithFontDescriptor(d, 16, nil)
}

func shape(_ s: String, _ f: CTFont) -> (names: [String], adv: [Int]) {
    let line = CTLineCreateWithAttributedString(NSAttributedString(
        string: s, attributes: [kCTFontAttributeName as NSAttributedString.Key: f]))
    var names: [String] = []; var adv: [Int] = []
    for run in CTLineGetGlyphRuns(line) as! [CTRun] {
        let n = CTRunGetGlyphCount(run)
        var g = [CGGlyph](repeating: 0, count: n)
        var a = [CGSize](repeating: .zero, count: n)
        CTRunGetGlyphs(run, CFRange(location: 0, length: n), &g)
        CTRunGetAdvances(run, CFRange(location: 0, length: n), &a)
        names += g.map { (CTFontCopyNameForGlyph(f, $0) as String?) ?? "?" }
        adv += a.map { Int($0.width.rounded()) }
    }
    return (names, adv)
}

var paths: [(String, String)] = []
for face in ["Regular", "Bold", "Italic", "BoldItalic"] {
    let jb = NSString(string: "~/Library/Fonts/JetBrainsMono-\(face).ttf").expandingTildeInPath
    if FileManager.default.fileExists(atPath: jb) {
        paths.append(("JetBrainsMono-\(face)", jb))
    }
}
for f in (try? FileManager.default.contentsOfDirectory(atPath: dir.path))?.sorted() ?? [] {
    if f.hasSuffix(".ttf") || f.hasSuffix(".otf") {
        paths.append((f, dir.appendingPathComponent(f).path))
    }
}

// Every sequence any of these fonts ligates, plus guards that must not.
let samples = ["!=", "=>", "->", "<=>", "===", "//", "///", "|>", "::", "/*",
               "<!--", "||", "!!", ";;", "...", "a:b", "www"]
let combos: [(String, [(String, Int)])] = [
    ("default", []),
    ("liga+calt on", [("liga", 1), ("calt", 1)]),
    ("liga on, calt off", [("liga", 1), ("calt", 0)]),
    ("liga off, calt on", [("liga", 0), ("calt", 1)]),
]

for (label, path) in paths {
    var broken: [String] = []
    var ragged: [String] = []
    for (cname, feats) in combos {
        guard let f = load(path, feats) else { continue }
        for s in samples {
            let r = shape(s, f)
            if r.names.count != s.count { broken.append("\(s) [\(cname)]") }
            if Set(r.adv).count > 1 { ragged.append("\(s) [\(cname)]") }
        }
    }
    let ligated = load(path, [("liga", 1), ("calt", 1)]).map {
        shape("!=", $0).names.joined(separator: "+")
    } ?? "?"
    let verdict = broken.isEmpty
        ? (ragged.isEmpty ? "ok" : "RAGGED ADVANCE: " + ragged.joined(separator: " "))
        : "COLLAPSES: " + broken.joined(separator: " ")
    print("  \(label.padding(toLength: 34, withPad: " ", startingAt: 0))"
        + "\(verdict.padding(toLength: 12, withPad: " ", startingAt: 0))  != -> \(ligated)")
}
