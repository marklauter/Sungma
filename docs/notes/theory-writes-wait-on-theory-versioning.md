---
title: Theory writes wait on theory versioning
type: note
summary: Theory versioning is settled by one timeline for facts and theories, and the write methods are settled, so a port for writing theories can now be designed.
status: evolving
---

# Theory writes wait on theory versioning

## Where it stands

- Theories enter only as theory documents (`docs/specs/theory-documents.md`), through `fixture::load_theories`, or built in code through `fixture::declare` (`crates/sungma-core/src/fixture.rs`), into `MemoryTheoryStore` (`crates/sungma-core/src/memory.rs`). `sungma-api` loads them once at startup.
- `TheoryStore` (`crates/sungma-core/src/store.rs`) is read-only and unversioned: `rewrite(theory, relation)` returns the current rewrite.
- A `Decision` (`crates/sungma-core/src/decision.rs`) records the fact `Revision` and the `Semantics` version, not which theories it was judged under. `replay` (`crates/sungma-core/src/replay.rs`) reads the current theories, so a theory change since the decision shows up as `Replay::Differs`, or as `Replay::Regrounded` when only the grounds move.
- `Theory::new` (`crates/sungma-core/src/theory.rs`) checks a theory whole and returns every problem. A theory is valid once built, and a write port keeps that: no stored theory fails to evaluate.

## The versioning questions are answered

[[one-timeline]] answers the three questions a write port raised. Facts and theories share one [[revision-clock]].

- **What identifies a theory version.** Its revision on the shared clock, the same as a fact's.
- **What a check at revision R reads.** The [[catalog]] at R: each theory version whose [[span]] covers R. A new theory can't judge old facts.
- **What replay pins.** The decision's revision implies the catalog, so a decision needs nothing more.

A caller holds a [[mark]], not a revision. A request with a mark is answered at that revision or later, and a node behind the mark replays the change log up to it before answering.

## Write methods

- Creating a theory that already exists is refused by the write pipeline.
- PUT is refused. A theory is never replaced in place.
- PATCH appends a new version, which supersedes the previous one and closes its span.

Duplicate theories are therefore gated on the write side. A reader never sees two versions of one theory at one revision, so neither the theory document parser nor the theory store checks for them.

## Storage

The earlier plan, YAML theory text stored with a grammar version, is withdrawn. Theory documents are JSON and carry no grammar version; the REST endpoint that takes them is versioned instead. How a theory version is stored is open.

## Pieces already in place

- `FactWriter` keeps each fact's history as spans (the revision that wrote it and the one that deleted it), and reads filter by revision. Theory versions keep history the same way.
- Theory and relation names intern in `Pool::Theories` and `Pool::Relations` through the `Interner` port. See [[interner]].
