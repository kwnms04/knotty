/// The half of a paste that is the app's: what a warning sheet shows of what
/// is about to go in.
///
/// Which runs are worth warning about is not here — that is the engine's, and
/// ``Session/pasteIsSafe(_:)`` is the way to ask. Nor is any of the
/// sanitizing, which is inside ``Session/paste(_:)`` and cannot be reached
/// past. All this decides is how much of a clipboard fits in a sheet. cf.
/// adr/0007.
public enum Paste {
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
