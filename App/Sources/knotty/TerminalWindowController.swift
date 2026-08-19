import AppKit

import KnottyRender

/// One window and everything under it: the session that feeds it and the view
/// that beats with it. cf. 05-swift-app 4.
final class TerminalWindowController: NSWindowController {
    /// The grid, fixed, and the window does not resize to another one. Reflow
    /// is the exception to the boundary's non-blocking contract and costs what
    /// the scrollback is long, so it arrives with the input path in M3 rather
    /// than in the milestone that proves the pipeline runs.
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
        // ask, so a window that opens on a display of another scale is one
        // whose cells were snapped to the wrong pixels. Following the window
        // instead means re-measuring and resizing, which is the resize path M2
        // leaves out.
        let scale = Double(NSScreen.main?.backingScaleFactor ?? 2)
        let metrics = CellMetrics.system(pointSize: pointSize, scale: scale)
        // The grid in device pixels is what the renderer places into and what
        // the drawable is sized to; the window is that in points.
        let pixels = NSSize(
            width: Double(Int32(columns) * metrics.width),
            height: Double(Int32(rows) * metrics.height)
        )
        let content = NSSize(width: pixels.width / scale, height: pixels.height / scale)

        let host = try SessionHost(
            columns: columns, rows: rows, scrollback: scrollback, metrics: metrics
        )

        // Resizable is left out of the style rather than taken away later: a
        // window that proves the pipeline runs does not offer what the
        // pipeline cannot yet do.
        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: content),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "knotty"
        window.contentView = try TerminalView(
            host: host, metrics: metrics, pixels: pixels, scale: scale
        )
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
