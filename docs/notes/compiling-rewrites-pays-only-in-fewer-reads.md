---
title: Compiling rewrites pays only in fewer reads
type: note
summary: Compiling each relation's rewrite into a plan when its namespace version loads would save little in dispatch, since a check's time goes to fact reads; it pays only if the plan skips reads, which needs namespaces to declare what subjects a relation holds.
status: evolving
cites:
  - "[[rewrite]]"
  - "[[namespace]]"
  - "[[subjectset]]"
---

# Compiling rewrites pays only in fewer reads

## The idea

Check walks a relation's [[rewrite]] tree on every check. `Closure::contains_node` (`crates/sungma-check/src/closure/check.rs`) matches on each node, and each `Computed` or `FactTo` node builds the next [[subjectset]] and looks its rewrite up in the [[namespace]] version again.

Compiling turns each relation's rewrite into something run directly, once per namespace version, when the version loads. Union becomes "any of these", intersection "all of these", exclusion "this unless that", and each leaf a function already bound to the ids it reads. Namespaces are few and change rarely, so the compiled form of every version can stay in memory.

## What it buys

1. **Less dispatch.** No match per node, and references resolved ahead of time, so a `Computed` node points straight at its relation's compiled form. This is worth little: a node's match costs nanoseconds, and a check's time goes to fact reads.
2. **Fewer reads.** A compiler that knows a branch can't hold the subject being checked can skip that branch's reads. Every `This` node costs two reads today, a point lookup for the subject and a read of the subjectsets stored under it, so each skipped branch saves at least two.

## What it costs

1. **Pruning needs subject types.** Sungma's namespaces don't declare which kinds of subject a relation holds, so nothing is known at compile time that rules a branch out. Pruning needs the namespace language to declare them, as in `viewer: identity | group#member`, and writes to refuse facts that break the declaration. That's a change to the grammar, the parser, the printer, the write validation and the specs. Without it, compiling buys only point 1.
2. **A second evaluator.** The compiled form has to judge exactly as the walk does: Kleene's three values for failed reads, cycles ending without an error, the depth limit, the epoch rule, and citing the facts behind a yes. Either the walk goes away, or both stay and the tests have to prove they agree.
3. **Async functions in Rust.** A node compiled to a function returns a future, and a tree of them has to be boxed: `Box<dyn Fn(..) -> BoxFuture<..>>`, with lifetimes tied to the closure's borrows. The walk already boxes a future per node (`contains_at`, `contains_node`), so this costs about the same allocations, but closures are harder to read, debug and test than a `match`.
4. **A plan, not closures.** Most of the gain comes without closures: compile each rewrite into a plan, a tree of plain data holding resolved relation pointers and pruning flags, and keep one walk that runs it. The plan is data, so it can be printed, compared and tested, and the walk stays one `match`.
5. **A compiler to own.** Compiling, caching per namespace version, and dropping old versions is new code, with its own tests and mutants, that doesn't change an answer. It earns its place only once measurements show checks spending reads on branches that couldn't hold the subject.

## Where it stands

Not planned. Revisit it with Check's own store, or once measured checks show reads spent on branches that couldn't match. It turns on one decision: whether namespaces declare subject types.
