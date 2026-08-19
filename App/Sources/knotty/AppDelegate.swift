import AppKit
import Foundation

/// One window, and the menu AppKit needs for the quit shortcut to exist.
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
            let terminal = try TerminalWindowController.spawningShell()
            terminal.showWindow(nil)
            self.terminal = terminal
        } catch {
            // No shell, no terminal. There is nothing to put in a window and
            // no path yet for telling anyone why, so this dies where it broke
            // and leaves the reason in the crash report. The sheet that would
            // say it out loud arrives with the rest of the event policy in M4.
            fatalError("knotty could not start a shell: \(error)")
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

        let menu = NSMenu()
        menu.addItem(applicationItem)
        return menu
    }
}
