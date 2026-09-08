import Foundation

import KnottyRender
import KnottySession

/// The one object that touches a session handle.
///
/// The boundary is written for calls that are serialized per session, and this
/// is what makes that structure rather than discipline: the handle lives here
/// and nowhere else, so there is no second path to it to race with. The view
/// above passes intent; it does not call. cf. 05-swift-app 4.
///
/// The renderer sits here for the same reason. A snapshot is borrowed for the
/// length of one call, and turning it into a frame is what has to happen
/// inside that scope — so the object that opens the scope is the one that
/// holds what reads it, and the view never sees a snapshot at all.
@MainActor
final class SessionHost {
    private let session: Session
    private var renderer: Renderer
    /// What one cell measures, on the display the window is on now.
    private var metrics: CellMetrics
    /// The face the configuration asked for, kept because a new raster loads
    /// it again and this is what said which.
    private var font: Config.Font
    /// What the theme said the cursor is drawn in, or nil when it named none
    /// — which is the renderer's cue to take the colour of the text the
    /// cursor stands on. cf. 04-renderer R1.
    private var cursorColor: Rgb?

    /// The grid the last resize sent.
    ///
    /// Zero rather than the counts the session was spawned with, so that the
    /// first layout always reaches the core: spawning could say how many cells
    /// there were but not how many pixels one of them is, and a resize is what
    /// fills that in.
    private var columns: UInt16 = 0
    private var rows: UInt16 = 0

    /// Which cell the cursor stood on when the last frame was taken, or nil
    /// when there was none to draw.
    ///
    /// Read off the snapshot rather than counted alongside it: an input method
    /// places its candidate window from where the cursor is, and a view
    /// keeping a second count of that is a view that can disagree with the
    /// terminal. cf. 05-swift-app 7.
    private(set) var cursorCell: (column: Int, row: Int)?

    /// What the window is called: the title of the last frame taken, or the
    /// app's own name when that frame carried none.
    ///
    /// Copied out for the reason ``cursorCell`` is read off the snapshot at
    /// all — the bytes belong to the borrow and a window title outlives every
    /// frame. Which name a frame asks for is ``Snapshot/windowTitle``'s to
    /// say; what stands here before there is a frame is the answer it gives
    /// to one that named nothing.
    private(set) var title = ProcessInfo.processInfo.processName

    /// Where the child is, or nil while nothing has said.
    ///
    /// Read off the frame the way ``title`` is, and for the same reason: the
    /// bytes belong to the borrow, and what is saved for a window outlives
    /// every frame. Filling it is the core's — a shell that sends OSC 7 is
    /// taken at its word and one that sends nothing has it read off the
    /// process — so there is nothing to decide here beyond what an empty path
    /// means, which is that no directory is known rather than that the child
    /// is in the root. cf. adr/0020.
    private(set) var workingDirectory: String?

    /// What to call when that directory changed, which is the window's saved
    /// state going stale.
    ///
    /// A closure for the reason ``onTitle`` is one, and the whole of what puts
    /// a `cd` in the store: saving on a timer instead would be work in an idle
    /// app. cf. 05-swift-app 9, adr/0020.
    var onWorkingDirectory: (() -> Void)?

    /// Whether the screen's URLs are being shown, which is ⌘ being held.
    ///
    /// The whole of when a screen is scanned for one: with ⌘ up nothing looks,
    /// so a lazy scan is what the feature costs the rest of the time. Set by
    /// the view, which is where a modifier arrives. cf. adr/0006.
    var linksShown = false

    /// The links the frame on the screen was drawn with, and none while ⌘ is
    /// up. What a ⌘ click is answered from.
    private var links: [Link] = []

    /// Whether a program is running in front of the shell.
    ///
    /// Asked of the session rather than kept off the last frame, which is the
    /// one thing here that is not. A job that prints nothing publishes no
    /// frame, so the newest one this object saw was taken before the job
    /// started — and what asks is a window being closed, which is not a
    /// moment there is a fresh frame for. cf. 05-swift-app 8.
    ///
    /// A boundary that refused answers no. A window whose session cannot be
    /// asked is not one to hold up: there is nothing left it can say is
    /// running.
    var isBusy: Bool { (try? session.foregroundBusy()) ?? false }

    /// What to call when that name changed, which is the window being
    /// renamed.
    ///
    /// A closure the way ``onWake(_:)`` takes one, and for the reason the
    /// ownership tree gives: the window is the controller's, and a view that
    /// renamed it would be a second object saying what a window is. It runs
    /// where the frame was taken, which is the main thread. cf. 05-swift-app 4.
    var onTitle: ((String) -> Void)?

    /// Spawn the user's login shell in `directory` behind a terminal of this
    /// size, drawn at these metrics and in these colours.
    ///
    /// Nil for `directory` leaves the shell wherever this process is, which is
    /// what a window with nothing saved for it opens in.
    ///
    /// The theme goes down to the core before anything is drawn: the palette
    /// is the terminal's own state and every cell's colours are resolved
    /// against it, so a screen captured before the injection is a screen in
    /// the wrong colours. cf. 02-ffi.
    init(
        columns: UInt16, rows: UInt16, scrollback: Int, metrics: CellMetrics,
        font: Config.Font, theme: Config.Theme, directory: String?
    ) throws {
        session = try Session(
            command: LoginShell.command, cols: columns, rows: rows, scrollback: scrollback,
            directory: directory
        )
        renderer = Renderer(metrics: metrics, faces: Faces(metrics: metrics, name: font.family))
        self.metrics = metrics
        self.font = font
        cursorColor = theme.cursor?.rgb
        try session.setTheme(theme)
    }

    /// Tell the session the grid it now has, and how big a cell is on the
    /// display it is drawn on.
    ///
    /// The view calls this on every layout, and this is what decides whether
    /// the core hears about it: the same grid drawn at the same cell goes no
    /// further, which is what keeps a drag off the reflow the boundary's
    /// non-blocking contract makes an exception of. A cell that changed size
    /// does go down even when the counts held — that is the pixel size the
    /// terminal reports, and the engine rewraps nothing for it. cf. 02-ffi.
    ///
    /// New metrics are a new raster: the cell is a different number of pixels
    /// and every glyph baked at the old size is the wrong shape. The renderer
    /// is replaced rather than told, which is the "atlas included" reset of
    /// 04-renderer R8 written out.
    ///
    /// ponytail: that loads all four faces and walks each one's GSUB again,
    /// and what they derive — a set of glyph ids, a window in cells, an
    /// overhang in cells — does not depend on the size it was measured at.
    /// Measured at 1.15ms for the four against the face this milestone loads,
    /// beside a reset that bakes every glyph on screen again; carrying the
    /// derivations across the new size is what to do if a family with larger
    /// tables ever makes it show.
    func resize(columns: UInt16, rows: UInt16, metrics: CellMetrics) {
        guard (columns, rows, metrics) != (self.columns, self.rows, self.metrics) else { return }
        if metrics != self.metrics {
            self.metrics = metrics
            renderer = Renderer(
                metrics: metrics, faces: Faces(metrics: metrics, name: font.family)
            )
        }
        (self.columns, self.rows) = (columns, rows)

        do {
            try session.resize(
                cols: columns, rows: rows,
                cellWidth: UInt32(metrics.width), cellHeight: UInt32(metrics.height)
            )
        } catch {
            report(error)
        }
    }

    /// Put a theme the file changed in force.
    ///
    /// **Redraw only, and no new raster.** The palette is the terminal's own
    /// state, so it goes back down to the core and every cell crosses in the
    /// new colours on the next frame — which the injection itself publishes.
    /// A glyph's coverage is not a function of what tints it, so nothing that
    /// was baked is stale. cf. 04-renderer R8, 05-swift-app 10.
    func apply(theme: Config.Theme) {
        cursorColor = theme.cursor?.rgb
        do {
            try session.setTheme(theme)
        } catch {
            report(error)
        }
    }

    /// Take a face the file changed.
    ///
    /// What the counts become is the window's answer, and the layout that
    /// follows the window moving is what carries it down. Two things here are
    /// what that layout cannot do for itself.
    ///
    /// **The raster is remade**, because a face is not a cell: two families
    /// can measure the same, and then the resize that answers a new cell has
    /// nothing to answer.
    ///
    /// **The grid is forgotten**, so the layout after it always reaches the
    /// core — the same reason the counts start at zero. A face that measures
    /// like the last one moves no window and changes no count, and nothing
    /// would publish a frame: the new glyphs would sit baked and waiting
    /// while the screen kept the old ones until the child next wrote
    /// something. What goes down is the grid it already had, which the engine
    /// rewraps nothing for.
    ///
    /// ponytail: where the size moved too, that resize remakes the raster a
    /// second time at the new cell — two four-face loads, about 2ms, on a
    /// save. Handing the new metrics in here would settle it, at the cost of
    /// a branch that has to know what the window is about to do.
    /// cf. 04-renderer R8, 05-swift-app 10.
    func apply(font: Config.Font) {
        self.font = font
        renderer = Renderer(metrics: metrics, faces: Faces(metrics: metrics, name: font.family))
        (columns, rows) = (0, 0)
    }

    /// Register what the session calls when it has something to be taken.
    ///
    /// It runs on the core's thread and may do nothing but wake a thread of
    /// its own, which is why this hands the closure straight through rather
    /// than wrapping anything of its own around it.
    func onWake(_ body: @escaping @Sendable () -> Void) throws {
        try session.onWake(body)
    }

    /// Take everything one wake left behind: the event queue emptied, the
    /// newest frame taken and turned into what draws it.
    ///
    /// Nil when there was nothing published to take. What comes out holds
    /// nothing of the snapshot it was made from — the renderer answers in
    /// values — so it outlives the borrow the way a drawer needs it to.
    func takeFrame() -> Frame? {
        do {
            try session.drainEvents()
            return try session.withSnapshot { frame(of: $0) }
        } catch {
            report(error)
            return nil
        }
    }

    /// The frame the screen draws as now, for a change that is the app's
    /// rather than the terminal's — ⌘ going down, and coming back up.
    ///
    /// Nil where no frame has been taken yet. The terminal publishes when it
    /// moves and it has not moved, so this reads the frame already taken
    /// rather than waiting for one that is not coming. No event drain either:
    /// nothing arrived to drain. cf. 05-swift-app 6.
    func redrawnFrame() -> Frame? {
        do {
            return try session.withHeldSnapshot { frame(of: $0) }
        } catch {
            report(error)
            return nil
        }
    }

    /// One snapshot as what draws it, and the things read off it on the way.
    private func frame(of snapshot: Snapshot) -> Frame {
        cursorCell =
            snapshot.cursor.visible
            ? (column: Int(snapshot.cursor.x), row: Int(snapshot.cursor.y)) : nil
        let called = snapshot.windowTitle
        if called != title {
            title = called
            onTitle?(called)
        }
        let directory = snapshot.pwd.isEmpty
            ? nil : String(decoding: snapshot.pwd, as: UTF8.self)
        if directory != workingDirectory {
            workingDirectory = directory
            onWorkingDirectory?()
        }
        // The one place a screen is scanned, and only while ⌘ asks — which is
        // what makes the feature free the rest of the time. cf. adr/0006.
        links = linksShown ? Link.scan(snapshot) : []
        return renderer.frame(for: snapshot, cursorColor: cursorColor, links: links)
    }

    /// Where a ⌘ click over a cell goes, or nil where the frame on the screen
    /// drew no link there.
    ///
    /// Read off what was drawn rather than scanned again: a second scan is a
    /// second judgement, and what opens has to be what the user saw
    /// underlined.
    func url(at cell: (column: UInt16, row: UInt16)) -> URL? {
        links.first { $0.covers(row: Int(cell.row), column: Int(cell.column)) }?.url
    }

    /// Hand one key to the session, which is what decides its bytes.
    ///
    /// The view says which key moved and what was held with it; this is the
    /// only object that calls with any of it. A key naming no physical key
    /// comes back as a refusal, which is a hole in the app's own table rather
    /// than a key that quietly did nothing.
    func send(_ key: KeyEvent) {
        do {
            try session.key(key)
        } catch {
            report(error)
        }
    }

    /// Hand one mouse event to the session, over the cell it happened on.
    ///
    /// Whether the child hears about it is the core's answer and not this
    /// one's: at a shell prompt nothing has asked, and the event stops here
    /// without anybody above having to know that. cf. adr/0017.
    func send(
        _ action: MouseAction,
        button: MouseButton?,
        mods: Modifiers,
        at cell: (column: UInt16, row: UInt16)
    ) {
        do {
            try session.mouse(action, button: button, mods: mods, x: cell.column, y: cell.row)
        } catch {
            report(error)
        }
    }

    /// Turn the wheel over a cell, in whole lines.
    ///
    /// One of three things comes of it and the terminal is what says which —
    /// a mouse code, the cursor keys, or the viewport moving. The last is why
    /// nothing here keeps a scroll position: the core moves the viewport and
    /// publishes, and the next frame is already scrolled.
    func wheel(
        deltaX: Int, deltaY: Int, mods: Modifiers, at cell: (column: UInt16, row: UInt16)
    ) {
        do {
            try session.wheel(
                deltaX: Int32(clamping: deltaX), deltaY: Int32(clamping: deltaY),
                x: cell.column, y: cell.row, mods: mods
            )
        } catch {
            report(error)
        }
    }

    /// Select from the cell a gesture began on out to the cell it is over
    /// now.
    ///
    /// The view hands over both ends because both travel: a word or a line is
    /// widened from each. What falls between them is the engine's, and no
    /// boundary is counted on this side. cf. 05-swift-app 4.
    func select(
        anchor: (column: UInt16, row: UInt16),
        to cell: (column: UInt16, row: UInt16),
        unit: SelectionUnit,
        rectangle: Bool
    ) {
        do {
            try session.select(
                anchor: (x: anchor.column, y: anchor.row),
                to: (x: cell.column, y: cell.row),
                unit: unit,
                rectangle: rectangle
            )
        } catch {
            report(error)
        }
    }

    /// Let go of the selection.
    func clearSelection() {
        do {
            try session.setSelection(nil)
        } catch {
            report(error)
        }
    }

    /// The selection as plain text, or nil when there is nothing selected or
    /// the session refused.
    func selectedText() -> String? {
        do {
            return try session.copySelection()
        } catch {
            report(error)
            return nil
        }
    }

    /// Move the viewport into the scrollback, up positive.
    ///
    /// What the autoscroll timer calls while a drag is held outside the
    /// window. The core moves the viewport and publishes, so no scroll
    /// position is kept here.
    func scrollViewport(lines: Int) {
        do {
            try session.scrollViewport(lines: Int32(clamping: lines))
        } catch {
            report(error)
        }
    }

    /// Tell the session the window gained or lost focus.
    func focus(gained: Bool) {
        do {
            try session.focus(gained: gained)
        } catch {
            report(error)
        }
    }

    /// Hand the session text that is already text.
    ///
    /// What an input method finished making, which is not a key and so has no
    /// encoding left to decide. Marked text never comes this way — it is not
    /// in the terminal until it is committed, and putting it in the grid is
    /// what would make cancelling it impossible. cf. 05-swift-app 7.
    func write(_ text: String) {
        do {
            try session.write(Array(text.utf8))
        } catch {
            report(error)
        }
    }

    /// Whether a clipboard is worth asking the user about first.
    ///
    /// The engine's judgement with the policy's own condition on top, which
    /// is ``Paste/warns(about:)``. It comes through here rather than being
    /// asked directly: the view passes intent and never reaches the boundary
    /// itself. cf. 05-swift-app 4, 8.
    func warnsBeforePasting(_ text: String) -> Bool {
        Paste.warns(about: text)
    }

    /// Put a clipboard in the terminal.
    ///
    /// The sanitizing and the wrapping are inside the call, so there is
    /// nothing here that could be told to skip them: a user who read the
    /// warning and went ahead reaches the same one. cf. adr/0007.
    func paste(_ text: String) {
        do {
            try session.paste(Array(text.utf8))
        } catch {
            report(error)
        }
    }

    /// Nothing can act on a broken session yet: it keeps its last screen and
    /// M3 has nothing to put in its place. Saying so beats a window that
    /// quietly stops moving. cf. 05-swift-app 8 for the policy that arrives in
    /// M4.
    private func report(_ error: Error) {
        FileHandle.standardError.write(Data("knotty: \(error)\n".utf8))
    }
}
