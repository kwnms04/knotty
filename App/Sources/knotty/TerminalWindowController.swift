import AppKit

import KnottyRender

/// One window and everything under it: the session that feeds it and the view
/// that beats with it. cf. 05-swift-app 4.
final class TerminalWindowController: NSWindowController {
    /// The grid a window opens at. What it is afterwards is what the user
    /// dragged it to, and the view is what measures that.
    private static let columns: UInt16 = 80
    private static let rows: UInt16 = 24
    /// The one size this milestone draws at. The configuration pipeline is
    /// M4's, so the face, the size and the grid are all constants.
    private static let pointSize = 13.0
    /// How much of what scrolled off a session keeps. Also M4's to configure,
    /// and applied to new sessions only when it is. cf. 05-swift-app 10.
    private static let scrollback = 10_000

    /// The only strong reference to the session there is.
    ///
    /// Quitting has to release it while the window is still up, and nothing
    /// else puts the child down — so it hangs off the controller rather than
    /// off the view AppKit holds.
    private var host: SessionHost?

    /// Spawn a shell and put a window around it.
    ///
    /// A factory rather than an initializer because the failure is the
    /// spawn's, and `NSWindowController.init()` is not one that can throw.
    static func spawningShell() throws -> TerminalWindowController {
        // The primary font decides the cell alone, and the grid is the cell
        // times the counts above. cf. 04-renderer R4.
        //
        // Measured against the main screen because there is no window yet to
        // ask. A window that opens on a display of another scale is one whose
        // cells were snapped to the wrong pixels for as long as it takes the
        // view to lay out, which is what re-measures them against the display
        // it really came up on.
        let scale = Double(NSScreen.main?.backingScaleFactor ?? 2)
        let metrics = CellMetrics.system(pointSize: pointSize, scale: scale)
        // The grid in device pixels, which is what the renderer places into.
        // The window is that in points, so it opens on whole cells and the
        // step it resizes by keeps it on them.
        let content = NSSize(
            width: Double(Int32(columns) * metrics.width) / scale,
            height: Double(Int32(rows) * metrics.height) / scale
        )

        let host = try SessionHost(
            columns: columns, rows: rows, scrollback: scrollback, metrics: metrics
        )

        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: content),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        window.title = "knotty"
        let view = try TerminalView(host: host, pointSize: pointSize, scale: scale)
        window.contentView = view
        // A key reaches a view through the responder chain, and a window whose
        // first responder is still itself answers a `keyDown` with a beep. The
        // window is what hands that out, so the object that made the window is
        // where it is handed out. cf. 05-swift-app 4.
        window.makeFirstResponder(view)
        window.center()

        let controller = TerminalWindowController(window: window)
        controller.host = host
        return controller
    }

    /// Release the session, which is what stops the child and collects it.
    /// Process exit alone does neither.
    func shutDown() {
        host = nil
    }
}
