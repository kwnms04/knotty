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

    /// Spawn the user's login shell behind a terminal of this size, drawn at
    /// these metrics.
    init(columns: UInt16, rows: UInt16, scrollback: Int, metrics: CellMetrics) throws {
        session = try Session(
            command: LoginShell.command, cols: columns, rows: rows, scrollback: scrollback
        )
        renderer = Renderer(metrics: metrics)
        self.metrics = metrics
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
            renderer = Renderer(metrics: metrics)
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
            return try session.withSnapshot { snapshot in
                cursorCell =
                    snapshot.cursor.visible
                    ? (column: Int(snapshot.cursor.x), row: Int(snapshot.cursor.y)) : nil
                return renderer.frame(for: snapshot)
            }
        } catch {
            report(error)
            return nil
        }
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

    /// Nothing can act on a broken session yet: it keeps its last screen and
    /// M3 has nothing to put in its place. Saying so beats a window that
    /// quietly stops moving. cf. 05-swift-app 8 for the policy that arrives in
    /// M4.
    private func report(_ error: Error) {
        FileHandle.standardError.write(Data("knotty: \(error)\n".utf8))
    }
}
