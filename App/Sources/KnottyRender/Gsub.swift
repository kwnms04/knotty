import CoreGraphics
import CoreText

/// What one traversal of a font's GSUB says about its ligatures.
///
/// Three answers out of one walk, which is the whole reason none of them is a
/// constant: they cost the same together as any one of them alone, and the six
/// fonts that were measured agreed on none of them. cf. adr/0016.
struct GsubRules {
    /// Glyphs a lookup can replace. A cell holding one cannot be a plain
    /// atlas lookup; every other cell can.
    var substitutable: Set<CGGlyph> = []
    /// Glyphs a lookup can produce. These are the ones whose ink leaves the
    /// cell, so they are what the overhang is measured over.
    var produced: Set<CGGlyph> = []
    /// The window, in cells: the longest input a rule matches, and how far
    /// either side of it a rule looks to decide.
    var input = 1
    var backtrack = 0
    var lookahead = 0
}

/// Read what `tags`' lookups cover out of a GSUB table.
///
/// Answers nil when the font has no GSUB or names none of `tags` — which is
/// the same statement as an empty participating set, and the shape the caller
/// wants: no ligature path at all rather than one with nothing in it.
///
/// Every read is bounds-checked and answers zero past the end, so a table
/// that is truncated or lying degrades into a smaller set rather than a
/// crash. Over-counting is the safe side of this: a glyph named here that no
/// rule ever fires on costs a shaping call, and one missed costs a ligature
/// that does not appear.
func readGsub(_ table: [UInt8], tags: Set<String>) -> GsubRules? {
    let t = Reader(table)
    let featureList = t.u16(6)
    let lookupList = t.u16(8)

    var queue: [Int] = []
    for i in 0..<t.u16(featureList) {
        let record = featureList + 2 + 6 * i
        guard tags.contains(t.tag(record)) else { continue }
        let feature = featureList + t.u16(record + 4)
        for j in 0..<t.u16(feature + 2) {
            queue.append(t.u16(feature + 4 + 2 * j))
        }
    }
    guard !queue.isEmpty else { return nil }

    var rules = GsubRules()
    var walked = Set<Int>()
    while let index = queue.popLast() {
        guard !walked.contains(index), index < t.u16(lookupList) else { continue }
        walked.insert(index)
        let lookup = lookupList + t.u16(lookupList + 2 + 2 * index)
        let type = t.u16(lookup)
        for i in 0..<t.u16(lookup + 4) {
            queue += rules.read(t, at: lookup + t.u16(lookup + 6 + 2 * i), type: type)
        }
    }
    return rules
}

extension GsubRules {
    /// One subtable, answering the lookups it nests.
    ///
    /// The types are OpenType's own numbering, and every one of them is here
    /// because leaving one out is a ligature that silently does not draw.
    fileprivate mutating func read(_ t: Reader, at subtable: Int, type: Int) -> [Int] {
        let format = t.u16(subtable)
        switch type {
        case 1:  // Single: one glyph for one glyph.
            let covered = t.coverage(subtable + t.u16(subtable + 2))
            substitutable.formUnion(covered)
            if format == 1 {
                let delta = t.i16(subtable + 4)
                produced.formUnion(covered.map { CGGlyph(truncatingIfNeeded: Int($0) + delta) })
            } else {
                produced.formUnion((0..<t.u16(subtable + 4)).map { CGGlyph(t.u16(subtable + 6 + 2 * $0)) })
            }
        case 2, 3:  // Multiple and Alternate: a set of glyphs for one glyph.
            substitutable.formUnion(t.coverage(subtable + t.u16(subtable + 2)))
            for i in 0..<t.u16(subtable + 4) {
                let set = subtable + t.u16(subtable + 6 + 2 * i)
                produced.formUnion((0..<t.u16(set)).map { CGGlyph(t.u16(set + 2 + 2 * $0)) })
            }
        case 4:  // Ligature: several glyphs folded into one.
            substitutable.formUnion(t.coverage(subtable + t.u16(subtable + 2)))
            for i in 0..<t.u16(subtable + 4) {
                let set = subtable + t.u16(subtable + 6 + 2 * i)
                for j in 0..<t.u16(set) {
                    let ligature = set + t.u16(set + 2 + 2 * j)
                    let components = t.u16(ligature + 2)
                    produced.insert(CGGlyph(t.u16(ligature)))
                    substitutable.formUnion(
                        (0..<max(0, components - 1)).map { CGGlyph(t.u16(ligature + 4 + 2 * $0)) }
                    )
                    input = max(input, components)
                }
            }
        case 5:
            return context(t, at: subtable, format: format, chained: false)
        case 6:
            return context(t, at: subtable, format: format, chained: true)
        // Extension: the same subtable, one indirection away. An extension
        // of an extension is not a thing OpenType has, and refusing it is
        // what keeps a table that points at itself from being a stack
        // overflow.
        case 7 where t.u16(subtable + 2) != 7:
            return read(t, at: subtable + t.u32(subtable + 4), type: t.u16(subtable + 2))
        case 8:  // Reverse chaining single, which substitutes as it looks back.
            substitutable.formUnion(t.coverage(subtable + t.u16(subtable + 2)))
            let back = t.u16(subtable + 4)
            let ahead = subtable + 6 + 2 * back
            backtrack = max(backtrack, back)
            lookahead = max(lookahead, t.u16(ahead))
            let substitutes = ahead + 2 + 2 * t.u16(ahead)
            produced.formUnion(
                (0..<t.u16(substitutes)).map { CGGlyph(t.u16(substitutes + 2 + 2 * $0)) }
            )
        default:
            break
        }
        return []
    }

    /// A context or chaining-context subtable, which is where the window
    /// comes from: what a rule matches, and what it looks at either side to
    /// decide.
    ///
    /// The two differ in their layout and not only in their content: a plain
    /// rule counts its nested lookups up front and a chained one counts them
    /// last, because the chained form has two more arrays to get past first.
    private mutating func context(
        _ t: Reader, at subtable: Int, format: Int, chained: Bool
    ) -> [Int] {
        var nested: [Int] = []

        switch format {
        case 1:  // Rules keyed by the first glyph, spelling out glyph ids.
            let first = Set(t.coverage(subtable + t.u16(subtable + 2)))
            for rule in t.rules(subtable, at: subtable + 4) {
                let body = t.body(rule, chained: chained)
                record(
                    input: first.union(body.items.map { CGGlyph($0) }),
                    cells: body.cells, back: body.back, ahead: body.ahead
                )
                nested += body.lookups
            }
        case 2:  // Rules keyed by glyph class, so the classes name the glyphs.
            let first = Set(t.coverage(subtable + t.u16(subtable + 2)))
            let byClass = t.classes(subtable + t.u16(subtable + (chained ? 6 : 4)))
            for rule in t.rules(subtable, at: subtable + (chained ? 10 : 6)) {
                let body = t.body(rule, chained: chained)
                record(
                    input: body.items.reduce(into: first) { $0.formUnion(byClass[$1] ?? []) },
                    cells: body.cells, back: body.back, ahead: body.ahead
                )
                nested += body.lookups
            }
        default:  // Format 3: one coverage table per position, no rule sets.
            let body = t.body(subtable + 2, chained: chained, covering: true)
            record(
                input: Set(body.items.flatMap { t.coverage(subtable + $0) }),
                cells: body.cells, back: body.back, ahead: body.ahead
            )
            nested += body.lookups
        }
        return nested
    }

    private mutating func record(input glyphs: Set<CGGlyph>, cells: Int, back: Int, ahead: Int) {
        substitutable.formUnion(glyphs)
        input = max(input, cells)
        backtrack = max(backtrack, back)
        lookahead = max(lookahead, ahead)
    }
}

/// A big-endian reader over one font table.
///
/// Everything past the end reads as zero, which is what a font table already
/// says for "nothing here": an offset of zero is absent and a count of zero
/// ends a loop. So a table that is short answers with less rather than
/// faulting, and no caller has a length to check.
///
/// The one place untrusted bytes are read, so the promise above is this
/// type's to keep and not its callers'.
struct Reader {
    private let bytes: [UInt8]

    init(_ bytes: [UInt8]) { self.bytes = bytes }

    func u8(_ i: Int) -> Int { i >= 0 && i < bytes.count ? Int(bytes[i]) : 0 }
    func u16(_ i: Int) -> Int { u8(i) << 8 | u8(i + 1) }
    func u32(_ i: Int) -> Int { u16(i) << 16 | u16(i + 2) }
    func i16(_ i: Int) -> Int {
        let value = u16(i)
        return value > 0x7FFF ? value - 0x1_0000 : value
    }

    /// A four-character tag, which is how a feature names itself.
    func tag(_ i: Int) -> String {
        String(decoding: (0..<4).map { UInt8(u8(i + $0)) }, as: UTF8.self)
    }

    /// Every glyph a table this long could name, which is every glyph id
    /// there is. A table claiming more than that is one whose counts are not
    /// to be believed, and stopping there is what keeps a corrupt one from
    /// being an allocation rather than a smaller answer.
    private static let everyGlyph = 0x1_0000

    /// The glyphs a coverage table covers, in either of its two shapes.
    func coverage(_ at: Int) -> [CGGlyph] {
        switch u16(at) {
        case 1:
            return (0..<min(u16(at + 2), Self.everyGlyph)).map { CGGlyph(u16(at + 4 + 2 * $0)) }
        case 2:
            var out: [CGGlyph] = []
            for i in 0..<u16(at + 2) where out.count < Self.everyGlyph {
                let range = at + 4 + 6 * i
                let first = u16(range)
                let last = u16(range + 2)
                if first <= last { out += (first...last).map { CGGlyph($0) } }
            }
            return out
        default:
            return []
        }
    }

    /// A class definition, read the way a rule uses it: class number to the
    /// glyphs in it. Class 0 is every glyph not named, which no rule here can
    /// enumerate — a rule keyed on it widens the window without widening the
    /// participating set, which is the safe direction.
    func classes(_ at: Int) -> [Int: Set<CGGlyph>] {
        var out: [Int: Set<CGGlyph>] = [:]
        var named = 0
        switch u16(at) {
        case 1:
            // The count is capped at what is left above the first glyph
            // rather than at the whole range: ids stop at 65535, so a table
            // whose first plus count runs past that is naming glyphs that do
            // not exist.
            let start = u16(at + 2)
            for i in 0..<min(u16(at + 4), Self.everyGlyph - start) {
                out[u16(at + 6 + 2 * i), default: []].insert(CGGlyph(start + i))
            }
        case 2:
            for i in 0..<u16(at + 2) where named < Self.everyGlyph {
                let range = at + 4 + 6 * i
                let first = u16(range)
                let last = u16(range + 2)
                guard first <= last else { continue }
                out[u16(range + 4), default: []].formUnion((first...last).map { CGGlyph($0) })
                named += last - first + 1
            }
        default:
            break
        }
        return out
    }

    /// Every rule in every rule set a format 1 or 2 subtable holds, as the
    /// offsets they start at. The two levels of indirection are the same
    /// shape at both, so they are walked as one.
    ///
    /// Both levels are counts, and a lying table multiplies them: the cap is
    /// on the answer rather than on either one, for the reason
    /// ``everyGlyph`` gives.
    func rules(_ subtable: Int, at sets: Int) -> [Int] {
        var out: [Int] = []
        for i in 0..<u16(sets) where out.count < Self.everyGlyph {
            let offset = u16(sets + 2 + 2 * i)
            guard offset != 0 else { continue }
            let set = subtable + offset
            for j in 0..<u16(set) where out.count < Self.everyGlyph {
                out.append(set + u16(set + 2 + 2 * j))
            }
        }
        return out
    }

    /// One rule, read as the three lengths and the middle array — which is
    /// all any caller here wants of it, whichever of the four layouts it is
    /// written in.
    ///
    /// `items` is the input array with its first element left out, because
    /// the first is already named by the subtable's own coverage; format 3
    /// names all of them itself, which is what `covering` says.
    struct Body {
        var items: [Int] = []
        var cells = 0
        var back = 0
        var ahead = 0
        var lookups: [Int] = []
    }

    func body(_ rule: Int, chained: Bool, covering: Bool = false) -> Body {
        var body = Body()
        var at = rule
        if chained {
            body.back = u16(at)
            at += 2 + 2 * body.back
        }
        body.cells = u16(at)
        // A plain rule counts its lookups here, before the input array; a
        // chained one counts them after the lookahead array instead.
        let counted = chained ? 0 : u16(at + 2)
        let items = at + (chained ? 2 : 4)
        let length = covering ? body.cells : max(0, body.cells - 1)
        body.items = (0..<length).map { u16(items + 2 * $0) }
        at = items + 2 * length
        if chained {
            body.ahead = u16(at)
            at += 2 + 2 * body.ahead
        }
        let records = chained ? at + 2 : at
        body.lookups = (0..<(chained ? u16(at) : counted)).map { u16(records + 4 * $0 + 2) }
        return body
    }
}
