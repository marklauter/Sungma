---
title: Read and write API
type: note
summary: "A proposal for adding fact reads, fact writes, and namespace reads and writes to the API, beside POST /check, on either store."
status: proposal
cites:
  - "[[fact]]"
  - "[[namespace]]"
  - "[[revision]]"
  - "[[revision-token]]"
  - "[[subjectset]]"
---

# Read and write API

This covers roadmap items 7 and 8, the Read and Write services, and the namespace write service in item 11. They join `POST /check` in `sungma-api`, as JSON over HTTP. gRPC comes with item 12.

## 1. Routes

1. Every route is a `POST` taking and giving JSON, like `/check`, and every reply carries `request_id`. Fact and namespace routes are prefixed by what they act on; `/check` acts on no one kind, so it keeps its path.
2. `POST /facts/write` writes facts. The body is the write request in [Fact documents](../specs/fact-documents.md#writes): `add` and `retire`, each a fact document. The reply gives the write's zookie.
3. `POST /facts/read` reads facts at one snapshot. The body names a subjectset with the keys a fact uses, `namespace`, `resource` and `relation`, and an optional `zookie`. With a `subject` too, it's a point read of that one fact. The reply gives the facts it found as a fact document, and the zookie it read at.
4. `POST /namespaces/write` writes a namespace. The body is one of:
   1. `{ "create": <document> }`;
   2. `{ "new_version": <document>, "based_on": <zookie> }`;
   3. `{ "retire": "<namespace>", "based_on": <zookie> }`.

   The reply gives the write's zookie. A conflict is refused with the zookie of the latest version, or `null` when there is none, so the caller can read it and try again.
5. `POST /namespaces/read` reads a namespace. The body is `{ "namespace": .., "zookie": .. }`, the zookie optional. The reply gives the version's document, the zookie of the version, which a later write passes as `based_on`, and the zookie it read at.
6. A read without a zookie reads at the head. A read with one reads at the head too, and is refused if the zookie is ahead of it, as a check is.

## 2. Names

1. Fact reads and writes never intern a name. A write naming a namespace or relation never interned is refused, as the spec says. A read naming one finds no facts, since no fact can mention it.
2. A namespace write interns its names through `LeasingInterner`, over the store's `NameReader` and `NameWriter`, before it writes the version. A write refused after that, by a conflict, leaves its names interned. That's harmless: an interned name changes nothing a read sees, and the next write of that namespace finds them.
3. `sungma-lang`'s loaders take `MemoryDictionary` and `MemoryStore` today. They split into parsing, which gives named values and touches no store, and resolving names through a `Dictionary` or an `Interner`. The fixture loaders stay, built on the same pieces.

## 3. Services

1. Like `CheckService`, each route's work lives in a service the API calls, generic over the ports it needs:
   1. `FactWriteService` and `NamespaceWriteService` in `sungma-write`, over an `Interner` or a `Dictionary` and the writers.
   2. `FactReadService` and `NamespaceReadService` in `sungma-check`, over a `Dictionary` and the readers. `sungma-check` is the read side; a rename can wait.
2. Writes aren't audited in this step. The audit log takes check decisions, and the Audit service will follow Watch's events.

## 4. The store

1. `AppState` becomes generic over its readers and writers. `main` runs on DynamoDB when `SUNGMA_DYNAMODB_URL` is set, with the table from `SUNGMA_DYNAMODB_TABLE`, and in memory otherwise.
2. The fixture arguments become optional. When given, they're loaded through the write services, so they seed either store.
3. Writes stamp revisions from `DevClock` for now. Wiring the NTS clock into the service is separate work.
4. A write's reply waits until the writer settles, so the zookie it gives is already safe to read at.

## 5. Errors

1. Errors stay `{ "request_id": .., "error": .. }`. RFC 9457 stays open.
2. Status codes:
   1. 400: a body that isn't well formed, or a name that breaks the grammar.
   2. 422: a refused write, such as an unknown name, a relation no latest version declares, or a namespace document that fails its checks. A refused namespace document lists every problem, as the parser gives them.
   3. 409: a namespace write whose `based_on` isn't the latest version.
   4. 502: the store or the clock failed. Both are services Sungma calls: DynamoDB, and the time servers behind the clock.
   5. 500: a fault in Sungma itself. Few paths lead there.
3. `/check` follows the same codes. A check whose store read fails gives 502 rather than 500.
4. These codes are mapped by hand for now. Roadmap item 14 gives every domain error a kind and a stable code, so the mapping becomes one table.

## 6. Spec drift

[Namespace writes](../specs/namespace-writes.md#storage) said a version is stored parsed and interned, while the DynamoDB adapter stores the printed document. The spec now allows both forms: what it guards against is storing the caller's text.

## 7. Not in this step

1. Reading by resource, every relation of one resource. The table keys facts by subjectset, so this needs an index.
2. Paging. A read gives every fact under the subjectset at once.
3. gRPC, audit of writes and opaque zookies. Callers aren't authenticated, and nothing here needs it.

## Appendix: left open

Found in review, after the API was built.

1. Fact writes and namespace retirements look names up with eventually consistent reads. On DynamoDB, a fact write made right after its namespace is created can miss the new names and be refused as naming something never declared. A fix reads a missed name again consistently, as a namespace read already does, which needs the writer to offer that read. DynamoDB Local reads consistently, so no test can show it.
2. A fact read looks up the names of each subject it finds one at a time, so a large subjectset repeats the same lookups. A lookup cache per read would cut them.
3. The fixture loader in `sungma-lang` resolves names its own way, beside `sungma::resolve`, because it doesn't await.
4. When a fact is in both `add` and `retire`, the retirement applies first, so the fact is held after the write. The fact documents spec doesn't say which comes first.
