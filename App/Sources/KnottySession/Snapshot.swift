import CKnotty

/// One terminal cell, as the boundary lays it out.
///
/// Aliased rather than mirrored. A Swift copy of the layout is a second truth
/// that can drift from the header, and an alias lets the drawing side read a
/// cell without importing the C boundary. cf. 05-swift-app 2.
public typealias Cell = KtCell

/// What a snapshot says about one row: its flags and, where selected, its
/// columns.
public typealias Row = KtRow

/// Where the cursor is and how it looks.
public typealias Cursor = KtCursor

/// Whether a session has a child and what has become of it.
///
/// The exit code rides on the one case it means anything for, which is what
/// the boundary says about it in prose.
public enum ChildState: Equatable {
    /// There is no child. A session with no PTY behind it has none.
    case none
    /// The child is still running.
    case running
    /// The child is gone, by this code or by 128 plus the signal that ended
    /// it.
    case exited(code: Int32)
}

/// Whether a session still works.
///
/// A different fact from ``ChildState`` and read apart from it: a session
/// that broke while its child went on running is a real pairing, and what
/// decides whether the window still takes input is this one.
public enum SessionState {
    /// Working.
    case ok
    /// Something inside it panicked. It keeps the last screen it published
    /// and refuses input.
    case broken
}

/// A borrowed view of one published frame.
///
/// Every pointer here is the core's own. They are valid for the length of the
/// ``Session/withSnapshot(_:)`` call that opened the view and not a moment
/// longer, so anything that has to outlive the frame — the title, the working
/// directory — is copied out by the consumer. That copy is what the
/// boundary's lending contract asks for. The scope is what a consumer is
/// given to respect, not yet something the compiler holds it to: the type
/// that would say so to the compiler needs a language feature still behind an
/// experimental flag.
///
/// What a frame also carries and this view does not yet lift out — the
/// two-level dirty, the grapheme table, whether a selection exists — is what
/// M2 has no reader for. The renderer redraws whole frames, draws ASCII, and
/// has no selection to draw; each of those arrives with the reader that needs
/// it.
public struct Snapshot {
    /// Viewport width in cells.
    public let cols: UInt16
    /// Viewport height in cells.
    public let rows: UInt16
    /// Row-major grid of `rows * cols` cells.
    public let cells: UnsafeBufferPointer<Cell>
    /// One entry per row.
    public let rowStates: UnsafeBufferPointer<Row>
    /// Where the cursor is and how it looks.
    public let cursor: Cursor
    /// Window title as UTF-8, control characters already removed.
    public let title: UnsafeBufferPointer<UInt8>
    /// Working directory as an absolute path in UTF-8.
    public let pwd: UnsafeBufferPointer<UInt8>
    /// What the session said of its child as this frame was taken.
    public let childState: ChildState
    /// What the session said of itself as this frame was taken.
    public let sessionState: SessionState

    init(_ view: KtSnapshotView) {
        cols = view.cols
        rows = view.rows
        cells = UnsafeBufferPointer(start: view.cells, count: Int(view.cols) * Int(view.rows))
        rowStates = UnsafeBufferPointer(start: view.row_state, count: Int(view.rows))
        cursor = view.cursor
        title = UnsafeBufferPointer(start: view.title.bytes, count: view.title.len)
        pwd = UnsafeBufferPointer(start: view.pwd.bytes, count: view.pwd.len)
        childState = ChildState(view.child_state, exitCode: view.child_exit_code)
        sessionState = SessionState(view.session_state)
    }
}

// The header numbers these into a byte, but Swift imports the constants as a
// type of their own rather than as that byte — hence the widening on every
// comparison. Neither switch has a fourth arm to meet: the ABI handshake in
// `ABI.requireMatch()` has already refused a library that names states this
// header does not.
extension ChildState {
    init(_ state: UInt8, exitCode: Int32) {
        switch state {
        case UInt8(KT_CHILD_STATE_NONE.rawValue): self = .none
        case UInt8(KT_CHILD_STATE_RUNNING.rawValue): self = .running
        case UInt8(KT_CHILD_STATE_EXITED.rawValue): self = .exited(code: exitCode)
        default: preconditionFailure("the boundary named child state \(state)")
        }
    }
}

extension SessionState {
    init(_ state: UInt8) {
        switch state {
        case UInt8(KT_SESSION_STATE_OK.rawValue): self = .ok
        case UInt8(KT_SESSION_STATE_BROKEN.rawValue): self = .broken
        default: preconditionFailure("the boundary named session state \(state)")
        }
    }
}
