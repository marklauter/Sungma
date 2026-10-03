# Sungma

Sungma: spirits bound by oath to protect the sacred teachings of the Dharma.

Sungma is a ReBAC. WIP.

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

For example:

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
```
