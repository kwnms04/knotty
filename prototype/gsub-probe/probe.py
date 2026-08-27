# /// script
# requires-python = ">=3.11"
# dependencies = ["fonttools>=4.53"]
# ///
"""PROTOTYPE — throwaway. Answers one question, then dies.

    Is the set of codepoints that participate in ligatures small enough
    to keep the fast path?  (docs/04-renderer.md R3, docs/open-questions.md)

Two sets, because they cost different things:

  SUBSTITUTABLE  a codepoint whose own glyph a lookup can replace.
                 A cell holding one of these cannot be an atlas lookup.
  CONTEXT-ONLY   a codepoint that only ever appears in backtrack or
                 lookahead.  Its own glyph never changes; it only decides
                 whether a neighbour's does.

Both over-count: a rule that covers a glyph may never fire on real text.
Over-counting is the safe side of a fast-path decision.

    uv run prototype/gsub-probe/probe.py
"""

import os
import sys
from fontTools.ttLib import TTFont, TTCollection

# What a terminal turns on.  JetBrains Mono puts everything in calt.
FEATURES = {"liga", "calt", "clig", "rlig"}

FONTS = [
    ("JetBrains Mono (primary)", "~/Library/Fonts/JetBrainsMono-Regular.ttf", None),
    ("Menlo (fallback)", "/System/Library/Fonts/Menlo.ttc", "Menlo-Regular"),
    ("SF Mono", "/System/Library/Fonts/SFNSMono.ttf", None),
]


class Probe:
    def __init__(self, font):
        self.font = font
        self.subst = set()      # glyphs a lookup can replace
        self.context = set()    # glyphs that only gate
        self.max_input = 1      # longest matched input run, in cells
        self.max_back = 0       # cells of backtrack a rule can demand
        self.max_ahead = 0
        self.open_class = False  # a rule matched class 0 == "anything else"

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
            lk = lookups[i]
            for sub in lk.SubTable:
                queue.extend(self.visit(sub, lk.LookupType))
        return seen_tags

    def visit(self, sub, lookup_type):
        while getattr(sub, "ExtSubTable", None) is not None:
            sub = sub.ExtSubTable
        t = getattr(sub, "LookupType", None) or lookup_type
        fmt = getattr(sub, "Format", None)

        if t == 1:
            self.subst |= set(sub.mapping)
        elif t == 2:
            self.subst |= set(sub.mapping)
        elif t == 3:
            self.subst |= set(sub.alternates)
        elif t == 4:
            for first, ligs in sub.ligatures.items():
                self.subst.add(first)
                for lig in ligs:
                    self.subst.update(lig.Component)
                    self.max_input = max(self.max_input, 1 + len(lig.Component))
        elif t == 6:
            return self.chain(sub, fmt)
        elif t == 5:
            return self.chain(sub, fmt, plain=True)
        elif t == 8:
            self.subst |= set(sub.Coverage.glyphs)
        return ()

    def chain(self, sub, fmt, plain=False):
        """Chain/plain context.  Returns nested lookup indices to follow."""
        nested = []

        if fmt == 3:
            inputs = [set(c.glyphs) for c in (sub.InputCoverage or [])]
            back = [set(c.glyphs) for c in (getattr(sub, "BacktrackCoverage", None) or [])]
            ahead = [set(c.glyphs) for c in (getattr(sub, "LookAheadCoverage", None) or [])]
            self.record(inputs, back, ahead)
            nested += [r.LookupListIndex for r in (sub.SubstLookupRecord or [])]
            return nested

        first = set(sub.Coverage.glyphs)

        if fmt == 1:
            attr = "ChainSubRuleSet" if not plain else "SubRuleSet"
            rule_attr = "ChainSubRule" if not plain else "SubRule"
            for rs in (getattr(sub, attr, None) or []):
                for rule in (getattr(rs, rule_attr, None) or []):
                    inputs = [first] + [{g} for g in (getattr(rule, "Input", None) or [])]
                    back = [{g} for g in (getattr(rule, "Backtrack", None) or [])]
                    ahead = [{g} for g in (getattr(rule, "LookAhead", None) or [])]
                    self.record(inputs, back, ahead)
                    nested += [r.LookupListIndex
                               for r in (rule.SubstLookupRecord or [])]
            return nested

        if fmt == 2:
            in_cls = self.by_class(getattr(sub, "InputClassDef", None))
            back_cls = self.by_class(getattr(sub, "BacktrackClassDef", None))
            ahead_cls = self.by_class(getattr(sub, "LookAheadClassDef", None))
            attr = "ChainSubClassSet" if not plain else "SubClassSet"
            rule_attr = "ChainSubClassRule" if not plain else "SubClassRule"
            for cs in (getattr(sub, attr, None) or []):
                if cs is None:
                    continue
                for rule in (getattr(cs, rule_attr, None) or []):
                    inputs = [first] + [self.cls(in_cls, c)
                                        for c in (getattr(rule, "Input", None) or [])]
                    back = [self.cls(back_cls, c)
                            for c in (getattr(rule, "Backtrack", None) or [])]
                    ahead = [self.cls(ahead_cls, c)
                             for c in (getattr(rule, "LookAhead", None) or [])]
                    self.record(inputs, back, ahead)
                    nested += [r.LookupListIndex
                               for r in (rule.SubstLookupRecord or [])]
            return nested

        return nested

    def by_class(self, classdef):
        out = {}
        for glyph, cls in (getattr(classdef, "classDefs", None) or {}).items():
            out.setdefault(cls, set()).add(glyph)
        return out

    def cls(self, table, n):
        if n == 0 and 0 not in table:
            self.open_class = True   # class 0 == every glyph not listed
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


def load(path, ps_name):
    if path.endswith(".ttc"):
        coll = TTCollection(path, lazy=False)
        return next(f for f in coll.fonts if f["name"].getDebugName(6) == ps_name)
    return TTFont(path, lazy=False, fontNumber=0)


def show(cps, limit=200):
    printable = "".join(chr(c) for c in cps if 0x21 <= c < 0x7F)
    rest = [c for c in cps if not (0x21 <= c < 0x7F)]
    line = f"      printable ASCII: {printable}" if printable else ""
    if rest:
        head = " ".join(f"U+{c:04X}" for c in rest[:limit])
        line += f"\n      other ({len(rest)}): {head}"
        if len(rest) > limit:
            line += " …"
    return line


def report(label, path, ps_name):
    path = os.path.expanduser(path)
    if not os.path.exists(path):
        print(f"\n{label}: not installed ({path})")
        return

    font = load(path, ps_name)
    cmap = font.getBestCmap()
    rev = {}
    for cp, gn in cmap.items():
        rev.setdefault(gn, set()).add(cp)

    p = Probe(font)
    tags = p.run(FEATURES)

    def to_cps(glyphs):
        return sorted({cp for g in glyphs for cp in rev.get(g, ())})

    subst = to_cps(p.subst)
    context = to_cps(p.context - p.subst)

    print(f"\n=== {label} ===")
    print(f"  features        : {sorted(tags) or 'none of ' + str(sorted(FEATURES))}")
    print(f"  cmap            : {len(cmap)} codepoints")
    print(f"  SUBSTITUTABLE   : {len(subst)}"
          f"  ({100 * len(subst) / max(len(cmap), 1):.1f}% of cmap)")
    if subst:
        print(show(subst))
    print(f"  CONTEXT-ONLY    : {len(context)}")
    if context:
        print(show(context, limit=40))
    print(f"  longest input   : {p.max_input} cells"
          f"   (backtrack {p.max_back}, lookahead {p.max_ahead})")
    if p.open_class:
        print("  NOTE: a rule matches class 0 (\"any glyph not otherwise "
              "classed\") — context is effectively unbounded there.")


if __name__ == "__main__":
    print(__doc__.split("\n\n")[1].strip())
    for label, path, ps in FONTS:
        report(label, path, ps)
