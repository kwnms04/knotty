# GSUB probe — findings

**Question.** Is the set of codepoints that participate in ligatures small
enough to keep the fast path? ([R3](../../docs/04-renderer.md#r3--shaping-unit),
[open-questions](../../docs/open-questions.md))

**Answer. Yes — and the fast path survives more intact than R3 assumed.**
Ligatures never need CTLine, because they never change how many cells a run
occupies. What they need instead is a per-cell contextual glyph choice over a
bounded window, and an atlas that tolerates left overhang.

Measured against JetBrains Mono Regular (the primary font), with Menlo and
SF Mono — the fallbacks — as controls.

## The numbers

| | JetBrains Mono | Menlo | SF Mono |
|---|---|---|---|
| ligature features | `calt` only | none | none |
| **substitutable codepoints** | **27** (2.0% of cmap) | 0 | 0 |
| context-only codepoints | 310 | 0 | 0 |
| longest matched input | **1 cell** | — | — |
| backtrack / lookahead | **4 / 5 cells** | — | — |

**The 27:** `!#$&(*+-./:;<=>?@[\]^_{|}~` and U+00DF.
Everything else in the font — every letter, every digit, all 1336 remaining
codepoints — can never have its own glyph replaced.

The 310 context-only codepoints (letters, digits, space) only *gate*: they
decide whether a neighbour substitutes, never what they themselves draw.
`a:b` keeps a plain colon; `::` ligates.

## What the shaping actually does

JetBrains Mono does not use type-4 ligature substitution for programming
ligatures at all. Its two type-4 lookups belong to `ccmp` (combining marks)
and `ordn` (`No.` → `№`) — neither reachable from `calt`/`liga`.

Instead every rule matches **exactly one input glyph** and consults up to 4
cells behind and 5 ahead. Verified through CoreText:

```
"!="   chars=2 glyphs=2 advances=[9, 9]   SPC  exclam_equal.liga
"<=>"  chars=3 glyphs=3 advances=[9, 9, 9] SPC SPC less_equal_greater.liga
"a:b"  chars=3 glyphs=3 advances=[9, 9, 9] a colon b
```

Leading cells become `SPC` (zero ink, full advance); the **last** cell carries
the whole ligature glyph, drawn with negative left bearing back across them.
Cell count and advance are invariant. The grid never moves.

## What this costs the renderer

- **Fast path stays for 98% of codepoints.** A cell is an atlas lookup unless
  its codepoint is one of the 27.
- **The ligature path is not CTLine.** One input cell, bounded window — it is
  a table lookup on (codepoint, ±window), not a shaper call. CTLine remains
  for what R3 already sent there: combining, ZWJ, emoji, fallback.
- **Sub-run shaping is exact, not approximate.** "부분 셰이핑 일치율" was
  posed as a rate to measure. It is a hard bound: any sub-run carrying ≥4
  cells of left context and ≥5 of right context shapes identically to the
  whole line. Confirmed — `x!=y`, `!=y`, `x!=`, `!=` all produce the same
  ligature glyph.
- **The atlas must accept overhang.** Across 153 `.liga` glyphs the worst ink
  extends **2.89 cells to the left** of its origin
  (`numbersign_numbersign_numbersign_numbersign.liga`); rightward overhang is
  0.02 cells, i.e. none. R6's shelf packing assumes uniform cell-sized entries
  and R4 says fallback glyphs are not clipped — a ligature glyph is neither
  cell-sized nor clippable, and its quad starts up to 3 cells left of the cell
  that owns it.
- **Damage tracking gains a bleed radius.** A ligature glyph drawn in cell *n*
  paints over cells *n-3..n*, and an edit at cell *n* can change what cells
  *n-4..n+5* draw. R2's line cache is keyed on whole line content, so it is
  unaffected — but any narrower dirty region is not.

## Reproducing

```sh
uv run prototype/gsub-probe/probe.py     # GSUB table: the sets and the window
swift prototype/gsub-probe/shape.swift   # CoreText: cell counts and ink extents
```
