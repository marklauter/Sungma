---
title: Theory documents
type: specification
summary: "A theory document declares one or more namespaces in TDL, Theory Definition Language. An optional theory name prefixes unqualified namespace names. Each namespace is checked and stored on its own, under its full name."
status: evolving
cites:
  - "[[theory]]"
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[subjectset]]"
  - "[[fact]]"
  - "[[resource]]"
  - "[[subject]]"
  - "[[closure]]"
  - "[[identity]]"
  - "[[sungma-authorization]]"
---

# Theory documents

A theory document declares one or more namespaces in TDL, Theory Definition Language. Any set of namespaces is a theory. Each relation declares its constraints, the kinds of subject its facts may bind, and its rewrite. Files end in `.theory`.

A document is applied, like a DDL script: Sungma checks each namespace and stores it under its fully qualified name.

## Why a theory

<!-- final -->
In logic, a theory is a set of sentences closed under deduction. Here the sentences are rewrites. When rewrites are combined with fact tuples, a graph is formed from which each subjectset's closure may be deduced. [Checks](#checks) keep that true.
<!-- /final -->

## TDL

A relation reads like a signature: its name, a colon and its constraints, then `=` and its rewrite. With no `=`, it reads `this`. With no constraints, it's computed.

```ebnf
⟨document⟩    ::= [ 'theory' ':' ⟨namespace name⟩ ] ⟨namespace⟩ { ⟨namespace⟩ }
⟨namespace⟩   ::= 'namespace' ':' ⟨namespace name⟩ { ⟨relation⟩ }
⟨relation⟩    ::= ⟨relation name⟩ ':' ⟨constraints⟩ [ '=' ⟨rewrite⟩ ]
                | ⟨relation name⟩ '=' ⟨rewrite⟩
⟨constraints⟩ ::= ⟨constraint⟩ { ',' ⟨constraint⟩ }
```

Grammars here are EBNF: `{ }` repeats, `[ ]` is optional, and `'x'…'y'` is a character range.

- Whitespace, line breaks included, only separates tokens. A rewrite may start on the next line and wrap.
- A rewrite runs until the next signature (a name followed by `:` or `=`), the next `namespace:`, or the end. A constraint list ends where no comma follows.
- `//` starts a comment to the end of the line. Comments aren't kept.
- There are no quotes, escapes, numbers or booleans, so every token is a name, a keyword or punctuation.
- A document is UTF-8 without a byte-order mark. Outside comments, a character the grammar doesn't use is refused.
- A document declares at least one namespace.

## Names

A namespace name is one or more names joined by dots, such as `drive.file`. A relation name is one name. Names are case-sensitive.

```ebnf
⟨namespace name⟩ ::= ⟨name⟩ { '.' ⟨name⟩ }
⟨relation name⟩  ::= ⟨name⟩     excluding 'this', 'namespace' and 'theory' in any casing
⟨name⟩           ::= ( ⟨letter⟩ | '_' ) { ⟨letter⟩ | ⟨digit⟩ | '_' }
⟨letter⟩         ::= 'a'…'z' | 'A'…'Z'
⟨digit⟩          ::= '0'…'9'
```

Reserved:

- `this` is the rewrite keyword, in lowercase. No relation may be named `this` in any casing, so `This | owner` is refused.
- `namespace` and `theory` are TDL keywords, so no relation may be named either, in any casing. They may still name namespaces and appear in constraints.
- No namespace may be named `identity`.
- A first segment of `sungma` is reserved for Sungma's own namespaces, so `sungma.audit` is reserved and `sungmax` isn't.

Names are checked once, where they enter. Past that, only checked names exist.

## Constraints

A relation whose rewrite reads `this` declares its constraints. A fact whose subject satisfies none of them is refused, so bad data can't quietly empty a closure. A constraint is:

- `identity`: an identity, such as a person's or a client's `sub`;
- a namespace, such as `folder`: a resource of it, such as `folder:root#...`;
- `namespace#relation`, such as `group#member`: a subjectset of that relation.

```ebnf
⟨constraint⟩ ::= 'identity' | ⟨namespace name⟩ [ '#' ⟨relation name⟩ ]
```

A relation declares at least one constraint, none twice, in any order. No constraint admits every subject, so every link can be checked.

A relation whose rewrite doesn't read `this` is computed, and declares none.

## Rewrite grammar

```ebnf
⟨rewrite⟩             ::= ⟨union⟩
⟨union⟩               ::= ⟨intersection⟩ { '|' ⟨intersection⟩ }
⟨intersection⟩        ::= ⟨exclusion⟩ { '&' ⟨exclusion⟩ }
⟨exclusion⟩           ::= ⟨term⟩ { '!' ⟨term⟩ }
⟨term⟩                ::= 'this' | ⟨computed-subjectset⟩ | ⟨fact-to-subjectset⟩ | '(' ⟨rewrite⟩ ')'
⟨computed-subjectset⟩ ::= ⟨relation name⟩
⟨fact-to-subjectset⟩  ::= '(' ⟨factset⟩ ',' ⟨computed-subjectset⟩ ')'
⟨factset⟩             ::= ⟨relation name⟩
```

`!` (exclusion) binds tightest, then `&` (intersection), then `|` (union), each reading left to right: `a | b & c` is `a | (b & c)`, and `a ! b ! c` is `(a ! b) ! c`.

- A name is the longest run of name characters, so `thisone` is one name.
- A run of one operator is one node: `a | b | c` is one union of three.
- Parentheses are kept: `(a | b) | c` is a union whose first operand is a union.

## Meaning

A rewrite defines the closure of each subjectset formed from its relation:

- `this` is the subjects the facts bind to the subjectset.
- A computed subjectset is the closure of the named relation on the same resource.
- A fact-to-subjectset takes the subjects the factset binds on the same resource. For each that is a resource or a subjectset, it takes the closure of the followed relation on that subject's resource. Identities are skipped.

A subjectset whose relation isn't declared has an empty closure.

## The theory name

The theory name is a prefix. The parser adds it and a dot to each unqualified namespace name, in headers and in constraints. Under `theory: sungma`, `root` means `sungma.root`, and `group#member` means `sungma.group#member`.

- A name that already starts with the theory name and a dot gets no prefix: under `theory: t`, `a` and `t.a` both name `t.a`.
- `identity` is never prefixed, and rewrites name only relations, so the prefix never touches them.
- Without a theory name, a document holds full names.
- A theory name follows the namespace name grammar.

Once parsed, the prefix is gone. Everything sees full names, and nothing records which document a namespace came from. A theory shows only in the shared first segment of its namespaces' names.

## Sample

[Sungma's own authorization](sungma-authorization.md#the-model):

```c
theory: sungma

// The deployment, one resource: sungma.root:sungma
namespace: root
  owner: identity
  admin: identity = this | owner
  namespace_creator: identity, group#member = this | admin
  group_creator: identity, group#member = this | admin
  operator: identity, group#member = this | admin
  auditor: identity, group#member = this | admin
  reader: identity, group#member = this | auditor
  writer: identity, group#member = this | admin
  fact_writer: identity, group#member = this | admin
  fact_reader: identity, group#member
    = this | fact_writer | auditor
  directory_reader: identity, group#member
    = this | auditor
  access_reader = auditor

namespace: namespace
  parent: root
  protected: root
  owner: identity, group#member = this | (parent, owner) ! (protected, owner)
  admin: identity, group#member = this | owner | (parent, admin) ! (protected, admin)
  writer: identity, group#member = this | admin | (parent, writer) ! (protected, writer)
  reader: identity, group#member = this | writer | fact_reader | auditor | (parent, reader)
  fact_writer: identity, group#member
    = this | admin | (parent, fact_writer) ! (protected, fact_writer)
  fact_reader: identity, group#member
    = this | fact_writer | auditor | (parent, fact_reader)
  auditor: identity, group#member
    = this | (parent, auditor)
  access_reader = admin | auditor

namespace: relation
  parent: namespace
  writer: identity, group#member = this | (parent, fact_writer)
  reader: identity, group#member = this | writer | (parent, fact_reader)

namespace: group
  parent: root
  owner: identity, group#member = this | (parent, admin)
  member: identity, group#member
  access_reader = owner | (parent, auditor)
```

## Checks

A document is refused, and declares nothing, if it breaks a rule of TDL, declares one namespace twice by its full name, or holds a namespace that fails a check.

A namespace is refused if:

- a relation is declared twice;
- a rewrite nests more than 100 levels (a leaf or a run of `|` or `&` is one level, and each `!` adds one), or groups more than 100 parentheses deep;
- a computed subjectset or a factset names an undeclared relation;
- a rewrite reads `this` without constraints, or a relation declares constraints without reading `this`;
- a constraint `namespace#relation` names a relation that namespace doesn't declare;
- a fact-to-subjectset's followed relation isn't declared by every namespace in its factset's constraints: with `parent: folder`, `(parent, viewer)` needs `folder` to declare `viewer`;
- computed subjectsets form a cycle, such as `viewer = editor` with `editor = viewer`. A cycle through a fact-to-subjectset, such as folders in folders, is allowed.

A reference to another namespace in the same document is checked against the document, in either order. A reference outside it is checked against that namespace's latest version.

## Limits

- A document: 4 MiB. That bounds its namespaces too.
- A namespace: 500 relations.
- A relation: 32 constraints.
- A full namespace name: 256 bytes.
- A relation name: 64 bytes.
- A rewrite: 4 KiB as written, comments included, measured before parsing.

## Errors

Errors come in document order, up to 50, then one that says there were more. Each carries a line, a column counted in characters from 1, and the namespace's full name. A whole-namespace problem, such as a cycle, is reported at its relation.

One mistake reports one error:

- A syntax error stops its rewrite, and parsing goes on with the next relation.
- A relation whose rewrite fails is still declared, so references to it don't dangle.
- Relations that reach each other report one cycle, at the first one the check meets, with a path back to it.

Each error is a variant of one enum, carrying the names involved. There are no string codes.

An error quotes at most 64 bytes, cut at a character boundary and marked as cut, with control and invisible or reordering characters, such as U+202E and U+200B, escaped. Errors reach logs, terminals and UIs, where a raw quote could forge a line, recolor a terminal or reorder text.

## Printing

The printer writes namespaces as a document that parses back as equal namespaces, and the same namespaces always print as the same text.

- Given a theory name, it writes it once at the top and takes it off each namespace name. Every namespace must start with it. A name that would still start with it once taken off prints in full.
- Namespaces print sorted by full name, then relations by name, then constraints, each comparing bytes. An author's order, comments and line breaks are lost.
- Each relation is one line, indented two spaces, with one space after each colon and comma and around `=` and each operator. `= this` is left out.
- Parentheses print only where the grammar needs them: a union inside a union, a union or intersection inside an intersection or on the left of `!`, and any operator on the right of `!`.
- Lines end in `\n`.
- It refuses, with a `PrintError`, what the parser would refuse: a rewrite printing past 4 KiB (a dense one can, once spaced out), a document past 4 MiB, an invalid or reserved name, and a union or intersection with one operand.

Author, Sungma's UI, shows and edits namespaces in TDL.

## Open

1. **Names outside the theory.** Under a theory name, every unqualified name gets the prefix, so a document can't name `file` or `drive.file` outside it.
2. **Writing a document.** One write per namespace, in turn, or one atomic write, which would come with [write ids](../notes/writes-commit-through-a-write-id.md).
3. **Namespaces that reference each other.** Neither can be written first while a reference to a missing namespace is refused.
4. **The theory name.** Whether it must be one segment, and whether only Sungma releases write in the `sungma` theory.

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents.

#### Documents

1. The sample parses to Sungma's four namespaces under their full names.
2. An empty document, or one of only comments, is refused. A namespace without relations parses.
3. No `=` reads `this`; no constraints is computed.
4. A rewrite on its signature's line, on the next line, or wrapped parses the same.
5. A comment at the end of any line changes nothing. A `#` outside a constraint is refused.
6. A quote, an escape, a number, a NUL, a byte-order mark, or any other character outside the grammar is refused.
7. A rewrite left open, or a missing namespace name, doesn't swallow the next signature or `namespace:`.
8. A document with one bad namespace declares none.

#### Theory name

1. The prefix applies to headers and constraints, never to `identity` or rewrites.
2. A name already starting with the theory name keeps it, and a document without one holds full names.

#### Names

1. Names follow the grammar. `.a`, `a.`, `a..b` and `a.1b` are refused.
2. `this`, `namespace` and `theory` in any casing are refused as relation names; `namespace` and `theory` are allowed as namespace names and in constraints.
3. `identity` is refused as a namespace name. The `sungma` first segment is enforced at write time, as [Sungma's own authorization](sungma-authorization.md#protected-namespaces) says, so the parser reads Sungma's own namespaces.
4. Checked names refuse `"`, `:`, `#`, whitespace and control characters.

#### Grammar

1. Precedence, left-to-right reading, runs as one node, kept parentheses, and longest-match names.
2. A syntax error names what was expected and what was found.

#### Checks

1. Each check in [Checks](#checks) refuses what it should, including a relation that excludes itself, and allows cycles through a fact-to-subjectset and relations that share a target.
2. An empty constraint list, an entry twice, or a malformed entry is refused.
3. A fact whose subject satisfies no constraint, or under a computed relation, is refused.
4. A dangling target is reported once per relation that names it, and a set of relations that reach each other is one cycle error with a path.
5. Redeclaring a relation checks the whole namespace again.

#### Errors

1. Errors come in document order and stop at the cap.
2. Lines and columns are right after multibyte characters and wrapped rewrites, and each error names its namespace.
3. Quotes are bounded and escaped.
4. A bad constraint reports one error, without reading a later constraint as a relation.

#### Limits

1. Each limit at its value and one past it, including 100 levels and 100 parentheses, a rewrite counting its comments, and a 256-byte dotted name.

#### Printing

1. The canonical form, `= this` left out, sorted constraints, and only needed parentheses.
2. The theory name is written once and taken off; a namespace without it is refused; a name that would still start with it prints in full.
3. Each refusal, and a document of exactly 4 MiB is written.

#### Round trips

1. Generated namespaces, with and without a theory name, reaching both depth limits, with wide runs and every kind of constraint: print, parse and print again gives equal namespaces and the same text.
2. Rewrite text generated from the grammar's tokens parses, prints and parses to an equal tree.

#### Hostile input

1. A chain of 500 computed subjectsets is accepted and a longer one refused; a large set of mutually reaching relations and far more relations than the limit are refused. Each finishes without a crash, within a bound that catches quadratic work.
2. A namespace built in code, past the document limits, passes the same checks without a crash.
3. A fuzz target and a CI property test run the parser on arbitrary input without a panic, stack overflow or abort.
4. Mutation testing leaves no surviving mutant in the parser, the printer or the checks.
