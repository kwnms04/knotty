import CKnotty
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

    public let font: Font

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
