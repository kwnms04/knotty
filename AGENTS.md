# Knotty

## Build prerequisites

`libghostty-vt-sys` builds the VT engine from ghostty sources with Zig, so a
**Zig 0.15.x** toolchain is needed. The pinned ghostty commit rejects 0.16 at
comptime. `brew install zig@0.15` or the official 0.15.2 tarball.

Two environment variables point the build at it, so nothing has to move on
`PATH`:

```sh
export ZIG=/opt/homebrew/opt/zig@0.15/bin/zig
export ZIG_SYSROOT=/Library/Developer/CommandLineTools/SDKs/MacOSX15.4.sdk
```

`ZIG` names the toolchain outright. Without it the build takes whatever `zig`
resolves to, which on a host that also has 0.16 means choosing between them
globally — `brew install zig@0.15` alone changes nothing, because brew keeps
it keg-only and unlinked.

`ZIG_SYSROOT` is for macOS 26, whose SDK 0.15.2 cannot link against: its
`libSystem.B.tbd` dropped the `arm64-macos` target. Zig finds the SDK by
spawning `xcrun` and reads no environment variable that would say otherwise,
so the alternative is shadowing `xcrun` on `PATH` for every process in the
shell. This passes `--sysroot` to `zig build` instead, which skips the
detection.

Both are hooks in the patched `libghostty-vt-sys`; cf.
[0012](docs/adr/0012-own-the-binding-layer.md). Neither is set in CI and
neither has to be: `ci.yml` installs the pinned tarball onto `PATH`, and its
`macos-15` runner ships an SDK 0.15.2 can link.

Wrong Zig fails deep — a `@compileError` inside ghostty's `build.zig` wrapped
in a build-script panic. Check `$ZIG` before blaming the crate.

## The app

`scripts/build-app.sh` is the whole of it: core, Swift side, shaders,
`build/knotty.app`, ad-hoc signature. `swift build` on its own leaves a binary
AppKit will not run properly — the window and the metallib both want a bundle —
so the run path is the `.app` during development too. cf.
[0014](docs/adr/0014-swiftpm-no-xcodeproj.md).

The script also checks the two boundary rules SwiftPM cannot express, since
AppKit is an SDK module and a system library's module map is transitive:
`KnottyRender` must not reach AppKit or a GPU device, and only `KnottySession`
may `import CKnotty`. cf.
[0015](docs/adr/0015-boundary-check-in-the-script.md).

On macOS 26 the Metal compiler is a separate download. `xcrun metal` names the
fix itself — `xcodebuild -downloadComponent MetalToolchain`. The `macos-15`
runner ships it.

`swift test --package-path App -c release` runs the Swift tests. They link the
same release staticlib the app does, so the script's `cargo build --release`
has to have happened first.

## Golden harness

`crates/knotty-harness` replays recorded terminal streams through the public C
ABI and compares what came out with a committed golden: the screen, the bytes
queued for the child, the events queued for the app, and the wake count.
`cargo test` checks them; only `KNOTTY_UPDATE_GOLDENS=1 cargo test -p
knotty-harness` writes them. Re-record by capturing an application under
`script(1)` and cutting the stream while its screen is still drawn — a normal
exit restores the primary screen and leaves the golden blank.

`recordings/synthetic.vt` is the exception: it is written by hand, not
captured, because no application rings the bell, copies to the clipboard,
queries its title and opens a synchronized output block in one run.

A `.vts` file beside them is a **script**: a line-oriented format in which
what the child sent and what the app did alternate, so that an encoding
depending on a mode can be reproduced at all — the sequence that sets the mode
arrives as output and the key read against it arrives from the app.

```text
out "\x1b[?1h"
key ArrowUp
key A ctrl
key A alt consumed=alt "å"
key Enter composing
```

What each word means is `parse`'s to say, in
`crates/knotty-harness/src/lib.rs` — including which key names a script may
use, since those are a list there rather than every key the header holds. An
unknown word fails the script saying so.

Scripts describe the same way recordings do and share the goldens directory,
so nothing about checking or rewriting them differs. They run on a small grid
rather than the recordings' 80×24: what a script is about is the bytes that
left for the child, and a screen nobody typed onto is only there to be
complete.

## Renderer goldens

`App/Tests/KnottyTests/goldens` holds what the renderer draws those same
recordings as: a rectangle and a colour for every cell, a rectangle for the
cursor, and for every glyph which one it is, where it sits and what tints it.
`swift test --package-path App -c release` checks them; only
`KNOTTY_UPDATE_RENDER_GOLDENS=1 swift test --package-path App -c release`
writes them. Its own variable and not the harness's, so that rewriting a
screen cannot quietly rewrite a drawing too.

Cell metrics are injected as constants and atlas coordinates are left out of
the comparison, which is what lets the files say the same thing on a runner
and on a development machine: neither a font's raster nor its advance is
promised across macOS versions. What is left is every judgement the renderer
makes.

## Fuzzer

`fuzz/` holds one libFuzzer target, `feed`, which rounds a detached session
through its whole cycle on arbitrary bytes. It is a workspace of its own, so
`cargo test --workspace` never builds it and the nightly toolchain stays out of
the PR path — `fuzz.yml` runs it nightly for an hour instead. Locally:

```sh
cargo +nightly fuzz run feed fuzz/corpus/feed crates/knotty-harness/recordings
```

The recordings ride along as a read-only seed corpus; new inputs are written to
`fuzz/corpus/feed/`, which is committed. Run `cargo +nightly fuzz cmin feed`
before committing what a session grew. It targets `knotty-core` and not the C
ABI the golden harness uses: the boundary catches panics and returns a status,
so a panic reached through it looks like a clean refusal.

What a CI run found comes home by hand — the job uploads it, nothing commits
it:

```sh
gh run download <run-id> -n fuzz-corpus -D fuzz/corpus/feed
cargo +nightly fuzz cmin feed
git add fuzz/corpus/feed
```

Anything longer than a few minutes wants the flags `fuzz.yml` passes, for the
reasons given there:

```sh
cargo +nightly fuzz run feed fuzz/corpus/feed crates/knotty-harness/recordings \
  -- -fork=1 -ignore_timeouts=0 -rss_limit_mb=8192 -max_total_time=3600
```

Without them a long run ends in an out-of-memory that is AddressSanitizer's and
not knotty's. An `oom-*` artifact is that, not a finding — check the live heap
in the report against the RSS before believing one.

A crash lands in `fuzz/artifacts/feed/` and replays with `cargo +nightly fuzz
run feed <file>`. Commit it into the corpus once it is fixed.

The `[patch.crates-io]` block in `fuzz/Cargo.toml` is a copy of the root one; a
patch only counts in the workspace it heads. They move together, and
`fuzz/Cargo.lock` is committed for the same reason the root's is: the patch
names a branch, so without a lock the fuzzer would build whatever that branch
had moved to and stop being the core CI builds.

## Bench

`crates/knotty-core/benches/runaway.rs` runs the two performance gates M1 ends
on: B4, taking a snapshot, and B5, getting a keystroke to the terminal — both
while a child floods its output. It builds the load and reads the clock itself
rather than wrapping a framework around a call, because neither number is a
function's cost.

```sh
cargo bench -p knotty-core
```

It prints percentiles and exits non-zero when either p99 misses its gate. Half
a minute, most of it the two ten-second scenarios.

**Run it in the reference environment section B of
`07-definition-of-done.md` names — 200×60, 120Hz, Retina — and read nothing
into a run anywhere else.** CI never runs it: a measurement taken beside
somebody else's job fails at random, and a gate that fails at random is one
somebody turns off. `cargo clippy --all-targets` still compiles it, so it
cannot rot unnoticed.

## Generated header

`include/knotty.h` is generated by cbindgen and committed. `cargo test -p
knotty-ffi` fails when it drifts from the Rust source; regenerate with
`KNOTTY_UPDATE_HEADER=1 cargo test -p knotty-ffi --test header`.

## Agent skills

### Issue tracker

Bug reports arrive as GitHub issues on `kwnms04/knotty` and are the only thing the tracker holds. Specs and implementation tickets are files under `.scratch/<feature-slug>/`.
See `docs/agents/issue-tracker.md`.

### Triage labels

The seven canonical triage roles — two category, five state — used verbatim as label strings.
See `docs/agents/triage-labels.md`.

### Domain docs

Single-context. The numbered chapters in `docs/` are the live specification;
`docs/adr/` records why each decision was made.
See `docs/agents/domain.md`.

## Commit convention

Conventional Commits. **Messages are written in English.**

```
type: description
```

Scopes are not in use yet.

| Type | Use |
|---|---|
| `feat` | New capability |
| `fix` | Bug fix |
| `perf` | Performance improvement |
| `refactor` | Restructuring with no behaviour change |
| `docs` | Documentation, specification chapters, ADRs |
| `test` | Tests |
| `build` | Build system and dependencies, including VT engine version bumps |
| `ci` | CI configuration |
| `style` | Formatting only |
| `chore` | Anything else |

Append `!` after the type and add a `BREAKING CHANGE:` footer when the commit
breaks something a consumer depends on. Name what is superseded in the footer,
not just that something broke.

**The ABI rule:** a commit that changes the ABI version constant must carry `!`,
and a commit that carries `!` on the C ABI must change it. The generated header
is frozen between milestones, and a struct layout change is an ABI change — so
this pair is checkable.

Nothing enforces this yet. No commitlint, no hook.
