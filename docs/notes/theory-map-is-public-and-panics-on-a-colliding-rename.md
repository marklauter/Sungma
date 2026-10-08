---
title: Theory::map is public and panics on a colliding rename
type: note
summary: "Theory::map lets any caller rename a theory's relations with a function it chooses, and panics when that function gives two relations one name."
status: evolving
tags: [bug]
cites:
  - "[[theory]]"
  - "[[relation]]"
---

# Theory::map is public and panics on a colliding rename

## The bug

`Theory::map` (`crates/sungma/src/theory.rs`) returns a new [[theory]] with every [[relation]] renamed by a function the caller passes. It became `pub` when theory documents moved into `sungma-lang`. There, `theory::load_theories` (`crates/sungma-lang/src/theory.rs`) calls it with the dictionary's `intern` as the rename, turning a `Theory<RelationName>` into a `Theory<RelationId>`.

A rename that gives two relations one name hits the assert in `Theory::indexed` and panics. Any caller outside the crate can trigger it:

```rust
theory.map(|_| RelationId(0)) // panics in Theory::indexed
```

The interner never gives two names one id, so `load_theories` can't panic. The risk is any other caller.

A one-to-one rename can't produce an invalid theory: it keeps every check `Theory::new` makes. It does produce a theory that `Theory::new` never checked under its new names.

## Why the rename exists

`Theory` is generic over how relations are named. The parser builds `Theory<RelationName>`, so `Theory::new` reports problems by name. The evaluator reads `Theory<RelationId>`. `map` converts between the two by copying every rewrite tree.

## Fix

The design is open. The fix revisits how a theory goes from names to ids. Options raised so far:

- Keep `map` crate-private, and give `sungma` one method that interns a named theory through the dictionary, so callers can't choose the rename.
- Return a `Result` from `map` instead of panicking on a collision.
- Intern names while parsing and build `Theory<RelationId>` directly, so there is no rename and no copy. A refused theory would leave its names in the dictionary, and `Theory::new` would report problems by id unless the parser maps them back to names.
