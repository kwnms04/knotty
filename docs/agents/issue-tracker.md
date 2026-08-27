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

Used by `/wayfinder`. Its child tickets **are** issues — they carry a question, collect an answer in comments and close, which is issue-shaped work and not the ticket file above. The **map** is the parent issue; each **child ticket** is a sub-issue.

- **Map**: the parent issue body holds Notes / Decisions-so-far / Fog.
- **Child ticket**: a sub-issue whose body is the question. A `Type:` line records the ticket type (`research`/`prototype`/`grilling`/`task`).
- **Blocking**: a native dependency. Sub-issue and dependency edges have no `gh issue` subcommand yet, so they go through GraphQL. Resolve node IDs first:

```
gh api graphql -f query='
  query($n:Int!){ repository(owner:"kwnms04",name:"knotty"){ issue(number:$n){ id } } }
' -F n=12
```

Then wire the edges:

```
# make CHILD a sub-issue of PARENT
gh api graphql -f query='
  mutation($p:ID!,$c:ID!){ addSubIssue(input:{issueId:$p, subIssueId:$c}){ clientMutationId } }
' -f p=<parent-id> -f c=<child-id>

# record that BLOCKED cannot start until BLOCKER is done
gh api graphql -f query='
  mutation($b:ID!,$k:ID!){ addBlockedBy(input:{issueId:$b, blockingIssueId:$k}){ clientMutationId } }
' -f b=<blocked-id> -f k=<blocker-id>
```

- **Frontier**: open, unblocked, unassigned sub-issues; lowest number wins.
- **Claim**: assign the issue to yourself (`gh issue edit <n> --add-assignee @me`) before any work.
- **Resolve**: comment the answer, close the issue, then append a context pointer (gist + link) to Decisions-so-far in the parent body.

## History

M0 ran on local markdown — parent spec and tickets under `.scratch/m0-headless-core/` — and was migrated to GitHub as parent #1 with tickets #2–#9. The original files remain in git history.

M1 (#11–#29) and M2 (#32–#36) ran their tickets as sub-issues, and M0's files were deleted when it migrated. **Tickets are files again.** Across those 25 ticket issues not one acceptance-criteria checkbox was ever ticked and every one carried the same `ready-for-agent` label, so the only issue feature they used was the closed bit — paid for with two GraphQL mutations each and a network round trip per read.

The M0–M2 tickets were brought back as files under `.scratch/`, their acceptance criteria ticked to record that they are done, and the migrated issues (#1–#23, #27, #29, #32–#36) deleted — a copy that answers nothing is a second place to look. Nineteen commit footers cite numbers that no longer resolve; the `Was: #NN` line in each file is what maps them back. Their milestone specs came with them. M2's spec, #31, was kept an issue for a day on the argument that an open issue says the milestone is in flight and a file cannot — but with its sub-issues and milestone gone it was down to an open bit and a label, and the one spec nobody could read without a network was the one being worked on. An unticked exit criterion in `spec.md` says the same thing from inside the repo.

The milestones went with them. M0's grouped nothing at all after the deletion, and M1's and M2's grouped a bug report or two — a grouping that small is read off the `.scratch/` directory name instead.

Bug reports (#28, #38, #40, #43) stay issues, and are now the only thing the tracker holds. A report arrives from outside, is triaged, and closes — that is issue-shaped work. Everything the project wrote about itself is in the repo.

`/wayfinder`'s child tickets are the loose end. The flow has never been run here, and its questions are authored inside the project the way a spec is, so whether they stay issues is open until it is.
