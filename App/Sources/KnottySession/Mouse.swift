import CKnotty

/// Which way a mouse moved.
public enum MouseAction: Sendable {
    /// A button went down.
    case press
    /// A button came back up.
    case release
    /// The pointer moved.
    case motion

    var raw: UInt8 {
        let action =
            switch self {
            case .press: KT_MOUSE_ACTION_PRESS
            case .release: KT_MOUSE_ACTION_RELEASE
            case .motion: KT_MOUSE_ACTION_MOTION
            }
        return UInt8(action.rawValue)
    }
}

/// Which button a mouse event is about.
public enum MouseButton: Sendable {
    /// No button, which only a motion can be.
    case none
    /// The left button.
    case left
    /// The right button.
    case right
    /// The middle button.
    case middle

    /// Which button `NSEvent.buttonNumber` names, or nil for one no terminal
    /// protocol has a code for.
    ///
    /// AppKit numbers them in the order the protocols do not: 1 is the right
    /// button and 2 the middle. Past those are the side buttons, which a
    /// terminal could carry — the engine numbers eleven — and which nothing
    /// here has yet been asked for.
    public init?(buttonNumber: Int) {
        switch buttonNumber {
        case 0: self = .left
        case 1: self = .right
        case 2: self = .middle
        default: return nil
        }
    }

    var raw: UInt8 {
        let button =
            switch self {
            case .none: KT_MOUSE_BUTTON_NONE
            case .left: KT_MOUSE_BUTTON_LEFT
            case .right: KT_MOUSE_BUTTON_RIGHT
            case .middle: KT_MOUSE_BUTTON_MIDDLE
            }
        return UInt8(button.rawValue)
    }
}

/// How many whole lines a wheel has turned, and the fraction of one it is
/// still holding.
///
/// A trackpad reports its inertia in pixels and reports a great many of them:
/// a single flick is hundreds of events, most of them a fraction of a line.
/// The core is told in lines, so what is left over between two calls has to
/// wait somewhere — and here is where, because the height a line is drawn at
/// is what turns one into the other and that belongs to the display side.
///
/// A wheel with detents reports in lines already. macOS ramps the magnitude of
/// those with how fast the wheel is spun and reports the slowest turn as a
/// tenth of a line, so a turn is rounded up to the one line it plainly was —
/// truncating it instead means a slow wheel does nothing at all.
public struct WheelLines: Sendable {
    /// The fraction of a line each axis is holding, in points.
    private var pending = (x: 0.0, y: 0.0)

    public init() {}

    /// Take a scroll event, and answer the whole lines it completed.
    ///
    /// `(0, 0)` for a precise event that did not finish a line, which is most
    /// of them — and the call that does not happen because of it is the point
    /// of this type.
    public mutating func lines(
        deltaX: Double,
        deltaY: Double,
        precise: Bool,
        cellSize: (width: Double, height: Double)
    ) -> (x: Int, y: Int) {
        guard precise else {
            return (x: detents(deltaX), y: detents(deltaY))
        }
        pending.x += deltaX
        pending.y += deltaY
        return (x: whole(&pending.x, cellSize.width), y: whole(&pending.y, cellSize.height))
    }

    /// The whole lines `pending` has come to, taken out of it.
    private func whole(_ pending: inout Double, _ line: Double) -> Int {
        // A cell is never no pixels, but a view mid-layout can still say so,
        // and dividing by it would answer with infinity. What was held is
        // dropped with it: a remainder that can never drain is one that grows
        // for as long as the view is that size.
        guard line > 0 else {
            pending = 0
            return 0
        }
        guard pending.magnitude >= line else { return 0 }
        let lines = (pending / line).rounded(.towardZero)
        pending -= lines * line
        return Int(lines)
    }

    /// What a wheel with detents turned, which is a count of lines already.
    private func detents(_ delta: Double) -> Int {
        guard delta != 0 else { return 0 }
        let whole = Int(delta.rounded(.towardZero))
        return delta > 0 ? max(1, whole) : min(-1, whole)
    }
}
