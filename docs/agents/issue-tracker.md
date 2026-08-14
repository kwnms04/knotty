# Issue tracker: GitHub Issues

Issues and specs for this repo live as GitHub issues on `kwnms04/knotty`. Drive them with the `gh` CLI.

## Conventions

- One feature per **parent issue**, whose body is the spec.
- Implementation tickets are **sub-issues** of that parent, one per ticket.
- Triage state is a **label** (see `triage-labels.md`), not a line in the body.
- Blocking is a **native dependency**, not prose. A ticket is workable when every issue in its `blockedBy` list is closed.
- Conversation is issue comments.

Do not restate a label or a dependency inside the issue body — the tracker owns both, and a copy in the text goes stale.

## When a skill says "publish to the issue tracker"

Create the issue, then attach it:

```
gh issue create -R kwnms04/knotty --title "<title>" --body-file <path> --label ready-for-agent
```

Sub-issue and dependency edges have no `gh issue` subcommand yet, so they go through GraphQL. Resolve node IDs first:

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

## When a skill says "fetch the relevant ticket"

```
gh issue view <number> -R kwnms04/knotty --comments
```

The user will normally pass the number or the URL directly.

## Finding the workable tickets

List candidates, then drop the ones still blocked:

```
gh issue list -R kwnms04/knotty --label ready-for-agent --state open

gh api graphql -f query='
  query($n:Int!){ repository(owner:"kwnms04",name:"knotty"){ issue(number:$n){
    blockedBy(first:20){ nodes{ number state } } } } }
' -F n=<number>
```

A ticket is workable when that list is empty or every node is `CLOSED`.

## Wayfinding operations

Used by `/wayfinder`. The **map** is the parent issue; each **child ticket** is a sub-issue.

- **Map**: the parent issue body holds Notes / Decisions-so-far / Fog.
- **Child ticket**: a sub-issue whose body is the question. A `Type:` line records the ticket type (`research`/`prototype`/`grilling`/`task`).
- **Blocking**: the native dependency above.
- **Frontier**: open, unblocked, unassigned sub-issues; lowest number wins.
- **Claim**: assign the issue to yourself (`gh issue edit <n> --add-assignee @me`) before any work.
- **Resolve**: comment the answer, close the issue, then append a context pointer (gist + link) to Decisions-so-far in the parent body.

## History

M0 was migrated here from a local-markdown tracker: parent #1 with tickets #2–#9, all closed as completed. The original files lived at `.scratch/m0-headless-core/` and remain in git history.
