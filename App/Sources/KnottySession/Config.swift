import CKnotty
import CoreServices
import Foundation

/// The configuration file, as the boundary hands it over.
///
/// **One blob and no getter per key.** The schema stands whole in
/// `knotty-config`; what is written here is the part of it this app reads
/// yet, and Swift ignores the rest of the object. A key that becomes useful
/// on this side arrives by growing this struct, and the header does not move
/// for it. cf. 02-ffi, 05-swift-app 10.
public struct Config: Decodable, Equatable, Sendable {
    /// The face the grid is measured from and drawn in.
    public struct Font: Decodable, Equatable, Sendable {
        public let family: String
        public let size: Double
    }

    /// The colours a screen is drawn in.
    ///
    /// A colour is three numbers by the time it is here: `#rrggbb` was parsed
    /// in `knotty-config`, and nothing on this side parses one again.
    public struct Theme: Decodable, Equatable, Sendable {
        /// A colour, as the blob spells one.
        ///
        /// Its own type rather than the boundary's ``Rgb``. That one is the
        /// header's layout, and teaching an imported struct to decode itself
        /// would be a second truth about it — this converts instead, at the
        /// one call that hands colours back across. cf. 05-swift-app 2.
        public struct Color: Decodable, Equatable, Sendable {
            public let r: UInt8
            public let g: UInt8
            public let b: UInt8

            /// The same colour, as everything that draws one names it.
            public var rgb: Rgb { Rgb(r: r, g: g, b: b) }
        }

        /// What a cell with no background of its own is drawn on.
        public let background: Color
        /// What a cell with no foreground of its own is drawn in.
        public let foreground: Color
        /// What the cursor is drawn in, or nil when the file named none —
        /// which is the rule that it takes the colour of the text it stands
        /// on, rather than a colour that is missing. cf. 04-renderer R1.
        public let cursor: Color?
        /// The sixteen the terminal's own colours are, in order.
        public let palette: [Color]
    }

    public let font: Font
    public let theme: Theme

    /// What a load came to.
    ///
    /// Both halves, never one or the other: a file that will not parse comes
    /// back as the defaults with a diagnostic beside them, because a window
    /// opens either way and the first load has no previous configuration to
    /// keep. cf. 05-swift-app 10.
    public struct Loaded: Sendable {
        public let config: Config
        /// What was wrong with the file, or nil when nothing was.
        public let diagnostic: String?
    }

    /// Where the file lives. The app's to say, because watching it is.
    public static let path = FileManager.default.homeDirectoryForCurrentUser
        .appending(path: ".config/knotty/config.toml")

    /// Read the configuration, or the defaults where there is no file.
    ///
    /// **A typo in the file is not what this throws on** — that is the
    /// diagnostic, and the configuration beside it is the defaults. What it
    /// throws on is a blob this side cannot read, which is the two sides
    /// built from different sources rather than anything the user did.
    public static func load(from path: URL = Config.path) throws -> Loaded {
        var handle: OpaquePointer?
        try check(
            "kt_config_load",
            Array(path.path(percentEncoded: false).utf8).withUnsafeBufferPointer {
                kt_config_load($0.baseAddress, $0.count, &handle)
            }
        )
        // A status of OK is the boundary promising an owned handle, the way
        // it is for a snapshot.
        guard let handle else {
            preconditionFailure("kt_config_load succeeded with no configuration")
        }
        defer { kt_config_free(handle) }

        var view = KtConfigView()
        try check("kt_config_view", kt_config_view(handle, &view))

        // Both runs point into the handle, so both are copied out before the
        // `defer` above releases it.
        let diagnostic = Self.text(view.diagnostic)
        return Loaded(
            config: try JSONDecoder().decode(Config.self, from: Data(Self.borrow(view.json))),
            diagnostic: diagnostic.isEmpty ? nil : diagnostic
        )
    }

    private static func borrow(_ text: KtText) -> UnsafeBufferPointer<UInt8> {
        UnsafeBufferPointer(start: text.bytes, count: text.len)
    }

    private static func text(_ text: KtText) -> String {
        String(decoding: borrow(text), as: UTF8.self)
    }
}

extension Config {
    /// Read the configuration again, keeping what is in force when the file
    /// will not parse.
    ///
    /// The one difference from ``load(from:)`` is the whole of what a reload
    /// is. A first load has nothing to keep and opens its windows on the
    /// defaults; every load after it happens under a configuration the user
    /// is working in, and a half-typed value is not a reason to take their
    /// colours away while they finish the line. What comes back is the
    /// diagnostic beside what they already had. cf. 05-swift-app 10.
    public static func reload(
        from path: URL = Config.path, keeping current: Config
    ) throws -> Loaded {
        let loaded = try load(from: path)
        guard let diagnostic = loaded.diagnostic else { return loaded }
        return Loaded(config: current, diagnostic: diagnostic)
    }

    /// The file, watched, so that saving it is what applies it.
    ///
    /// **The watch is the app's.** The core is handed a configuration; it
    /// knows nothing about a file, and nothing about one changing.
    /// cf. 05-swift-app 10.
    ///
    /// It sits in this target beside the load rather than up in `knotty`,
    /// which stretches the facade that 05-swift-app 2 makes of this one — an
    /// FSEvents stream is not the C boundary. What holds it here is that its
    /// subject is this type's: the file, whose reading and whose changing are
    /// one thing to know about. What that facade is for — that nothing above
    /// calls the boundary itself — is untouched.
    ///
    /// It watches the directory rather than the file. An editor saves by
    /// writing a new file beside the old one and renaming it over the top,
    /// which leaves a descriptor opened on what was there holding a file no
    /// path leads to any more — so the file itself is the one thing a watch
    /// on it cannot follow. The directory covers that and a write in place
    /// both.
    ///
    /// Not itself main-actor state: what is isolated is the callback, which
    /// the queue below is what makes true. The stream is a handle, and a
    /// `deinit` cannot be on an actor to close one.
    public final class Watch {
        /// How quiet the file has to go before the change is called a change.
        ///
        /// **The debounce is ours and not the stream's.** A stream's own
        /// latency is a rate limit and not a wait: the first event after a
        /// quiet spell arrives at once and only what follows it is held, so
        /// two saves a moment apart come back as two — the second of them
        /// reading a file an editor may still be in the middle of writing.
        /// What is wanted is the other shape, the one that fires when the
        /// writing stopped. cf. 05-swift-app 10.
        private static let debounce = 0.2

        /// What the callback is given, since a C function pointer carries no
        /// context of its own — and where the wait above is kept.
        ///
        /// On the main actor because what it holds runs there, which is also
        /// what lets the callback below carry it over: an isolated class is a
        /// `Sendable` one, where the raw pointer it came out of is not.
        @MainActor
        private final class Sink {
            private let body: @MainActor () -> Void
            /// The call this has put off, still waiting to be made.
            private var pending: DispatchWorkItem?

            init(_ body: @escaping @MainActor () -> Void) { self.body = body }

            /// Something happened: call back once the file has held still.
            ///
            /// Every event pushes the call further out, so a burst — an
            /// editor writing, renaming and chmod-ing for one save, or a hand
            /// on ⌘S twice — is one call, made after the last of them.
            func schedule() {
                pending?.cancel()
                // Weak, or the work item would hold this object up past the
                // watch that owns it, and fire on a window that has gone.
                let work = DispatchWorkItem { [weak self] in
                    MainActor.assumeIsolated { self?.body() }
                }
                pending = work
                DispatchQueue.main.asyncAfter(deadline: .now() + Watch.debounce, execute: work)
            }
        }

        private let sink: Sink
        private let stream: FSEventStreamRef

        /// Call `onChange` on the main queue when the file may have moved.
        ///
        /// "May have" is the whole of what a watch says: what arrives is that
        /// something under the directory changed, and reading the file again
        /// is what says whether any of it was this one. Nothing is compared
        /// here — the diff is the reload's, against what is in force.
        ///
        /// Nil where the system refused a stream, which leaves the file
        /// unwatched rather than taking the windows down: by the time this is
        /// asked for there is a shell running in one, and a watch is not
        /// worth a terminal.
        @MainActor
        public init?(path: URL = Config.path, onChange: @escaping @MainActor () -> Void) {
            sink = Sink(onChange)
            var context = FSEventStreamContext(
                version: 0,
                // Unretained: this object owns both the sink and the stream,
                // and the stream is stopped in `deinit` before either goes.
                info: Unmanaged.passUnretained(sink).toOpaque(),
                retain: nil,
                release: nil,
                copyDescription: nil
            )
            let directory = path.deletingLastPathComponent().path(percentEncoded: false)
            guard
                let stream = FSEventStreamCreate(
                    nil,
                    { _, info, _, _, _, _ in
                        guard let info else { return }
                        // The queue set below is the main one, which is what
                        // makes this true rather than hopeful.
                        MainActor.assumeIsolated {
                            let sink = Unmanaged<Sink>.fromOpaque(info).takeUnretainedValue()
                            sink.schedule()
                        }
                    },
                    &context,
                    [directory] as CFArray,
                    FSEventStreamEventId(kFSEventStreamEventIdSinceNow),
                    // No latency of the stream's own: the wait that matters is
                    // the one above, and a second number here would be a
                    // second answer to the same question.
                    0,
                    FSEventStreamCreateFlags(
                        // Per file, so that a directory holding more than this
                        // one does not call back for its neighbours' sake.
                        kFSEventStreamCreateFlagFileEvents
                            // And along the path to it, which is what makes a
                            // directory that is not there yet watchable at
                            // all: writing the configuration for the first
                            // time makes `~/.config/knotty` as well as the
                            // file in it, and without this the stream would
                            // have resolved a path that did not exist and
                            // stayed silent until the next launch. Measured
                            // against a directory already in place, it costs
                            // nothing — the same two events at the same
                            // milliseconds.
                            | kFSEventStreamCreateFlagWatchRoot
                    )
                )
            else {
                return nil
            }
            self.stream = stream
            FSEventStreamSetDispatchQueue(stream, DispatchQueue.main)
            FSEventStreamStart(stream)
        }

        deinit {
            FSEventStreamStop(stream)
            FSEventStreamInvalidate(stream)
            FSEventStreamRelease(stream)
        }
    }
}
