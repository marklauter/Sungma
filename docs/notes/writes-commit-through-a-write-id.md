---
title: Writes could commit through a write id
type: note
summary: "A write too big for one DynamoDB transaction can still be atomic if its items carry a write id, not a revision, and one small transaction commits the write by recording the revision that id stands for. Adopting it changes how every write and every read works."
status: evolving
cites:
  - "[[revision]]"
  - "[[revision-clock]]"
  - "[[fact]]"
  - "[[span]]"
  - "[[namespace]]"
  - "[[epoch]]"
---

# Writes could commit through a write id

It's a proposal. [Table](../specs/table.md) describes how writes work today, and links here from its open question.

## The problem

Sungma keeps everything in one DynamoDB table, as [Table](../specs/table.md) specifies. Every write today is one unit of work, a single DynamoDB transaction, that does two things at once:

- It stores the write's items: [[span]]s and pointers for [[fact]]s, entries for [[namespace]] versions.
- It moves the head, the one item that holds the latest write's [[revision]], to the write's revision, on condition the head hasn't moved since the writer read it.

Reads happen at a revision no later than the head, so a write appears all at once when its transaction commits. [One timeline](../specs/one-timeline.md) builds on that.

A DynamoDB transaction holds at most 100 items and 4 MB. A fact takes up to three items, a pointer and two spans, so one write can carry only about 30 facts. Two kinds of write need more:

- Creating a namespace in [Sungma's own authorization](../specs/sungma-authorization.md) also writes a `parent` fact for each of its relations, and a namespace may declare 500.
- Loading or changing many facts at once, such as a bulk import.

Splitting a big write into several transactions loses atomicity: between them, readers see half the write.

## Answers that don't work

- **Retry until done, undo on failure** (a saga). Readers see the half-done state while it runs, and undoing can clash with writes that came in between.
- **Lock, then write in pieces.** A lock stops other writers but not readers. They see the pieces unless every read also checks the lock.
- **Write far in the future, then move everything back.** Moving every item to its real revision at the end is itself a write of more than 100 items.

## The proposal

Every item a write stores carries a **write id** in place of a revision. A separate **status record** for the write says whether it is open, committed, or aborted, and, once committed, which revision it committed at. A read treats an item stamped with a write id as stamped with that write's revision if the write committed, and as absent otherwise.

So a big write can store its items in as many transactions as it needs. None of them is visible until one small transaction commits the whole write by flipping its status record to committed.

### Write ids

A write takes a fresh write id before it stores anything. Write ids come from a pool counter, the way interned names get their ids ([Table](../specs/table.md#names)), so they are unique and cheap to hand out.

### The status record

Each write has one status record, keyed by its write id. It holds:

- **Its state:** open, committed, or aborted.
- **A lease** while open: the time after which another writer may abort it.
- **The revision it committed at,** once committed. This is the map from write id to revision. It never changes after the commit.
- **The request id** of the request that started it, for retries (see [Retries](#retries)).

### What gets stamped

Every place that holds a revision today holds a write id instead:

- A span's written-by and retired-by.
- A fact pointer's record of which span is open. The pointer is also the marker that tells a second writer this fact is in use (see [Conflicts](#conflicts)).
- A namespace entry's revision, and the namespace's latest item.

Some of these are key parts, not attributes. A span's sort key and a namespace entry's sort key include the revision. A DynamoDB key can't change, so a stamped item either keeps its write id in its key for good, or is rewritten under its revision later. That is a layout decision for [Table](../specs/table.md).

### The commit

A write commits in one transaction holding two items:

1. The head, moved to a new revision R, on condition it still holds the revision the writer read.
2. The write's status record, set to committed at R, on condition it is still open.

The write's items were stored before, so the commit stays this small however big the write is.

R comes from the [[revision-clock]] at commit time, and the writer waits out commit-wait before acknowledging, as every write does today ([The clock](../specs/clock.md)). The revision is chosen at commit, not at the start. So if the head moves while a big write is storing its items, nothing is lost: only the commit retries, with a new R. The items stay where they are.

A write small enough for one transaction does all of it at once: its items, its status record committed at R, and the head move, in one transaction. That is today's write with a status record added, so the one mechanism covers big and small writes.

### Reads

A read at revision Q meets items stamped with write ids, and looks each write id up:

- Committed at some R at or before Q: the item counts as stamped with R.
- Committed after Q, open, or aborted: the item counts as absent. A retired-by stamp from such a write counts as not retired.

A committed status never changes, so a reader can cache it forever, and many write ids can be looked up in one batch read. Reads of old data meet only committed write ids that are already cached, so only reads near the head pay a lookup.

A background sweep can rewrite committed items with their revision, after which no read needs the lookup. The status record stays: a read that saw the write id a moment before the sweep must still be able to look it up.

### Conflicts

Two writes can touch the same fact while one of them is open. The fact's pointer settles it. A write that changes a fact sets the fact's pointer to its own write id, on condition the pointer still holds what the writer read. A second write that finds a pointer holding another write's id reads that write's status:

- Committed or aborted: the second write goes ahead.
- Open with its lease unexpired: the second write waits, or gives up and reports a conflict.
- Open with its lease expired: the second write sets that write's status to aborted, on condition it is still open, and goes ahead.

Because the commit is conditional on the status still being open, an aborted write can never commit afterwards, even if it was only slow.

### Leases and the sweep

An open write renews its lease while it works. A write whose lease runs out is aborted by whoever meets it first: a conflicting writer, or the sweep. The sweep also deletes the items of aborted writes, and restores the pointers and spans they changed, so the table doesn't fill with debris. Nothing an aborted write stored is ever visible, so the sweep can run slowly.

### Retries

A request can time out after its write committed, with only the reply lost. A retry under a new write id would apply the same change twice. So every request carries a request id, the status record keeps it, and a retry first looks for a committed write with its request id. If it finds one, it returns that write's revision instead of writing again.

### Order

- A namespace write names the version it was based on ([Namespace writes](../specs/namespace-writes.md#concurrent-writes)). Of two saves based on one version, the second to commit is refused, so two saves never land out of order unnoticed.
- Fact writes carry no base version. They commit in the order sent only when the caller waits for each acknowledgment before sending the next. That is true today as well.

## A worked example

Alice adds two facts in one request, with request id `q1`.

1. The writer takes write id 1, stores status 1 as open with a lease and request `q1`, and stores the first fact's span and pointer stamped with write id 1.
2. DynamoDB stops answering. The request times out before the second fact is stored. Write 1 is open, with half its items stored. A reader that meets the first fact's span looks up write 1, finds it open, and treats the fact as absent. Nothing is visible.
3. The request goes back in the queue. Meanwhile other writes commit and move the head on.
4. The retry finds no committed write with request `q1`, so it takes write id 5 and stores status 5 as open.
5. It goes to store the first fact and finds the fact's pointer holding write id 1. Status 1 is open and its lease has expired, so it sets status 1 to aborted and goes ahead. It stores both facts stamped with write id 5.
6. It commits: it reads the head, takes the next revision R from the clock, moves the head to R and sets status 5 to committed at R, in one transaction, then waits out commit-wait and acknowledges.
7. Readers at R or later see both facts, from R on. Readers before R see neither. No reader ever saw write 1's half.
8. Later, the sweep deletes write 1's span and restores the pointer it changed, and rewrites write 5's items with R.

If write 1 had committed and only its reply was lost, step 4 would have found request `q1` committed and returned its revision without writing again.

## What else changes if it's adopted

It changes every write and every read, so it is all or nothing: reads must handle write ids wherever they appear, and every write must respect another write's pointer markers.

- [Table](../specs/table.md): the status record as a new item kind, write ids in place of revisions, the key question above, and the unit of work for each kind of write.
- [One timeline](../specs/one-timeline.md): how a read decides what counts, as in [Reads](#reads).
- [Namespace writes](../specs/namespace-writes.md): a namespace write may store more than one unit of work holds, and may store facts.

## What it costs

- A status record per write, kept for good, or until no read can still need it.
- A lookup for reads that meet uncommitted or recently committed write ids, mostly cached.
- A sweep to run.
- Conflicting writes on one fact wait for each other, or for a lease to expire when a writer dies.

## Other answers

- **A head lease.** The big write reserves the head, stores every item at the next revision R in pieces, and moves the head to R last. Reads stop at the head, so nothing at R shows early. Other writes wait the whole time, and the head must never move past R while R's items remain: a dead writer's items have to be undone, or R marked aborted, first. Simpler for readers, but every big write stops every other write.
- **A cap on items per write.** Big changes are split into several versions by the caller. Simple, but an odd limit for an author to run into.
- **Writing the extra items after the commit.** Works only where their absence fails closed, such as `parent` facts that grant access once they exist.

## Prior art

- Iceberg and Delta Lake commit a table version by flipping one pointer to it.
- Postgres marks each row with the transaction that wrote it, and keeps every transaction's status in a commit log.
- Google's Percolator builds multi-row transactions on Bigtable, which has none, by committing through one primary record.

## Open

1. Whether stamped items keep their write id in their key for good, or are rewritten under their revision by the sweep.
2. How long a lease is, and how often an open write renews it.
3. Whether a conflicting writer waits for an open write or gives up at once.
4. Whether status records are kept forever or dropped after a grace period longer than any read.
5. Whether this or a head lease is chosen.
