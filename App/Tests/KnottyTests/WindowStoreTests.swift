import AppKit
import Testing

@testable import knotty

/// A domain of this test's own, so that what one test writes is not what the
/// next one reads and neither touches the app's real windows.
private func store(_ body: (WindowStore) throws -> Void) rethrows {
    let name = "knotty-windows-\(UUID().uuidString)"
    let defaults = UserDefaults(suiteName: name)!
    defer { defaults.removePersistentDomain(forName: name) }
    try body(WindowStore(defaults: defaults))
}

@Test func windowsComeBackWhereTheyWereSaved() {
    store { store in
        let states = [
            WindowState(frame: NSRect(x: 12, y: 34, width: 640, height: 400), directory: "/tmp"),
            WindowState(frame: NSRect(x: -80, y: 900, width: 300, height: 1_000), directory: nil),
        ]
        store.save(states)

        #expect(store.load() == states)
    }
}

@Test func aFrameWithAFractionSurvivesTheRoundTrip() {
    store { store in
        let state = WindowState(
            frame: NSRect(x: 10.5, y: 20.25, width: 640.75, height: 400.5), directory: nil
        )
        store.save([state])

        #expect(store.load() == [state])
    }
}

@Test func nothingSavedIsNoWindows() {
    store { store in
        #expect(store.load().isEmpty)
    }
}

@Test func savingAgainReplacesTheWindowsThatWereThere() {
    store { store in
        store.save([
            WindowState(frame: NSRect(x: 0, y: 0, width: 100, height: 100), directory: nil),
            WindowState(frame: NSRect(x: 0, y: 0, width: 200, height: 200), directory: nil),
        ])
        store.save([])

        #expect(store.load().isEmpty)
    }
}

/// A rectangle that will not read back is a window of no size, and one window
/// fewer beats one window nobody can grab.
@Test func anEntryThatIsNotARectangleIsDropped() {
    store { store in
        store.defaults.set(
            [
                ["frame": "not a rectangle"],
                ["directory": "/tmp"],
                ["frame": "{{0, 0}, {0, 0}}"],
                ["frame": "{{5, 5}, {640, 400}}", "directory": "/var"],
            ],
            forKey: "windows"
        )

        #expect(
            store.load() == [
                WindowState(frame: NSRect(x: 5, y: 5, width: 640, height: 400), directory: "/var")
            ]
        )
    }
}

/// `defaults write` takes whatever it is given, and a frame that is not a
/// number is one the grid cannot be divided out of.
@Test func aFrameThatIsNotFiniteIsDropped() {
    store { store in
        store.defaults.set(
            [["frame": "{{0, 0}, {inf, inf}}"], ["frame": "{{nan, 0}, {640, 400}}"]],
            forKey: "windows"
        )

        #expect(store.load().isEmpty)
    }
}

/// Written by another version, or by a hand with `defaults write`.
@Test func aValueThatIsNotAListOfWindowsIsNoWindows() {
    store { store in
        store.defaults.set("windows", forKey: "windows")

        #expect(store.load().isEmpty)
    }
}

/// Unset is the state of every machine nobody changed the setting on, and the
/// unticked box it stands for is the one that keeps windows.
@Test func windowsAreRestoredUntilTheSettingSaysOtherwise() {
    store { store in
        #expect(store.restoresWindows)

        store.defaults.set(false, forKey: "NSQuitAlwaysKeepsWindows")
        #expect(!store.restoresWindows)

        store.defaults.set(true, forKey: "NSQuitAlwaysKeepsWindows")
        #expect(store.restoresWindows)
    }
}

private let screens = [
    NSRect(x: 0, y: 0, width: 1_920, height: 1_080),
    NSRect(x: 1_920, y: 0, width: 1_440, height: 900),
]
private let fallback = NSRect(x: 0, y: 0, width: 1_920, height: 1_055)

@Test func aFrameOnAScreenOpensExactlyWhereItWas() {
    let frame = NSRect(x: 2_000, y: 100, width: 640, height: 400)

    #expect(WindowStore.placed(frame, screens: screens, fallback: fallback) == frame)
}

/// Parked mostly off the side is still parked: the user put it there.
@Test func aFrameHangingOffAScreenIsLeftAlone() {
    let frame = NSRect(x: -600, y: 100, width: 640, height: 400)

    #expect(WindowStore.placed(frame, screens: screens, fallback: fallback) == frame)
}

/// The display it was on is unplugged. The size is kept — the grid follows the
/// frame — and the position is not.
@Test func aFrameOnNoScreenComesBackInTheMiddleOfTheOne() {
    let frame = NSRect(x: 5_000, y: 4_000, width: 640, height: 400)

    #expect(
        WindowStore.placed(frame, screens: screens, fallback: fallback)
            == NSRect(x: 640, y: 327.5, width: 640, height: 400)
    )
}

/// Sharing an edge is having nothing on the screen.
@Test func aFrameTouchingAScreenOnlyAtItsEdgeIsRecentred() {
    let frame = NSRect(x: 3_360, y: 0, width: 640, height: 400)

    #expect(
        WindowStore.placed(frame, screens: screens, fallback: fallback)
            == NSRect(x: 640, y: 327.5, width: 640, height: 400)
    )
}

/// No screens at all is what a locked or headless machine answers, and the
/// caller has nowhere better to put the window than where it was.
@Test func withNoScreensAtAllTheFrameIsTakenAsItStands() {
    let frame = NSRect(x: 5_000, y: 4_000, width: 640, height: 400)

    #expect(WindowStore.placed(frame, screens: [], fallback: frame) == frame)
}
