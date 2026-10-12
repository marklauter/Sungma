---
title: Namespace writes wait on namespace versioning
type: note
summary: Namespace versioning is settled by one timeline for facts and namespaces, and the write methods are settled, so a port for writing namespaces can now be designed.
status: evolving
---

# Namespace writes wait on namespace versioning

## Where it stands

- Namespaces enter only as namespace documents (`docs/specs/namespace-documents.md`), through `namespace::load_namespaces` (`crates/sungma-lang/src/namespace.rs`), or built in code by the tests through `Namespace::new`, into `MemoryNamespaceStore` (`crates/sungma-memory/src/lib.rs`). `sungma-api` loads them once at startup.
- `NamespaceStore` (`crates/sungma/src/store.rs`) is read-only and unversioned: `rewrite(namespace, relation)` returns the current rewrite.
- A `Decision` (`crates/sungma/src/decision.rs`) records the fact `Revision` and the `Semantics` version, not which namespaces it was judged under. `replay` (`crates/sungma/src/replay.rs`) reads the current namespaces, so a namespace change since the decision shows up as `Replay::Differs`, or as `Replay::Regrounded` when only the grounds move.
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

The earlier plan, YAML namespace text stored with a grammar version, is withdrawn. Namespace documents are JSON and carry no grammar version; the REST endpoint that takes them is versioned instead. How a namespace version is stored is open.

## Open questions

A production spec of namespace writes needs these settled. Questions 1, 3 and 7 shape the rest.

1. **What PATCH carries.** A whole namespace document that becomes the next version, or a change to some relations. What the create method carries.
   Leaning: a whole document. A document supersedes its namespace, and `Namespace::new` then checks the next version whole. A change to some relations would have to be merged before it could be checked.
2. **Retiring a namespace.** Whether a namespace can be retired, closing its last span, and what then happens to its facts and to fact-to-subjectsets in other namespaces that reach it.
3. **Concurrent writes.** Whether a write names the version or [[revision-token]] it expects, as `If-Match` does, and is refused on a mismatch.
   Leaning: yes, with the revision token as the `If-Match` value. Without it, two PATCHes from the same version silently overwrite each other.
4. **Effects on facts.** Whether a namespace write reports the facts it orphans, refuses past some count, or does neither. [[one-timeline]] settles what epochs and orphaned facts mean.
   Leaning: neither. [[one-timeline]] accepts that a namespace write never scans the graph, and reporting or counting orphans needs that scan.
5. **The response.** Whether a write returns the new version's revision token, and the canonical printed document, so the caller sees what was stored.
6. **Reading namespaces.** Whether there is a GET for a namespace at a revision token, and a list of a namespace's versions.
7. **Storage and ports.** A `NamespaceWriter` port beside `FactWriter`, and `NamespaceStore::rewrite` reading at a revision. Whether the stored form is the document text or the parsed namespace.
   Leaning: store the parsed, interned namespace. The printer gives back the same document every time, because it sorts relations by name.
8. **Errors.** The HTTP status for each refusal, and the RFC 9457 body. Document errors already carry a relation and column.
9. **Propagation.** How a new version reaches other nodes, and how namespace changes appear in the Watch stream.
   Partly settled: namespace versions go into the same change log as facts, and a node behind a request's revision token replays the log up to it.
10. **Who may write.** Authorization of namespace writes, roadmap item 15. The spec may defer it, but says so.

## The clock port

Fact writes and namespace writes draw revisions from one [[revision-clock]], so the clock is a port of its own in the `clock` module, beside `Revision`, `Span` and `RevisionToken`.

- `Clock::now()` returns a `Reading { settled, revision }` that brackets true time, as TrueTime's interval does. `Clock::wait(stamped)` resolves once the clock's `settled` time has passed `stamped.revision`; the adapter supplies the timer, so the base crate needs no async runtime.
- The reading's half-width is a worst-case bound, never an average: the error at the last sync (half the round trip) plus the maximum drift rate times the time since. An underestimate reverses revisions silently. On Linux, chrony (`adjtimex` `maxerror`) or AWS ClockBound supply it; Windows reports root delay and dispersion through `w32tm` but has no API for the bound.
- A single-node adapter, such as SQLite, uses a counter with `settled == revision`, so its wait is free.
- A writer takes `stamped = now()` while it holds its locks, commits at `stamped.revision`, and calls `wait(stamped)` before acknowledging. Any write that starts after the acknowledgement gets a later revision.
- A read at revision t waits until its node's safe time reaches t, so no write at or before t is still in flight.
- The writer ports call the clock inside the commit. The domain never ticks it.
- `Revision` stays in `sungma`; the `Clock` port and `Reading` live in `sungma-clock` with its implementations.

## Pieces already in place

- `FactWriter` keeps each fact's history as spans (the revision that wrote it and the one that deleted it), and reads filter by revision. Namespace versions keep history the same way.
- Namespace and relation names intern in `Pool::Namespaces` and `Pool::Relations` through the `Interner` port. See [[interner]].
