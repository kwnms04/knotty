import AppKit

/// What is kept of a window between two runs: the rectangle it stood in and
/// the directory its shell was in. cf. 05-swift-app 9, adr/0020.
///
/// Not the scrollback and not the grid. The grid is derived from the frame on
/// the way back up, so a font that changed while the app was down moves the
/// cells rather than the window.
struct WindowState: Equatable {
    var frame: NSRect
    /// Where the shell is spawned again, or nil where nothing was read off
    /// the child — which opens the window wherever this process is.
    var directory: String?
}

/// The windows of the last run, in `UserDefaults`.
///
/// **macOS state restoration is not used.** `talagentd` re-snapshots a
/// restorable window every time its contents change, and a terminal's contents
/// change without stopping; we restore no scrollback, so what that machinery
/// buys is a rectangle and a path we can write ourselves. cf. adr/0020.
///
/// Takes the domain rather than reaching for `.standard`, which is what lets
/// the serialization be read back in a test without a window or a running app.
struct WindowStore {
    let defaults: UserDefaults

    /// Where the windows are written. Under the app's own domain, so
    /// `defaults delete` on it is the whole of forgetting them.
    private static let key = "windows"
    private static let frameKey = "frame"
    private static let directoryKey = "directory"

    /// Whether the last run's windows are wanted at all.
    ///
    /// The system setting "close windows when quitting an application" writes
    /// this, and it is absent until someone touches it — so absent has to mean
    /// restore, which is what the unticked box means. Reading it with
    /// ``UserDefaults/bool(forKey:)`` alone would answer `false` on a machine
    /// nobody ever set it on, and no window would ever come back.
    var restoresWindows: Bool {
        defaults.object(forKey: "NSQuitAlwaysKeepsWindows") as? Bool ?? true
    }

    /// Put the windows there are now in place of the ones that were there.
    ///
    /// Called on the events that move a window and on the ones that open,
    /// close or `cd` in it — never on a timer. A save that ran while nothing
    /// happened would be work in an idle app, which is the one thing B7 does
    /// not allow. cf. adr/0020.
    func save(_ states: [WindowState]) {
        defaults.set(
            states.map { state in
                var entry = [Self.frameKey: NSStringFromRect(state.frame)]
                entry[Self.directoryKey] = state.directory
                return entry
            },
            forKey: Self.key
        )
    }

    /// What the last run left, oldest window first.
    ///
    /// Anything that does not read back as a window is dropped rather than
    /// opened. A window of no size is worse than one window fewer, and this
    /// domain is one a person can write into.
    func load() -> [WindowState] {
        guard let entries = defaults.array(forKey: Self.key) as? [[String: String]] else {
            return []
        }
        return entries.compactMap { entry in
            guard let text = entry[Self.frameKey] else { return nil }
            let frame = NSRectFromString(text)
            // What is derived from a frame — how many cells it holds — is
            // arithmetic that a rectangle written in by hand can break, and
            // `NSRectFromString` answers zeroes for a string that was never
            // one at all.
            guard frame.width > 0, frame.height > 0,
                [frame.minX, frame.minY, frame.width, frame.height].allSatisfy(\.isFinite)
            else { return nil }
            return WindowState(frame: frame, directory: entry[Self.directoryKey])
        }
    }

    /// Where a saved frame opens.
    ///
    /// **On the frame it was saved at, unless no screen is under it any more.**
    /// A display that was unplugged leaves its windows somewhere nobody can
    /// reach, so those come back the size they were, in the middle of the
    /// screen there is. Any overlap at all is enough to be left alone — a
    /// window the user parked mostly off the side is where they put it — and
    /// a frame that only shares an edge with a screen has none of itself on
    /// one, which is what `intersects` already says.
    static func placed(_ frame: NSRect, screens: [NSRect], fallback: NSRect) -> NSRect {
        guard !screens.contains(where: { $0.intersects(frame) }) else { return frame }
        return NSRect(
            x: fallback.midX - frame.width / 2,
            y: fallback.midY - frame.height / 2,
            width: frame.width,
            height: frame.height
        )
    }
}
