# Issue tracker: GitHub issues and ticket files

Specs live as GitHub issues on `kwnms04/knotty`; implementation tickets live as files in this repo. Drive the issues with the `gh` CLI.

## Conventions

- One feature per **parent issue**, whose body is the spec.
- Implementation tickets are **files, not issues** — one per file under `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, committed.
- Blocking is the **file numbering**, blockers first, plus a `Blocked by:` line naming the numbers it waits on.
- A ticket is **done when its acceptance criteria are ticked** in the file, in the commit that implements it. A ticket is workable when every ticket its `Blocked by:` names is done.
- Triage state is a **label** (see `triage-labels.md`), not a line in the body. Labels are for issues; a ticket file needs none, since tickets are agent-grabbable by construction.
- Conversation is issue comments.
- **GitHub milestones are unused.** The M0…M5 axis is `docs/08-milestones.md`, and which milestone a ticket belongs to is the `.scratch/<feature-slug>/` directory it sits in. Once tickets left the tracker a milestone grouped one spec issue, which is what a spec issue already is.

Do not restate a label or a milestone's exit criteria inside the issue body — the tracker owns the first and the chapter owns the second, and a copy in the text goes stale.

**GitHub Projects are deliberately unused.** Nothing in this flow reads a board: "which tickets are workable" is the file order below, not a column. Reach for a Project when a question comes up that the issue list genuinely cannot answer.

## When a skill says "publish to the issue tracker"

**A spec** becomes an issue:

```
gh issue create -R kwnms04/knotty --title "<title>" --body-file <path> \
  --label ready-for-agent
```

**Tickets** become files. One per ticket under `.scratch/<feature-slug>/issues/<NN>-<slug>.md`, numbered from `01` in dependency order, each carrying its `Blocked by:` line. Never a single combined file. Commit them naming the spec's issue number.

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

The M0–M2 tickets were brought back as files under `.scratch/`, their acceptance criteria ticked to record that they are done, and the migrated issues (#1–#23, #27, #29, #32–#36) deleted — a copy that answers nothing is a second place to look. Nineteen commit footers cite numbers that no longer resolve; the `Was: #NN` line in each file is what maps them back. Their milestone specs came with them, except M2's: **the spec stays an issue while its milestone is in flight**, because an open issue says so and a file cannot. #31 becomes `.scratch/m2-first-pixel/spec.md` when M2 closes.

The milestones went with them. M0's grouped nothing at all after the deletion, and M1's and M2's grouped a bug report or two and one open spec — a grouping that small is read off the spec issue and the `.scratch/` directory name instead.

Bug reports (#28, #38, #40, #43) stay issues. A report arrives from outside, gets triaged, and closes — that is issue-shaped work, and none of them came from `/to-tickets`.
