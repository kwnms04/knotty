import Foundation

import KnottySession

/// The one object that touches a session handle.
///
/// The boundary is written for calls that are serialized per session, and this
/// is what makes that structure rather than discipline: the handle lives here
/// and nowhere else, so there is no second path to it to race with. The view
/// above passes intent; it does not call. cf. 05-swift-app 4.
@MainActor
final class SessionHost {
    private let session: Session

    /// Spawn the user's login shell behind a terminal of this size.
    init(columns: UInt16, rows: UInt16, scrollback: Int) throws {
        session = try Session(
            command: LoginShell.command, cols: columns, rows: rows, scrollback: scrollback
        )
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
    /// newest frame received.
    ///
    /// Nothing is drawn with the frame yet — the Metal path is the next
    /// ticket. What stands here is the beat, and what proves it is that a
    /// frame comes out at all.
    func takeFrame() {
        do {
            try session.drainEvents()
            _ = try session.withSnapshot { _ in }
        } catch {
            // Nothing can act on this yet: a broken session keeps its last
            // screen and M2 has nothing to put in its place. Saying so beats
            // a window that quietly stops moving. cf. 05-swift-app 8 for the
            // policy that arrives in M4.
            FileHandle.standardError.write(Data("knotty: \(error)\n".utf8))
        }
    }
}
