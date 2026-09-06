import AppKit

import KnottyRender
import KnottySession

/// One window and everything under it: the session that feeds it and the view
/// that beats with it. cf. 05-swift-app 4.
final class TerminalWindowController: NSWindowController {
    /// The grid a window opens at. What it is afterwards is what the user
    /// dragged it to, and the view is what measures that.
    private static let columns: UInt16 = 80
    private static let rows: UInt16 = 24
    /// How much of what scrolled off a session keeps. A constant and not a
    /// key: what the configuration opens is what hurts daily without a
    /// rebuild, and this is not one of them. cf. 05-swift-app 10.
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
    static func spawningShell(config: Config) throws -> TerminalWindowController {
        // The primary font decides the cell alone, and the grid is the cell
        // times the counts above. cf. 04-renderer R4.
        //
        // Measured against the main screen because there is no window yet to
        // ask. A window that opens on a display of another scale is one whose
        // cells were snapped to the wrong pixels for as long as it takes the
        // view to lay out, which is what re-measures them against the display
        // it really came up on.
        let scale = Double(NSScreen.main?.backingScaleFactor ?? 2)
        let font = config.font
        let metrics = CellMetrics.system(
            pointSize: font.size, scale: scale, name: font.family
        )
        // The grid in device pixels, which is what the renderer places into.
        // The window is that in points, so it opens on whole cells and the
        // step it resizes by keeps it on them.
        let content = NSSize(
            width: Double(Int32(columns) * metrics.width) / scale,
            height: Double(Int32(rows) * metrics.height) / scale
        )

        let host = try SessionHost(
            columns: columns, rows: rows, scrollback: scrollback,
            metrics: metrics, font: font
        )

        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: content),
            styleMask: [.titled, .closable, .miniaturizable, .resizable],
            backing: .buffered,
            defer: false
        )
        // What the terminal was told to call itself, or the app's name until
        // something says otherwise. The window is held weakly by what renames
        // it: the session holds the closure and the window's own tree ends at
        // the session. cf. 06-integration.
        window.title = host.title
        host.onTitle = { [weak window] title in window?.title = title }
        // A window merged into another's tab group is not a window the app
        // opened, and "prefer tabs when opening documents" would do that to
        // every ⌘N. Tabs are not v1's. cf. 05-swift-app 3, adr/0010.
        window.tabbingMode = .disallowed
        let view = try TerminalView(host: host, font: font, scale: scale)
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
    ///
    /// The view goes out of the window with it. A display link holds its
    /// target and this view holds the link, so the pair keeps itself alive
    /// past the window that closed — and taking the view out is what runs the
    /// teardown that breaks it. One window never showed this; the second one
    /// is what makes it a leak. cf. `TerminalView.viewDidMoveToWindow`.
    func shutDown() {
        host = nil
        window?.contentView = NSView()
    }
}
