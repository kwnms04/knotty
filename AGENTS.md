# Knotty

## Agent skills

### Issue tracker

Issues live as markdown files under `.scratch/<feature>/` in this repo.
See `docs/agents/issue-tracker.md`.

### Triage labels

The five canonical triage roles, used verbatim as label strings.
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
