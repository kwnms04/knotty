#!/usr/bin/env bash
#
# The one command the app is built with: core, Swift side, shaders, bundle,
# signature. cf. adr/0014.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
bundle="$root/build/knotty.app"

# Two boundary rules the target graph does not check for us. AppKit and Metal
# are SDK modules, importable from any target at all; and SwiftPM hands a
# system library's module map to every transitive dependent, so `import
# CKnotty` compiles well outside the facade. Both are checked here instead.
if grep -rnE '^import +(AppKit|Cocoa|Metal|MetalKit|QuartzCore)' \
        "$root/App/Sources/KnottyRender"; then
    echo "KnottyRender must not reach AppKit or a GPU device — 04-renderer R9" >&2
    exit 1
fi
if grep -rnE '^import +CKnotty' "$root/App/Sources" "$root/App/Tests" \
        --exclude-dir=KnottySession; then
    echo "only KnottySession may import CKnotty — 05-swift-app 2" >&2
    exit 1
fi

# SwiftPM cannot build the core and does not know it has to come first. Getting
# the order wrong fails at link time rather than quietly.
cargo build --manifest-path "$root/Cargo.toml" --release -p knotty-ffi
swift build --package-path "$root/App" -c release

# SwiftPM copies Metal sources into a bundle rather than compiling them, so the
# compile — and with it the syntax check — is ours.
mkdir -p "$root/build"
xcrun metal -o "$root/build/default.metallib" "$root/App/Sources/knotty/Shaders.metal"

rm -rf "$bundle"
mkdir -p "$bundle/Contents/MacOS" "$bundle/Contents/Resources"
cp "$root/App/Info.plist" "$bundle/Contents/Info.plist"
cp "$(swift build --package-path "$root/App" -c release --show-bin-path)/knotty" \
    "$bundle/Contents/MacOS/knotty"
cp "$root/build/default.metallib" "$bundle/Contents/Resources/default.metallib"

# Ad-hoc: enough for the app to run on the machine that built it, which is all
# M2 asks. Notarisation is M5's.
codesign --force --sign - "$bundle"

echo "$bundle"
