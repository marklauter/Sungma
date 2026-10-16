---
title: The DynamoDB adapter
type: specification
summary: "The DynamoDB adapter keeps facts, namespace versions, interned names and the head in one table on one timeline: each fact span, namespace entry and name is an item under a binary key, and each write is one transaction that moves the head."
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

The DynamoDB adapter implements Sungma's storage ports on one DynamoDB table. The table holds every [[fact]] [[span]], every [[namespace]] version, the interned names, and the head, all on one [[revision]] timeline, as [One timeline](one-timeline.md) specifies. Development and tests run it against DynamoDB Local. Nothing in the adapter is specific to Local.

## The crate

The crate is `sungma-dynamodb`, on the official `aws-sdk-dynamodb` crate and tokio. Readers and writers are separate types, so code that only reads is given only a reader.

- `Table` is a table name and the client that reaches it. Cloning a table shares the client. `Table::create` creates the table.
- `DynamoReader` implements the read ports in `sungma::store`: `FactReader`, `NamespaceReader` and `NameReader`. It takes no clock.
- `DynamoWriter<C: Clock>` implements the write ports in `sungma-write`: `FactWriter`, `NamespaceWriter` and `NameWriter`. The reads it makes inside a write are its own, not the read ports.
- `local(endpoint)` builds a client for DynamoDB Local at an endpoint such as `http://localhost:8000`.

No writer port extends a reader port. `LeasingInterner` takes a `NameReader` and a `NameWriter`, and a service that never mints, such as Check, reads names through a `ReadingDictionary` over a `NameReader` alone. A name never changes its id, so a cache belongs there. `MemoryStore` and `MemoryDictionary` implement both sides, for tests.

## The table

The table declares only its hash key `pk` and its range key `sk`, both binary, and is billed per request. Every other value is a native attribute, and each kind of item has its own attributes.

Binary keys sort byte by byte, so each key is built from bytes that sort in the order reads need:

- A namespace, relation or name id is 4 bytes, big-endian.
- A revision is 8 bytes, big-endian.
- A resource id or an [[identity]] is its bytes followed by a zero byte. Both are printable ASCII, `!` to `~`, so a zero byte never appears in one. It sorts below every printable character, so a prefix read never picks up a longer id.
- Each key starts with a tag byte that names the kind of item.

| Item | `pk` | `sk` | Attributes |
| --- | --- | --- | --- |
| Fact span | `1`, namespace id, relation id, resource id | subject, revision written | `written`, `retired` |
| Fact pointer | as a span | subject | `open` |
| Namespace latest item | `2`, namespace id | `0` | as its entry |
| Namespace entry | `2`, namespace id | `1`, revision | `revision`, and `document` and `epochs` or `retired` |
| Name pool counter | `3`, pool | `0` | `next` |
| Name to id | `3`, pool | `1`, name | `id` |
| Id to name | `3`, pool | `2`, id | `name` |
| Head | `4` | `0` | `revision` |

Interned ids keep keys small. The longest sort key is a span of a [[subjectset]] subject with the longest resource id, 530 bytes, under DynamoDB's limit of 1,024.

The `key` module builds and parses every key. `key::describe` prints a key as text for logs and test failures, such as `fact 0:readme#2 identity alice @42`, with bytes it can't read as hex. A key Sungma didn't write, or an item without an attribute Sungma writes, is refused with a `StoreError` that names the key.

## Facts

Each span of a fact is one item. Its `pk` is the fact's subjectset: the namespace id, the [[relation]] id, then the resource id. Its `sk` is the [[subject]], then the revision that wrote the span. A subject's key sorts as the kernel sorts subjects:

- An identity is tag `1` and the identity.
- A subjectset is tag `2`, the namespace id, the resource id and the relation id.
- A [[resource]] member is tag `3`, the namespace id and the resource id.

`written` is the revision that wrote the span. `retired` is the revision that retired it, and is absent while the span is open. A span is never removed.

Each fact has one more item, its pointer: the same `pk`, and the subject alone as its `sk`, so it sorts before the fact's spans. Its `open` attribute is the revision that wrote the open span, and is absent when no span is open. Writes are conditioned on it, so two writers can't both open a span for one fact.

A read at revision R queries the subjectset's partition and filters in the store for `written <= R` and either no `retired` or `retired > R`. A pointer has no `written`, so the filter drops it.

- `subjects` reads the whole partition.
- `subjectsets` reads the sort keys that start with tag `2`.
- `written` reads the sort keys that start with the subject.

DynamoDB filters after it reads, so a read pays for every span in the partition, closed ones included, and its cost grows with the subjectset's history rather than its live facts.

## Namespace versions

A namespace's entries are items keyed by tag `1` and the revision the entry opens at:

- A version has `revision`; `document`, the namespace document; and `epochs`, a map from relation id to its [[epoch]].
- A retirement has `revision`, and `retired` set to true.

A version is stored as its document. A write prints the version with names in place of ids, through the printer in [Namespace documents](namespace-documents.md#printing), and stores it minified, so every stored document has the printer's one form. A version whose namespace or relations have no interned name is refused. A read parses the document, maps its names to ids, and builds the version with `Version::stored` from its stored epochs. `Version::stored` refuses a version where a declared relation has no epoch, an undeclared relation has one, or an epoch is later than the version's revision.

Each namespace has one more item, its latest item, at tag `0`: a copy of its newest entry. It is the newest entry the reader has seen, not a promise that no newer write is on its way.

`version(namespace, R)` reads the latest item, and uses it when its `revision` is at or before R; a retirement there reads as no version. Otherwise a newer entry landed after R, and the read queries the entries at or before R, descending, limit 1.

## Names

A name pool is a partition: tag `3` and the pool, `0` for namespaces and `1` for relations. Each name has two items, one from the name to its id and one from the id to its name.

`insert_if_absent` puts both items in one transaction, each on condition it doesn't exist yet. When the transaction meets another in flight, it tries again, a bounded number of times. When a condition fails, another writer stored the name first, and a consistent read returns its id.

The pool's counter is its `next` attribute. `lease` adds the count to `next` in one atomic update, on condition the result fits in a `u32`, and returns the ids it passed over. When the result wouldn't fit, the lease moves `next` to the end and takes what was left. A lease once the ids run out is empty.

## The head

The head is one item whose `revision` is the revision of the latest write, or 0 before the first. Facts and namespaces share it, so `FactReader::head` includes namespace writes.

Every fact write and every namespace write is one transaction whose first item moves the head to the write's revision, on condition the head is below it. A write stamped below a revision already committed is refused and tried again, so no write lands at or below a revision already committed. Every write updates the head, so concurrent writes contend for it.

## Writes

A writer stamps its writes from its clock and waits out commit-wait before it acknowledges, as [The clock](clock.md) specifies. It takes each write's reading under an async lock, so on one node revisions follow the order writes land in. `DynamoWriter::new` takes any clock. `DynamoWriter::dev` reads the head and starts a `DevClock` after it, so a restarted writer doesn't reuse a revision already committed.

A write that loses a race is tried again, a bounded number of times, and each try after the first reads consistently, so it sees what beat it. When the head's condition refused the write, the writer reads the head and moves its clock past it before the next try, so a writer that fell behind catches up in one try.

### Fact writes

A fact write:

1. Reads the latest item of each namespace its inserts name, in a fact's subjectset or in a subjectset subject, and judges the inserts with `sungma_write::fact::check_inserts`.
2. Reads each fact's pointer, and folds the fact's writes in the batch into the one change they make when applied in order, using `opens_span` to decide whether an insert opens a span. A span opened and closed within the batch covers no revision, so it isn't stored.
3. Builds one transaction: the head, a check that each latest item it read is unchanged, and for each fact its pointer, the span it closes, and the span it opens. A fact whose writes change nothing gets a check that its pointer is unchanged instead.

The transaction holds the head, one item per namespace judged, and up to three items per fact. DynamoDB allows at most 100 items in a transaction, and the writer refuses a larger batch with a `StoreError` before it takes a reading.

### Namespace writes

A namespace write reads the latest item and compares the latest version's revision with the version the write is based on, as [Namespace writes](namespace-writes.md#concurrent-writes) specifies. A create is based on no version. A retirement of a namespace with no version is refused. When the comparison fails on an eventually consistent read, the write tries again with a consistent read. When it fails on a consistent read, the write is refused as a conflict naming the latest version.

The transaction holds three items: the head, the new entry on condition it doesn't exist, and the latest item, overwritten on condition it still holds the entry the writer read, or doesn't exist when the writer read none. Of two writes based on one version, one is refused.

## Consistency

Reads are eventually consistent. A read is a snapshot, stale once it's taken, so a read that lags a little is no worse. A write's conditions are checked against the stored items, so a stale read inside a write gets the write refused and tried again with consistent reads, never applied wrongly.

Names are stored before the versions that use them. A version read that misses a name reads it again consistently.

## DynamoDB Local

`scripts/dynamodb.sh` starts DynamoDB Local unless something already answers on port 8000: in memory, with a shared database, from an image pinned by digest. The pre-commit and pre-push hooks call it. CI runs the same image as a service container on its Linux jobs. Windows runners can't run the service, so they skip the crate's tests.

## Required tests

1. **The shared suite.** `sungma-write`'s `testkit` feature holds the tests every store runs, generic over the ports, and `store_tests!` runs them against `MemoryStore` and the adapter.
2. **Keys.** Subject keys sort as subjects do, a span's key reads back as its subject and revision, a fact's spans sort by revision after its pointer, and a key Sungma didn't write is refused.
3. **Items.** An item Sungma didn't write is refused on read, and a namespace write whose namespace or relations have no interned name is refused.
4. **Races.** Of several writers writing one namespace version, exactly one succeeds. An insert that races a relation's drop lands before it or not at all.
5. **The head.** A writer behind the head catches up in one try.
6. **Limits.** A write past one transaction is refused before it is sent.
7. **Names.** A name is stored once and read both ways, a lease near the end of the ids takes what is left, and a `LeasingInterner` mints through the adapter.
8. **Mutation testing.** `cargo mutants` leaves no surviving mutant in the adapter, apart from the exclusions in `.cargo/mutants.toml`, each with its reason.

The tests read `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default, and give each test a table of its own, so they run in parallel. Each run deletes tables that earlier runs left behind. With no endpoint they fail with a message that says to run `scripts/dynamodb.sh`; they don't skip, so coverage counts them.
