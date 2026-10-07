---
title: Theory documents
type: specification
summary: "A theory document is the JSON form of one theory: an object keyed by the theory's name, mapping each relation name to a rewrite written as an expression string."
status: evolving
cites:
  - "[[theory]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[subjectset]]"
  - "[[fact]]"
  - "[[resource]]"
  - "[[subject]]"
  - "[[closure]]"
  - "[[identity]]"
---

# Theory documents

A theory document is the JSON form of one [[theory]]. It maps the theory's name to its [[relation]]s, and each relation's name to a [[rewrite]] written as an expression string. A theory is the unit a write declares, so a document holds exactly one. Sungma parses a document into a typed rewrite tree for each relation, checks the theory whole, and prints a theory back into a document that parses as an equal theory.

## Sample

The `file` theory: files sit in folders, and `(parent, viewer)` reads the viewers of the folder a file sits in, under the `folder` theory.

```json
{
  "file": {
    "owner": "this",
    "parent": "this",
    "editor": "this | owner",
    "viewer": "(this | editor | (parent, viewer)) ! banned",
    "auditor": "this & viewer",
    "banned": "this"
  }
}
```

## Document

- A document is one JSON object with exactly one key, the theory's name. Its value is an object mapping the theory's relation names to their expressions.
- Every expression is a string. A relation that takes only the facts bound to it is written `"owner": "this"`. A relation whose value is anything other than a non-empty string, such as `null`, `""`, a number or an object, is refused.
- A theory without relations is written `{}`. Any other value for the theory, such as `null` or `[]`, is refused.
- A document declares exactly one theory. An empty document or `{}` declares nothing, and an object with two or more keys declares more than one, so both are refused as parse errors. An API that takes a document answers either as a bad request.
- An object with the same key twice is refused, including a relation named twice. JSON leaves duplicate keys to the reader, and a reader that keeps the last one would hide a mistake.
- Key order carries no meaning.
- Names and expressions, and their length limits, are checked after JSON escapes are decoded, so `"\u006fwner"` is the name `owner`.

## Limits

A document is refused if it breaks any of these limits:

- A document is at most 4 MiB. The other limits allow about 2.1 MB of compact JSON; the rest is room for whitespace and escapes.
- A theory declares at most 500 relations.
- A theory or relation name is at most 64 bytes.
- An expression is at most 4 KiB (4,096 bytes).

The rewrite depth limit is under [Checks](#checks).

## Names

A relation name is one name. A theory name is one or more names joined by dots, such as `file` or `drive.file`; the dots carry no meaning. A theory name is also the theory part of a [[resource]] name, as `file` is in `file:design.md`. The grammars in this spec are EBNF: `::=` defines, `|` alternates, `{ }` repeats zero or more times, quoted text is literal, and `'x'…'y'` is an inclusive character range.

```ebnf
⟨theory name⟩   ::= ⟨name⟩ { '.' ⟨name⟩ }
⟨relation name⟩ ::= ⟨name⟩     excluding 'this'
⟨name⟩          ::= ⟨name-start⟩ { ⟨name-char⟩ }
⟨name-start⟩    ::= ⟨letter⟩ | '_'
⟨name-char⟩     ::= ⟨letter⟩ | ⟨digit⟩ | '_'
⟨letter⟩        ::= 'a'…'z' | 'A'…'Z'
⟨digit⟩         ::= '0'…'9'
```

Names are case-sensitive: `Owner` and `owner` are two different names. Nothing is lowercased.

The keyword is `this`, in lowercase only. No relation may be named `this` in any casing, so `This` and `THIS` are refused as names. A name that differs from the keyword only in case would read as the keyword. The core accepts these names; the document reserves them.

## Rewrite grammar

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
```

The grammar defines three operators. In binding order, they are:

- `!` exclusion, which binds tightest
- `&` intersection
- `|` union

So `a | b & c` is `a | (b & c)`. Each level reads left to right, so `a ! b ! c` is `(a ! b) ! c`.

The following rules apply on top of the grammar:

- Whitespace, including line breaks, ends a token and is otherwise ignored.
- A name is the longest run of name characters, so `thisone` is a name and not `this` followed by `one`.
- A parenthesized pair with a comma is a fact-to-subjectset. Parentheses around anything else group.
- A run of one operator parses as a single node with one operand per term, so `a | b | c` is one union of three.
- Parentheses are kept as structure. `(a | b) | c` is a union whose first operand is a union, not a union of three.

## Meaning

A relation's rewrite defines the [[closure]] of each [[subjectset]] formed from that relation. Each term contributes a set of [[subject]]s, taken for the subjectset in hand:

- `this` is the subjects the [[fact]]s bind to the subjectset.
- A computed subjectset is the closure of the subjectset formed from the same resource and the named relation.
- A fact-to-subjectset reads the subjects the facts bind to the subjectset formed from the same resource and the factset. For each subject that is a resource member or a subjectset, it takes the closure of the subjectset formed from that subject's resource and the computed relation. An [[identity]] has no resource and is skipped.

A subjectset whose relation its theory doesn't declare has an empty closure. A fact-to-subjectset reaches one when the theory of a resource it follows lacks the computed relation.

## Checks

A theory is checked whole when it is declared, and is refused if any of the following holds:

- A relation is declared twice.
- A union or intersection is empty. The grammar can't produce one, but a rewrite tree from another source can.
- A rewrite nests more than 100 levels deep. A leaf is one level, a run of `|` or `&` is one level however many operands it has, and each `!` adds a level. Grouping parentheses nested more than 100 deep are also refused, before the tree is built.
- A computed subjectset, or the factset of a fact-to-subjectset, names a relation the theory doesn't declare. The computed relation of a fact-to-subjectset belongs to the theory of the resource each fact binds, so it isn't checked.
- Computed subjectsets form a cycle, such as `"viewer": "editor"` with `"editor": "viewer"`, or `"viewer": "this ! viewer"`. A cycle through a fact-to-subjectset, such as folders inside folders, reads a fact at each step and is allowed.

## Errors

The parser reports every problem in a document, not only the first, in document order. Each error carries the line and column where its problem starts. A problem found only by checking a whole theory, such as a cycle, is reported at the relation where it is found.

The parser refuses a document if it finds any problem, and declares nothing from it.

Each error is a position and a kind. The kind is a variant of one enum with a variant for each problem this spec names, such as a relation whose value isn't a string, a reserved name, a broken limit, a rewrite syntax error, a dangling reference or a cycle. A variant carries the names involved. Callers match on the variant. There are no string codes.

## Printing

The printer writes a document the parser reads back as an equal theory.

- Every relation prints as `"name": "expression"`, including `"name": "this"`.
- A theory without relations prints as `"name": {}`.
- Operators print with a space on each side, and a fact-to-subjectset prints as `(factset, computed)`.
- The printer adds parentheses only where the grammar needs them:
  - A union inside a union is parenthesized.
  - A union or intersection inside an intersection is parenthesized.
  - A union or intersection on the left of `!` is parenthesized.
  - Any operator node on the right of `!` is parenthesized.
- Relations print sorted by name, comparing bytes. Order carries no meaning, so a theory holds its relations unordered, and the printer sorts them so one theory always prints as the same text.
- Objects indent by two spaces, one key per line, and lines end in `\n` on every platform.
- Printing a relation named `this`, or a rewrite that references one, is a caller defect.
