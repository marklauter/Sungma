# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a ReBAC. WIP.

## Development

`rust-toolchain.toml` installs the stable toolchain with clippy, rustfmt and rust-analyzer. Enable the pre-commit hook, which checks formatting and lints as CI does, once per clone:

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
