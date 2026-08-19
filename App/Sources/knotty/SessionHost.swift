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
    private let renderer: Renderer

    /// Spawn the user's login shell behind a terminal of this size, drawn at
    /// these metrics.
    init(columns: UInt16, rows: UInt16, scrollback: Int, metrics: CellMetrics) throws {
        session = try Session(
            command: LoginShell.command, cols: columns, rows: rows, scrollback: scrollback
        )
        renderer = Renderer(metrics: metrics)
    }

    /// A page side, in device pixels — the size of the texture the frames
    /// below place their glyphs on.
    var atlasSide: Int32 { renderer.atlasSide }

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
            return try session.withSnapshot { renderer.frame(for: $0) }
        } catch {
            // Nothing can act on this yet: a broken session keeps its last
            // screen and M2 has nothing to put in its place. Saying so beats
            // a window that quietly stops moving. cf. 05-swift-app 8 for the
            // policy that arrives in M4.
            FileHandle.standardError.write(Data("knotty: \(error)\n".utf8))
            return nil
        }
    }
}
