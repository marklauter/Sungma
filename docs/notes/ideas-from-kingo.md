---
title: Ideas from Kingo
summary: Sixteen ideas from Kingo, the legacy ReBAC, each with its Sungma status as of 2026-10-04. One is in place, three run counter to Sungma, and two confirmed bugs surfaced along the way.
status: evolving
---

# Ideas from Kingo

Kingo (`D:\projects\kingo\kingo`, C#) is the legacy attempt at a ReBAC. Its source is about 1,200 lines of model, validation and theory documents with no evaluator; its value is in `docs/` and in its tests of the theory front end. Paths below that start `docs/`, `src/` or `tests/` are in Kingo; Sungma paths start `crates/`.

Terminology: a Sungma theory is a Kingo namespace. A Kingo theory is a named bundle of namespaces and the unit of atomic change.

Not a source: the `rewrite-interpreters` branch (checkout at `D:\projects\kingo\kingo-rewrite-interpreters`). `docs/todos/reconcile.md` records it as a dead end, rooted in treating late-bound relation names as URNs. The main checkout's `docs/todos/implement-contains-and-expand.md` is not part of that dead end; reconcile §3.4 keeps it and discards the branch's edits.

## Confirmed bugs

Both were reproduced with throwaway tests on 2026-10-04.

- `theory::validate` overflows the stack on a chain of 20,000 computed relations (`r0: r1`, `r1: r2`, …). `find_cycle` in `crates/sungma-core/src/theory.rs` recurses once per relation. Kingo walks with an explicit stack (`src/Kingo.Theories/Namespace.cs:90-150`) and tests the 20k chain (`tests/Kingo.Theories.Tests/NamespaceTests.cs:362-372`).
- `MAX_REWRITE_DEPTH` (100) is unreachable through JSON. `fixture::load_theories` uses serde_json's default recursion limit of 128, and each `{"union":[…]}` level costs two. A 63-deep rewrite loads; a 64-deep one fails as `FixtureError::Json` ("recursion limit exceeded"), never `TheoryError::TooDeep`.

## Sungma status at a glance

| # | Idea | Sungma status |
|---|---|---|
| 1 | Undefined relation mid-walk is an error | Counter: treated as the empty set |
| 2 | Errors combine by Kleene absorption | In place |
| 3 | Wrong-shaped factset member is an error | Counter: followed or skipped silently |
| 4 | Drift prevented at write time | Missing |
| 5 | Facts and theories share one timeline | Open, leaning this way |
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

## Ideas

1. **An undefined relation reached mid-walk is an error, not the empty set.** Under exclusion, the empty set fails open: an undefined `banned` means nobody is banned.
   - **Sungma: counter.** `crates/sungma-core/src/extent.rs` (module doc) and `README.md` treat an undeclared theory or relation as the empty set. An uninterned name in a request is likewise denied without error (`README.md`, `crates/sungma-core/src/resolve.rs`).
   - Source: `docs/todos/implement-contains-and-expand.md`, error families 1 and 4.
2. **Errors combine by Kleene absorption.** A deciding value absorbs an error: `false` in intersection, `true` in union, and `a = false` or `b = true` in `a ! b`. The verdict then never depends on evaluation order, so parallel evaluation stays sound.
   - **Sungma: in place.** `Extent::contains_node` and `contains_this` keep the first error and return it only when no operand settles the result (`crates/sungma-core/src/extent.rs`, module doc). Expand still fails on any error, since it returns no verdict.
   - Source: `docs/glossary/kleene-absorption.md`.
3. **A wrong-shaped factset member is an error.** Under `(parent, viewer)` only a resource member traverses. A subjectset member or identity is a modeled error, and so is a resource member under `this`.
   - **Sungma: counter.** `Extent::fact_targets` follows subjectsets to their resource and skips identities. `contains_this` doesn't check for resource members.
   - Source: `docs/todos/implement-contains-and-expand.md` conditions 5, 6 and 9; `docs/todos/resource-fact-case.md`.
4. **Fact/theory drift is prevented at write time.** Fact writes are checked against the current theory. A theory write that would strand live facts is refused, so removal takes two steps. Ideas 1 and 3 then become backstops that never fire.
   - **Sungma: missing.** There is no theory write path; this feeds roadmap items 9 and 13.
   - Source: `docs/decisions/preventing-drift-between-facts-and-theories.md`.
5. **Facts and theories share one timeline.** Theory versions take revisions from the fact sequence, the zookie selects the theory version, and a decision records only the zookie.
   - **Sungma: open.** [[theory-writes-wait-on-theory-versioning]] asks these questions and leans this way.
   - Source: `docs/todos/storage-versioning-design.md`, `docs/specs/catalog.md`.
6. **Writes are apply and drop only.** Batches are atomic, idempotent and validated on their end state. Preconditions replace a strict create.
   - **Sungma: partial.** `FactWriter` applies atomic batches. Idempotence and preconditions are unspecified.
   - Source: `docs/notes/graph-operations.md`, `docs/todos/graph-document-is-bulk-dml.md`.
7. **The name grammar is enforced.** Names match one grammar, normalize to lowercase, and `this` and `...` are reserved.
   - **Sungma: missing.** A theory declaring relations `this`, `...`, `a#b`, `a b`, `Viewer` and `viewer` loads, and so does a theory named `a:b`. The first-`:`, last-`#` split in `crates/sungma-core/src/name.rs` is sound only if theory names exclude `:` and relation names exclude `#`.
   - Source: `src/Kingo/IdentifierGrammar.cs`, `tests/Kingo.Tests/RelationNameTests.cs`.
8. **Validation reports every error, each with a stable code.** Codes such as `namespace.rewrite_cycle` map onto the RFC 9457 `type` field.
   - **Sungma: missing.** `theory::validate` stops at the first error, and errors carry message text only.
   - Source: `src/Kingo.Theories/Diagnostics/ErrorCodes.cs`, `tests/Kingo.Theories.Tests/NamespaceTests.cs:180-299`.
9. **Invalid rewrite trees can't be built.** Empty operators and excess depth are refused at construction.
   - **Sungma: partial.** `Rewrite` variants are public, and only `Theory::new` checks them.
   - Source: `src/Kingo.Theories/SubjectSetRewrite.cs`, `SubjectSetRewrite.Depth.cs`.
10. **A rewrite text grammar with a printer that round-trips.** Precedence `!` > `&` > `|`, left-associative `!`, minimal parentheses on print, and a hostile-input suite: 20k nested parentheses, a 20k `!` chain, and a 500-wide union that isn't depth.
    - **Sungma: missing.** Theories load from JSON only; roadmap item 12.
    - Source: `src/Kingo.Documents/RewriteExpressionParser.cs`, `RewriteExpressionPrinter.cs`, `tests/Kingo.Documents.Tests/`.
11. **Grounds are kept from callers.** Grounds reveal other subjects' memberships, and a witness can't prove a negative under `!`, so Kingo kept proof off decisions.
    - **Sungma: partial.** The `POST /check` response carries only the verdict, but `Decision` and the audit record carry grounds. Nothing documents that grounds omit the excluded side.
    - Source: `docs/todos/implement-contains-and-expand.md`.
12. **Tenant isolation is structural.** If the engine decides isolation, a theory bug is a cross-tenant breach. Scope the request by account before the engine runs.
    - **Sungma: missing.**
    - Source: `docs/todos/decide-whether-tenant-isolation-is-structural.md`.
13. **An actor catalog with explicit attack paths.** Wrong denies get reported and wrong allows don't, so overgrant must be caught by the audit record or by examples.
    - **Sungma: missing.**
    - Source: `docs/notes/actor-catalog.md`.
14. **The core carries no formats or adapters.** Documents and test doubles live in their own crate, with a dependency check on the core.
    - **Sungma: counter.** `sungma-core` depends on serde_json and ships `fixture` and `memory`.
    - Source: `docs/decisions/deciding-which-types-parse-text.md`, `tests/Kingo.Testing/ArchitectureTestsBase.cs`.
15. **Stricter build gates.** A `[workspace.lints]` table with pedantic lints and a reason on each allow, per-crate coverage, and branch coverage.
    - **Sungma: partial.** Clippy runs with default lints under `-D warnings`, and coverage is workspace-wide lines only (`.githooks/pre-push`, `.github/workflows/rust.yml`). Mutation testing and the dependency audit go beyond Kingo.
    - Source: `Directory.Build.props`.
16. **Smaller evaluation rules.**
    - `DepthExceeded` carries its bound. **Sungma: present.**
    - The depth bound counts only fact-driven re-entries, since computed crossings are acyclic by construction. **Sungma: diverges;** `Extent::contains_at` counts every subjectset.
    - A cycle is rotated to start at its least relation name, so two reports compare equal. **Sungma: missing** (`TheoryError::RewriteCycle`).
    - A request's subject is narrowed to an identity. **Sungma: diverges;** `POST /check` accepts a subjectset.
    - Source: `docs/todos/implement-contains-and-expand.md` condition 3, `docs/todos/rewrite-cycles-are-domain-values.md`.
