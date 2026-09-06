import Foundation

/// A URL on the screen, and the cells it is drawn over.
///
/// Made by ``scan(_:)`` and read by two: the renderer, which underlines those
/// cells, and the click, which opens the URL. One scan answers both, so what
/// a ⌘ click opens is what the ⌘ underlined rather than a second judgement
/// taken a moment later. cf. 01-architecture, adr/0006.
public struct Link: Equatable {
    /// Where it goes. Only `http` and `https` ever reach here.
    public let url: URL
    /// The cells it covers: one run per screen row, because a line that ended
    /// by running out of columns is drawn on more than one of them.
    public let runs: [Run]

    /// One row, and the columns of it a link covers. Half-open.
    public struct Run: Equatable {
        public let row: Int
        public let columns: Range<Int>

        public init(row: Int, columns: Range<Int>) {
            self.row = row
            self.columns = columns
        }
    }

    /// Whether this link is drawn over a cell, which is the whole of what a
    /// ⌘ click asks.
    public func covers(row: Int, column: Int) -> Bool {
        runs.contains { $0.row == row && $0.columns.contains(column) }
    }
}

extension Link {
    /// Every URL on one frame's screen.
    ///
    /// **`http://` and `https://` and nothing else.** A terminal screen is
    /// full of dotted tokens, and a scan that took a scheme-less domain would
    /// underline `Cargo.toml`, `main.rs` and `v1.2.3` — half a screen lit up
    /// is noise rather than signal, and the point of showing the links is
    /// knowing what is about to be clicked.
    ///
    /// Read out of the snapshot's own `cells`, `graphemes` and `row_state`,
    /// which is why the ABI does not grow for this: the app already has the
    /// text, and where a URL is is the app's judgement rather than the
    /// terminal's. cf. 01-architecture, adr/0006.
    ///
    /// Called only while ⌘ is held, which is what makes it cost nothing the
    /// rest of the time: nothing scans a screen nobody has asked about.
    public static func scan(_ snapshot: Snapshot) -> [Link] {
        let cols = Int(snapshot.cols)
        let rows = Int(snapshot.rows)
        guard cols > 0 else { return [] }

        var links: [Link] = []
        var row = 0
        while row < rows {
            // One logical line: this row and every row it ran on into. A URL
            // that reached the last column and carried on is one URL, and
            // joining the rows before the scan is what makes it one — the
            // alternative is two halves, neither of which parses.
            var last = row
            while last + 1 < rows, snapshot.rowStates[last].isWrapped { last += 1 }
            let line = (row...last).flatMap { row in
                (0..<cols).map { character(of: snapshot.cells[row * cols + $0]) }
            }
            links.append(contentsOf: found(in: line, cols: cols, from: row))
            row = last + 1
        }
        return links
    }

    /// The links in one logical line, over cells counted from the row it
    /// starts on.
    private static func found(in line: [Unicode.Scalar], cols: Int, from first: Int) -> [Link] {
        var links: [Link] = []
        var index = 0
        while index < line.count {
            // The scheme has to start a token. Without that `shttp://x` is a
            // link, which is the same false positive the scheme rule is here
            // to keep off the screen.
            guard let scheme = scheme(of: line, at: index),
                index == 0 || !isAlphanumeric(line[index - 1])
            else {
                index += 1
                continue
            }

            let host = index + scheme
            var end = host
            // Every cell that cannot be in a URL already reads as a space, so
            // this is the whole of where one ends. cf. ``character(of:)``
            while end < line.count, line[end] != " " { end += 1 }
            // Trailing punctuation belongs to the sentence rather than to the
            // URL. Each of these is legal in one, so this is a guess — and
            // the guess that a full stop ended a sentence is right far more
            // often than the one that it ended a path.
            while end > host, trailing.contains(line[end - 1]) { end -= 1 }

            // A scheme with no host after it is not an address. `URL` is
            // asked as well as the scan, so nothing that failed to parse is
            // ever handed to a browser. cf. adr/0007.
            guard end > host,
                let url = URL(string: String(String.UnicodeScalarView(line[index..<end])))
            else {
                index += 1
                continue
            }
            links.append(Link(url: url, runs: runs(index..<end, cols: cols, from: first)))
            index = end
        }
        return links
    }

    /// How long the scheme at `index` is, or nil where there is none.
    private static func scheme(of line: [Unicode.Scalar], at index: Int) -> Int? {
        schemes.first { line[index...].starts(with: $0) }?.count
    }

    /// Which cells a stretch of the line falls on, a run per row it crosses.
    ///
    /// The line was made by laying whole rows end to end, so which row a
    /// character is on is what its index divides to — and a link that wrapped
    /// comes back as the two runs it is drawn as.
    private static func runs(_ range: Range<Int>, cols: Int, from first: Int) -> [Run] {
        var runs: [Run] = []
        var index = range.lowerBound
        while index < range.upperBound {
            let row = index / cols
            let end = min(range.upperBound, (row + 1) * cols)
            runs.append(
                Run(row: first + row, columns: (index - row * cols)..<(end - row * cols))
            )
            index = end
        }
        return runs
    }

    /// What one cell contributes to the line a scan reads: the character it
    /// holds, or a space for everything that cannot be part of a URL.
    ///
    /// A blank, a control character, a cluster the grapheme table holds and
    /// either half of a wide character all come back as a space — which is
    /// what lets the scan above ask whether a cell is a space and mean
    /// whether the URL ended there.
    private static func character(of cell: Cell) -> Unicode.Scalar {
        guard !cell.isOverflow, !cell.isWide, !cell.isWideTail,
            let scalar = Unicode.Scalar(cell.codepoint), allowed.contains(scalar)
        else { return " " }
        return scalar
    }

    /// The two schemes, longest first.
    private static let schemes = ["https://", "http://"].map { Array($0.unicodeScalars) }

    /// What may be in a URL: RFC 3986's unreserved and reserved characters,
    /// and the percent that escapes everything else. A space, a control
    /// character and anything outside ASCII end one.
    ///
    /// ponytail: no balanced-paren rule, so a URL inside brackets loses its
    /// closing one to the trim below. What that costs is the tail of a
    /// Wikipedia address; what it saves is counting depth over a screen. Pair
    /// the brackets here if a real address is ever seen cut short.
    private static let allowed = Set(
        "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-._~:/?#[]@!$&'()*+,;=%"
            .unicodeScalars
    )

    /// What comes off the end of a match.
    private static let trailing = Set(".,;:!?')]}".unicodeScalars)

    /// Whether a character is one a token can run on from, which is what says
    /// a scheme did not start in the middle of one.
    private static func isAlphanumeric(_ scalar: Unicode.Scalar) -> Bool {
        ("a"..."z").contains(scalar) || ("A"..."Z").contains(scalar)
            || ("0"..."9").contains(scalar)
    }
}
