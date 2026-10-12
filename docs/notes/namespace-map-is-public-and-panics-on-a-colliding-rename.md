---
title: Namespace::map is public and panics on a colliding rename
type: note
summary: "Namespace::map lets any caller rename a namespace's relations with a function it chooses, and panics when that function gives two relations one name."
status: evolving
tags: [bug]
cites:
  - "[[namespace]]"
  - "[[relation]]"
---

# Namespace::map is public and panics on a colliding rename

## The bug

`Namespace::map` (`crates/sungma/src/namespace.rs`) returns a new [[namespace]] with every [[relation]] renamed by a function the caller passes. It became `pub` when namespace documents moved into `sungma-lang`. There, `namespace::load_namespaces` (`crates/sungma-lang/src/namespace.rs`) calls it with the dictionary's `intern` as the rename, turning a `Namespace<RelationName>` into a `Namespace<RelationId>`.

A rename that gives two relations one name hits the assert in `Namespace::indexed` and panics. Any caller outside the crate can trigger it:

```rust
namespace.map(|_| RelationId(0)) // panics in Namespace::indexed
```

The interner never gives two names one id, so `load_namespaces` can't panic. The risk is any other caller.

A one-to-one rename can't produce an invalid namespace: it keeps every check `Namespace::new` makes. It does produce a namespace that `Namespace::new` never checked under its new names.

## Why the rename exists

`Namespace` is generic over how relations are named. The parser builds `Namespace<RelationName>`, so `Namespace::new` reports problems by name. The evaluator reads `Namespace<RelationId>`. `map` converts between the two by copying every rewrite tree.

## Fix

The design is open. The fix revisits how a namespace goes from names to ids. Options raised so far:

- Keep `map` crate-private, and give `sungma` one method that interns a named namespace through the dictionary, so callers can't choose the rename.
- Return a `Result` from `map` instead of panicking on a collision.
- Intern names while parsing and build `Namespace<RelationId>` directly, so there is no rename and no copy. A refused namespace would leave its names in the dictionary, and `Namespace::new` would report problems by id unless the parser maps them back to names.
