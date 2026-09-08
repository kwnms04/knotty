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

    /// Whether a program is running in front of this window's shell.
    ///
    /// A window whose session has already been released has nothing running
    /// in it, which is what keeps shutting one down twice quiet.
    var isBusy: Bool { host?.isBusy ?? false }

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

        // Nowhere in particular: this app opens every window in its own
        // working directory. What a restored window opens in is the directory
        // it was saved with, which is the milestone's next step.
        let host = try SessionHost(
            columns: columns, rows: rows, scrollback: scrollback,
            metrics: metrics, font: font, theme: config.theme, directory: nil
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
        let view = try TerminalView(host: host, font: font, theme: config.theme, scale: scale)
        window.contentView = view
        // A key reaches a view through the responder chain, and a window whose
        // first responder is still itself answers a `keyDown` with a beep. The
        // window is what hands that out, so the object that made the window is
        // where it is handed out. cf. 05-swift-app 4.
        window.makeFirstResponder(view)
        window.center()

        let controller = TerminalWindowController(window: window)
        controller.host = host
        // Set rather than inherited: `NSWindowController` does not make itself
        // the delegate of a window it was handed, and the delegate is the only
        // thing asked whether a close may go ahead.
        window.delegate = controller
        return controller
    }

    /// The view under the window, while there is one.
    ///
    /// Nil after ``shutDown()``, which takes it out — so a configuration that
    /// changed while a window was closing reaches nothing, rather than
    /// reaching a session that is already gone.
    private var view: TerminalView? { window?.contentView as? TerminalView }

    /// Put a face the file changed in force: the same grid, at a new cell,
    /// in a window that moved to hold it.
    ///
    /// **The grid is what is kept and the window is what gives way.** A
    /// terminal whose columns held has nothing to reflow — no `SIGWINCH`, no
    /// rewrap, and none of the cost of one on a scrollback of any size. The
    /// other way round, the window would have to be snapped back to whole
    /// cells afterwards anyway, so "the window stays put" would not be true
    /// either. It is what Terminal.app and iTerm2 do. cf. 05-swift-app 10.
    func apply(font: Config.Font) {
        guard let window, let view else { return }
        host?.apply(font: font)
        window.setFrame(Self.fitted(content: view.use(font: font), of: window), display: true)
    }

    /// Put colours the file changed in force, in the core and in the view.
    func apply(theme: Config.Theme) {
        host?.apply(theme: theme)
        view?.use(theme: theme)
    }

    /// Say what is wrong with the configuration file, or take it back.
    func show(diagnostic: String?) {
        view?.show(diagnostic: diagnostic)
    }

    /// Where a window holding this much content goes.
    ///
    /// The top left stays where it was, because that is the corner the text
    /// starts in and the one the eye is on. What will not fit on the screen
    /// is given up — and giving up frame is giving up grid, since the layout
    /// under it divides whatever it is left by the cell. That is the whole of
    /// "a grid that shrinks to what the screen can hold": no second count of
    /// the cells, and no second place where one could disagree with the
    /// other. cf. 05-swift-app 10.
    private static func fitted(content: NSSize, of window: NSWindow) -> NSRect {
        var size = window.frameRect(forContentRect: NSRect(origin: .zero, size: content)).size
        let corner = NSPoint(x: window.frame.minX, y: window.frame.maxY)
        guard let visible = window.screen?.visibleFrame else {
            return NSRect(
                x: corner.x, y: corner.y - size.height, width: size.width, height: size.height
            )
        }
        size.width = min(size.width, visible.width)
        size.height = min(size.height, visible.height)
        // Held inside the screen after the size settled, so that a window
        // that grew against an edge comes back in rather than off.
        return NSRect(
            x: min(max(corner.x, visible.minX), visible.maxX - size.width),
            y: min(max(corner.y - size.height, visible.minY), visible.maxY - size.height),
            width: size.width,
            height: size.height
        )
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

extension TerminalWindowController: NSWindowDelegate {
    /// Warn before a window with a program running in it goes away.
    ///
    /// A prompt with nothing at it closes without a word, which is the whole
    /// point: a warning that comes up every time is one that gets clicked
    /// through, and then it is not there for the `make` either.
    /// cf. 05-swift-app 8.
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        guard isBusy else { return true }
        return confirmEndingPrograms(
            verb: "Close", consequence: "Closing this window ends what is running in it."
        )
    }
}

/// Ask before a running program is taken down with the window it is in,
/// answering whether to go ahead.
///
/// Modal rather than the sheet a paste warning gets, and the one place this
/// app runs a modal on purpose: `windowShouldClose(_:)` and
/// `applicationShouldTerminate(_:)` are both answered on the spot with a
/// value, and a sheet answers in a callback — so a sheet here would mean
/// closing the window twice, once to ask and once to mean it.
///
/// What is running is not named. Which program it is needs a lookup the
/// boundary has no call for, and "something is running" is the whole of what
/// v1 says. cf. 05-swift-app 8.
///
/// Cancel is first for the reason the paste warning gives: it is the button
/// return presses, and the answer that cannot be undone should not be the one
/// a stray keystroke gives.
@MainActor func confirmEndingPrograms(verb: String, consequence: String) -> Bool {
    let alert = NSAlert()
    alert.alertStyle = .warning
    alert.messageText = "A program is still running"
    alert.informativeText = consequence
    alert.addButton(withTitle: "Cancel")
    alert.addButton(withTitle: verb)
    return alert.runModal() == .alertSecondButtonReturn
}
