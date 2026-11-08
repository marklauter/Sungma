---
title: Theory documents
type: specification
summary: "A theory document declares one or more namespaces in one text. Its theory name is a prefix the parser adds to unqualified namespace names, and nothing else: each namespace is checked and stored on its own, under its full name. It's written in Sungma schema language, a form for people, or in TOML."
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

```text
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

The start of the same document in TOML:

```toml
theory = "sungma"

[root.owner]
subjects = ["identity"]

[root.admin]
rewrite = "this | owner"
subjects = ["identity"]
```

## The theory name

The theory name is a prefix. The parser adds it, and a dot, to each unqualified namespace name in the document: the name in each namespace's header, and the namespace part of each accepted subject. In the sample, `root` means `sungma.root`, and `group#member` means `sungma.group#member`.

- `identity` is never prefixed. It names an [[identity]], not a namespace.
- Rewrites name only [[relation]]s, never namespaces, so the prefix never touches a [[rewrite]].
- A document without a theory name holds full names.
- The theory name follows the namespace name grammar in [Namespace documents](namespace-documents.md#names).

Once parsed, nothing remembers the prefix. Each namespace is checked, stored and read under its full name, and checks, facts and storage see only full names. Nothing records which document or theory a namespace came from. The theory shows only in the shared first segment of its namespaces' names.

## Sungma schema language

JSON and TOML suit the wire, but a person can't reason about a theory written in them. Keys, quotes and table headers bury the rewrites, and a relation's subjects and rewrite land on separate lines. Sungma schema language is the form written for people. It parses to the same namespaces as the JSON and TOML forms.

Each relation reads like a signature: its name, a colon and the [[subject]]s its facts may bind, then its rewrite after `=`.

- A relation with subjects and no `=` reads `this`, as `owner: identity` does.
- A relation with `=` and no subjects accepts no facts, as `access_reader = auditor` does.
- A relation with both follows [Accepted subjects](namespace-documents.md#accepted-subjects): its rewrite has to read `this`.
- Subjects are accepted subjects as the namespace document spec defines them, separated by commas, at least one, none twice.
- The rewrite follows the [rewrite grammar](namespace-documents.md#rewrite-grammar), unchanged.

```ebnf
⟨document⟩  ::= [ 'theory' ':' ⟨namespace name⟩ ] ⟨namespace⟩ { ⟨namespace⟩ }
⟨namespace⟩ ::= 'namespace' ':' ⟨namespace name⟩ { ⟨relation⟩ }
⟨relation⟩  ::= ⟨relation name⟩ ':' ⟨subjects⟩ [ '=' ⟨rewrite⟩ ]
              | ⟨relation name⟩ '=' ⟨rewrite⟩
⟨subjects⟩  ::= ⟨accepted subject⟩ { ',' ⟨accepted subject⟩ }
```

The rules on top of the grammar:

- Whitespace, line breaks included, only separates tokens. Indentation is for readers. A rewrite can start on its signature's line or on the next one, `=` included, and can wrap onto as many lines as it needs.
- A subject list or a rewrite ends where its grammar can't take the next token. Subjects are separated by commas, and two terms never sit side by side without an operator, so the next relation's name ends the one before it. A relation needs no terminator.
- `namespace:` opens a namespace, and its relations run until the next `namespace:` or the end of the document.
- `theory` and `namespace` are reserved relation names, in every form, as `this` is. Otherwise `namespace: root` could be a relation named `namespace` that accepts `root`. Reserving them in every form keeps every namespace printable in this one. They stay usable as namespace names, so `sungma.namespace` and `parent: namespace` are fine.
- `//` starts a comment that runs to the end of the line. `#` can't start one, because subjectsets use it. Comments aren't kept.
- There are no quotes, escapes, numbers or booleans. Every token is a name, a keyword or punctuation, so the parser never guesses a value's type, and `no` or `on` is only ever a name.
- A document is UTF-8 without a byte-order mark. Outside comments, any character the grammar doesn't use is refused.

A document with one namespace and no theory name is a namespace document in this form.

## TOML form

A theory document in TOML follows the [TOML form](namespace-documents.md#toml-form) of a namespace document, with two changes:

- It may hold one or more namespaces' tables.
- It may start with a `theory` key, a string holding the theory name.

A namespace with relations is written through its relations' tables, such as `[root.owner]`, so its own header is needed only when it has no relations. A namespace name that is still dotted after the prefix is taken off is quoted, as in `["drive.file".viewer]`.

## Checks

A theory document is refused if:

- it breaks a rule of its form;
- it declares one namespace twice, by its full name;
- any namespace in it fails a check in [Namespace documents](namespace-documents.md#checks).

A reference from one namespace to another in the same document, through an accepted subject or a fact-to-subjectset, is checked against the document. The two namespaces don't have to be written in a particular order. A reference to a namespace outside the document is checked as a namespace document's would be.

A refused document declares nothing.

## Limits

The [Limits](namespace-documents.md#limits) of a namespace document apply to each namespace a theory document holds. The document's own limits are [open](#open).

## Errors

Errors follow [Namespace documents](namespace-documents.md#errors): problems in document order, up to 50, with quotes bounded and escaped. Each error also carries the namespace it was found in, by its full name.

In Sungma schema language, an error carries a line and a column in the document itself. A rewrite isn't inside a string, so its columns are the document's columns. In TOML, an error inside a rewrite carries the column in the rewrite, as in a namespace document.

## Printing

The printer writes a set of namespaces as a theory document that the parser reads back as equal namespaces.

- Given a theory name, the printer takes it off every namespace name that starts with it and a dot, and writes it once at the top. Every namespace it's given has to start with it.
- Namespaces print sorted by full name, comparing bytes. Within a namespace, relations print sorted by name, comparing bytes, as in a namespace document. Alphabetical order is the easiest for readers to find a relation in. An author's own order and line breaks are lost when a document is printed again.
- In Sungma schema language, each relation prints on one line, indented two spaces, with one space after each colon and comma and around `=` and each operator, and no padding to line up columns. A blank line comes before each namespace. Subjects print sorted, comparing bytes, and parentheses print only where the grammar needs them, as in a namespace document.
- In TOML, the printer writes the `theory` key first, then each relation as a table of its own, as a namespace document's printer does.

Author, Sungma's UI, shows and edits namespaces in Sungma schema language. Sungma stores each namespace in the JSON form of a namespace document.

## Open

1. **Names from outside the theory.** With a theory name, an unqualified name means a namespace in the theory. How a document names a namespace outside it, especially one with no dots such as `file`, needs a rule: always the full name, or a mark for "outside this theory". Until then, a dotted name after a theory name is ambiguous: `drive.file` could be `sungma.drive.file` or `drive.file`.
2. **Writing a theory document.** Whether it's one write per namespace, sent in turn, or one write that stores all of them atomically. One write per namespace needs nothing new. Atomic writes of any size would come with [write ids](../notes/writes-commit-through-a-write-id.md).
3. **A JSON form.** Whether a theory document has a JSON form, and how the printer writes one.
4. **Limits.** How many namespaces a theory document may hold, and how big it may be.
5. **The theory name.** Whether it must be one segment, and whether only Sungma releases write documents in the reserved `sungma` theory.
6. **A namespace named `theory` in TOML.** A top-level `theory` key and a namespace whose unqualified name is `theory` would collide, and TOML refuses a key written twice.

## Required tests

The parser takes untrusted input in a security system, so its tests cover hostile input as well as valid documents. The classes in [Namespace documents](namespace-documents.md#required-tests) apply to Sungma schema language and to the TOML form of a theory document. These cases are required on top of them:

1. The sample parses to the four namespaces of [Sungma's own authorization](sungma-authorization.md#the-model), under their full names.
2. The sample in Sungma schema language and in TOML parse to equal namespaces.
3. The theory name prefixes namespace headers and the namespace part of accepted subjects, and never `identity` or anything in a rewrite.
4. A document without a theory name holds full names, and one with a single namespace parses to the same namespace as its namespace document.
5. A rewrite on its signature's line, on the next line, and wrapped over several lines parse to equal namespaces.
6. A relation with subjects and no `=` reads `this`, and a relation with `=` and no subjects accepts no facts.
7. `theory` and `namespace` are refused as relation names, in every form, and allowed as namespace names and in accepted subjects.
8. Comments are allowed at the end of any line and change nothing. A `#` outside an accepted subject is refused.
9. A quote, an escape, a number, a byte-order mark, and any character outside the grammar are refused.
10. A namespace declared twice, by an unqualified name and by its full name, is refused.
11. A reference between two namespaces in one document is checked against the document, in either order.
12. A document with one bad namespace declares none of its namespaces.
13. An error in Sungma schema language carries the document's line and column, after multibyte characters and after a rewrite that wraps, and names its namespace.
14. Print then parse gives equal namespaces, and printing again gives the same text, in each form, with and without a theory name.
15. The printer refuses a theory name that a namespace it's given doesn't start with.
16. Fuzzing and a property test run the Sungma schema language parser on arbitrary input without a panic, a stack overflow or an abort.
17. Mutation testing leaves no surviving mutant in the Sungma schema language parser or printer.
