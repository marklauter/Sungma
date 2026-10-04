# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a ReBAC. WIP.

## Architecture

Sungma is a Zanzibar-style authorization service. It answers one question: is subject S in set `resource#relation` at revision R? It has two crates, a pure core library and a thin HTTP shell around it. Below is how it fits together, from bottom to top.

**The model** (`model.rs`)
- A **fact** is `subjectset@subject`, for example `file:design.md#owner@carol`. The subject is one of three things:
  - an identity (`carol`);
  - another subjectset (`group:eng#member`);
  - a resource itself (`folder:root#...`), which is how parent links are written.
- Every string is interned to a `u32` newtype such as `TheoryId` or `RelationId`. These are like C# `readonly record struct TheoryId(uint Value)`. Past the edge, the core never handles a string.
- Each write bumps a global `Revision`, and every read is "as of" a revision.

**Theories** (`theory.rs`, `rewrite.rs`)
- A theory declares the relations for one kind of resource. Each relation has a **rewrite**, a small expression tree. `Rewrite` is a Rust enum, which is a closed discriminated union; in C# it would be an abstract record with sealed subclasses. Its cases are:
  - `this`: the stored facts;
  - computed: another relation on the same resource;
  - fact-to-subjectset: follow a fact, such as `parent`, then evaluate a relation on the resource it names;
  - union, intersection and exclusion.
- `validate` checks a theory whole when it is declared. It refuses duplicates, empty operators, nesting deeper than 100 levels, undeclared names and computed cycles. So any theory that gets stored is guaranteed to evaluate.

**Storage ports** (`store.rs`, `memory.rs`, `intern.rs`)
- The ports are async traits, which play the role of C# interfaces. Reads and writes are separate:
  - `Dictionary`: name to id, and back;
  - `FactStore`: keyed like a wide-column store, with the subjectset as partition key and the subject as sort key;
  - `TheoryStore`: the rewrite for a theory and relation;
  - `FactWriter`: a batch of inserts and deletes, applied atomically at the next revision;
  - `Interner`: mints ids for new names, each in its pool: theory names, relation names, the resource ids of each theory, and identities. Relations share one pool because `(parent, viewer)` means `viewer` in whichever theory the parent fact names.
- Where a guarantee spans nodes, Sungma runs the protocol and a store supplies primitives. A fact keeps the revision that wrote it and the one that deleted it, so reads at any revision work on any store. `LeasingInterner` mints ids over a `NameStore`: each node leases a block of ids from a shared counter, and a conditional insert makes the first node to store a name win. [docs/interner.md](docs/interner.md) has the sequence.
- `memory.rs` has in-memory fakes of the stores. There is no real database yet.

**Evaluation** (`extent.rs`, the heart of the project)
- `Extent` walks a rewrite tree recursively against the stores at a fixed revision. It never builds the full set of subjects; it short-circuits as soon as the answer is known.
- `decide` returns an `Outcome`: allowed, with the facts that prove it (its "grounds"), or denied.
- `contains` does the same walk without collecting proof. That's done with a `Proof` trait, implemented once for `Vec<Fact>` and once for `()`, so one generic walk serves both.
- `expand` returns one level of the tree.
- Cycles in the data, such as folders inside folders, end quietly. A walk through more than 100 subjectsets fails with an error.

**Decisions and replay** (`decision.rs`, `replay.rs`)
- A `Decision` records the question, the revision, the `SEMANTICS` version and the outcome with its grounds.
- `replay` re-judges a recorded decision at its revision and reports one of three results: `Matches`, `Differs`, or `SemanticsChanged` (the evaluation rules have changed since). This is the audit story.
- One limitation: theories aren't versioned yet, so replay reads the current theories.

**The edge** (`name.rs`, `resolve.rs`, `check.rs`)
- `name.rs` parses `theory:id#relation` once, splitting at the first `:` and the last `#`.
- `resolve.rs` maps names to ids. A name that was never interned is denied without touching the fact store.
- `check.rs` has `CheckService`, which does three things: resolve, decide, then append an `AuditRecord` to an `AuditLog` (in memory for now).

**Fixtures and the API**
- `fixture.rs` loads theories and facts from the JSON files under `tests/fixtures`. This JSON is a stopgap until the YAML theory format exists.
- `sungma-api` is an axum app with one route, `POST /check`. It loads the fixtures at startup and returns `{request_id, verdict, zookie}`. Errors are always JSON.

**Not built yet**
- Writes over the API (facts are only loaded from fixtures).
- A real store.
- Theory versioning.
- The YAML theory parser.
- RFC 9457 error bodies.
- Expand over HTTP.
- Authentication.

The [Sample](#sample) below (files, folders, groups) and its trace for "is alice a viewer of design.md?" are the best concrete walkthrough.

## Development

`rust-toolchain.toml` installs the stable toolchain with clippy, rustfmt and rust-analyzer. Enable the hooks once per clone. Before each commit they check formatting, lints, docs and tests; before each push, coverage, mutation testing of the branch's changes and the dependency audit, as CI does. The push checks need `cargo-llvm-cov`, `cargo-mutants` and `cargo-audit`.

```sh
git config core.hooksPath .githooks
```

## Check API

```sh
F=crates/sungma-core/tests/fixtures
cargo run -p sungma-api -- $F/docs.theories.json $F/docs.facts.json

curl -d '{"set": "file:design.md#viewer", "identity": "alice"}'   -H 'content-type: application/json' localhost:8080/check
# {"request_id":"sungma-1","verdict":"allowed","zookie":{"revision":16}}
```

The subject is an `identity` or a `subjectset`; an optional
`"zookie": {"revision": n}` asks for a revision at least that fresh.
`SUNGMA_ADDR` sets the listen address.

## Facts

A fact binds a subject to a subjectset. A theory names a kind of resource and declares its relations.

```ebnf
⟨fact⟩            ::= ⟨subjectset⟩ '@' ⟨subject⟩
⟨subject⟩         ::= ⟨identity⟩ | ⟨subjectset⟩ | ⟨resource member⟩
⟨resource member⟩ ::= ⟨resource⟩ '#' '...'
⟨subjectset⟩      ::= ⟨resource⟩ '#' ⟨relation name⟩
⟨resource⟩        ::= ⟨theory name⟩ ':' ⟨resource id⟩

⟨identity⟩        ::= opaque string
⟨resource id⟩     ::= opaque string
⟨theory name⟩     ::= ⟨name⟩
⟨relation name⟩   ::= ⟨name⟩
```

Identities and resource ids are owned by external systems, and Sungma puts no constraints on them. Because they may contain any delimiter, this grammar describes the model rather than a parseable text format.

## Rewrites

A relation's rewrite is an expression over the relations of its theory.

```ebnf
⟨rewrite⟩             ::= ⟨union⟩
⟨union⟩               ::= ⟨intersection⟩ { '|' ⟨intersection⟩ }
⟨intersection⟩        ::= ⟨exclusion⟩ { '&' ⟨exclusion⟩ }
⟨exclusion⟩           ::= ⟨term⟩ { '!' ⟨term⟩ }
⟨term⟩                ::= 'this'
                        | ⟨computed-subjectset⟩
                        | ⟨fact-to-subjectset⟩
                        | '(' ⟨rewrite⟩ ')'

⟨computed-subjectset⟩ ::= ⟨relation name⟩
⟨fact-to-subjectset⟩  ::= '(' ⟨factset⟩ ',' ⟨computed-subjectset⟩ ')'
⟨factset⟩             ::= ⟨relation name⟩

⟨relation name⟩       ::= ⟨name⟩     excluding 'this'
⟨name⟩                ::= ⟨name-start⟩ { ⟨name-char⟩ }
⟨name-start⟩          ::= ⟨letter⟩ | '_'
⟨name-char⟩           ::= ⟨letter⟩ | ⟨digit⟩ | '_'
⟨letter⟩              ::= 'a'…'z' | 'A'…'Z'
⟨digit⟩               ::= '0'…'9'
```

`!` (exclusion) binds tightest, then `&` (intersection), then `|` (union). Each level reads left to right, so `a | b & c` is `a | (b & c)` and `a ! b ! c` is `(a ! b) ! c`.

Each term names a set of subjects:

- `this` is the subjects of the facts stored under the subjectset in hand.
- A computed subjectset is a relation evaluated on the resource in hand.
- A fact-to-subjectset reads the facts under the factset, then evaluates the computed subjectset on each resource those facts name, under that resource's theory.

A relation declared without a rewrite is `this`. A relation the theory doesn't declare, such as one reached through a fact-to-subjectset whose target theory lacks it, names the empty set.

A theory is checked whole when it is declared. It is refused if a relation is declared twice, a union or intersection is empty, a rewrite nests more than 100 levels deep, a computed subjectset or factset names a relation the theory doesn't declare, or computed subjectsets form a cycle, such as `viewer: this ! viewer`. A cycle through a fact-to-subjectset, like folders inside folders, reads a fact at each step and is allowed.

## Sample

Three theories for documents: files sit in folders, folders sit in folders, and groups hold members.

```yaml
file:
  - owner
  - parent
  - editor: this | owner
  - viewer: (this | editor | (parent, viewer)) ! banned
  - auditor: this & viewer
  - banned

folder:
  - owner
  - parent
  - viewer: (this | (parent, viewer)) ! banned
  - banned

group:
  - member
```

Some facts under it, written in the fact notation above:

```text
group:eng#member@alice
group:eng#member@bob
folder:root#viewer@group:eng#member
folder:root#viewer@erin
folder:specs#parent@folder:root#...
folder:specs#banned@erin
file:design.md#parent@folder:specs#...
file:design.md#owner@carol
file:design.md#banned@bob
file:design.md#auditor@alice
file:design.md#auditor@dave
```

So for `file:design.md#viewer`: carol is a viewer through `editor` and `owner`, alice through the folders and group eng, bob is in eng but banned on the file, erin is a viewer of root but banned on specs, and dave is no viewer, so not an auditor either.

A trace: is alice a viewer of file:design.md?

```text
file:design.md#viewer   (this | editor | (parent, viewer)) ! banned
├─ this                → no direct facts                     false
├─ editor              → file:design.md#editor (this | owner) false
└─ (parent, viewer)    → parent fact names folder:specs
   folder:specs#viewer   (this | (parent, viewer)) ! banned
   └─ (parent, viewer) → folder:root
      folder:root#viewer
      └─ this          → subjectset group:eng#member
         group:eng#member → this: alice                       TRUE
      ! folder:root#banned  → false  ⇒ true
   ! folder:specs#banned    → false  ⇒ true
! file:design.md#banned     → false  ⇒ true
```
