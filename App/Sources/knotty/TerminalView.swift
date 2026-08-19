import AppKit
import QuartzCore
import os

/// The view the terminal appears in, and the owner of the beat it appears on.
///
/// Nothing is drawn here yet; the Metal layer is the next ticket. What stands
/// here is the loop 05-swift-app 6 describes — a wake resumes the link, a tick
/// takes the frame, and a tick with nothing behind it stops the link again.
final class TerminalView: NSView {
    /// Whose frames these are.
    ///
    /// Weak, because the window controller owns it and quitting has to be able
    /// to release it while the window is still up: releasing the session is
    /// what puts the child down.
    private weak var host: SessionHost?

    /// Raised by the core's thread, lowered by a tick. The whole of what the
    /// wake callback is allowed to touch.
    private let pending = Pending()

    private var link: CADisplayLink?

    init(host: SessionHost) throws {
        self.host = host
        super.init(frame: .zero)

        // All the core's thread does is raise the flag, and it crosses to main
        // only on the raise that changed it. N wakes become one block, which
        // is the coalescing the boundary already does, carried on to this side.
        try host.onWake { [pending, weak self] in
            guard pending.raise() else { return }
            DispatchQueue.main.async {
                MainActor.assumeIsolated { self?.link?.isPaused = false }
            }
        }
    }

    @available(*, unavailable)
    required init?(coder: NSCoder) {
        fatalError("knotty builds its views in code")
    }

    /// The link belongs to the display the view is on, so it is asked for once
    /// the view knows which that is. It follows the view to another display on
    /// its own.
    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        // A link holds its target, and this view holds the link — so a view
        // that leaves its window and is never invalidated is a pair that keeps
        // each other alive. One window makes that invisible; the second one
        // makes it a leak.
        guard window != nil else {
            link?.invalidate()
            link = nil
            return
        }
        guard link == nil else { return }
        let link = displayLink(target: self, selector: #selector(tick))
        // Running rather than paused: a wake that arrived before there was a
        // link left the flag raised, and the first tick is what finds it.
        link.add(to: .main, forMode: .common)
        self.link = link
    }

    @objc private func tick() {
        // One tick with nothing behind it stops the link, with no hysteresis.
        // Coming back costs a frame; what it buys is an idle app that does
        // nothing at all. cf. 05-swift-app 6.
        guard pending.lower() else {
            link?.isPaused = true
            return
        }
        host?.takeFrame()
    }
}

/// The flag between the core's thread and the main queue.
///
/// A lock rather than an atomic, which is what would say this in one word: the
/// atomics live in `Synchronization`, and reaching for them would raise the
/// app's floor to macOS 15 for a single bit. Held for one read and one write
/// either way, and uncontended on every path there is.
private final class Pending: Sendable {
    private let raised = OSAllocatedUnfairLock(initialState: false)

    /// Raise it, answering whether this call is the one that raised it — which
    /// is the only time a block is worth posting to main.
    func raise() -> Bool {
        raised.withLock { raised in
            defer { raised = true }
            return !raised
        }
    }

    /// Lower it, answering whether it had been raised.
    func lower() -> Bool {
        raised.withLock { raised in
            defer { raised = false }
            return raised
        }
    }
}
