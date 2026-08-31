import Testing

import KnottySession

/// What the clipboard comes to on the way to the child. The judgement and the
/// sanitizing are both the engine's; what these hold is that the app's one
/// route to a child runs them. cf. adr/0007.
@Test func aPasteArrivesWrappedWhenTheChildAskedForBrackets() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("\u{1b}[?2004h".utf8))
    _ = try session.takeWrites()

    try session.paste(Array("echo hello".utf8))

    #expect(try session.takeWrites() == Array("\u{1b}[200~echo hello\u{1b}[201~".utf8))
}

/// **The warning is what can be skipped; the sanitizing is not.** This is the
/// paste a user makes after reading the sheet and going ahead — no check
/// asked, no argument that would have kept the control bytes — and they are
/// spaces all the same. There is no second call on this side that pastes
/// without this.
@Test func aPasteIsSanitizedWithoutTheCheckHavingBeenAsked() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)

    try session.paste(Array("a\u{0}b\u{1b}c\u{7f}d".utf8))

    #expect(try session.takeWrites() == Array("a b c d".utf8))
}

/// The attack the wrapping alone would not survive: content carrying the end
/// sequence would close the brackets early and leave the rest read as
/// commands. The escape is a control byte, so it is gone before the wrapping
/// goes on.
@Test func theEndSequenceInTheContentDoesNotBreakTheWrapping() throws {
    let session = try Session(cols: cols, rows: rows, scrollback: scrollback)
    try session.feed(Array("\u{1b}[?2004h".utf8))
    _ = try session.takeWrites()

    try session.paste(Array("a\u{1b}[201~rm -rf /".utf8))

    let queued = try session.takeWrites()
    #expect(queued == Array("\u{1b}[200~a [201~rm -rf /\u{1b}[201~".utf8))
}

/// What the engine judges, asked before there is anything to paste into —
/// which is why it takes no session.
@Test func theEngineJudgesARunWithoutASessionBehindIt() {
    #expect(Session.pasteIsSafe(Array("a plain command".utf8)))
    // A newline is what a shell runs the moment it arrives.
    #expect(!Session.pasteIsSafe(Array("one\ntwo".utf8)))
    // And the end sequence is what would let the rest out of the wrapping.
    #expect(!Session.pasteIsSafe(Array("a\u{1b}[201~b".utf8)))
    #expect(Session.pasteIsSafe([]))
}

/// The policy on top of it, which is this side's to own: the sheet is shown
/// for what the engine calls unsafe, and for a carriage return besides.
/// cf. adr/0007, 05-swift-app 8.
@Test func thePolicyWarnsAboutEveryLineEndingAndNotAboutPlainText() {
    #expect(!Paste.warns(about: "a plain command"))
    #expect(!Paste.warns(about: ""))
    // The two the engine sees.
    #expect(Paste.warns(about: "one\ntwo"))
    #expect(Paste.warns(about: "a\u{1b}[201~b"))
}

/// The one the engine's check does not see. A lone carriage return is a line
/// ending, and a shell runs one exactly as it runs a newline — so a clipboard
/// of old-Mac line endings must not go in unasked.
@Test func aLoneCarriageReturnIsWarnedAboutThoughTheEngineCallsItSafe() {
    #expect(Session.pasteIsSafe(Array("one\rtwo".utf8)))
    #expect(Paste.warns(about: "one\rtwo"))
    // And in a run carrying both, where CR and LF are one grapheme — which is
    // why the policy reads bytes and not characters.
    #expect(Paste.warns(about: "one\r\ntwo"))
}

/// The one judgement about a paste this side makes: how much of a clipboard
/// fits in a sheet. Short of both ceilings it is shown whole.
@Test func aPreviewShortOfBothCeilingsIsTheWholeText() {
    #expect(Paste.preview(of: "echo hello") == "echo hello")
    #expect(Paste.preview(of: "one\ntwo\nthree") == "one\ntwo\nthree")
}

/// And over either of them it stops, saying so. A thousand short lines and
/// one enormous line are both clipboards, and neither fits.
@Test func aPreviewOverEitherCeilingSaysItStopped() {
    let many = (1...20).map(String.init).joined(separator: "\n")
    #expect(Paste.preview(of: many, lines: 3) == "1\n2\n3\n…")

    let long = String(repeating: "x", count: 40)
    #expect(Paste.preview(of: long, characters: 10) == String(repeating: "x", count: 10) + "\n…")

    // Exactly at a ceiling is not over it.
    #expect(Paste.preview(of: "1\n2\n3", lines: 3) == "1\n2\n3")
    #expect(Paste.preview(of: "xxx", characters: 3) == "xxx")
    // And a run that ended on the ceiling leaves an empty piece past it,
    // which is not something cut off — a trailing newline is the ordinary
    // way a copied line arrives.
    #expect(Paste.preview(of: "1\n2\n3\n", lines: 3) == "1\n2\n3\n")
}
