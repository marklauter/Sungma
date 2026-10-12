---
title: Models refuse invalid states
type: note
summary: Each model enforces its own rules, so an invalid one can't be built; this note records how each gap found was settled.
status: evolving
---

# Models refuse invalid states

A model enforces every rule it can see on its own, in its constructor or its type, so an invalid value can't exist. A rule that spans models, such as a fact naming a relation its namespace declares, is the write pipeline's job.

## Settled

- **`Rewrite`** (`crates/sungma/src/rewrite.rs`): union and intersection operands are `NonEmpty` (`crates/sungma/src/non_empty.rs`), so an empty operator can't be built.
- **`Decision`** (`crates/sungma/src/decision.rs`): an allowed outcome's grounds are `NonEmpty`, since every membership rests on at least one fact.
- **`Namespace::map`** (`crates/sungma/src/namespace.rs`): refuses a rename that gives two relations one name, and renames each name once, so references can't dangle.
- **Names** (`crates/sungma/src/name.rs`): `new_unchecked` stays public. Choosing checked or unchecked is the caller's job.
- **Ids** (`crates/sungma/src/id.rs`): the public `u32` fields of `NamespaceId` and `RelationId` stay. A made-up id names nothing, so it breaks nothing. Only namespace and relation names are interned; see [[interner]].
