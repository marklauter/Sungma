---
title: Writes could commit through a write id
type: note
summary: "A write that needs more items than one DynamoDB transaction holds can still be atomic if its items carry a write id, not a revision, and one small transaction commits the write by recording the revision that id stands for. Its items need only small transactions, one per fact. Adopting it changes how every write and every read works."
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

Sungma keeps everything in one DynamoDB table, as [Table](../specs/table.md) specifies. A [[fact]] is one assertion, such as "Carol owns file a". A [[namespace]] declares the relations its facts can use, and changes by new versions. Each write takes a [[revision]], a commit timestamp from the writer's [[revision-clock]], and everything lives on one timeline of revisions.

The table holds these items:

- **Spans.** A fact holds over one or more [[span]]s, each an interval of revisions. A span is one item. It records the revision that wrote it, and the revision that retired it once it closes. A span is never removed.
- **Fact pointers.** Each fact has one pointer item. It records which of the fact's spans is open, if any. Every write to a fact is conditioned on its pointer, so two writers can't both open a span for one fact.
- **Namespace entries.** Each namespace version is one entry item, keyed by the revision it opens at. A version records an [[epoch]] for each relation it declares: the revision from which facts under that relation count. A version that adds a relation, or declares a dropped one again, sets its epoch to the version's own revision.
- **The namespace's latest item.** Each namespace has one item that holds a copy of its newest entry. Every namespace write overwrites it in place. A fact write reads it to judge the fact against the namespace.
- **The head.** One item that holds the latest write's revision.

Every write today is one unit of work, a single DynamoDB transaction. It stores the write's items and moves the head to the write's revision, on condition the head hasn't moved since the writer read it. A fact write also checks, in the same transaction, that each namespace latest item it judged against is unchanged.

Reads happen at a revision no later than the head, so a write appears all at once when its transaction commits. [One timeline](../specs/one-timeline.md) builds on that.

After the transaction, the writer waits out commit-wait before it acknowledges: it acknowledges only once every clock has passed the write's revision, as [The clock](../specs/clock.md) specifies. So any write that starts later takes a later revision.

A DynamoDB transaction holds at most 100 items and 4 MB. A fact takes up to three items, a pointer and two spans, so one write can carry only about 30 facts. Two kinds of request need more:

- Creating a namespace in [Sungma's own authorization](../specs/sungma-authorization.md) also writes a `parent` fact for each of its relations, and a namespace may declare 500.
- Loading or changing many facts at once, such as a bulk import.

Splitting such a write into several transactions loses atomicity: between them, readers see half the write.

## Answers that don't work

- **Retry until done, undo on failure** (a saga). Readers see the half-done state while it runs, and undoing can clash with writes that came in between.
- **Lock, then write in pieces.** A lock stops other writers but not readers. They see the pieces unless every read also checks the lock.
- **Write far in the future, then move everything back.** Moving every item to its real revision at the end is itself a write of more than 100 items.

## The proposal

Every item a write stores carries a **write id** in place of a revision. A separate **status record** for the write says whether it is open, committed, or aborted, and, once committed, which revision it committed at. A read treats an item stamped with a write id as stamped with that write's revision if the write committed, and as absent otherwise.

So a write can store its items with plain writes, as many as it needs. None of them is visible until one small transaction commits the whole write by setting its status record to committed.

One background job, **the sweep**, cleans up after writes. It has three duties:

1. Abort open writes whose lease has expired.
2. Undo what aborted writes stored.
3. Rewrite committed writes' items with their revision, and fold their claims into the committed state, so later reads need no lookup.

Nothing an aborted or open write stored is ever visible, so the sweep can run slowly.

### Write ids

A write's id comes from the request that asks for it. Every write request carries an `Idempotency-Key` header, the IETF draft standard for this: a unique id the caller generates once for each logical write, and sends again unchanged on every retry of that write. A server-generated request id, such as ASP.NET Core's `TraceIdentifier`, can't serve, because a retry is a new HTTP request and gets a new one. Only the caller knows that a second request is the first one again.

A write id is the caller's `sub`, the request's idempotency key, and an attempt number: Mallory's request `q1`, attempt 1, and attempt 2 if attempt 1 is aborted. Scoping it to the caller means one caller can't collide with another's keys, by guessing or by accident. Write ids are made in the order writes start, not the order they commit.

### The status record

Each write has one status record, keyed by its write id. It holds:

- **Its state:** open, committed, or aborted. Committed and aborted are final.
- **A lease expiry** while open: a time after which another writer may abort it.
- **The revision it committed at,** once committed. This is the map from write id to revision. It never changes after the commit.
- **Nothing more for retries:** its write id already holds the caller, the idempotency key and the attempt (see [Retries](#retries)).
- **The partitions it touched.** The write appends each partition to this list before it stores anything there, so the sweep can find the write's items without scanning the table. Partitions, not single items, keep the record within DynamoDB's item size.

### Leases

The writer sets the lease expiry from its own clock, and renews it while it works. Renewal is conditional: it succeeds only while the status is still open. A write that was aborted while it was slow can't renew itself back to life, and its commit is refused too.

Another writer judges a lease by its own clock, and allows for clock uncertainty. It treats the lease as expired only once the earliest true time its clock allows is past the expiry.

### What gets stamped

Every place that holds a revision today holds a write id instead:

- A span's revision that wrote it, and its revision that retired it.
- A namespace entry's revision, and the epoch of each relation the version adds or brings back. That epoch is the version's own revision, which isn't known until commit, so it is stamped with the write id and resolved on read like any other stamp.

A write that stores its items in the same transaction as its commit knows its revision R when it stores them (see [The commit](#the-commit)). So it stamps R directly, and readers need no lookup for it. It still writes a status record, and it still respects other writes' claims.

### Claims

The fact pointer and the namespace latest item each hold two things apart:

- **The committed state:** for a pointer, which span is open as of the last committed write; for a latest item, the last committed entry.
- **A claim:** the write id of a write that is changing it, and the state that write will leave.

Google's Percolator, a transaction layer on Bigtable, keeps its lock and write columns apart in the same way.

A write changes a fact by setting a claim on its pointer, on condition the pointer is unchanged since the write read it. A namespace write claims its namespace's latest item the same way. The state a claim describes is current only once its write commits. Until then, the committed state stands.

When a write reads a pointer or latest item that carries a claim, it looks up the claim's write:

- **Committed:** the claim's state is current. The new write moves it into the committed state as it sets its own claim.
- **Aborted:** the committed state stands. The new write replaces the claim.
- **Open, lease expired:** the new write sets that write's status to aborted, on condition it is still open, and goes on as for aborted.
- **Open, lease live:** the new write gives up at once and reports a conflict. Writes never wait on each other.

A write that gives up sets its own status to aborted, and the sweep clears its claims. The caller, or the writer's bounded retry, tries again later with backoff, under a new write id. A retry reads afresh: it rereads every fact and latest item before it claims, since a claim it met may since have committed and changed what the write should do. Today's writes already work this way: they lose a race and retry.

A claim is cleared or replaced only once its write is committed or aborted. The commit requires the write to be still open, so a write that commits still holds every claim it set.

### Storing the items

A write opens with one conditional write: its status record, set to open, stored only if no record with that write id exists. It then stores its items in groups. Nothing it stores is visible until its status changes to committed, so the groups don't need to land together.

Items that only make sense together are one group, stored in one small transaction. A fact's group is its pointer claim, conditional on the pointer still holding what the writer read, the span it closes, the span it opens, and its index item by subject. Either the whole group is stored or none of it is. So if a writer dies partway, every fact it touched is fully stored or not touched, the sweep never finds a claimed pointer without its spans or a span without its index item, and a write never holds a claim on a fact it didn't finish storing. A namespace's latest item and the entry it copies are a group in the same way.

An item that stands alone is an ordinary plain write.

The write id makes a write atomic for readers. The small transactions keep each fact's own items consistent in storage.

### The commit

Apart from the opening, the commit is the write's only transaction. It holds:

1. The head, moved to a new revision R, on condition it still holds the revision the writer read.
2. The write's status record, set to committed at R, on condition it is still open.
3. A check for each namespace latest item the write judged facts against, that the item is unchanged and carries no open claim.

The checks keep today's guard. Without them, a fact could land under a relation that was dropped and declared again while the write was storing its items.

The write's items were stored before, so the commit stays this small however many facts the write carries.

R comes from the revision-clock at commit time, and the writer waits out commit-wait before acknowledging. The revision is chosen at commit, not at the start. So if the head moves while a write is storing its items, nothing is lost: only the commit retries, with a new R. The items stay where they are. Before each try, the write is authorized again at the new head, as [Sungma's own authorization](../specs/sungma-authorization.md#authorizing-a-write) requires, and its namespace checks are made again. A role retired by a write that committed in between can't carry it through.

The item limit now applies only to the commit: the head, the status record, and about 98 namespaces judged. It no longer limits how many facts a write carries.

A write with few items may instead store everything in its commit: its items stamped with R, its status record committed at R, its checks and the head move, in one transaction. That is today's write with a status record added. It pays the transactional price for every item, but saves a round trip and spares readers a lookup, so it suits writes of a few items.

### Reads

A read at revision Q meets items stamped with write ids, and looks each write id up:

- Committed at some R at or before Q: the item counts as stamped with R.
- Committed after Q, open, or aborted: the item counts as absent. A retirement stamped by such a write counts as not retired.

Treating an open write as absent is safe. When the write commits, its R is above the head at that moment. A reader never reads above the head it saw, and the head only moves forward, so the write can't later commit at or before Q.

A read looks each write id up once, with a consistent read, unless it already holds the write's final state. It uses that one answer for the whole read. Otherwise a read paging through a partition could see a write as open on one page and committed on the next, and so see half of it. Committed and aborted are final, so a reader can cache both forever. It never caches open. Many write ids can be looked up in one batch read.

Reads of old data meet only write ids already cached, or items the sweep has rewritten, so mostly only reads near the head pay a lookup. After the sweep rewrites an item, the status record stays: a read that saw the write id a moment before must still be able to look it up.

### Keys

A span's sort key holds the revision that wrote it, and a namespace entry's sort key holds the revision it opens at. Some reads rely on that order, such as "the newest entry at or before R". Write ids come in start order, not commit order, so a write id in a key would break that order.

A DynamoDB key can't change. Rewriting an item under a new key means a put and a delete, which aren't atomic with a range read, so a query running at that moment can see a fact twice or miss it.

So either keys keep a revision, with the write id as an attribute and the item stored under a placeholder until commit, or reads stop relying on key order. That choice is open.

### The sweep's restores

To undo an aborted write, the sweep clears its claims and stamps, each on condition it still holds the aborted write's id. A later write may have met the aborted claim and taken the fact over. An unconditional restore would wipe that later write out, losing a committed write or bringing back a retired fact.

Because a pointer keeps its committed state apart from its claim, clearing the claim is the whole restore. The sweep never has to remember a previous value.

### Retries

A request can time out after its write committed, with only the reply lost. The retry carries the same idempotency key, so it finds the earlier attempts: status records for one caller and key share a partition, keyed by attempt number, and one consistent read returns them all. The retry looks at the latest attempt:

- **Committed:** the retry returns that write's revision without writing again.
- **Aborted:** the retry opens the next attempt.
- **Open, lease live:** the retry doesn't wait. It reports that the request is still in progress, and the caller checks again later.
- **Open, lease expired:** the retry aborts that attempt, on condition it's still open, and opens the next one.

No separate item maps a key to its write, so nothing has to be kept in step with the status records.

### Order

- A namespace write names the version it was based on ([Namespace writes](../specs/namespace-writes.md#concurrent-writes)). Of two saves based on one version, the second to commit is refused, so two saves never land out of order unnoticed.
- Fact writes have an order problem. They carry no base version, so two writes to the same fact can commit in either order unless the caller waits for each acknowledgment before sending the next. That is true today as well. Giving fact writes a base version, as namespace writes have, is the likely fix. It's a separate problem from write ids, to be settled on its own.

### The writer

[Table](../specs/table.md#writes) says a writer handles one write at a time, so its revisions follow the order its writes land in. A write with many items takes much longer than one with few. Whether it ties up its writer for the whole time, or a writer can run it alongside short writes, is open.

## A worked example

Alice adds facts x and y in one request, with idempotency key `q1`. Her first attempt is write `alice/q1/1`, written below as write 1, and stores its items before its commit.

1. The writer stores status 1 as open, with a lease, on condition no status 1 exists. It appends x's partition to status 1, then stores x's new span stamped with write id 1 and sets a claim by write 1 on x's pointer.
2. The writer's process dies before it stores y. Write 1 is open, with half its items stored, and nothing renews its lease. A reader that meets x's span looks up write 1, finds it open, and treats x as absent. Nothing is visible.
3. Alice's request times out, and she retries under `q1`. The retry reads the attempts for Alice's `q1`, finds write 1 open with its lease still live, and reports that the request is still in progress.
4. Meanwhile other writes commit and move the head on.
5. Alice retries again. Now write 1's lease has expired, by the retry's clock with its uncertainty allowed for. The retry sets status 1 to aborted, on condition it is still open. It opens the next attempt, `alice/q1/2`, written below as write 5, storing status 5 as open on condition no status 5 exists.
6. Write 5 reads x's pointer and finds write 1's claim. Write 1 is aborted, so the pointer's committed state stands, and write 5 replaces the claim with its own. It appends x's and y's partitions to status 5, and stores x's span and y's span and pointer claim, all stamped with write id 5.
7. It commits: it reads the head, takes the next revision R from the clock, and in one transaction moves the head to R, sets status 5 to committed at R, and checks the namespace latest item it judged x and y against. It waits out commit-wait and acknowledges.
8. Readers at R or later see both facts, from R on. Readers before R see neither. No reader ever saw write 1's half.
9. Later, the sweep undoes write 1. It deletes the span write 1 stored. It would clear write 1's claim on x's pointer, but that claim now holds write 5, so the condition fails and the sweep leaves the pointer alone. It rewrites write 5's items with R.

If write 1 had committed and only its reply was lost, step 3 would have found write 1 committed and returned its revision without writing again.

## Two writes on one fact

Writes A and B both change fact x.

1. A claims x's pointer and goes on storing its other items.
2. B reads x's pointer, finds A's claim, and finds A open with a live lease. B gives up at once, sets its own status to aborted, and reports a conflict.
3. A commits at R.
4. B's caller retries after a backoff, under a new write id. The retry reads x afresh. It finds A's claim, and A committed, so A's change to x is current. The retry works out its own change from that state, moves A's state into the committed state, sets its own claim, and commits at a later revision.

## Hazards

Each hazard below is handled in the design above.

1. **Epochs.** A version that adds or brings back a relation takes its own revision as that relation's epoch, which isn't known until commit. The epoch is stamped with the write id and resolved on read ([What gets stamped](#what-gets-stamped)).
2. **Dropped and redeclared relations.** A fact write that stores its items before its commit could land a fact under a relation that was dropped and declared again meanwhile. The commit checks each namespace latest item the write judged against ([The commit](#the-commit)).
3. **Restoring after an abort.** Restoring a pointer would need its previous value. The pointer and latest item keep the claim apart from the committed state, so a restore only clears the claim ([Claims](#claims)).
4. **Unconditional restores.** Clearing a claim that a later write has taken over would lose that write, or bring back a retired fact. The sweep clears a claim only if it still holds the aborted write's id ([The sweep's restores](#the-sweeps-restores)).
5. **A slow write renewing after abort.** An aborted write could renew its lease and then commit. Renewal and commit both require the status to be still open ([Leases](#leases)).
6. **Finding an aborted write's items.** Without a list, the sweep would have to scan the table. The status record lists the partitions the write touched ([The status record](#the-status-record)).
7. **Half a write within one read.** A read paging through a partition could see a write open on one page and committed on the next. A read looks each write id up once and uses that answer throughout ([Reads](#reads)).
8. **Duplicate retries.** A retry that looked only for committed writes would start a second write beside one still open. A retry reads every attempt for its caller and idempotency key, consistently, and acts on the latest in any state ([Retries](#retries)).
9. **Stale reads before claiming.** A claim a write gave up on may since have committed. A retry rereads every fact before it claims ([Claims](#claims)).
10. **Write ids in keys.** Write ids come in start order, not commit order, and reads rely on revision order in keys. Keys keep a revision, or reads stop relying on key order, and rewriting a key isn't atomic with a range read ([Keys](#keys)).
11. **Lookups for every write.** A write that stores its items in its commit stamps its revision directly, so readers look nothing up for it ([What gets stamped](#what-gets-stamped)).
12. **A writer tied up.** A writer handles one write at a time, and a write with many items takes long. Whether it blocks its writer is open ([The writer](#the-writer)).

## Its own crate

The protocol knows nothing about facts or namespaces. Write ids, status records, leases, claims, the commit, the sweep and idempotency keys make writes of any size atomic on a store that offers single items and small transactions. So the protocol is a crate of its own, generic over a small key-value trait: get, put, conditional put, a range read, and a small transaction of conditional puts. Each store implements that trait. Sungma's table is its first user.

Every store runs the same protocol, including stores that have transactions of any size, such as SQLite. One mechanism means one code path, one test suite run against every store, and the same behavior wherever Sungma runs.

What a read does with a write id stays in Sungma's read path. The crate resolves a write id to committed at a revision, open or aborted. When a stamped span counts is the rule in [One timeline](../specs/one-timeline.md).

## What else changes if it's adopted

It changes every write and every read, so it is all or nothing: reads must handle write ids wherever they appear, and every write must respect other writes' claims.

- [Table](../specs/table.md): the status record and the index item by subject as new item kinds, keyed by caller, idempotency key and attempt, write ids in place of revisions, claims on pointers and latest items, the key question above, and the unit of work for each kind of write.
- [One timeline](../specs/one-timeline.md): how a read decides what counts, as in [Reads](#reads).
- [Namespace writes](../specs/namespace-writes.md): a namespace write may store more than one unit of work holds, and may store facts.
- The API: every write request carries an `Idempotency-Key` header.

## What it costs

- A small transaction for each fact's group of items, at DynamoDB's transactional price, which is twice a plain write's. Items that stand alone are plain writes at half today's price. A group is a handful of items, far under the 100-item limit, so a write of any size still commits.
- A status record per write, kept for good, or until no read can still need it.
- The list of touched partitions on each status record.
- A lookup for reads that meet uncommitted or recently committed write ids, mostly cached.
- A sweep to run.
- Writes that meet an open claim give up and retry. Two writes with many items that keep touching the same facts can keep making each other give up. Backoff and bounded retries limit that.
- A dead writer's claims block their facts until its lease expires.

## Other answers

- **A head lease.** A write reserves the head, stores every item at the next revision R in as many pieces as it needs, and moves the head to R last. Reads stop at the head, so nothing at R shows early. Other writes wait the whole time, and the head must never move past R while R's items remain: a dead writer's items have to be undone, or R marked aborted, first. Simpler for readers, but every such write stops every other write.
- **A cap on items per write.** Large changes are split into several versions by the caller. Simple, but an odd limit for an author to run into.
- **Writing the extra items after the commit.** The namespace commits first, and its `parent` facts follow in later transactions. Readers then see the namespace for a while without some of its facts. That is safe only where a missing fact can only deny access, never grant it, which is what "fails closed" means. Under an exclusion, a missing fact on the excluded side widens access. So this works only for facts that never sit on the excluded side of an exclusion.

## Prior art

- Iceberg and Delta Lake commit a table version by flipping one pointer to it.
- Postgres marks each row with the transaction that wrote it, and keeps every transaction's status in a commit log.
- Google's Percolator builds multi-row transactions on Bigtable, which has none, by committing through one primary record. It keeps each row's lock apart from its committed writes.

## Open

1. **Key layout.** Whether keys keep a revision, with the write id as an attribute and a placeholder key until commit, or reads stop relying on key order.
2. **Leases.** How long a lease is, how often an open write renews it, and the exact rule for judging expiry under clock uncertainty.
3. **Status records.** Whether they are kept forever or dropped after a grace period longer than any read.
4. **Stamp size.** A write id holds the caller's `sub`, an idempotency key and an attempt, which is long to stamp on every item. Whether items carry it whole, or carry a short id that the status record maps to it.
5. **The writer.** Whether a write with many items blocks its writer, or runs alongside short writes.
6. **The choice.** Whether this or a head lease is chosen.
7. **How fact writes are ordered.** Whether a fact write names the version of each fact it was based on, as a namespace write does.
