---
title: Namespace writes wait on namespace versioning
type: note
summary: Namespace versioning is settled by one timeline for facts and namespaces, and the write methods are settled, so a port for writing namespaces can now be designed.
status: evolving
---

# Namespace writes wait on namespace versioning

## Where it stands

- Namespaces enter only as namespace documents (`docs/specs/namespace-documents.md`), through `namespace::load_namespaces` (`crates/sungma-lang/src/namespace.rs`), or built in code by the tests through `Namespace::new`, into `MemoryStore` (`crates/sungma-memory/src/lib.rs`). `sungma-api` loads them once at startup.
- `NamespaceReader` (`crates/sungma/src/store.rs`) reads the version whose span covers a revision.
- A `Decision` (`crates/sungma/src/decision.rs`) records the fact `Revision` and the `Semantics` version, not which namespaces it was judged under. `replay` (`crates/sungma/src/replay.rs`) reads the namespaces of the decision's revision.
- `Namespace::new` (`crates/sungma/src/namespace.rs`) checks a namespace whole and returns every problem. A namespace is valid once built, and a write port keeps that: no stored namespace fails to evaluate.

## The versioning questions are answered

[[one-timeline]] answers the three questions a write port raised. Facts and namespaces share one [[revision-clock]].

- **What identifies a namespace version.** Its revision on the shared clock, the same as a fact's.
- **What a check at revision R reads.** The [[catalog]] at R: each namespace version whose [[span]] covers R. A new namespace can't judge old facts.
- **What replay pins.** The decision's revision implies the catalog, so a decision needs nothing more.

A caller holds a [[revision-token]], not a revision. A request with a revision token is answered at that revision or later, and a node behind the revision token replays the change log up to it before answering.

## Write methods

- Creating a namespace that already exists is refused by the write pipeline.
- PUT is refused. A namespace is never replaced in place.
- PATCH appends a new version, which supersedes the previous one and closes its span.

Duplicate namespaces are therefore gated on the write side. A reader never sees two versions of one namespace at one revision, so neither the namespace document parser nor the namespace store checks for them.

## Storage

Namespace documents are JSON and carry no grammar version; the REST endpoint that takes them is versioned instead. How a namespace version is stored is open.

## Settled

- **What a write carries.** One whole namespace document, whether it creates the namespace or writes a new version. A document supersedes its namespace, and `Namespace::new` checks the next version whole; a change to some relations would have to be merged before it could be checked.
- **Retiring a namespace.** Retiring closes a namespace's last span. [One timeline](../specs/one-timeline.md#retiring-a-namespace) says what happens to its facts and why writing it again doesn't resurrect them. Which call retires it is the transport's concern.
- **Concurrent writes.** A namespace write carries the revision token of the version it was based on, as an etag; a write that creates the namespace expects none. The write is refused when the namespace's latest version has moved on, and the caller reads the latest version and tries again. The `NamespaceWriter` port takes the expected version, and each adapter meets it with its store's conditional write, as the interner does with `insert_if_absent`.
- **Effects on facts.** A namespace write never scans facts, so it neither reports the facts it orphans nor refuses past some count. With billions of facts, a scan would make every namespace write as slow as the graph is large. [[one-timeline]] says which facts count afterward.
- **Storage and ports.** A `NamespaceWriter` port in `sungma-write`, beside `FactWriter`, writes namespace versions, and `NamespaceReader::version` reads at a revision. A namespace version is stored as its printed document; the printer gives back the same document every time, because it sorts relations by name.

## Open questions

Each is answered with the roadmap item that needs it, so the namespace writes spec says where each is answered and goes no further.

1. **The response.** Whether a write returns the new version's revision token, and the canonical printed document, so the caller sees what was stored.
   Deferred to the transports, roadmap item 14.
2. **Reading namespaces.** Whether there is a GET for a namespace at a revision token, and a list of a namespace's versions.
   Deferred to the transports, roadmap item 14.
3. **Errors.** The HTTP status for each refusal, and the RFC 9457 body. Document errors already carry a relation and column.
   Deferred to the transports, roadmap item 14.
4. **Propagation.** How a new version reaches other nodes, and how namespace changes appear in the Watch stream.
   Partly settled: namespace versions go into the same change log as facts, and a node behind a request's revision token replays the log up to it.
   Deferred to the Watch service, roadmap item 11.
5. **Who may write.** Authorization of namespace writes.
   Deferred to authentication and authorization, roadmap item 15; see [[check-takes-a-caller-token-and-a-subject-token]].

## The clock port

Fact writes and namespace writes draw revisions from one [[revision-clock]], so the clock is a port of its own in `sungma-clock`.

- `Clock::now()` returns a `Reading { settled, revision }` that brackets true time, as TrueTime's interval does. `Clock::wait(stamped)` resolves once the clock's `settled` time has passed `stamped.revision`; the adapter supplies the timer, so the base crate needs no async runtime.
- The reading's half-width is a worst-case bound, never an average: the error at the last sync (half the round trip) plus the maximum drift rate times the time since. An underestimate reverses revisions silently. On Linux, chrony (`adjtimex` `maxerror`) or AWS ClockBound supply it; Windows reports root delay and dispersion through `w32tm` but has no API for the bound.
- A single-node adapter, such as SQLite, uses a counter with `settled == revision`, so its wait is free.
- A writer takes `stamped = now()` while it holds its locks, commits at `stamped.revision`, and calls `wait(stamped)` before acknowledging. Any write that starts after the acknowledgement gets a later revision.
- A read at revision t waits until its node's safe time reaches t, so no write at or before t is still in flight.
- The writer ports call the clock inside the commit. The domain never ticks it.
- `Revision` stays in `sungma`; the `Clock` port and `Reading` live in `sungma-clock` with its implementations.

## Pieces already in place

- `FactWriter` keeps each fact's history as spans (the revision that wrote it and the one that retired it), and reads filter by revision. Namespace versions keep history the same way.
- Namespace and relation names intern in `Pool::Namespaces` and `Pool::Relations` through the `Interner` port. See [[interner]].
