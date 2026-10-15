---
title: DynamoDB adapter design
type: note
summary: "A proposal for roadmap item 4: one DynamoDB table holding facts, namespace versions, names and the head on one timeline, with DynamoDB Local for development."
status: proposal
cites:
  - "[[fact]]"
  - "[[span]]"
  - "[[namespace]]"
  - "[[epoch]]"
  - "[[revision]]"
  - "[[revision-clock]]"
---

# DynamoDB adapter design

This is roadmap item 4. Development runs against DynamoDB Local in a container, but nothing here is specific to Local, so the same adapter runs against DynamoDB.

## 1. Crate and SDK

1. The crate is `sungma-dynamodb`. Readers and writers are separate types, so code that only reads is given only a reader. Both share one DynamoDB client.
   1. `DynamoReader` implements `FactReader`, `NamespaceReader` and `NameReader`. It takes no clock.
   2. `DynamoWriter<C: Clock>` implements `FactWriter`, `NamespaceWriter` and `NameWriter`. The reads it makes inside a write, of pointers and latest items, are its own, not the read ports.
2. The store ports become three pairs of reader and writer, and no writer extends a reader:
   1. `FactStore` becomes `FactReader`, and `FactWriter` no longer requires it.
   2. `NamespaceStore` becomes `NamespaceReader`, and `NamespaceWriter` no longer requires it.
   3. `NameReader` is new and has the two point reads, a name's id and an id's name. `NameStore` becomes `NameWriter`, with `insert_if_absent` and `lease`, and no longer requires `Dictionary`.
   4. `Dictionary` and `Interner` stay above the store. `LeasingInterner` is given a `NameReader` and a `NameWriter` and implements both. Check, which never mints, gets a `Dictionary` over a `NameReader` alone, the natural place for a cache, since a name never changes its id.
   5. `MemoryStore` and `MemoryDictionary` still implement both sides, for tests.
3. It uses the official `aws-sdk-dynamodb` and `aws-config` crates, on tokio.

## 2. The table

1. One table. Only the hash key `pk` and the range key `sk` are declared, both binary. Every other value is a native attribute of its own, and each kind of item has its own attributes.
2. Binary keys sort byte by byte, so keys are built from bytes that sort in the order reads need:
   1. A namespace, relation or name id is 4 bytes, big-endian.
   2. A revision is 8 bytes, big-endian.
   3. A resource id or identity is its bytes followed by a zero byte. Both are printable ASCII, `!` to `~`, so a zero byte never appears in one, and it sorts below all of them, so a prefix read never picks up a longer id.
   4. Each key starts with a tag byte that says what kind of item it is.
3. Interned ids keep keys small, so more spans fit in each read and each page a cache holds.
4. The longest key is a span of a subjectset subject: 530 bytes with the longest resource id, under DynamoDB's 1024-byte limit.
5. Nobody reads the table directly; DynamoDB encrypts it at rest. For tests and logs, one module builds and parses every key, and prints a key as text, such as `fact file:readme#viewer identity alice @42`.

## 3. Facts

1. Each span of a fact is one item.
   1. `pk` is the subjectset: tag `1`, the namespace id, the relation id, then the resource id.
   2. `sk` is the subject, then the revision that wrote the span. An identity is tag `1` and the identity. A subjectset is tag `2`, the namespace id, the resource id and the relation id, so subjects sort as the kernel sorts them. A resource member is tag `3`, the namespace id and the resource id.
   3. `written` is the revision that wrote the span. `retired` is the revision that retired it, absent while the span is open. A span is never removed.
2. A read at revision R filters in the store: `written <= R` and either no `retired` or `retired > R`. DynamoDB filters after it reads, so a read pays for every span in the range, closed ones included, and its cost grows with the subjectset's history rather than its live facts. That suits a cache that holds whole partitions. If it becomes a problem, open spans can move to a range of their own.
   1. `subjects` queries the subjectset's partition.
   2. `subjectsets` queries it for keys starting with tag `2`.
   3. `written` queries it for keys starting with the subject.
3. Each fact has one more item, its pointer: the same `pk`, and `sk` is the subject with no revision. Its `open` attribute is the revision that wrote the open span, absent when none is open. Writes are conditioned on it, so two nodes can't both open a span for one fact.

## 4. Namespace versions

1. `pk` is tag `2` and the namespace id. `sk` is tag `1` and the revision the entry opens at.
   1. A version has `revision`, `document`, the namespace document as text, and `epochs`, a map from relation id to its epoch.
   2. A retirement has `revision` and `retired`, set to true.
2. Each namespace has one more item, `sk` = tag `0`, its latest item: a copy of its latest entry, as `:latest` names the newest image of a container. It is the latest the reader has seen, not a promise that no newer write is on its way. A namespace write stores the new entry and overwrites the latest item in one transaction, on condition the latest item's `revision` is the write's `based_on`, which gives `based_on` its etag behaviour. A create has no `based_on`, so it is conditioned on the namespace having no latest item yet; of two creates, one is refused.
3. Reads:
   1. `version(namespace, R)` point-reads the latest item and uses it when its `revision` is at or before R.
   2. Otherwise a newer version landed after R, or R is in the past, and it queries the entries at or before R, descending, limit 1.
   3. Point reads batch, so one `BatchGetItem` loads the latest versions of up to 100 namespaces in one round trip. A query can't batch.
4. A `Version` read back from the store is built from its stored epochs, so `Version` needs a constructor for stored versions. It refuses a version where a relation the namespace declares has no epoch, or an epoch is later than the version's revision. `History` still orders versions on the write path.
5. A namespace is stored as its document, as SQLite keeps each table's `CREATE` statement. A write prints the version it's given with names in place of ids, so every stored document has the one form the printer gives, as a post office rewrites an address in its standard format, and stores it minified. Epochs stay native attributes. A read parses the document and maps its names to ids. Rewrites aren't modeled as items or attributes.

## 5. Names

1. `pk` is tag `3` and the pool: `0` for namespaces, `1` for relations.
2. Each name has two items: `sk` = tag `1` and the name, with an `id` attribute, and `sk` = tag `2` and the id, with a `name` attribute.
3. `insert_if_absent` writes both in one transaction, on condition the name item doesn't exist. When the condition fails, it reads the stored id.
4. The pool's counter is the item `sk` = tag `0`. `lease` is one atomic update that adds the count to its `next` attribute, on condition the result stays within `u32`, and returns the new value. When it would overflow, the lease takes what's left, and a lease once the ids run out is empty.

## 6. The head and one timeline

1. One item, `pk` = tag `4` and `sk` = tag `0`, with a `revision` attribute.
2. Every fact write and every namespace write is a single transaction, which also updates the head on condition its `revision` is below the new one. A write stamped below a revision already committed is refused and tried again, so a read at R sees every write at or before R. Every write updates this one item, so concurrent writes contend for it, and DynamoDB refuses one of two that land at once. That is accepted until it shows up as a performance problem.
3. A fact write also checks the latest item of each namespace it touches, so `check_inserts` judged the inserts against the versions that were latest when the write landed.
4. Facts and namespaces share the table and the head, so `FactReader::head` includes namespace writes.
5. Reads are eventually consistent. Every read is a snapshot, stale once it's taken, so a read that lags a little is no worse. Reads inside a write are guarded too: the write's conditions are checked against the stored items, so a stale read before a write gets the write refused, not applied wrongly.
6. The writer is generic over its clock, `DynamoWriter<C: Clock>`, so the same code runs with any clock. Development uses `DevClock::starting_after(head)`, read when the writer opens. A write takes its reading under an async lock, as `MemoryStore` does under its mutex.
7. When the head condition refuses a write, the writer reads the head and moves its clock past it before it takes a new reading, so a writer that fell behind catches up in one try, not one revision at a time.
8. A transaction holds at most 100 items. A fact takes up to three: its new span, its pointer, and the span it closes when an insert reopens a fact written before its relation's epoch. With the head and one check per namespace, a batch holds about 32 facts. The store refuses a larger batch with a `StoreError` before it sends it.
9. A transaction can't touch one item twice, and a batch may write the same fact more than once, such as an insert and a retirement. The writer folds each fact's writes into the one change they make when applied in order, as `MemoryStore` applies them, and the shared tests cover such batches.

## 7. Rules

1. Inserts go through `sungma_write::fact::check_inserts` and `opens_span`, with what the latest items and the pointers give. The adapter doesn't restate either rule.

## 8. Tests

1. The store tests in `sungma-memory/tests` move to a shared suite, a `testkit` module behind a feature in `sungma-write`, generic over the ports. Both stores run it.
2. The DynamoDB tests read `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default, and give each test its own table, so they run in parallel.
3. With no endpoint they fail, with a message saying to run `scripts/dynamodb.sh`. They don't skip, because coverage has to count them.

## 9. Container

1. `scripts/dynamodb.sh` starts DynamoDB Local unless it's already running: in memory, shared database, port 8000, with the image pinned by digest.
2. The pre-commit hook calls the script.
3. CI runs the same digest as a service container on the Linux job.
