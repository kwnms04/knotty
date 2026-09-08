import AppKit
import Foundation

import KnottySession

/// The windows, and the menu AppKit needs for the shortcuts that open, close
/// and quit them to exist.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The session registry: one controller per window, and the only strong
    /// reference to any of them. cf. 05-swift-app 4.
    private var terminals: [TerminalWindowController] = []

    /// What a window is opened with: what the file last said, and what the
    /// next reload is diffed against. cf. 05-swift-app 4, 10.
    ///
    /// Nil only until the first load. Nothing that asks for a window — the
    /// menu, the Dock icon — runs before it.
    private var config: Config?

    /// What tells this object the file was saved.
    ///
    /// Held for the app's lifetime, because letting go of it is what stops
    /// the watch. Nil where the system would not give one, which costs the
    /// following and not the app.
    private var watch: Config.Watch?

    /// What the file was last wrong about, or nil when it last parsed.
    ///
    /// Kept because a window opened after the typo has to say it too: what
    /// that window is running is the configuration this diagnostic is the
    /// reason for, and a fresh window with a clean top row would be the app
    /// saying the file is fine. cf. 05-swift-app 10.
    private var diagnostic: String?

    /// Where the windows of the last run are kept, and where this run's go.
    private let store = WindowStore(defaults: .standard)

    /// Whether the app is on its way out.
    ///
    /// `applicationWillTerminate(_:)` runs while every window is still up, and
    /// the windows are sent `willCloseNotification` after it — so a quit that
    /// went on saving would follow the whole set with an empty one and there
    /// would be nothing to come back to. Measured, not assumed.
    private var isTerminating = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.mainMenu()

        // What a window with no directory saved for it opens in: the app
        // process moves to where a shell should start rather than staying
        // wherever it was launched from, and the child inherits that. A
        // restored window does not come through here — it is spawned in the
        // directory it was saved with, which is per session and not
        // process-wide. cf. adr/0020.
        FileManager.default.changeCurrentDirectoryPath(NSHomeDirectory())

        // ⌘W closes a window and nothing else hears about it, so this is
        // where the registry shrinks. Every window is watched at once rather
        // than one at a time: what a controller would have to be told is
        // exactly what it cannot say for itself.
        NotificationCenter.default.addObserver(
            self, selector: #selector(windowWillClose(_:)),
            name: NSWindow.willCloseNotification, object: nil
        )
        // A window being dragged or resized is the other half of what moves
        // what is saved for it; the `cd` inside it is the session's to say.
        // Nothing here is on a timer: a save that ran while nothing happened
        // would be work in an idle app, which B7 does not have.
        // cf. 05-swift-app 9, adr/0020.
        for moved in [NSWindow.didMoveNotification, NSWindow.didResizeNotification] {
            NotificationCenter.default.addObserver(
                self, selector: #selector(windowMovedOrResized(_:)), name: moved, object: nil
            )
        }

        do {
            // A file that will not parse is not a reason not to start: what
            // comes back is the defaults with a diagnostic beside them, and
            // the window that opens on them is where it is said — after the
            // window, because there is nowhere to say it before one.
            // cf. 05-swift-app 10.
            let loaded = try Config.load()
            config = loaded.config
            try openSaved(config: loaded.config)
            show(diagnostic: loaded.diagnostic)
            // From here, saving the file is what applies it: nothing is
            // restarted to try a size.
            watch = Config.Watch { [weak self] in self?.reload() }
        } catch {
            // No shell, no terminal. There is nothing to put in a window and
            // no path yet for telling anyone why, so this dies where it broke
            // and leaves the reason in the crash report. The sheet that would
            // say it out loud arrives with the rest of the event policy in M4.
            //
            // A configuration that could not be read at all lands here too,
            // which is the two sides built from different sources rather than
            // anything in the file — a typo comes back as the diagnostic
            // above and starts a window all the same.
            fatalError("knotty could not start: \(error)")
        }

        NSApp.activate(ignoringOtherApps: true)
    }

    /// One question for all the windows, asked when any one of them has
    /// something running.
    ///
    /// Asked once rather than once per window. Someone who meant to quit
    /// should not have to answer a question per terminal, and what they are
    /// being asked — is anything running — is one question about the set.
    /// A terminate does not put windows through `windowShouldClose(_:)`, so
    /// nothing is asked twice. cf. 05-swift-app 8.
    func applicationShouldTerminate(
        _ sender: NSApplication
    ) -> NSApplication.TerminateReply {
        guard terminals.contains(where: { $0.isBusy }) else { return .terminateNow }
        return confirmEndingPrograms(
            verb: "Quit", consequence: "Quitting ends what is running."
        ) ? .terminateNow : .terminateCancel
    }

    /// Releasing the session is what puts the child down and collects it, so
    /// quitting goes through that rather than through process exit.
    ///
    /// What is saved for the windows is whatever the last event that touched
    /// one wrote, and from here on nothing more is written — the windows are
    /// about to be torn down, and that is not them being closed.
    func applicationWillTerminate(_ notification: Notification) {
        isTerminating = true
        terminals.forEach { $0.shutDown() }
    }

    /// The last window closing leaves the app up —
    /// `applicationShouldTerminateAfterLastWindowClosed` is not implemented
    /// and its default is `false` — so the Dock icon is what asks for the
    /// next one. cf. 05-swift-app 3.
    func applicationShouldHandleReopen(
        _ sender: NSApplication, hasVisibleWindows: Bool
    ) -> Bool {
        if terminals.isEmpty { newWindow(nil) }
        // What AppKit does with the windows there are is still its own: a
        // minimised window comes back rather than being replaced.
        return true
    }

    /// ⌘N: one more window on a shell of its own.
    @MainActor @objc private func newWindow(_ sender: Any?) {
        guard let config else { return }
        do {
            try open(config: config)
        } catch {
            // Unlike the failure at launch, this one has an app around it:
            // the windows already up are still good and the user is the one
            // who asked, so it says what went wrong and stays rather than
            // taking the running terminals down with it.
            NSAlert(error: error).runModal()
        }
    }

    /// The file was saved: read it again, and hand round what moved.
    ///
    /// **Item by item, and not the whole configuration.** What each one costs
    /// is different — a colour is a redraw and a face is every glyph baked
    /// again — so applying all of it on every save would put the atlas reset
    /// behind a light/dark switch. cf. 04-renderer R8, 05-swift-app 10.
    ///
    /// A file that will not parse changes nothing and says so: the windows go
    /// on in the configuration they were already running, which is the one
    /// that has to survive being typed in.
    @MainActor private func reload() {
        guard let current = config else { return }
        // A blob this side cannot read at all is the two sides built from
        // different sources, which nothing typed into the file can cause and
        // no banner would help with. The windows keep what they have.
        guard let loaded = try? Config.reload(keeping: current) else { return }
        show(diagnostic: loaded.diagnostic)

        let next = loaded.config
        if next.font != current.font { terminals.forEach { $0.apply(font: next.font) } }
        if next.theme != current.theme { terminals.forEach { $0.apply(theme: next.theme) } }
        config = next
    }

    /// Put a diagnostic in front of every window, or take it back with nil,
    /// and remember it for the windows there are not yet.
    @MainActor private func show(diagnostic: String?) {
        self.diagnostic = diagnostic
        terminals.forEach { $0.show(diagnostic: diagnostic) }
    }

    /// Open the windows the last run left, or one window where it left none.
    ///
    /// **`NSQuitAlwaysKeepsWindows` is honoured although none of this is macOS
    /// state restoration.** What that setting says is whether windows come
    /// back, not which machinery is to bring them; a user who turned it off
    /// and got windows anyway would be right to call it broken.
    ///
    /// It is the reading that stops and not the writing. A run with the
    /// setting off still saves what it has, so turning it back on comes back
    /// to the windows of the last run rather than to whichever ones were
    /// standing when it was turned off. cf. adr/0020.
    ///
    /// **Absent is not off.** The key is missing on a machine where nobody has
    /// touched the setting, and a missing key is nobody having said — so what
    /// stands there is knotty's own answer, which is that windows come back.
    /// Reading absent as off would mean the headline of this feature never
    /// happening until the user went looking for a checkbox.
    @MainActor private func openSaved(config: Config) throws {
        let saved = store.restoresWindows ? store.load() : []
        guard !saved.isEmpty else { return try open(config: config) }
        for state in saved {
            try open(config: config, state: state)
        }
    }

    /// Spawn a shell, put a window around it and keep the controller.
    @MainActor private func open(config: Config, state: WindowState? = nil) throws {
        var state = state
        let terminal: TerminalWindowController
        do {
            terminal = try TerminalWindowController.spawningShell(config: config, state: state)
        } catch {
            // **A saved directory that is not there any more opens the window
            // anyway.** Moved, deleted, on a volume nobody mounted this
            // morning — the spawn throws, and this runs at launch, so the
            // window that cannot be opened is the app that cannot start. The
            // store would still hold the same directory on the next launch and
            // on every one after it, with nothing but `defaults delete` to get
            // out of: an app that talks itself out of starting is worse than a
            // window that came up in the wrong place. The next frame writes
            // where the shell really is, so it mends itself.
            guard state?.directory != nil else { throw error }
            state?.directory = nil
            terminal = try TerminalWindowController.spawningShell(config: config, state: state)
        }
        // A window opened exactly over the last one is one the user cannot
        // tell is there, and every fresh window opens centred. AppKit steps it
        // down and right from the one opened before it. A restored window has
        // a place of its own and is left standing in it.
        if state == nil, let previous = terminals.last?.window, let window = terminal.window {
            window.cascadeTopLeft(from: previous.cascadeTopLeft(from: .zero))
        }
        terminals.append(terminal)
        terminal.onSavedStateChange = { [weak self] in self?.saveWindows() }
        terminal.show(diagnostic: diagnostic)
        terminal.showWindow(nil)
        saveWindows()
    }

    /// Put the windows there are now in the store, in the order they were
    /// opened.
    ///
    /// All of them for a change to one: what is written is a list, so there is
    /// nothing smaller to write, and a window's own state is asked of it
    /// rather than kept here twice.
    @MainActor private func saveWindows() {
        guard !isTerminating else { return }
        store.save(terminals.compactMap(\.savedState))
    }

    /// A window was dragged or resized.
    ///
    /// Named for both, because it is registered for both — and not
    /// `windowDidMove(_:)`, which is an `NSWindowDelegate` method this object
    /// does not implement and would be read as implementing.
    ///
    /// A drag posts this the whole way across the screen and each one is a
    /// write; that is a user with a window in their hand, not an idle app.
    /// A window nothing here opened — a sheet, an alert — writes the same list
    /// back, which is why it is not worth telling them apart.
    @MainActor @objc private func windowMovedOrResized(_ notification: Notification) {
        saveWindows()
    }

    /// Let go of a window that closed, which is what puts its child down.
    ///
    /// Watched rather than delegated: the array is here, and a controller
    /// that had to reach back into it for the one thing it cannot do itself
    /// would be the registry written twice. A window nothing here opened —
    /// a sheet, an alert — is not one of ours and falls through.
    @MainActor @objc private func windowWillClose(_ notification: Notification) {
        guard
            let window = notification.object as? NSWindow,
            let terminal = window.windowController as? TerminalWindowController
        else { return }
        terminal.shutDown()
        terminals.removeAll { $0 === terminal }
        // A window the user closed is a window that does not come back.
        saveWindows()
    }

    /// An app with no menu has no quit shortcut either, which is why the
    /// minimum is a menu and not nothing.
    ///
    /// ⌘N and ⌘W are the whole of what the File menu is for. `Close` names no
    /// target, so the responder chain answers it with the window being typed
    /// into and AppKit greys it out when there is none — which is also what
    /// leaves the app up with no windows.
    ///
    /// Copy and paste are here for a second reason as well as their own. A
    /// menu's key equivalent is offered before any view sees the event, so ⌘C
    /// reaching the item is also ⌘C not reaching the child — where it would
    /// have been encoded as a key like any other. ⌃C is the one that
    /// interrupts, and nothing here touches it.
    private static func mainMenu() -> NSMenu {
        let quit = NSMenuItem(
            title: "Quit knotty",
            action: #selector(NSApplication.terminate(_:)),
            keyEquivalent: "q"
        )
        let applicationMenu = NSMenu(title: "knotty")
        applicationMenu.addItem(quit)

        let applicationItem = NSMenuItem()
        applicationItem.submenu = applicationMenu

        let newWindow = NSMenuItem(
            title: "New Window",
            action: #selector(AppDelegate.newWindow(_:)),
            keyEquivalent: "n"
        )
        let close = NSMenuItem(
            title: "Close",
            action: #selector(NSWindow.performClose(_:)),
            keyEquivalent: "w"
        )
        let fileMenu = NSMenu(title: "File")
        fileMenu.addItem(newWindow)
        fileMenu.addItem(close)

        let fileItem = NSMenuItem()
        fileItem.submenu = fileMenu

        let copy = NSMenuItem(
            title: "Copy",
            action: #selector(NSText.copy(_:)),
            keyEquivalent: "c"
        )
        let paste = NSMenuItem(
            title: "Paste",
            action: #selector(NSText.paste(_:)),
            keyEquivalent: "v"
        )
        let editMenu = NSMenu(title: "Edit")
        editMenu.addItem(copy)
        editMenu.addItem(paste)

        let editItem = NSMenuItem()
        editItem.submenu = editMenu

        let menu = NSMenu()
        menu.addItem(applicationItem)
        menu.addItem(fileItem)
        menu.addItem(editItem)
        return menu
    }
}
