import AppKit
import Foundation

import KnottySession

/// One window, and the menu AppKit needs for the quit, copy and paste
/// shortcuts to exist.
final class AppDelegate: NSObject, NSApplicationDelegate {
    /// The session registry, at the one size M2 has a path to. What would open
    /// a second window is the menu item M4 adds. cf. 05-swift-app 4.
    private var terminal: TerminalWindowController?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.mainMenu()

        // The child inherits this, and there is no working directory in the
        // call that spawns one — so the app process moves to where the shell
        // should start rather than wherever it was launched from. Process-wide,
        // which is what the window restoration of M4 cannot be built on; the
        // defect is recorded in open-questions.
        FileManager.default.changeCurrentDirectoryPath(NSHomeDirectory())

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

            let terminal = try TerminalWindowController.spawningShell(config: loaded.config)
            terminal.showWindow(nil)
            self.terminal = terminal
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
        terminal?.shutDown()
    }

    /// An app with no menu has no quit shortcut either, which is why the
    /// minimum is a menu and not nothing.
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
        menu.addItem(editItem)
        return menu
    }
}
