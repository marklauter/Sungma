---
title: The DynamoDB adapter
type: specification
summary: "Sungma keeps facts, namespace versions, interned names and the head in one key-value table on one timeline: each fact span, namespace entry and name is an item under a key that sorts in the order reads need, and each write is one unit of work that moves the head. On DynamoDB, a unit of work is a transaction."
status: evolving
cites:
  - "[[fact]]"
  - "[[span]]"
  - "[[namespace]]"
  - "[[epoch]]"
  - "[[revision]]"
  - "[[revision-clock]]"
  - "[[subjectset]]"
  - "[[subject]]"
  - "[[identity]]"
  - "[[resource]]"
  - "[[relation]]"
---

# The DynamoDB adapter

Sungma keeps everything in one key-value table: every [[fact]] [[span]], every [[namespace]] version, the interned names, and the head, all on one [[revision]] timeline, as [One timeline](one-timeline.md) specifies. In production the table is a DynamoDB table. Tests run it against DynamoDB Local. Local reads consistently whatever a read asks for, and runs one transaction at a time, so tests can't reach the paths for stale reads and transaction conflicts.

## The table and the contexts

The table knows nothing of what its items mean. It does four things:

- It gets one item by its key.
- It queries a run of one partition, by a prefix or a range of sort keys, either way, up to a limit.
- It commits a unit of work: steps that insert, update or check items, each on its conditions, all of them or none.
- It hands out numbers from a counter.

Each context lays out its own items on the table and maps them to its own shapes. The names context keeps the names. The write context writes facts and namespace versions, and Check and the API's reads read them back. One layout, shared by the contexts, says where each kind of item lives and what it holds, and each kind keeps its partitions under a tag of its own, so no two kinds meet.

A context is given a reader, a writer, or both, so code that only reads can't write. The interner mints through a name reader and a name writer. A context that never mints, such as Check, reads names through a name reader alone. A name never changes its id, so a cache belongs there.

A table in memory, a sorted map, passes the same tests as the DynamoDB table, for tests.

## The table

Each item has a partition key and a sort key. Within a partition, items are kept in sort key order, and a read takes a run of them by a prefix or a range. Every other value is an attribute of its own, and each kind of item has its own attributes.

Keys compare symbol by symbol, so each key is written so that it sorts in the order reads need:

- An id and a revision are written at a fixed width, most significant digit first, so they sort as numbers do.
- A resource id or an [[identity]] is followed by an end mark that sorts below every character either may hold, so a prefix read never picks up a longer id.
- Each key starts with a tag that names the kind of item.

| Item | Partition | Sort key | Holds |
| --- | --- | --- | --- |
| Fact span | the fact's subjectset | the subject, then the revision that wrote the span | when the span was written and retired |
| Fact pointer | the fact's subjectset | the subject | the revision that wrote the open span |
| Namespace latest item | the namespace | sorts first | a copy of the newest entry |
| Namespace entry | the namespace | the revision the entry opens at | a version or a retirement |
| Name pool counter | the pool | sorts first | the next id to lease |
| Name to id | the pool | the name | its id |
| Id to name | the pool | the id | its name |
| Head | the head | fixed | the latest write's revision |

Interned ids keep keys small. The longest key, a span of a [[subjectset]] subject with the longest resource id, fits within DynamoDB's key limit.

Every key reads back as its parts: its kind, ids, resource id or identity, and revision. A key Sungma didn't write, or an item without an attribute Sungma writes, is refused with a store error that names the key.

## Facts

Each span of a fact is one item. Its partition key is the fact's subjectset: the namespace id, the [[relation]] id, then the resource id. Its sort key is the [[subject]], then the revision that wrote the span. A subject's key sorts as the kernel sorts subjects: identities, then subjectsets, then [[resource]] members.

A span records the revision that wrote it, and the revision that retired it once it closes. A span is never removed.

Each fact has one more item, its pointer: the same partition key, and the subject alone as its sort key, so it sorts before the fact's spans. It records the revision that wrote the fact's open span, and records none when no span is open. Writes are conditioned on it, so two writers can't both open a span for one fact.

A read at revision R queries the subjectset's partition, and the reader keeps the spans written at or before R and not retired at or before R. A pointer records no written revision, so the reader drops it.

- Reading a subjectset's subjects reads the whole partition.
- Reading its subjectset subjects reads the sort keys that start with the subjectset tag.
- Reading when one fact was written reads the sort keys that start with its subject.

The reader keeps spans after the table reads them, so a read pays for every span in the partition, closed ones included, and its cost grows with the subjectset's history rather than its live facts.

## Namespace versions

A namespace's entries are items keyed by the revision the entry opens at:

- A version holds its revision, its namespace document, and the [[epoch]] of each relation it declares, keyed by relation id.
- A retirement holds its revision and is marked retired.

A version is stored as its document. A write prints the version with names in place of ids, through the printer in [Namespace documents](namespace-documents.md#printing), so every stored document has the printer's one form. A version whose namespace or relations have no interned name is refused. A read parses the document, maps its names to ids, and rebuilds the version from its stored epochs. The read refuses a version where a declared relation has no epoch, an undeclared relation has one, or an epoch is later than the version's revision.

Each namespace has one more item, its latest item, which sorts before its entries: a copy of its newest entry. A read of it can be stale; it isn't a promise that no newer write is on its way.

A read of a namespace at R reads the latest item, and uses it when it opens at or before R; a retirement there reads as no version. Otherwise a newer entry landed after R, and the read takes the newest entry that opens at or before R.

## Names

A name pool is a partition, one for namespace names and one for relation names. Each name has two items, one from the name to its id and one from the id to its name.

Inserting a name puts both items in one unit of work, each on condition it doesn't exist yet. When the unit of work meets another in flight, it tries again, a bounded number of times. When a condition fails, another writer stored the name first, and a consistent read returns its id.

Each pool has a counter. A lease advances the counter by the count asked for in one atomic update, on condition the result stays within the id range, and returns the ids it passed over. When the result would pass the end of the range, the lease takes what was left. A lease once the ids run out is empty.

## The head

The head is one item that holds the revision of the latest write, or zero before the first. Facts and namespaces share it, so the head a fact reader reports includes namespace writes.

Every fact write and every namespace write is one unit of work whose first step moves the head to the write's revision, on condition the head still holds the revision the writer read. A writer stamps above the head it read, so once a write commits, no later write lands at or before its revision. Every write moves the head, so concurrent writes contend for it.

## Writes

A writer stamps its writes from its [[revision-clock]] and waits out commit-wait before it acknowledges, as [The clock](clock.md) specifies. A writer handles one write at a time, so its revisions follow the order its writes land in. A writer takes any clock. A development writer reads the head and counts revisions on from it, so a restarted writer doesn't reuse a revision already committed.

A write reads what it depends on consistently. A write that loses a race is tried again, a bounded number of times, and sees what beat it. When the writer's clock is at or below the head it read, it moves its clock past the head and tries again. When the head's condition refused the write, the writer reads the head and moves its clock past it before the next try, so each race it loses costs it one try.

A write whose only refused steps are spans that its facts' pointers vouched for didn't lose a race: trying again would read the same pointers. It fails with a store error that names those spans.

### Fact writes

A fact write:

1. Reads the latest item of each namespace its inserts name, in a fact's subjectset or in a subjectset subject, and judges the inserts against those versions, as [One timeline](one-timeline.md#fact-writes) specifies.
2. Reads each fact's pointer, and folds the fact's writes in the batch into the one change they make when applied in order. An insert opens a span by the rule in [One timeline](one-timeline.md#epochs-and-orphaned-facts). A span opened and closed within the batch covers no revision, so it isn't stored.
3. Builds one unit of work: the head, a check that each latest item it read is unchanged, and for each fact its pointer, the span it closes, and the span it opens. A fact whose writes change nothing gets a check that its pointer is unchanged instead.

The unit of work holds the head, one item per namespace judged, and up to three items per fact. A batch that needs more items than one unit of work holds is refused as too large before the writer takes a reading, so the caller can split it. A unit of work holds 100 items, as many as DynamoDB allows in one transaction.

### Namespace writes

A namespace write reads the latest item and compares the latest version's revision with the version the write is based on, as [Namespace writes](namespace-writes.md#concurrent-writes) specifies. A create is based on no version. A retirement of a namespace with no version is refused. When the comparison fails, the write is refused as a conflict naming the latest version.

The unit of work holds three items: the head, the new entry on condition it doesn't exist, and the latest item, overwritten on condition it still holds the entry the writer read, or doesn't exist when the writer read none. Of two writes based on one version, one is refused.

## Consistency

Reads outside a write are eventually consistent. A read is a snapshot, stale once it's taken, so a read that lags a little is no worse. A write reads consistently, and its conditions are checked against the stored items, so a write that raced another is refused and tried again, never applied wrongly.

Names are stored before the versions that use them. A version read that misses a name reads it again consistently.

## Required tests

1. **The shared suites.** The DynamoDB table passes the tests every table passes, as the memory table does. The contexts over each table pass the same store tests and the same writer tests.
2. **Keys.** Subject keys sort as subjects do, a span's key reads back as its subject and revision, a fact's spans sort by revision after its pointer, and a key Sungma didn't write is refused.
3. **Items.** An item Sungma didn't write is refused on read, and a namespace write whose namespace or relations have no interned name is refused.
4. **Races.** Of several writers writing one namespace version, exactly one succeeds. An insert that races a relation's drop lands before it or not at all.
5. **The head.** A writer behind the head catches up in one try, and a writer that loses race after race spends one try per race.
6. **Limits.** A write past one unit of work is refused before it is sent, and a query that runs past one page of DynamoDB's reads on and keeps its limit.
7. **Names.** A name is stored once and read both ways, a lease near the end of the ids takes what is left, and the interner mints through the table.
8. **Mutation testing.** No mutant survives the tests of its own crate, apart from recorded exclusions, each with its reason.

Each test has a table of its own, so tests don't share state. Without DynamoDB Local the tests fail rather than skip.
