# Architecture

Sungma is a Zanzibar-style authorization service. It answers one question: is subject S in the subjectset `namespace:resource#relation` at revision R? The [README](../README.md) lists its crates. Below is how they fit together, from bottom to top.

**The model** (`graph.rs`, `id.rs`, `name.rs`)
- A **fact** binds a subject to a subjectset, for example `carol` to `file:design.md#owner`. The subject is one of three things:
  - an identity (`carol`);
  - another subjectset (`group:eng#member`);
  - a resource itself (`folder:root`), which is how parent links are written.
- Namespace and relation names are interned to `u32` newtypes, `NamespaceId` and `RelationId`. These are like C# `readonly record struct NamespaceId(uint Value)`. Resource ids and identities stay names, `ResourceId` and `Identity`, each holding its text in an `Arc<str>` so a clone shares it.
- Each write is stamped with a `Revision` from the revision clock, and every read is "as of" a revision.

**Namespaces** (`namespace.rs`, `rewrite.rs`)
- A namespace declares the relations for one kind of resource. Each relation has a **rewrite**, a small expression tree. `Rewrite` is a Rust enum, which is a closed discriminated union; in C# it would be an abstract record with sealed subclasses. Its cases are:
  - `this`: the stored facts;
  - computed: another relation on the same resource;
  - fact-to-subjectset: follow a fact, such as `parent`, then evaluate a relation on the resource it names;
  - union, intersection and exclusion.
- `Namespace::new` checks a namespace whole when it is built, and refuses it with every problem it finds: duplicates, nesting deeper than 100 levels, undeclared names and computed cycles. So every `Namespace` is valid, and any namespace that gets stored is guaranteed to evaluate.

**Storage ports** (`store.rs` in `sungma` and `sungma-write`, `intern.rs`, the `sungma-memory` crate)
- The ports are async traits, which play the role of C# interfaces. Reads and writes are separate:
  - `Dictionary`: name to id, and back;
  - `FactStore`: keyed like a wide-column store, with the subjectset as partition key and the subject as sort key;
  - `NamespaceStore`: the rewrite for a namespace and relation;
  - `FactWriter`: a batch of inserts and deletes, applied atomically and stamped with one revision;
  - `NamespaceWriter`: creates a namespace, writes its next version, or retires it, and refuses the write unless the namespace is still at the version it was based on;
  - `Interner`: mints ids for new names, each in its pool: namespace names and relation names. Relations share one pool because `(parent, viewer)` means `viewer` in whichever namespace the parent fact names.
- Where a guarantee spans nodes, Sungma runs the protocol and a store supplies primitives. A fact keeps the revision that wrote it and the one that deleted it, so reads at any revision work on any store. `LeasingInterner` mints ids over a `NameStore`: each node leases a block of ids from a shared counter, and a conditional insert makes the first node to store a name win. [interner.md](interner.md) has the sequence.
- The `sungma-memory` crate has in-memory fakes of the stores. There is no real database yet.

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
- One limitation: namespaces aren't versioned yet, so replay reads the current namespaces.

**The edge** (`name.rs`; `resolve.rs` and `lib.rs` in `sungma-check`)
- `name.rs` parses `namespace:id#relation` once, splitting at the first `:` and the last `#`.
- `resolve.rs` maps namespace and relation names to ids. A name that was never interned is denied without touching the fact store. Resource ids and identities stay names.
- `sungma-check` has `CheckService`, which does three things: resolve, decide, then append an `AuditRecord` to an `AuditLog` (in memory for now).

**Languages and the API**
- `sungma-lang`'s `namespace.rs` parses and prints namespace documents: one namespace per JSON object, each relation's rewrite an expression such as `"(this | editor) ! banned"`. [specs/namespace-documents.md](specs/namespace-documents.md) specifies them.
- `sungma-lang`'s `namespace.rs` loads namespace documents, and its `fact.rs` loads fact documents, from the files under `crates/sungma/tests/fixtures`.
- `sungma-api` is an axum app with one route, `POST /check`. It loads the fixtures at startup and returns `{request_id, verdict, zookie}`. Errors are always JSON.

**Not built yet**
- Writes over the API (facts are only loaded from fixtures).
- A real store.
- Namespace versioning.
- RFC 9457 error bodies.
- Expand over HTTP.
- Authentication.

The [Sample](#sample) (files, folders, groups) and its trace for "is alice a viewer of design.md?" are the best concrete walkthrough.

## Facts

A fact binds a subject to a subjectset. A fact document is a JSON array of facts, and each part of a fact has its own key:

```json
[
  { "namespace": "file", "resource": "readme", "relation": "owner",
    "subject": { "identity": "anne@example.com" } },
  { "namespace": "folder", "resource": "root", "relation": "viewer",
    "subject": { "namespace": "group", "resource": "eng", "relation": "member" } },
  { "namespace": "file", "resource": "readme", "relation": "parent",
    "subject": { "namespace": "folder", "resource": "root" } }
]
```

- `namespace`, `resource` and `relation` name the subjectset, and `subject` holds the subject.
- A subject's keys say which kind it is: `identity` alone is an identity, `namespace` and `resource` are a resource, and `namespace`, `resource` and `relation` are a subjectset.
- Resources and identities are owned by external systems, such as emails, paths or URIs. They are printable ASCII, `!` to `~`.

[specs/fact-documents.md](specs/fact-documents.md) specifies fact documents.

## Rewrites

A relation's rewrite is an expression over the relations of its namespace.

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
- A fact-to-subjectset reads the facts under the factset, then evaluates the computed subjectset on each resource those facts name, under that resource's namespace.

A relation that takes only the facts bound to it is written `this`. A relation the namespace doesn't declare, such as one reached through a fact-to-subjectset whose target namespace lacks it, names the empty set.

A namespace is checked whole when it is declared. It is refused if a relation is declared twice, a rewrite nests more than 100 levels deep, a computed subjectset or factset names a relation the namespace doesn't declare, or computed subjectsets form a cycle, such as `viewer: this ! viewer`. A cycle through a fact-to-subjectset, like folders inside folders, reads a fact at each step and is allowed.

## Sample

Three namespaces for documents: files sit in folders, folders sit in folders, and groups hold members. Each is a namespace document:

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
  { "namespace": "group", "resource": "eng", "relation": "member", "subject": { "identity": "alice" } },
  { "namespace": "group", "resource": "eng", "relation": "member", "subject": { "identity": "bob" } },
  { "namespace": "folder", "resource": "root", "relation": "viewer",
    "subject": { "namespace": "group", "resource": "eng", "relation": "member" } },
  { "namespace": "folder", "resource": "root", "relation": "viewer", "subject": { "identity": "erin" } },
  { "namespace": "folder", "resource": "specs", "relation": "parent",
    "subject": { "namespace": "folder", "resource": "root" } },
  { "namespace": "folder", "resource": "specs", "relation": "banned", "subject": { "identity": "erin" } },
  { "namespace": "file", "resource": "design.md", "relation": "parent",
    "subject": { "namespace": "folder", "resource": "specs" } },
  { "namespace": "file", "resource": "design.md", "relation": "owner", "subject": { "identity": "carol" } },
  { "namespace": "file", "resource": "design.md", "relation": "banned", "subject": { "identity": "bob" } },
  { "namespace": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "alice" } },
  { "namespace": "file", "resource": "design.md", "relation": "auditor", "subject": { "identity": "dave" } }
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
