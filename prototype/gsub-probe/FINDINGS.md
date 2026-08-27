# GSUB probe — findings

**Question.** Is the set of codepoints that participate in ligatures small
enough to keep the fast path? ([R3](../../docs/04-renderer.md#r3--shaping-unit),
[open-questions](../../docs/open-questions.md))

**Answer. Yes — but not as a constant. The fast path holds in every font
measured; the numbers that define it do not generalise, so the renderer has
to derive them at font load.**

Measured across every ligature font a v1 user is likely to pick, because M4
makes the face configurable and the DoD puts ligatures in v1 scope. An
earlier pass measured only JetBrains Mono and drew conclusions from it; three
of the four numbers below turned out to be font-specific, and one earlier
claim was wrong in a way one font could not show.

## The numbers

| font | substitutable | % of cmap | input | back | ahead | left overhang | right overhang | collapses cells |
|---|---:|---:|---:|---:|---:|---:|---:|---|
| Menlo | — | — | — | — | — | — | — | no ligature features |
| SF Mono | — | — | — | — | — | — | — | no ligature features |
| JetBrains Mono | 27 | 2.0% | 1 | 4 | 5 | 2.89 | 0.02 | no |
| Fira Code | 140 | 8.8% | 1 | 3 | 4 | **2.96** | 0.02 | no |
| Hasklig | 13 | 0.9% | 1 | 2 | 3 | 1.88 | 0.00 | no |
| Iosevka | 106 | 1.4% | **4** | 3 | 3 | 1.42 | 1.38 | no |
| Cascadia Code | 85 | 3.5% | **4** | 3 | 4 | 0.82 | **2.77** | Arabic only |
| Monaspace Neon | **271** | **11.0%** | 3 | 1 | 3 | 1.79 | 0.34 | table says yes, shaping says no |

Overhang is in cells. Bold marks the worst case, which is what the design
has to budget for.

## The four questions

**Q1 — is the participating set small?** Yes, everywhere. The worst is
Monaspace at 11.0% of its cmap; the median is under 4%. **At least 89% of
codepoints keep the plain atlas path in every font measured.**

The sets are not interchangeable. JetBrains Mono's 27 are punctuation.
Fira Code adds `ijlwx` and 108 Greek codepoints. Monaspace's 271 include
most of the Latin alphabet. **A hard-coded set is wrong for five of the six
fonts.**

**Q2 — how wide is the context window?** Bounded and small, but not the same
shape. JetBrains Mono and Fira Code match one input cell and look up to 4
back and 5 ahead. Iosevka and Cascadia match up to **4 input cells**. The
budget the renderer needs is **4 input cells, 4 back, 5 ahead — a 13-cell
window**, worst case across all six.

**Q3 — how far does ink leave its cell?** Up to **2.96 cells left**
(Fira Code) and **2.77 cells right** (Cascadia Code). Three techniques, and
they point in different directions:

- **ligature-last** — leading cells become a blank (`SPC`, `.spacer`,
  `emptyAdvanceWidth`), the final cell draws the whole ligature backwards.
  JetBrains Mono, Fira Code, Monaspace. Overhangs **left**.
- **ligature-first** — the first cell draws it, trailing cells become `LIG`
  fillers. Cascadia Code. Overhangs **right**.
- **join** — every cell draws its own share (`.join-l`). Iosevka. Overhangs
  both ways, but least of the three.

So the atlas must accept glyphs up to ~4 cells wide, and a glyph's quad can
start 3 cells before or extend 3 cells after the cell that owns it. R6's
shelf packing assumes cell-sized entries; that assumption does not survive.

**Q4 — does any font collapse cells?** **No — verified, not assumed.** Across
all six fonts and four feature combinations (default, `liga+calt`, `liga`
only, `calt` only), CoreText returned exactly one glyph per character with a
uniform advance every time.

This is the claim the single-font pass got wrong for the right reason: it
read "JetBrains Mono substitutes per cell" and generalised it into a law. It
is not a law. Two fonts *do* carry cell-collapsing type-4 ligatures reachable
from the enabled features:

- **Cascadia Code** collapses 4 → 1, but only for Arabic — `ﷲ` and
  lam-lam-heh. Not a programming ligature.
- **Monaspace** collapses `!=`, `//`, `///`, `||`, `!!`, `;;`, `;;;`, `...`
  in `liga`. Shaping still returns two glyphs: the font pairs each fold with
  an expansion that restores the count.

The grid survives because these fonts take care to preserve it, not because
GSUB cannot break it. **A font that does not take that care would break the
renderer's central assumption**, so this needs a load-time check, not an
assumption.

## What this means for R3

1. **Derive, don't hard-code.** The participating set and the window come
   from the font's GSUB at load, the way `probe.py` computes them. Then a
   font change re-derives the fast path instead of invalidating it.
2. **Budget the worst case, not the current font.** 13-cell scan window,
   ~4-cell-wide atlas entries, ±3 cells of quad overhang.
3. **Assert the grid at load.** Shape a short probe string and compare glyph
   count to character count. If a font collapses, refuse the ligature path
   for it and fall back to per-cell rendering — the alternative is a terminal
   whose columns silently stop lining up.
4. **Enable both `liga` and `calt`.** Neither alone is enough: Monaspace's
   ligatures need `liga` and do not fire under `calt` alone; JetBrains Mono's
   are the other way round.
5. **Sub-run shaping is exact within the window.** Any sub-run carrying 4
   cells of left context and 5 of right shapes identically to the whole line.
   "부분 셰이핑 일치율" was posed as a rate to measure; it is a hard bound.

## Caveat

**Hasklig produces no ligatures at all** under CoreText, in any of the four
feature combinations — `!=` shapes as `exclam equal`. Its table numbers above
are real but unreached. Not chased down; it does not move any bound.

## Reproducing

```sh
prototype/gsub-probe/fetch.sh          # downloads the five fonts (needs gh)
uv run prototype/gsub-probe/probe.py   # GSUB tables: sets, windows, overhang
swift prototype/gsub-probe/shape.swift # CoreText: cell counts, per feature combo
```

Fonts are downloaded, not committed.
