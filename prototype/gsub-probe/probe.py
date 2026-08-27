# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools>=4.53"]
# ///
"""PROTOTYPE — throwaway. Four questions, across every ligature font a v1
user might pick.  (docs/04-renderer.md R3, docs/open-questions.md)

  Q1  How big is the participating set?   fast path is worth it if it stays small
  Q2  How wide is the context window?     scan cost per cell
  Q3  How far does ink overhang the cell? atlas and damage budget
  Q4  Does any font collapse cells?       if yes, CTLine is unavoidable

SUBSTITUTABLE  a codepoint whose own glyph a lookup can replace.  A cell
               holding one cannot be a plain atlas lookup.
CONTEXT-ONLY   a codepoint that only appears in backtrack or lookahead.  Its
               own glyph never changes; it only decides a neighbour's.

Both over-count: a rule covering a glyph may never fire.  Over-counting is
the safe side of a fast-path decision.

    ./fetch.sh && uv run probe.py
"""

import glob
import os
import sys
from fontTools.pens.boundsPen import BoundsPen
from fontTools.ttLib import TTFont, TTCollection

FEATURES = {"liga", "calt", "clig", "rlig"}

HERE = os.path.dirname(os.path.abspath(__file__))
SYSTEM = [
    ("Menlo", "/System/Library/Fonts/Menlo.ttc", "Menlo-Regular"),
    ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf", None),
] + [
    (f"JetBrainsMono-{face}",
     os.path.expanduser(f"~/Library/Fonts/JetBrainsMono-{face}.ttf"), None)
    for face in ("Regular", "Bold", "Italic", "BoldItalic")
]


class Probe:
    def __init__(self, font):
        self.font = font
        self.subst = set()       # glyphs a lookup can replace
        self.context = set()     # glyphs that only gate
        self.outputs = set()     # glyphs a lookup can produce
        self.max_input = 1       # longest matched input, in cells
        self.max_back = 0
        self.max_ahead = 0
        self.collapse = 1        # components a type-4 ligature folds into one
        self.open_class = False

    def run(self, tags):
        gsub = self.font.get("GSUB")
        if gsub is None:
            return set()
        table = gsub.table
        seen_tags, queue = set(), []
        for fr in table.FeatureList.FeatureRecord:
            if fr.FeatureTag in tags:
                seen_tags.add(fr.FeatureTag)
                queue.extend(fr.Feature.LookupListIndex)
        lookups = table.LookupList.Lookup
        done = set()
        while queue:
            i = queue.pop()
            if i in done or i >= len(lookups):
                continue
            done.add(i)
            for sub in lookups[i].SubTable:
                queue.extend(self.visit(sub, lookups[i].LookupType))
        return seen_tags

    def visit(self, sub, lookup_type):
        while getattr(sub, "ExtSubTable", None) is not None:
            sub = sub.ExtSubTable
        t = getattr(sub, "LookupType", None) or lookup_type
        fmt = getattr(sub, "Format", None)

        if t in (1, 2):
            self.subst |= set(sub.mapping)
            for v in sub.mapping.values():
                self.outputs |= {v} if isinstance(v, str) else set(v)
        elif t == 3:
            self.subst |= set(sub.alternates)
            for v in sub.alternates.values():
                self.outputs |= set(v)
        elif t == 4:
            for first, ligs in sub.ligatures.items():
                self.subst.add(first)
                for lig in ligs:
                    self.subst.update(lig.Component)
                    self.outputs.add(lig.LigGlyph)
                    n = 1 + len(lig.Component)
                    self.max_input = max(self.max_input, n)
                    self.collapse = max(self.collapse, n)
        elif t in (5, 6):
            return self.chain(sub, fmt, plain=(t == 5))
        elif t == 8:
            self.subst |= set(sub.Coverage.glyphs)
            self.outputs |= set(getattr(sub, "Substitute", None) or [])
        return ()

    def chain(self, sub, fmt, plain=False):
        nested = []
        if fmt == 3:
            # Plain context (type 5) names its inputs `Coverage`; the chained
            # form (type 6) splits them into Input/Backtrack/LookAhead.
            inputs = getattr(sub, "InputCoverage", None)
            if inputs is None:
                inputs = getattr(sub, "Coverage", None) or []
                if not isinstance(inputs, list):
                    inputs = [inputs]
            self.record(
                [set(c.glyphs) for c in inputs],
                [set(c.glyphs) for c in (getattr(sub, "BacktrackCoverage", None) or [])],
                [set(c.glyphs) for c in (getattr(sub, "LookAheadCoverage", None) or [])])
            return [r.LookupListIndex for r in (sub.SubstLookupRecord or [])]

        first = set(sub.Coverage.glyphs)
        if fmt == 1:
            sets = getattr(sub, "SubRuleSet" if plain else "ChainSubRuleSet", None)
            rule_attr = "SubRule" if plain else "ChainSubRule"
            for rs in (sets or []):
                for rule in (getattr(rs, rule_attr, None) or []):
                    self.record(
                        [first] + [{g} for g in (getattr(rule, "Input", None) or [])],
                        [{g} for g in (getattr(rule, "Backtrack", None) or [])],
                        [{g} for g in (getattr(rule, "LookAhead", None) or [])])
                    nested += [r.LookupListIndex for r in (rule.SubstLookupRecord or [])]
            return nested

        if fmt == 2:
            in_c = self.by_class(getattr(sub, "InputClassDef", None))
            bk_c = self.by_class(getattr(sub, "BacktrackClassDef", None))
            ah_c = self.by_class(getattr(sub, "LookAheadClassDef", None))
            sets = getattr(sub, "SubClassSet" if plain else "ChainSubClassSet", None)
            rule_attr = "SubClassRule" if plain else "ChainSubClassRule"
            for cs in (sets or []):
                if cs is None:
                    continue
                for rule in (getattr(cs, rule_attr, None) or []):
                    self.record(
                        [first] + [self.cls(in_c, c)
                                   for c in (getattr(rule, "Input", None) or [])],
                        [self.cls(bk_c, c)
                         for c in (getattr(rule, "Backtrack", None) or [])],
                        [self.cls(ah_c, c)
                         for c in (getattr(rule, "LookAhead", None) or [])])
                    nested += [r.LookupListIndex for r in (rule.SubstLookupRecord or [])]
            return nested
        return nested

    def by_class(self, classdef):
        out = {}
        for glyph, cls in (getattr(classdef, "classDefs", None) or {}).items():
            out.setdefault(cls, set()).add(glyph)
        return out

    def cls(self, table, n):
        if n == 0 and 0 not in table:
            self.open_class = True
            return set()
        return table.get(n, set())

    def record(self, inputs, back, ahead):
        for s in inputs:
            self.subst |= s
        for s in back + ahead:
            self.context |= s
        self.max_input = max(self.max_input, len(inputs))
        self.max_back = max(self.max_back, len(back))
        self.max_ahead = max(self.max_ahead, len(ahead))


def cell_advance(font):
    """Monospace: the advance nearly every glyph shares."""
    hmtx = font["hmtx"]
    counts = {}
    for name in font.getBestCmap().values():
        if name in hmtx.metrics:
            adv = hmtx[name][0]
            counts[adv] = counts.get(adv, 0) + 1
    if not counts:
        return font["head"].unitsPerEm, 0.0
    total = sum(counts.values())
    adv, n = max(counts.items(), key=lambda kv: kv[1])
    return adv, n / total


def overhang(font, glyphs, adv):
    """Worst ink outside the cell, in cells, over the substitution outputs."""
    gs = font.getGlyphSet()
    left = right = 0.0
    worst = None
    for name in glyphs:
        if name not in gs:
            continue
        pen = BoundsPen(gs)
        try:
            gs[name].draw(pen)
        except Exception:
            continue
        if pen.bounds is None:
            continue
        x_min, _, x_max, _ = pen.bounds
        l = max(0.0, -x_min / adv)
        r = max(0.0, (x_max - adv) / adv)
        if l > left:
            left, worst = l, name
        right = max(right, r)
    return left, right, worst


def load(path, ps_name=None):
    if path.endswith(".ttc"):
        coll = TTCollection(path, lazy=False)
        return next(f for f in coll.fonts
                    if f["name"].getDebugName(6) == ps_name)
    return TTFont(path, lazy=False, fontNumber=0)


def discover():
    out = list(SYSTEM)
    for path in sorted(glob.glob(os.path.join(HERE, "fonts", "*.ttf")) +
                       glob.glob(os.path.join(HERE, "fonts", "*.otf"))):
        base = os.path.basename(path)
        # fetch.sh prefixes each file with its repo name; the face is the rest.
        out.append((base.split("-", 1)[1] if "-" in base else base, path, None))
    return out


def show(cps, limit=60):
    printable = "".join(chr(c) for c in cps if 0x21 <= c < 0x7F)
    rest = [c for c in cps if not (0x21 <= c < 0x7F)]
    line = f"      ASCII: {printable}" if printable else "      ASCII: (none)"
    if rest:
        head = " ".join(f"U+{c:04X}" for c in rest[:limit])
        line += f"\n      other ({len(rest)}): {head}"
        if len(rest) > limit:
            line += " …"
    return line


def blame_of(font, tags):
    """Which feature does the collapsing?  Arabic's rlig folding lam-alef is a
    different fact from liga folding "!=" — only the latter breaks the grid."""
    out = {}
    for tag in sorted(tags):
        solo = Probe(font)
        solo.run({tag})
        if solo.collapse > 1:
            out[tag] = solo.collapse
    return out


def report(label, path, ps_name, rows):
    if not os.path.exists(path):
        print(f"\n=== {label}: not installed ({path})")
        return
    font = load(path, ps_name)
    cmap = font.getBestCmap()
    rev = {}
    for cp, gn in cmap.items():
        rev.setdefault(gn, set()).add(cp)

    p = Probe(font)
    tags = p.run(FEATURES)
    adv, mono_share = cell_advance(font)

    def to_cps(glyphs):
        return sorted({cp for g in glyphs for cp in rev.get(g, ())})

    subst = to_cps(p.subst)
    context = to_cps(p.context - p.subst)
    pct = 100 * len(subst) / max(len(cmap), 1)
    left, right, worst = overhang(font, p.outputs, adv)

    if QUIET:
        rows.append((label, len(subst), pct, p.max_input, p.max_back, p.max_ahead,
                     left, right, p.collapse, bool(tags), blame_of(font, tags),
                     mono_share, len(cmap)))
        return
    print(f"\n=== {label} ===")
    print(f"  file            : {os.path.basename(path)}")
    print(f"  features        : {sorted(tags) or '(no ligature features)'}")
    print(f"  cmap / monospace: {len(cmap)} codepoints,"
          f" {100 * mono_share:.0f}% share one advance")
    print(f"  Q1 SUBSTITUTABLE: {len(subst)}  ({pct:.1f}% of cmap)")
    if subst:
        print(show(subst))
    print(f"     context-only : {len(context)}")
    print(f"  Q2 window       : input {p.max_input} cell(s),"
          f" backtrack {p.max_back}, lookahead {p.max_ahead}")
    print(f"  Q3 overhang     : left {left:.2f} cells, right {right:.2f} cells"
          + (f"   (worst: {worst})" if worst else ""))
    blame = blame_of(font, tags)
    if p.collapse > 1:
        who = ", ".join(f"{t} ×{n}" for t, n in blame.items()) or "unattributed"
        print(f"  Q4 collapses    : YES — up to {p.collapse} cells into 1  ({who})")
    else:
        print("  Q4 collapses    : no (cell count invariant)")
    if p.open_class:
        print("  NOTE: a rule matches class 0 (any glyph not otherwise classed)")

    rows.append((label, len(subst), pct, p.max_input, p.max_back, p.max_ahead,
                 left, right, p.collapse, bool(tags), blame, mono_share, len(cmap)))


QUIET = "--summary" in sys.argv

if __name__ == "__main__":
    print(__doc__.split("\n\n")[0].split("\n", 1)[1].strip())
    rows = []
    for label, path, ps in discover():
        report(label, path, ps, rows)

    print("\n" + "=" * 90)
    print(f"{'font':<34}{'cmap':>6}{'mono%':>7}{'subst':>7}{'in':>4}{'bk':>4}"
          f"{'ah':>4}{'left':>7}{'right':>7}{'collapse':>10}")
    print("-" * 90)
    for (label, n, pct, mi, mb, ma, l, r, col, has, blame, mono, ncmap) in rows:
        label = label[:34]
        if not has:
            print(f"{label:<34}{ncmap:>6}{100 * mono:>6.0f}%{'—':>7}{'—':>4}"
                  f"{'—':>4}{'—':>4}{'—':>7}{'—':>7}{'no ligs':>10}")
            continue
        tag = ",".join(blame) if blame else ""
        col_s = f"{tag}×{col}" if col > 1 else "no"
        print(f"{label:<34}{ncmap:>6}{100 * mono:>6.0f}%{n:>7}{mi:>4}{mb:>4}{ma:>4}"
              f"{l:>7.2f}{r:>7.2f}{col_s:>10}")
