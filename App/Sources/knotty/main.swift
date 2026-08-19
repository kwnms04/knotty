import AppKit

import KnottySession

// The first thing the process does. Nothing read across the boundary after
// this point is safe if the two sides disagree about layouts.
ABI.requireMatch()

let application = NSApplication.shared
application.setActivationPolicy(.regular)
let delegate = AppDelegate()
application.delegate = delegate
application.run()
