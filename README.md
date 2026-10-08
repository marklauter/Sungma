![Sungma](https://raw.githubusercontent.com/marklauter/Sungma/main/assets/logo/sungma-sml.png "Sungma")

# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a ReBAC. WIP.

## Architecture

Sungma is a Zanzibar-style authorization service. It answers one question: is subject S in set `resource#relation` at revision R? It has three crates: `sungma`, a pure core library; `sungma-lang`, which reads theories and facts from text; and `sungma-api`, a thin HTTP shell around them. Below is how it fits together, from bottom to top.

**The model** (`model.rs`)
- A **fact** binds a subject to a subjectset, for example `carol` to `file:design.md#owner`. The subject is one of three things:
  - an identity (`carol`);
  - another subjectset (`group:eng#member`);
  - a resource itself (`folder:root`), which is how parent links are written.
- Every string is interned to a `u32` newtype such as `TheoryId` or `RelationId`. These are like C# `readonly record struct TheoryId(uint Value)`. Past the edge, the core never handles a string.
- Each write bumps a global `Revision`, and every read is "as of" a revision.

**Theories** (`theory.rs`, `rewrite.rs`)
- A theory declares the relations for one kind of resource. Each relation has a **rewrite**, a small expression tree. `Rewrite` is a Rust enum, which is a closed discriminated union; in C# it would be an abstract record with sealed subclasses. Its cases are:
  - `this`: the stored facts;
  - computed: another relation on the same resource;
  - fact-to-subjectset: follow a fact, such as `parent`, then evaluate a relation on the resource it names;
  - union, intersection and exclusion.
- `Theory::new` checks a theory whole when it is built, and refuses it with every problem it finds: duplicates, empty operators, nesting deeper than 100 levels, undeclared names and computed cycles. So every `Theory` is valid, and any theory that gets stored is guaranteed to evaluate.

**Storage ports** (`store.rs`, `memory.rs`, `intern.rs`)
- The ports are async traits, which play the role of C# interfaces. Reads and writes are separate:
  - `Dictionary`: name to id, and back;
  - `FactStore`: keyed like a wide-column store, with the subjectset as partition key and the subject as sort key;
  - `TheoryStore`: the rewrite for a theory and relation;
  - `FactWriter`: a batch of inserts and deletes, applied atomically at the next revision;
  - `Interner`: mints ids for new names, each in its pool: theory names, relation names, the resource ids of each theory, and identities. Relations share one pool because `(parent, viewer)` means `viewer` in whichever theory the parent fact names.
- Where a guarantee spans nodes, Sungma runs the protocol and a store supplies primitives. A fact keeps the revision that wrote it and the one that deleted it, so reads at any revision work on any store. `LeasingInterner` mints ids over a `NameStore`: each node leases a block of ids from a shared counter, and a conditional insert makes the first node to store a name win. [docs/interner.md](docs/interner.md) has the sequence.
- `memory.rs` has in-memory fakes of the stores. There is no real database yet.

**Evaluation** (`closure.rs`, the heart of the project)
- `Closure` walks a rewrite tree recursively against the stores at a fixed revision. It never builds the full set of subjects; it short-circuits as soon as the answer is known.
- `decide` returns an `Outcome`: allowed, with the facts that prove it (its "grounds"), or denied.
- `contains` does the same walk without collecting proof. That's done with a `Proof` trait, implemented once for `Vec<Fact>` and once for `()`, so one generic walk serves both.
- `expand` returns one level of the tree.
- Cycles in the data, such as folders inside folders, end quietly. A walk through more than 100 subjectsets ends the whole check with an error, since the limit caps the work one check may do.
- Store errors follow Kleene logic. A failed read is an unknown verdict, and an operand that settles the result outweighs it: a true union operand, a false intersection operand, or a false base or true excluded side of an exclusion. The verdict doesn't depend on evaluation order. The check fails only when nothing settles the result.

**Decisions and replay** (`decision.rs`, `replay.rs`)
- A `Decision` records the question, the revision, the `SEMANTICS` version and the outcome with its grounds.
- `replay` re-judges a recorded decision at its revision and reports one of four results: `Matches`, `Regrounded` (the same verdict on other grounds), `Differs`, or `SemanticsChanged` (the evaluation rules have changed since). This is the audit story.
- One limitation: theories aren't versioned yet, so replay reads the current theories.

**The edge** (`name.rs`, `resolve.rs`, `check.rs`)
- `name.rs` parses `theory:id#relation` once, splitting at the first `:` and the last `#`.
- `resolve.rs` maps names to ids. A name that was never interned is denied without touching the fact store.
- `check.rs` has `CheckService`, which does three things: resolve, decide, then append an `AuditRecord` to an `AuditLog` (in memory for now).

**Languages and the API**
- `sungma-lang`'s `theory.rs` parses and prints theory documents: one theory per JSON object, each relation's rewrite an expression such as `"(this | editor) ! banned"`. [docs/specs/theory-documents.md](docs/specs/theory-documents.md) specifies them.
- `sungma-lang`'s `theory.rs` loads theory documents, and its `fact.rs` loads fact documents, from the files under `crates/sungma/tests/fixtures`.
- `sungma-api` is an axum app with one route, `POST /check`. It loads the fixtures at startup and returns `{request_id, verdict, zookie}`. Errors are always JSON.

**Not built yet**
- Writes over the API (facts are only loaded from fixtures).
- A real store.
- Theory versioning.
- RFC 9457 error bodies.
- Expand over HTTP.
- Authentication.

The [Sample](#sample) below (files, folders, groups) and its trace for "is alice a viewer of design.md?" are the best concrete walkthrough.

## Development

`rust-toolchain.toml` installs the stable toolchain with clippy, rustfmt and rust-analyzer. Enable the hooks once per clone. Before each commit they check formatting, lints, docs and tests; before each push, coverage, mutation testing of the branch's changes and the dependency audit, as CI does. The push checks need `cargo-llvm-cov`, `cargo-mutants` and `cargo-audit`.

```sh
git config core.hooksPath .githooks
```

`fuzz/` holds `cargo-fuzz` targets for the theory and fact document parsers. CI runs each for a minute; locally they need the nightly toolchain:

```sh
cargo +nightly fuzz run document
cargo +nightly fuzz run fact
```

## Check API

```sh
F=crates/sungma/tests/fixtures
cargo run -p sungma-api -- $F/docs.facts.json $F/theories/*.json

curl -d '{"set": "file:design.md#viewer", "identity": "alice"}'   -H 'content-type: application/json' localhost:8080/check
# {"request_id":"sungma-1","verdict":"allowed","zookie":{"revision":16}}
```

The subject is an `identity` or a `subjectset`; an optional
`"zookie": {"revision": n}` asks for a revision at least that fresh.
`SUNGMA_ADDR` sets the listen address.

## Facts

A fact binds a subject to a subjectset. A fact document is a JSON array of facts, and each part of a fact has its own key:

```json
[
  { "theory": "file", "resource": "readme", "relation": "owner",
    "subject": { "identity": "anne@example.com" } },
  { "theory": "folder", "resource": "root", "relation": "viewer",
    "subject": { "theory": "group", "resource": "eng", "relation": "member" } },
  { "theory": "file", "resource": "readme", "relation": "parent",
    "subject": { "theory": "folder", "resource": "root" } }
]
```

- `theory`, `resource` and `relation` name the subjectset, and `subject` holds the subject.
- A subject's keys say which kind it is: `identity` alone is an identity, `theory` and `resource` are a resource, and `theory`, `resource` and `relation` are a subjectset.
- Resources and identities are owned by external systems, such as emails, paths or URIs. Any character but whitespace is allowed.

[docs/specs/fact-documents.md](docs/specs/fact-documents.md) specifies fact documents.

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

A relation that takes only the facts bound to it is written `this`. A relation the theory doesn't declare, such as one reached through a fact-to-subjectset whose target theory lacks it, names the empty set.

A theory is checked whole when it is declared. It is refused if a relation is declared twice, a union or intersection is empty, a rewrite nests more than 100 levels deep, a computed subjectset or factset names a relation the theory doesn't declare, or computed subjectsets form a cycle, such as `viewer: this ! viewer`. A cycle through a fact-to-subjectset, like folders inside folders, reads a fact at each step and is allowed.

## Sample

Three theories for documents: files sit in folders, folders sit in folders, and groups hold members. Each is a theory document:

```json
{
  "file": {
    "auditor": "this & viewer",
    "banned": "this",
    "editor": "this | owner",
    "owner": "this",
    "parent": "this",
    "viewer": "(this | editor | (parent, viewer)) ! banned"
  }
}

{
  "folder": {
    "banned": "this",
    "owner": "this",
    "parent": "this",
    "viewer": "(this | (parent, viewer)) ! banned"
  }
}

{
  "group": {
    "member": "this"
  }
}
```

Some facts under it, as a fact document (`crates/sungma/tests/fixtures/docs.facts.json` holds them all):

```json
[
  { "theory": "group", "resource": "eng", "relation": "member", "subject": { "identity": "alice" } },
  { "theory": "group", "resource": "eng", "relation": "member", "subject": { "identity": "bob" } },
  { "theory": "folder", "resource": "root", "relation": "viewer",
    "subject": { "theory": "group", "resource": "eng", "relation": "member" } },
  { "theory": "folder", "resource": "root", "relation": "viewer", "subject": { "identity": "erin" } },
  { "theory": "folder", "resource": "specs", "relation": "parent",
    "subject": { "theory": "folder", "resource": "root" } },
  { "theory": "folder", "resource": "specs", "relation": "banned", "subject": { "identity": "erin" } },
  { "theory": "file", "resource": "design.md", "relation": "parent",
    "subject": { "theory": "folder", "resource": "specs" } },
  { "theory": "file", "resource": "design.md", "relation": "owner", "subject": { "identity": "carol" } },
  { "theory": "file", "resource": "design.md", "relation": "banned", "subject": { "identity": "bob" } },
  { "theory": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "alice" } },
  { "theory": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "dave" } }
]
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
