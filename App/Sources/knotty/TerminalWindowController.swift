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

    /// What a bell does, out of the configuration file. The one event policy
    /// that is a setting. cf. 05-swift-app 8.
    private var bellMode = Config.Bell.Mode.visual

    /// Whether this session may write the clipboard, or nil while nobody has
    /// been asked yet.
    ///
    /// The whole of the policy: asked once, and the answer stands until the
    /// window goes. It hangs off the controller because the window is what the
    /// answer is about — another window asks again. cf. 05-swift-app 8.
    private var clipboardWrites: Bool?

    /// Whether a program is running in front of this window's shell.
    ///
    /// A window whose session has already been released has nothing running
    /// in it, which is what keeps shutting one down twice quiet.
    var isBusy: Bool { host?.isBusy ?? false }

    /// What is kept of this window for the next run, or nil once the window
    /// has gone.
    ///
    /// Asked of the window rather than remembered, so that there is one answer
    /// to where the window is and AppKit has it. cf. 05-swift-app 9.
    var savedState: WindowState? {
        guard let window else { return nil }
        return WindowState(frame: window.frame, directory: host?.workingDirectory)
    }

    /// What to call when what would be saved for this window changed on its
    /// own, which is the shell's directory moving.
    ///
    /// Moving and resizing the window is not in here: AppKit posts those, and
    /// the object that keeps the windows hears them for all of them at once.
    var onSavedStateChange: (() -> Void)?

    /// Spawn a shell and put a window around it, on `state` where a run before
    /// this one left one.
    ///
    /// A factory rather than an initializer because the failure is the
    /// spawn's, and `NSWindowController.init()` is not one that can throw.
    static func spawningShell(
        config: Config, state: WindowState? = nil
    ) throws -> TerminalWindowController {
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
        let style: NSWindow.StyleMask = [.titled, .closable, .miniaturizable, .resizable]
        // **The frame is the truth and the grid is derived from it.** A window
        // that was saved comes back the rectangle it was, and how many cells
        // that holds is whatever the cell measures now — so a font changed
        // while the app was down moves the grid rather than the window.
        // A window nothing was saved for goes the other way: the counts above
        // are what it has, and the rectangle follows from them.
        // cf. 05-swift-app 9.
        let restored = state.map {
            WindowStore.placed(
                $0.frame,
                screens: NSScreen.screens.map(\.frame),
                fallback: NSScreen.main?.visibleFrame ?? $0.frame
            )
        }
        // The grid in device pixels, which is what the renderer places into.
        // The window is that in points, so a fresh one opens on whole cells and
        // the step it resizes by keeps it on them.
        let content =
            restored.map { NSWindow.contentRect(forFrameRect: $0, styleMask: style).size }
            ?? NSSize(
                width: Double(Int32(columns) * metrics.width) / scale,
                height: Double(Int32(rows) * metrics.height) / scale
            )
        let grid =
            restored == nil
            ? (columns: columns, rows: rows)
            : (
                columns: cells(content.width * scale, per: metrics.width),
                rows: cells(content.height * scale, per: metrics.height)
            )

        // Where a restored window's shell comes up, and nowhere in particular
        // for every other one — which leaves it in this process's own working
        // directory. cf. adr/0020.
        let host = try SessionHost(
            columns: grid.columns, rows: grid.rows, scrollback: scrollback,
            metrics: metrics, font: font, theme: config.theme, directory: state?.directory
        )

        let window = NSWindow(
            contentRect: NSRect(origin: .zero, size: content),
            styleMask: style,
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
        if let restored { window.setFrame(restored, display: false) } else { window.center() }

        let controller = TerminalWindowController(window: window)
        controller.host = host
        controller.bellMode = config.bell.mode
        // The window's own, for the reason 05-swift-app 8 makes them policy:
        // every one of the three answers is a window's to give. Weak, because
        // the controller is what owns the session the closure hangs off.
        host.onEvent = { [weak controller] event in controller?.answer(event) }
        host.onChildExit = { [weak controller] code in controller?.finish(code: code) }
        // A `cd` is the one thing in the saved state that moves without AppKit
        // saying so. Weak, because the controller is what owns the session the
        // closure hangs off.
        host.onWorkingDirectory = { [weak controller] in controller?.onSavedStateChange?() }
        // Set rather than inherited: `NSWindowController` does not make itself
        // the delegate of a window it was handed, and the delegate is the only
        // thing asked whether a close may go ahead.
        window.delegate = controller
        return controller
    }

    /// How many whole cells that many device pixels hold.
    ///
    /// Never none: a terminal of no rows is one nothing can be written to, and
    /// a frame saved smaller than a single cell is worth one cell rather than
    /// a refusal.
    private static func cells(_ pixels: Double, per size: Int32) -> UInt16 {
        UInt16(clamping: max(1, Int(pixels) / Int(size)))
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

    /// Take a bell the file changed. Nothing is redrawn for it — it is read
    /// on the next bell and not before.
    func apply(bell: Config.Bell) {
        bellMode = bell.mode
    }

    /// Answer one event the session queued. cf. 05-swift-app 8.
    private func answer(_ event: Event) {
        switch event {
        case .bell:
            ring()
        case .clipboardWrite(let text):
            copy(text)
        case .childExited:
            // Nothing, and not because there is nothing to do. The exit goes
            // out both ways and the queue is the losable one, so what the
            // window is closed on is the frame — ``SessionHost/onChildExit``.
            // cf. 02-ffi.
            break
        }
    }

    /// What a bell comes to: the mode the file names, and the Dock on top of
    /// it when this is not the window being looked at — a flash nobody can see
    /// is a bell nobody was told about.
    ///
    /// The badge is cleared where the looking happens, which is `AppDelegate`:
    /// the Dock has one icon for all the windows, and clearing it is about the
    /// app coming forward rather than about this window.
    private func ring() {
        switch bellMode {
        case .off: return
        case .visual: view?.flash()
        case .sound: NSSound.beep()
        case .bounce: NSApp.requestUserAttention(.informationalRequest)
        }
        guard !(NSApp.isActive && window?.isKeyWindow == true) else { return }
        NSApp.dockTile.badgeLabel = "●"
    }

    /// Put what the child asked to copy on the pasteboard, having asked the
    /// user once.
    ///
    /// A remote tmux's `OSC 52` is what this is for: the copy happens on the
    /// far side of the ssh and the clipboard it is bound for is this one.
    /// Asked because a program that can write the clipboard unasked can empty
    /// it, and asked once because a question per copy is one that gets
    /// clicked through. cf. 05-swift-app 8.
    private func copy(_ text: String) {
        if let allowed = clipboardWrites {
            if allowed { Self.put(text) }
            return
        }
        // A window with a sheet already up is one whose question has not been
        // answered yet, and a write nobody has said yes to is not one to make.
        // Dropping it is the answer that can be taken back.
        guard let window, window.attachedSheet == nil else { return }

        let alert = NSAlert()
        alert.messageText = "Let this terminal write to the clipboard?"
        alert.informativeText = """
            A program in this window asked to put text on the clipboard. \
            The answer is kept until the window closes.
            """
        // Refusing first, which is the button ⏎ presses: the clipboard is
        // something the user has in hand, and overwriting it is the answer a
        // stray keystroke should not give.
        alert.addButton(withTitle: "Don't Allow")
        alert.addButton(withTitle: "Allow")
        alert.beginSheetModal(for: window) { [weak self] response in
            let allowed = response == .alertSecondButtonReturn
            self?.clipboardWrites = allowed
            if allowed { Self.put(text) }
        }
    }

    private static func put(_ text: String) {
        let pasteboard = NSPasteboard.general
        pasteboard.clearContents()
        pasteboard.setString(text, forType: .string)
    }

    /// The child is gone: the window goes with it on a clean exit, and stays
    /// on any other saying what ended it.
    ///
    /// ⌃D at a prompt is the first of those and the reason it is not a
    /// setting — a window left standing on a shell that is not there any more
    /// is one the user closes by hand every time. cf. 05-swift-app 8.
    private func finish(code: Int32) {
        guard code == 0 else {
            // Nothing more will be published, so what the title says now is
            // what it goes on saying.
            window?.title = "\(host?.title ?? "knotty") — exited \(code)"
            return
        }
        // Not from here: this is answered from inside the frame the view is
        // taking, and closing the window takes that view out of it mid-tick.
        DispatchQueue.main.async { [weak self] in self?.window?.close() }
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
