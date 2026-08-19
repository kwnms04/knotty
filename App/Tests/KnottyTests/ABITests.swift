import Testing

import KnottySession

/// The one thing that must hold before anything else is read across the
/// boundary: the header this side compiled against and the library it links
/// describe the same layouts.
@Test func headerAndLibraryAgreeOnTheABIVersion() {
    #expect(ABI.linked == ABI.expected)
}
