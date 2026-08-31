/// The half of a paste that is the app's: which runs are worth a sheet, and
/// what that sheet shows of one.
///
/// **Not the sanitizing**, which is the engine's, is inside
/// ``Session/paste(_:)``, and happens whichever way anything here answered.
/// The policy is the half adr/0007 puts on this side: it says when to ask,
/// never what to strip.
public enum Paste {
    /// Whether `text` is worth a warning sheet under the default policy.
    ///
    /// 05-swift-app 8's "multiline only", which counts three things. Two are
    /// the engine's — a newline, and the bracketed terminator that would end
    /// the wrapping early — and ``Session/pasteIsSafe(_:)`` is how they are
    /// asked for.
    ///
    /// The third is a lone carriage return, which the engine's check does not
    /// look for. It is a line ending all the same, and a shell runs one the
    /// moment it arrives exactly as it runs a newline — so a clipboard of
    /// old-Mac line endings would otherwise go in unasked. Counting a
    /// condition the engine does not is what the split in adr/0007 allows:
    /// the policy is this side's, and only the policy.
    ///
    /// The "control characters" half of §8's condition belongs to the
    /// "always" setting, which arrives with the config pipeline in M4.
    public static func warns(about text: String) -> Bool {
        let bytes = Array(text.utf8)
        // Read off the bytes rather than the characters: CR and LF together
        // are one grapheme, so a search over characters would miss the return
        // in a run that has both.
        return !Session.pasteIsSafe(bytes) || bytes.contains(0x0D)
    }

    /// What a warning shows of `text`, cut down to something a sheet can hold.
    ///
    /// Two ceilings because a clipboard can be long either way: a thousand
    /// short lines and one enormous line both have to end somewhere. An
    /// ellipsis on its own line says the cut happened, so a preview that
    /// stops early is never read as the whole of what is about to run.
    public static func preview(
        of text: String,
        lines: Int = 10,
        characters: Int = 512
    ) -> String {
        var shown = text
        var cut = false

        // One more split than is wanted, so that having gone over is a count
        // rather than a scan of the whole clipboard. The extra piece is
        // everything past the ceiling — and it is empty for a run that ended
        // exactly on it, where nothing was cut and nothing should say so.
        let broken = shown.split(
            separator: "\n",
            maxSplits: lines,
            omittingEmptySubsequences: false
        )
        if broken.count > lines, let rest = broken.last, !rest.isEmpty {
            shown = broken.prefix(lines).joined(separator: "\n")
            cut = true
        }
        if shown.count > characters {
            shown = String(shown.prefix(characters))
            cut = true
        }

        return cut ? shown + "\n…" : shown
    }
}
