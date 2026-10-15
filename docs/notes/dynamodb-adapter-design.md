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

1. The crate is `sungma-dynamodb`. Its store, `DynamoStore<C: Clock>`, implements all six ports: `FactStore`, `FactWriter`, `NamespaceStore`, `NamespaceWriter`, `Dictionary` and `NameStore`.
2. It uses the official `aws-sdk-dynamodb` and `aws-config` crates, on tokio.

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
   2. `sk` is the subject, then the revision that wrote the span. An identity is tag `1` and the identity. A subjectset is tag `2`, the namespace id, the relation id and the resource id. A resource member is tag `3`, the namespace id and the resource id.
   3. `written` is the revision that wrote the span. `retired` is the revision that retired it, absent while the span is open. A span is never removed.
2. A read at revision R filters in the store: `written <= R` and either no `retired` or `retired > R`.
   1. `subjects` queries the subjectset's partition.
   2. `subjectsets` queries it for keys starting with tag `2`.
   3. `written` queries it for keys starting with the subject.
3. Each fact has one more item, its pointer: the same `pk`, and `sk` is the subject with no revision. Its `open` attribute is the revision that wrote the open span, absent when none is open. Writes are conditioned on it, so two nodes can't both open a span for one fact.

## 4. Namespace versions

1. `pk` is tag `2` and the namespace id. `sk` is tag `1` and the revision the entry opens at.
   1. A version has `revision`, a map from relation id to its rewrite, and a map from relation id to its epoch.
   2. A retirement has `revision` and `retired`, set to true.
2. Each namespace has one more item, `sk` = tag `0`, its current item: a copy of its latest entry, as `:latest` names the newest image of a container. A namespace write stores the new entry and overwrites the current item in one transaction, on condition the current item's `revision` is the write's `based_on`, which gives `based_on` its etag behaviour.
3. Reads:
   1. `version(namespace, R)` at the head is a point read of the current item.
   2. At an earlier R, it queries the entries at or before R, descending, limit 1.
   3. Point reads batch, so one `BatchGetItem` loads the current versions of up to 100 namespaces in one round trip. A query can't batch.
4. A `Version` read back from the store is built from its stored epochs, so `Version` needs a constructor for stored versions. It refuses a version where a relation the namespace declares has no epoch, or an epoch is later than the version's revision. `History` still orders versions on the write path.
5. Open question: how to store a rewrite. DynamoDB nests maps and lists only 32 levels deep, and the rewrite grammar has no such limit. Either each rewrite is a binary attribute in a compact id form, or rewrites are capped at a depth that fits and stored as native maps.

## 5. Names

1. `pk` is tag `3` and the pool: `0` for namespaces, `1` for relations.
2. Each name has two items: `sk` = tag `1` and the name, with an `id` attribute, and `sk` = tag `2` and the id, with a `name` attribute.
3. `insert_if_absent` writes both in one transaction, on condition the name item doesn't exist. When the condition fails, it reads the stored id.
4. The pool's counter is the item `sk` = tag `0`. `lease` is one atomic update that adds the count to its `next` attribute, on condition the result stays within `u32`, and returns the new value. When it would overflow, the lease takes what's left, and a lease once the ids run out is empty.

## 6. The head and one timeline

1. One item, `pk` = tag `4` and `sk` = tag `0`, with a `revision` attribute.
2. Every fact write and every namespace write is a single transaction, which also updates the head on condition its `revision` is below the new one. A write stamped below a revision already committed is refused and tried again, so a read at R sees every write at or before R, and unrelated writes don't queue behind each other.
3. A fact write also checks the `current` item of each namespace it touches, so `check_inserts` judged the inserts against versions that were really current.
4. Facts and namespaces share the table and the head, so `FactStore::head` includes namespace writes.
5. The clock is generic. Development uses `DevClock::starting_after(head)`, read when the store opens. A write takes its reading under an async lock, as `MemoryStore` does under its mutex.
6. A transaction holds at most 100 items. A fact takes two, its span and its pointer, plus the head and one check per namespace, so a batch holds about 45 facts. The store refuses a larger batch with a `StoreError`.

## 7. Rules

1. Inserts go through `sungma_write::fact::check_inserts` and `opens_span`, with what the `current` items and the pointers give. The adapter doesn't restate either rule.

## 8. Tests

1. The store tests in `sungma-memory/tests` move to a shared suite, a `testkit` module behind a feature in `sungma-write`, generic over the ports. Both stores run it.
2. The DynamoDB tests read `SUNGMA_DYNAMODB_URL`, `http://localhost:8000` by default, and give each test its own table, so they run in parallel.
3. With no endpoint they fail, with a message saying to run `scripts/dynamodb.sh`. They don't skip, because coverage has to count them.

## 9. Container

1. `scripts/dynamodb.sh` starts DynamoDB Local unless it's already running: in memory, shared database, port 8000, with the image pinned by digest.
2. The pre-commit hook calls the script.
3. CI runs the same digest as a service container on the Linux job.
