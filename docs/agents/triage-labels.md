# Triage Labels

The skills speak in terms of seven canonical triage roles — two category, five state. This file maps those roles to the actual label strings used in this repo's issue tracker.

| Label in mattpocock/skills | Label in our tracker | Meaning                                  |
| -------------------------- | -------------------- | ---------------------------------------- |
| `bug`                      | `bug`                | Something is broken                      |
| `enhancement`              | `enhancement`        | New feature or improvement               |
| `needs-triage`             | `needs-triage`       | Maintainer needs to evaluate this issue  |
| `needs-info`               | `needs-info`         | Waiting on reporter for more information |
| `ready-for-agent`          | `ready-for-agent`    | Fully specified, ready for an AFK agent  |
| `ready-for-human`          | `ready-for-human`    | Requires human implementation            |
| `wontfix`                  | `wontfix`            | Will not be actioned                     |

These exist as real GitHub labels on `kwnms04/knotty`. Apply one with `gh issue create --label <name>` or `gh issue edit <n> --add-label <name>`.

When a skill mentions a role (e.g. "apply the AFK-ready triage label"), use the corresponding label string from this table.
