import AppKit
import Foundation

import KnottySession

/// The windows, and the menu AppKit needs for the shortcuts that open, close
/// and quit them to exist.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The session registry: one controller per window, and the only strong
    /// reference to any of them. cf. 05-swift-app 4.
    private var terminals: [TerminalWindowController] = []

    /// What a window is opened with, read once at launch. Watching the file
    /// and handing round what changed is M4's next ticket; what stands here
    /// is the one value every window is opened from. cf. 05-swift-app 4, 10.
    ///
    /// Nil only until that read. Nothing that asks for a window — the menu,
    /// the Dock icon — runs before it.
    private var config: Config?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.mainMenu()

        // The child inherits this, and there is no working directory in the
        // call that spawns one — so the app process moves to where the shell
        // should start rather than wherever it was launched from. Process-wide,
        // which is what the window restoration of M4 cannot be built on; the
        // defect is recorded in open-questions.
        FileManager.default.changeCurrentDirectoryPath(NSHomeDirectory())

        // ⌘W closes a window and nothing else hears about it, so this is
        // where the registry shrinks. Every window is watched at once rather
        // than one at a time: what a controller would have to be told is
        // exactly what it cannot say for itself.
        NotificationCenter.default.addObserver(
            self, selector: #selector(windowWillClose(_:)),
            name: NSWindow.willCloseNotification, object: nil
        )

        do {
            // A file that will not parse is not a reason not to start: what
            // comes back is the defaults with a diagnostic beside them. The
            // banner that shows one is M4's window work; until then it goes
            // where every other thing this app has no window for goes.
            // cf. 05-swift-app 10.
            let loaded = try Config.load()
            if let diagnostic = loaded.diagnostic {
                FileHandle.standardError.write(
                    Data("knotty: \(Config.path.path(percentEncoded: false)): \(diagnostic)\n".utf8)
                )
            }

            config = loaded.config
            try open(config: loaded.config)
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

    /// Releasing the session is what puts the child down and collects it, so
    /// quitting goes through that rather than through process exit.
    func applicationWillTerminate(_ notification: Notification) {
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

    /// Spawn a shell, put a window around it and keep the controller.
    @MainActor private func open(config: Config) throws {
        let terminal = try TerminalWindowController.spawningShell(config: config)
        // A window opened exactly over the last one is one the user cannot
        // tell is there, and every window opens centred. AppKit steps it down
        // and right from the one opened before it.
        if let previous = terminals.last?.window, let window = terminal.window {
            window.cascadeTopLeft(from: previous.cascadeTopLeft(from: .zero))
        }
        terminals.append(terminal)
        terminal.showWindow(nil)
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
