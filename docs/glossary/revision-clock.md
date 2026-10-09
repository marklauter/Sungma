---
title: revision clock
type: definition
summary: "The clock every write is versioned from, whose revisions order writes as they happened in real time."
status: evolving
---

The clock every write is versioned from, whose revisions order writes as they happened in real time.

## Examples

- A theory version written at revision 3 and replaced at revision 7 holds over [3, 7) on the revision clock. Revision 3 is that version.
- A write acknowledged at revision 5 is followed by a write on another node. The second write gets a revision after 5.

## Rationale

A theory version is identified by its revision, and a fact's span opens and closes at revisions, so every write takes its version from this one clock. Spans, snapshots and catalogs are all measured against it. A revision is a timestamp, or a counter on a single node. Commit-wait keeps revisions in real-time order across nodes; see [[clock]].
