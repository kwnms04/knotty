import CoreGraphics
import Testing

@testable import KnottyRender

/// A table that lies about its counts answers with a smaller set rather than
/// crashing or allocating for what it claims — the promise `readGsub` opens
/// with. Both counts below are ones a real font never carries.

/// Glyph ids stop at 65535, so a class definition whose first glyph plus
/// count runs past that is naming glyphs that cannot exist.
@Test func aClassDefinitionRunningPastTheLastGlyphIdStopsThere() {
    // Format 1, first glyph 65535, four glyphs after it — three of which
    // have no id to be.
    let table: [UInt8] = [0, 1, 0xFF, 0xFF, 0, 4, 0, 7, 0, 7, 0, 7, 0, 7]
    #expect(Reader(table).classes(0) == [7: Set([CGGlyph(65535)])])
}

/// Rule sets and the rules in them are two counts a table gives separately,
/// and the answer is their product — so the cap belongs on the answer.
@Test func ruleCountsThatMultiplyOutPastEveryGlyphAreCappedThere() {
    let table: [UInt8] = [
        0xFF, 0xFF,  // 65535 rule sets
        0, 10, 0, 10,  // the first two pointing at the same place
        0, 0, 0, 0,  // and the rest absent
        0xFF, 0xFF,  // where 65535 rules are claimed
    ]
    #expect(Reader(table).rules(0, at: 0).count <= 0x1_0000)
}
