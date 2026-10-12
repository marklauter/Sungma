---
title: Models refuse invalid states
type: note
summary: Each model enforces its own rules, so an invalid one can't be built; this note lists the models that don't yet.
status: evolving
---

# Models refuse invalid states

A model enforces every rule it can see on its own, in its constructor or its type, so an invalid value can't exist. A rule that spans models, such as a fact naming a relation its namespace declares, is the write pipeline's job.

## Gaps

- **`Namespace`** (`crates/sungma/src/namespace.rs`): `Namespace::new` accepts a namespace with no relations, and `print` in `crates/sungma-lang/src/namespace.rs` prints one. `Namespace::map` panics when two relations rename to one name instead of refusing; see [[namespace-map-is-public-and-panics-on-a-colliding-rename]].
- **Names** (`crates/sungma/src/name.rs`): `new_unchecked` is public, so any caller skips validation, not only storage.
- **`Rewrite`** (`crates/sungma/src/rewrite.rs`): a public enum, so `Union(vec![])` and `Intersection(vec![])` can be built anywhere; only `Namespace::new` refuses them. A non-empty operand type would make them unexpressible.
- **`Decision`** (`crates/sungma/src/decision.rs`): public fields allow `Outcome::Allowed { grounds: vec![] }`, though every membership rests on at least one fact.
- **Ids** (`crates/sungma/src/id.rs`): `NamespaceId` and `RelationId` have public `u32` fields, so any caller mints an id. Only namespace and relation names are interned; see [[interner]].
