import CKnotty

/// The handshake the header demands of every consumer.
public enum ABI {
    /// What the header this target compiled against says.
    public static let expected = UInt32(KT_ABI_VERSION)

    /// What the linked library answers.
    public static var linked: UInt32 { kt_abi_version() }

    /// Refuses to go on when the two disagree. Reading a struct whose layout
    /// moved is worse than not starting at all.
    public static func requireMatch() {
        guard linked == expected else {
            fatalError("knotty ABI mismatch: header says \(expected), library says \(linked)")
        }
    }
}
