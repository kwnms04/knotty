import AppKit

/// One window, and the menu AppKit needs for the quit shortcut to exist.
final class AppDelegate: NSObject, NSApplicationDelegate {
    private var window: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        NSApp.mainMenu = Self.mainMenu()

        // Not resizable: reflow is the one blocking call across the boundary,
        // so the window that proves the pipeline runs does not offer it.
        let window = NSWindow(
            contentRect: NSRect(x: 0, y: 0, width: 640, height: 384),
            styleMask: [.titled, .closable, .miniaturizable],
            backing: .buffered,
            defer: false
        )
        window.title = "knotty"
        window.center()
        window.makeKeyAndOrderFront(nil)
        self.window = window

        NSApp.activate(ignoringOtherApps: true)
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
