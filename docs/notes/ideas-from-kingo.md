---
title: Ideas from Kingo
summary: Fourteen ideas from Kingo, the legacy ReBAC, each with its Sungma status as of 2026-10-04. One runs counter to Sungma, and two confirmed bugs surfaced along the way.
status: evolving
---

# Ideas from Kingo

Kingo (`D:\projects\kingo\kingo`, C#) is the legacy attempt at a ReBAC. Its source is about 1,200 lines of model, validation and namespace documents with no evaluator; its value is in `docs/` and in its tests of the namespace front end. Paths below that start `docs/`, `src/` or `tests/` are in Kingo; Sungma paths start `crates/`.

Terminology: a Sungma namespace is a Kingo namespace. A Kingo theory, like a Sungma theory, is a set of namespaces. Kingo's was also named and was the unit of atomic change.

Not a source: the `rewrite-interpreters` branch (checkout at `D:\projects\kingo\kingo-rewrite-interpreters`). `docs/todos/reconcile.md` records it as a dead end, rooted in treating late-bound relation names as URNs. The main checkout's `docs/todos/implement-contains-and-expand.md` is not part of that dead end; reconcile §3.4 keeps it and discards the branch's edits.

## Confirmed bugs

Both were reproduced with throwaway tests on 2026-10-04.

- Fixed: `namespace::validate` overflowed the stack on a chain of 20,000 computed relations (`r0: r1`, `r1: r2`, …), recursing once per relation. `Namespace::new` now walks with its own stack and is tested on a 100,000-relation chain (`crates/sungma/tests/namespace.rs`).
- `MAX_REWRITE_DEPTH` (100) was unreachable through the nested JSON fixture form, whose levels hit serde_json's recursion limit first. Namespace documents write rewrites as expression strings, so a 101-level rewrite now reaches `NamespaceError::TooDeep` (`crates/sungma-lang/tests/namespace.rs`, `a_rewrite_nests_at_most_max_depth`).

## Sungma status at a glance

| # | Idea | Sungma status |
|---|---|---|
| 1 | Relation epoch keeps orphaned facts from resurrecting | Missing |
| 4 | Drift prevented at write time | Missing |
| 5 | Facts and namespaces share one timeline | Open, leaning this way |
| 6 | Writes are apply and drop only | Partial |
| 7 | Name grammar enforced | Missing |
| 8 | Every error reported, with stable codes | Missing |
| 9 | Invalid rewrite trees can't be built | Partial |
| 10 | Rewrite text grammar and round-trip printer | Missing |
| 11 | Grounds kept from callers | Partial |
| 12 | Tenant isolation is structural | Missing |
| 13 | Actor catalog with attack paths | Missing |
| 14 | Core carries no formats or adapters | Counter |
| 15 | Stricter build gates | Partial |
| 16 | Smaller evaluation rules | Mixed |

## Facts and namespaces on one timeline

The proposed mechanism for ideas 1, 4 and 5 is in [[one-timeline]].

### 1. A relation epoch keeps orphaned facts from resurrecting

The epoch exists for one purpose: when a later namespace version re-declares a dropped relation, the facts orphaned by the drop must not come back to life. Without it, re-adding `owner` in `file` v3 would revive every `owner` fact orphaned by v2, restoring grants that were dropped. A namespace version that drops a relation leaves the facts stored under it in place. Under that version they count as nonexistent, because the relation is undeclared there. Each namespace version records, for each relation, the revision at which it was last declared: its epoch. Evaluation counts a fact only if its span covers the revision and its `written` revision is at or after the relation's epoch in the namespace version the snapshot selects.

- Re-adding a dropped relation in a later version moves its epoch, so older facts under it stay dead and old grants don't resurrect.
- A replay or expand at an earlier revision reads the earlier version and its epoch, so the facts are valid there.
- The namespace write is cheap: no fact is rewritten, no cross-namespace scan runs, and facts carry no namespace version. Late binding means the catalog can't see which other namespaces reach a dropped relation, and scanning the fact graph on every namespace write is impractical. Dropping a relation therefore empties it everywhere, including on the excluded side of another namespace's exclusion, which can widen access there. That widening is accepted.
- Re-applying a fact whose span started before the epoch has to open a new span. Today an insert of a live fact is a no-op (`crates/sungma-memory/src/lib.rs`, `Span`), which would leave a re-asserted grant dead.
- Every reader of the graph applies the same epoch filter: Check, Expand, Read, Watch and dumps. A reader that skips it shows orphans as live.
- The eager alternative closes the orphans' spans in the namespace write. It gives the same visible facts at every revision and needs no reader filter, but the write costs as much as there are orphans.
- This replaces the namespace-write half of idea 4, which refuses a namespace write that strands live facts. It depends on idea 5: `replay` reads the current namespaces today ([[namespace-writes-wait-on-namespace-versioning]]).
- **Sungma: missing.** `Namespace` has no epochs, and `crates/sungma/src/closure.rs` treats an undeclared relation as the empty set with no epoch check.
- Source: decided in conversation on 2026-10-04. It departs from Kingo's `docs/decisions/preventing-drift-between-facts-and-theories.md`, which refuses the namespace write, and keeps that decision's rule that facts carry no namespace version.

### 4. Fact/namespace drift is prevented at write time

Fact writes are checked against the current namespace. A namespace write that would strand live facts is refused, so removal takes two steps.

- **Sungma: missing.** There is no namespace write path; this feeds roadmap items 9 and 13.
- Source: `docs/decisions/preventing-drift-between-facts-and-theories.md`.

### 5. Facts and namespaces share one timeline

Namespace versions take revisions from the fact sequence, the zookie selects the namespace version, and a decision records only the zookie.

- **Sungma: open.** [[namespace-writes-wait-on-namespace-versioning]] asks these questions and leans this way.
- Source: `docs/todos/storage-versioning-design.md`, `docs/specs/catalog.md`.

## Other ideas

### 6. Writes are apply and drop only

Batches are atomic, idempotent and validated on their end state. Preconditions replace a strict create.

- **Sungma: partial.** `FactWriter` applies atomic batches. Idempotence and preconditions are unspecified.
- Source: `docs/notes/graph-operations.md`, `docs/todos/graph-document-is-bulk-dml.md`.

### 7. The name grammar is enforced

Names match one grammar, normalize to lowercase, and `this` and `...` are reserved.

- **Sungma: missing.** A namespace declaring relations `this`, `...`, `a#b`, `a b`, `Viewer` and `viewer` loads, and so does a namespace named `a:b`. The first-`:`, last-`#` split in `crates/sungma/src/name.rs` is sound only if namespace names exclude `:` and relation names exclude `#`.
- Source: `src/Kingo/IdentifierGrammar.cs`, `tests/Kingo.Tests/RelationNameTests.cs`.

### 8. Validation reports every error, each with a stable code

Codes such as `namespace.rewrite_cycle` map onto the RFC 9457 `type` field.

- **Sungma: missing.** `namespace::validate` stops at the first error, and errors carry message text only.
- Source: `src/Kingo.Theories/Diagnostics/ErrorCodes.cs`, `tests/Kingo.Theories.Tests/NamespaceTests.cs:180-299`.

### 9. Invalid rewrite trees can't be built

Empty operators can't be built, and excess depth is refused at construction.

- **Sungma: done.** Union and intersection operands are `NonEmpty`, and `Namespace::new` refuses excess depth.
- Source: `src/Kingo.Theories/SubjectSetRewrite.cs`, `SubjectSetRewrite.Depth.cs`.

### 10. A rewrite text grammar with a printer that round-trips

Precedence `!` > `&` > `|`, left-associative `!`, minimal parentheses on print, and a hostile-input suite: 20k nested parentheses, a 20k `!` chain, and a 500-wide union that isn't depth.

- **Sungma: missing.** Namespaces load from JSON only; roadmap item 12.
- Source: `src/Kingo.Documents/RewriteExpressionParser.cs`, `RewriteExpressionPrinter.cs`, `tests/Kingo.Documents.Tests/`.

### 11. Grounds are kept from callers

Grounds reveal other subjects' memberships, and a witness can't prove a negative under `!`, so Kingo kept proof off decisions.

- **Sungma: partial.** The `POST /check` response carries only the verdict, but `Decision` and the audit record carry grounds. Nothing documents that grounds omit the excluded side.
- Source: `docs/todos/implement-contains-and-expand.md`.

### 12. Tenant isolation is structural

If the engine decides isolation, a namespace bug is a cross-tenant breach. Scope the request by account before the engine runs.

- **Sungma: missing.**
- Source: `docs/todos/decide-whether-tenant-isolation-is-structural.md`.

### 13. An actor catalog with explicit attack paths

Wrong denies get reported and wrong allows don't, so overgrant must be caught by the audit record or by examples.

- **Sungma: missing.**
- Source: `docs/notes/actor-catalog.md`.

### 14. The core carries no formats or adapters

Documents and test doubles live in their own crate, with a dependency check on the core.

- **Sungma: partial.** Namespace documents and fact JSON live in `sungma-lang`, so the core no longer depends on serde. The core still ships `memory`, and nothing checks its dependencies.
- Source: `docs/decisions/deciding-which-types-parse-text.md`, `tests/Kingo.Testing/ArchitectureTestsBase.cs`.

### 15. Stricter build gates

A `[workspace.lints]` table with pedantic lints and a reason on each allow, per-crate coverage, and branch coverage.

- **Sungma: partial.** Clippy runs with default lints under `-D warnings`, and coverage is workspace-wide lines only (`.githooks/pre-push`, `.github/workflows/rust.yml`). Mutation testing and the dependency audit go beyond Kingo.
- Source: `Directory.Build.props`.

### 16. Smaller evaluation rules

- `DepthExceeded` carries its bound. **Sungma: present.**
- The depth bound counts only fact-driven re-entries, since computed crossings are acyclic by construction. **Sungma: diverges;** `Closure::contains_at` counts every subjectset.
- A cycle is rotated to start at its least relation name, so two reports compare equal. **Sungma: missing** (`NamespaceError::RewriteCycle`).
- A request's subject is narrowed to an identity. **Sungma: diverges;** `POST /check` accepts a subjectset.
- Source: `docs/todos/implement-contains-and-expand.md` condition 3, `docs/todos/rewrite-cycles-are-domain-values.md`.
