---
title: Theory documents
type: specification
summary: "A theory document declares one or more namespaces in one text. Its theory name is a prefix the parser adds to unqualified namespace names, and nothing else: each namespace is checked and stored on its own, under its full name. It's written in Sungma schema language, a form for people."
status: evolving
cites:
  - "[[theory]]"
  - "[[namespace]]"
  - "[[relation]]"
  - "[[rewrite]]"
  - "[[subject]]"
  - "[[identity]]"
  - "[[sungma-authorization]]"
---

# Theory documents

A theory document declares one or more [[namespace]]s in one text, usually the namespaces of one [[theory]]. A [namespace document](namespace-documents.md) holds exactly one namespace. A theory document lets people read and write related namespaces together, such as the four of [Sungma's own authorization](sungma-authorization.md), which only make sense together.

A theory document is notation. Sungma parses it into namespaces and checks and stores each one as a namespace document says. Nothing stores the document or the theory.

## Sample

Sungma's own model, in Sungma schema language:

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

## The theory name

The theory name is a prefix. The parser adds it, and a dot, to each unqualified namespace name in the document: the name in each namespace's header, and the namespace part of each constraint. In the sample, `root` means `sungma.root`, and `group#member` means `sungma.group#member`.

- A name that already starts with the theory name and a dot is a full name, and gets no prefix. Under `theory: t`, both `a` and `t.a` name `t.a`.
- `identity` is never prefixed. It names an [[identity]], not a namespace.
- Rewrites name only [[relation]]s, never namespaces, so the prefix never touches a [[rewrite]].
- A document without a theory name holds full names.
- The theory name follows the namespace name grammar in [Namespace documents](namespace-documents.md#names).

Once parsed, nothing remembers the prefix. Each namespace is checked, stored and read under its full name, and checks, facts and storage see only full names. Nothing records which document or theory a namespace came from. The theory shows only in the shared first segment of its namespaces' names.

## Sungma schema language

JSON suits the wire, but a person can't reason about a theory written in it. Keys and quotes bury the rewrites, and a relation's constraints and rewrite land on separate lines. Sungma schema language is the form written for people. It parses to the same namespaces as the JSON form.

Each relation reads like a signature: its name, a colon and its constraints, the kinds of [[subject]] its facts may bind, then its rewrite after `=`.

- A relation with constraints and no `=` reads `this`, as `owner: identity` does.
- A relation with `=` and no constraints is computed, as `access_reader = auditor` is.
- A relation with both follows [Constraints](namespace-documents.md#constraints): its rewrite has to read `this`.
- Constraints are separated by commas, at least one, none twice, each as the namespace document spec defines it.
- The rewrite follows the [rewrite grammar](namespace-documents.md#rewrite-grammar), unchanged.

```ebnf
⟨document⟩  ::= [ 'theory' ':' ⟨namespace name⟩ ] ⟨namespace⟩ { ⟨namespace⟩ }
⟨namespace⟩ ::= 'namespace' ':' ⟨namespace name⟩ { ⟨relation⟩ }
⟨relation⟩  ::= ⟨relation name⟩ ':' ⟨constraints⟩ [ '=' ⟨rewrite⟩ ]
              | ⟨relation name⟩ '=' ⟨rewrite⟩
⟨constraints⟩  ::= ⟨constraint⟩ { ',' ⟨constraint⟩ }
```

The rules on top of the grammar:

- Whitespace, line breaks included, only separates tokens. Indentation is for readers. A rewrite can start on its signature's line or on the next one, `=` included, and can wrap onto as many lines as it needs.
- A rewrite runs until the next relation's signature, a name followed by `:` or `=`, or until the next `namespace:` or the end. A constraint list ends where no comma follows. A relation needs no terminator.
- `namespace:` opens a namespace, and its relations run until the next `namespace:` or the end of the document.
- `theory` and `namespace` are reserved relation names, in every form, as `this` is. Otherwise `namespace: root` could be a relation named `namespace` constrained to `root`. Reserving them in every form keeps every namespace printable in this one. They stay usable as namespace names, so `sungma.namespace` and `parent: namespace` are fine.
- `//` starts a comment that runs to the end of the line. `#` can't start one, because subjectsets use it. Comments aren't kept.
- There are no quotes, escapes, numbers or booleans. Every token is a name, a keyword or punctuation, so the parser never guesses a value's type, and `no` or `on` is only ever a name.
- A document is UTF-8 without a byte-order mark. Outside comments, any character the grammar doesn't use is refused.

A document with one namespace and no theory name is a namespace document in this form.

## Checks

A theory document is refused if:

- it breaks a rule of Sungma schema language;
- it declares one namespace twice, by its full name;
- any namespace in it fails a check in [Namespace documents](namespace-documents.md#checks).

A reference from one namespace to another in the same document, through a constraint or a fact-to-subjectset, is checked against the document. The two namespaces don't have to be written in a particular order. A reference to a namespace outside the document is checked as a namespace document's would be.

A refused document declares nothing.

## Limits

The [Limits](namespace-documents.md#limits) of a namespace document apply to each namespace a theory document holds:

- Each relation's rewrite is at most 4 KiB as written, from its first token to its last, comments between them included, as a namespace document's expression string is. It's measured before it's parsed.
- A full namespace name, the theory name and a dot included, is at most 256 bytes.

A theory document is at most 4 MiB. That also bounds how many namespaces it holds, so there's no separate count.

## Errors

Errors follow [Namespace documents](namespace-documents.md#errors): problems in document order, up to 50, with quotes bounded and escaped. Each error also carries the namespace it was found in, by its full name.

In Sungma schema language, an error carries a line and a column in the document itself. A rewrite isn't inside a string, so its columns are the document's columns.

## Printing

The printer writes a set of namespaces as a theory document that the parser reads back as equal namespaces.

- Given a theory name, the printer takes it off every namespace name that starts with it and a dot, and writes it once at the top. Every namespace it's given has to start with it. A name that would still start with the theory name once it's taken off prints in full, so it reads back as the same name.
- The printer refuses a rewrite that prints longer than 4 KiB, as a dense one spaced out can, and a document longer than 4 MiB, since the parser would refuse either.
- Namespaces print sorted by full name, comparing bytes. Within a namespace, relations print sorted by name, comparing bytes, as in a namespace document. Alphabetical order is the easiest for readers to find a relation in. An author's own order and line breaks are lost when a document is printed again.
- In Sungma schema language, each relation prints on one line, indented two spaces, with one space after each colon and comma and around `=` and each operator, and no padding to line up columns. A blank line comes before each namespace. Constraints print sorted, comparing bytes, and parentheses print only where the grammar needs them, as in a namespace document.

Author, Sungma's UI, shows and edits namespaces in Sungma schema language. Sungma stores each namespace in the JSON form of a namespace document.

## Open

1. **Names from outside the theory.** With a theory name, every name that doesn't start with it gets it as a prefix, so a document can't name a namespace outside the theory, such as `file` or `drive.file`. That needs a rule: a mark for "outside this theory", or no outside names in a theory document.
2. **Writing a theory document.** Whether it's one write per namespace, sent in turn, or one write that stores all of them atomically. One write per namespace needs nothing new. Atomic writes of any size would come with [write ids](../notes/writes-commit-through-a-write-id.md).
3. **A JSON form.** Whether a theory document has a JSON form, and how the printer writes one.
4. **The theory name.** Whether it must be one segment, and whether only Sungma releases write documents in the reserved `sungma` theory.

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents. The classes in [Namespace documents](namespace-documents.md#required-tests) apply to Sungma schema language. These cases are required on top of them:

1. The sample parses to the four namespaces of [Sungma's own authorization](sungma-authorization.md#the-model), under their full names.
2. The theory name prefixes namespace headers and the namespace part of constraints, and never `identity` or anything in a rewrite.
3. A document without a theory name holds full names, and one with a single namespace parses to the same namespace as its namespace document.
4. A rewrite on its signature's line, on the next line, and wrapped over several lines parse to equal namespaces.
5. A relation with constraints and no `=` reads `this`, and a relation with `=` and no constraints is computed.
6. `theory` and `namespace` are refused as relation names, in every form, and allowed as namespace names and in constraints.
7. Comments are allowed at the end of any line and change nothing. A `#` outside a constraint is refused.
8. A quote, an escape, a number, a byte-order mark, and any character outside the grammar are refused.
9. A namespace declared twice, by an unqualified name and by its full name, is refused.
10. A reference between two namespaces in one document is checked against the document, in either order.
11. A document with one bad namespace declares none of its namespaces.
12. An error in Sungma schema language carries the document's line and column, after multibyte characters and after a rewrite that wraps, and names its namespace.
13. Print then parse gives equal namespaces, and printing again gives the same text, with and without a theory name.
14. The printer refuses a theory name that a namespace it's given doesn't start with.
15. Under a theory name, a name that already starts with it is its full name, and prints in full when it would still start with it once the theory name is taken off.
16. A rewrite of 4 KiB as written, from its first token to its last, is accepted, and one byte more is refused, comments inside it counting. The printer refuses a rewrite that prints past 4 KiB, and writes a document of exactly 4 MiB.
17. A rewrite left open, or a missing namespace name, doesn't take the next relation's signature or the next `namespace:`.
18. Fuzzing and a property test run the Sungma schema language parser on arbitrary input without a panic, a stack overflow or an abort.
19. Mutation testing leaves no surviving mutant in the Sungma schema language parser or printer.
