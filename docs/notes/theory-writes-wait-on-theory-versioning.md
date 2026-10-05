---
title: Theory writes wait on theory versioning
type: note
summary: A port for writing theories has to say which theory a check or replay at revision R reads, so it is designed together with theory versioning, not before.
status: evolving
---

# Theory writes wait on theory versioning

## Where it stands

- Theories enter only from fixture JSON, through `fixture::load_theories` and `fixture::declare` (`crates/sungma-core/src/fixture.rs`), into `MemoryTheoryStore` (`crates/sungma-core/src/memory.rs`). `sungma-api` loads them once at startup.
- `TheoryStore` (`crates/sungma-core/src/store.rs`) is read-only and unversioned: `rewrite(theory, relation)` returns the current rewrite.
- A `Decision` (`crates/sungma-core/src/decision.rs`) records the fact `Revision` and the `Semantics` version, not which theories it was judged under. `replay` (`crates/sungma-core/src/replay.rs`) reads the current theories, so a theory change since the decision shows up as `Replay::Differs`, or as `Replay::Regrounded` when only the grounds move.
- `theory::validate` (`crates/sungma-core/src/theory.rs`) checks a theory whole when it is declared. A write port keeps that: no stored theory fails to evaluate.

## Why a write port waits

Writing theories forces the versioning questions:

- **What identifies a theory version.** Either theories have a sequence of their own, or theory writes take revisions from the fact revision sequence, so one revision pins facts and theories together.
- **What a check at revision R reads.** Either the theories as of R, or the current ones. Reading the current ones lets a new theory judge old facts.
- **What replay pins.** Replay has to read the theories a decision was made under. Either the decision records them, or its revision implies them.

The storage form is settled: the YAML theory text with a grammar version, parsed on load. The JSON form is a stopgap.

## Pieces already in place

- `FactWriter` keeps each fact's history as spans (the revision that wrote it and the one that deleted it), and reads filter by revision. Theories could keep history the same way, which favours sharing the fact revision sequence.
- Theory and relation names intern in `Pool::Theories` and `Pool::Relations` through the `Interner` port. See [[interner]].
