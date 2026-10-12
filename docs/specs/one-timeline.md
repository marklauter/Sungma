---
title: One timeline for facts and namespaces
type: specification
summary: "Facts and namespace versions hold over spans of one revision sequence; a revision token names a snapshot of both, and epochs keep orphaned facts dead."
status: evolving
cites:
  - "[[revision]]"
  - "[[revision-token]]"
  - "[[snapshot]]"
  - "[[span]]"
  - "[[epoch]]"
  - "[[orphaned-fact]]"
  - "[[graph]]"
  - "[[catalog]]"
  - "[[decision]]"
---

# One timeline for facts and namespaces

Proposed. Every [[fact]] and every [[namespace]] version holds over a [[span]] of one [[revision]] sequence, so one revision selects both the [[graph]] and the [[catalog]].

## Revisions and revision tokens

- A revision is a commit timestamp. Clocks are synchronized by NTP, and each write absorbs the clock uncertainty by commit-wait: it takes timestamp t and acknowledges only once every clock has passed t. A reader therefore never needs the uncertainty bound.
- Callers never hold a bare revision. They hold a [[revision-token]], an opaque token naming a [[snapshot]] by its revision.
- A caller gets a revision token from Sungma. A write returns the revision token of its revision, and a check, read or expand returns the revision token it was evaluated at. A client stores the revision token from a content-change check alongside that version of its content.
- On a request, a revision token is a floor: the read is at that revision or later. A request without a revision token reads the latest snapshot. There is no staleness tolerance between the two.
- On a [[decision]], the revision token names the revision judged, and replay reads that revision.

## Spans

- A write opens a span at its revision. A delete closes it at its revision. A fact holds at revision R when a span covers R.
- A fact deleted and written again holds over two spans.
- A namespace version holds over a span the same way. Writing a new version closes the previous one's span.

## Epochs and orphaned facts

- Each namespace version records an [[epoch]] for each relation it declares: the revision at which the relation was last declared. A version that keeps a relation keeps its epoch. A version that adds a relation, or declares a dropped one again, sets its epoch to the version's revision.
- At revision R, a fact counts only when its namespace version at R declares its relation and the fact's span opened at or after that relation's epoch.
- A fact under a relation its namespace dropped is an [[orphaned-fact]]. It counts as nonexistent. Nothing deletes it, and replays before the drop still see it.
- Declaring the relation again moves its epoch, so orphaned facts stay dead and old grants don't resurrect.
- A write of a fact whose current span opened before its relation's epoch opens a new span. Otherwise a grant asserted again after the relation returns would stay dead.
- Every reader applies the same rule: decide, expand, read, watch and dumps.
- An epoch is per relation, not per namespace version. Writing a new version changes no facts, so facts under a relation the new version keeps still count. Only a relation that was dropped and declared again gets a new epoch.

## Retiring a namespace

- Retiring a namespace closes its current version's span and opens no new one. From then on no version declares its relations, so every fact under it is orphaned.
- Writing the namespace again after it was retired declares each of its relations again. Each gets the new version's revision as its epoch, so facts from before the retirement don't resurrect.

## Sample

The `file` namespace over time, and the facts under it:

| Revision | Write | Epochs after it |
| --- | --- | --- |
| 10 | Version 1 declares `owner` and `banned`. | `owner` 10, `banned` 10 |
| 12 | Fact `file:a#owner@carol`. | |
| 14 | Fact `file:a#banned@bob`. | |
| 20 | Version 2 drops `banned`. | `owner` 10 |
| 30 | Version 3 declares `banned` again. | `owner` 10, `banned` 30 |
| 35 | Fact `file:a#banned@erin`. | |
| 40 | `file` is retired. | none |
| 50 | Version 4 declares `owner` and `banned`. | `owner` 50, `banned` 50 |
| 60 | Fact `file:a#banned@bob` written again. | |

What a read at each revision counts:

| Read at | `owner@carol` | `banned@bob` | `banned@erin` | Why |
| --- | --- | --- | --- | --- |
| 15 | counts | counts | | Version 1; both written at or after epoch 10. |
| 25 | counts | dead | | Version 2 doesn't declare `banned`. `owner` keeps epoch 10. |
| 35 | counts | dead | counts | Version 3: `banned` has epoch 30, and 14 is before it; 35 isn't. |
| 45 | dead | dead | dead | Retired: no version declares anything. |
| 55 | dead | dead | dead | Version 4: every epoch is 50, and every fact is older. |
| 65 | dead | counts | dead | Bob's fact, written again at 60, opened a new span after epoch 50. |

Bob's ban doesn't come back at 35, and none of the three facts comes back at 55. Writing bob's fact again at 60 restores it deliberately: its old span opened before the epoch, so the write opens a new one. A replay at 15 still reads version 1 and finds bob banned.

## What is accepted

- A namespace write never scans the graph and never rewrites facts. Relation names in rewrites are bound late, so the catalog can't tell which other namespaces reach a dropped relation. Dropping a relation empties it everywhere, including on the excluded side of another namespace's exclusion, which can widen access there.
- The eager alternative closes orphaned facts' spans in the namespace write. It shows the same facts at every revision, but the write costs as much as there are orphans.

## Fact writes

A fact write is checked against the namespace version at the write's revision: the fact's namespace must declare its relation, and a subjectset subject's namespace must declare that subject's relation. A fact that fails would be orphaned from its first revision. The check reports the caller's mistake instead of storing a fact that can never count.

## Related

- [[ideas-from-kingo]], ideas 1, 4 and 5.
- [[namespace-writes-wait-on-namespace-versioning]].
