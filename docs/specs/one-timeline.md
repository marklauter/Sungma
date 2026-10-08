---
title: One timeline for facts and theories
type: specification
summary: "Facts and theory versions hold over spans of one revision sequence; a revision token names a snapshot of both, and epochs keep orphaned facts dead."
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

# One timeline for facts and theories

Proposed. Every [[fact]] and every [[theory]] version holds over a [[span]] of one [[revision]] sequence, so one revision selects both the [[graph]] and the [[catalog]].

## Revisions and revision tokens

- A revision is a commit timestamp. Clocks are synchronized by NTP, and each write absorbs the clock uncertainty by commit-wait: it takes timestamp t and acknowledges only once every clock has passed t. A reader therefore never needs the uncertainty bound.
- Callers never hold a bare revision. They hold a [[revision-token]], an opaque token naming a [[snapshot]] by its revision.
- A caller gets a revision token from Sungma. A write returns the revision token of its revision, and a check, read or expand returns the revision token it was evaluated at. A client stores the revision token from a content-change check alongside that version of its content.
- On a request, a revision token is a floor: the read is at that revision or later. A request without a revision token reads the latest snapshot. There is no staleness tolerance between the two.
- On a [[decision]], the revision token names the revision judged, and replay reads that revision.

## Spans

- A write opens a span at its revision. A delete closes it at its revision. A fact holds at revision R when a span covers R.
- A fact deleted and written again holds over two spans.
- A theory version holds over a span the same way. Writing a new version closes the previous one's span.

## Epochs and orphaned facts

- Each theory version records an [[epoch]] for each relation it declares: the revision at which the relation was last declared. A version that keeps a relation keeps its epoch. A version that adds a relation, or declares a dropped one again, sets its epoch to the version's revision.
- At revision R, a fact counts only when its theory version at R declares its relation and the fact's span opened at or after that relation's epoch.
- A fact under a relation its theory dropped is an [[orphaned-fact]]. It counts as nonexistent. Nothing deletes it, and replays before the drop still see it.
- Declaring the relation again moves its epoch, so orphaned facts stay dead and old grants don't resurrect.
- A write of a fact whose current span opened before its relation's epoch opens a new span. Otherwise a grant asserted again after the relation returns would stay dead.
- Every reader applies the same rule: decide, expand, read, watch and dumps.

## What is accepted

- A theory write never scans the graph and never rewrites facts. Relation names in rewrites are bound late, so the catalog can't tell which other theories reach a dropped relation. Dropping a relation empties it everywhere, including on the excluded side of another theory's exclusion, which can widen access there.
- The eager alternative closes orphaned facts' spans in the theory write. It shows the same facts at every revision, but the write costs as much as there are orphans.

## Fact writes

A fact write is checked against the theory version at the write's revision: the fact's theory must declare its relation, and a subjectset subject's theory must declare that subject's relation. A fact that fails would be orphaned from its first revision. The check reports the caller's mistake instead of storing a fact that can never count.

## Related

- [[ideas-from-kingo]], ideas 1, 4 and 5.
- [[theory-writes-wait-on-theory-versioning]].
