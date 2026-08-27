# Issue tracker: an inbox on GitHub, the work in the repo

What arrives from outside — bug reports, feature requests — lives as GitHub issues on `kwnms04/knotty`. Drive them with the `gh` CLI. What the project writes about itself — specs and tickets — lives as files in this repo.

## Conventions

- One feature per **spec**, `.scratch/<feature-slug>/spec.md`, carrying its milestone's exit criteria as tickboxes.
- Implementation tickets are **files, not issues** — one per file under `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, committed.
- Blocking is the **file numbering**, blockers first, plus a `Blocked by:` line naming the numbers it waits on.
- A ticket is **done when its acceptance criteria are ticked** in the file, in the commit that implements it. A ticket is workable when every ticket its `Blocked by:` names is done.
- Triage state is a **label** (see `triage-labels.md`), not a line in the body. Labels are for issues; a ticket file needs none, since tickets are agent-grabbable by construction.
- A milestone is **in flight while its spec has an unticked exit criterion**, and that is the whole of what says so.
- Conversation is issue comments. It is for the reports that arrive as issues; nothing in the repo waits on a comment.
- **GitHub milestones are unused.** The M0…M5 axis is `docs/08-milestones.md`, and which milestone a file belongs to is the `.scratch/<feature-slug>/` directory it sits in.

Do not restate a label inside an issue body — the tracker owns it, and a copy in the text goes stale. A spec's exit criteria are the one deliberate copy: `docs/08-milestones.md` states the criterion because that is what defines the milestone, and the spec file carries the tickbox because something has to record whether it has been met. Fix the chapter first if the two ever disagree.

**GitHub Projects are deliberately unused.** Nothing in this flow reads a board: "which tickets are workable" is the file order below, not a column. Reach for a Project when a question comes up that the issue list genuinely cannot answer.

## Driving the tracker

`gh` infers the repo from `git remote -v`, so none of these has to name it.

- **Create**: `gh issue create --title "..." --body "..." --label <name>`. Heredoc for a multi-line body.
- **Read**: `gh issue view <number> --comments`
- **List**: `gh issue list --state open --json number,title,body,labels,comments --jq '[.[] | {number, title, body, labels: [.labels[].name], comments: [.comments[].body]}]'`, with `--label` and `--state` filters as needed.
- **Comment**: `gh issue comment <number> --body "..."`
- **Label**: `gh issue edit <number> --add-label "..."` / `--remove-label "..."`
- **Close**: `gh issue close <number> --comment "..."`

## Pull requests as a request surface

**No.** `/triage` reads this flag, and issues are the only request surface here — every pull request so far has been the author's own, so there is no external-PR queue to run through the states.

Flipping it to yes means taking the `gh pr` half of the GitHub template in `setup-matt-pocock-skills`: the same labels and the same states, read against the diff instead of the body.

## When a skill says "publish to the issue tracker"

Nothing a skill writes goes to the tracker any more — both of its outputs are files.

**A spec** becomes `.scratch/<feature-slug>/spec.md`. Copy the milestone's exit criteria from `docs/08-milestones.md` in as tickboxes; they are what says the milestone is still running.

**Tickets** become files. One per ticket under `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01` in dependency order, each carrying its `Blocked by:` line. Never a single combined file. Commit them with the spec.

## When a skill says "fetch the relevant ticket"

```
cat .scratch/<feature-slug>/issues/<NN>-<slug>.md
```

The user will normally pass the path directly. A `Was: #NN` line in each file
names the issue it was migrated from. That issue is gone, so the number now
resolves only against git history and against this line.

## Finding the workable tickets

```
ls .scratch/<feature-slug>/issues/
```

The lowest-numbered ticket whose `Blocked by:` list is all ticked.

## Wayfinding operations

`/wayfinder` has never been run here. Whether its child tickets are issues or files — whether the ticket rule above reaches them at all — is settled the first time it is used, and not before. No wiring is written down until then.

## History

M0–M2 ran their specs and tickets as GitHub issues. They came back as files under `.scratch/`, their acceptance criteria ticked to record that they are done, and the migrated issues (#1–#23, #27, #29, #32–#36) were deleted: across 25 ticket issues the only issue feature ever used was the closed bit, and a copy that answers nothing is a second place to look. The GitHub milestones went with them, having grouped a bug report or two at most.

Nineteen commit footers cite numbers that no longer resolve. The `Was: #NN` line in each ticket file is what maps them back.

Bug reports (#28, #38, #40, #43) stay issues, and are now the only thing the tracker holds. A report arrives from outside, is triaged, and closes — that is issue-shaped work. Everything the project wrote about itself is in the repo.
