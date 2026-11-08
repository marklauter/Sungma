---
title: Theory documents
type: note
summary: "A theory document is one file that declares several related namespaces under a shared first segment, written once at the top as theory = \"sungma\". It's shorthand only: each namespace is stored under its full name, exactly as a namespace document would store it."
status: evolving
cites:
  - "[[theory]]"
  - "[[namespace]]"
  - "[[sungma-authorization]]"
---

# Theory documents

This note is to become a spec, `docs/specs/theory-documents.md`, once its open questions settle.

A [[theory]] is a set of [[namespace]]s that together settle one domain's questions. Sungma's own authorization model, [[sungma-authorization]], is one: `sungma.root`, `sungma.namespace`, `sungma.relation` and `sungma.group` only make sense together.

A namespace document holds one namespace. A theory document holds a theory's namespaces in one file, and names their shared first segment once:

```toml
theory = "sungma"

[root.owner]
subjects = ["identity"]

[namespace.parent]
subjects = ["root"]
```

## It's shorthand

The theory name is a prefix the document adds when it's parsed. `root` in the document means `sungma.root`, and `group#member` means `sungma.group#member`. Nothing else changes:

- Each namespace is stored under its full name, as a namespace document would store it.
- Names stay immutable, and checks, facts and storage see only full names.
- Nothing in storage records which theory a namespace came from. The theory exists in the document, and in the shared first segment of the names.

It works like a file-scoped `namespace Sungma;` declaration in C#: it saves writing the prefix, and has no effect on what is compiled.

## Proposed: Sungma schema language

JSON and TOML are fine on the wire, but a person can't reason about a theory written in them. The keys, quotes and table headers bury the rewrites, and one relation's subjects and rewrite end up on separate lines. Sungma schema language is a third form beside JSON and TOML, written for people. It parses to the same namespaces as the other two forms.

Sungma's own model, written in it:

```text
theory sungma

// The deployment, one resource: sungma.root:sungma
namespace root
  owner             [identity]
  admin             [identity]               = this | owner
  namespace_creator [identity, group#member] = this | admin
  group_creator     [identity, group#member] = this | admin
  operator          [identity, group#member] = this | admin
  auditor           [identity, group#member] = this | admin
  reader            [identity, group#member] = this | auditor
  writer            [identity, group#member] = this | admin
  fact_writer       [identity, group#member] = this | admin
  fact_reader       [identity, group#member] = this | fact_writer | auditor
  directory_reader  [identity, group#member] = this | auditor
  access_reader                              = auditor

namespace namespace
  parent        [root]
  protected     [root]
  owner         [identity, group#member] = this | (parent, owner) ! (protected, owner)
  admin         [identity, group#member] = this | owner | (parent, admin) ! (protected, admin)
  writer        [identity, group#member] = this | admin | (parent, writer)
  reader        [identity, group#member] = this | writer | fact_reader | auditor | (parent, reader)
  fact_writer   [identity, group#member] = this | admin | (parent, fact_writer)
  fact_reader   [identity, group#member] = this | fact_writer | auditor | (parent, fact_reader)
  auditor       [identity, group#member] = this | (parent, auditor)
  access_reader                          = admin | auditor

namespace relation
  parent [namespace]
  writer [identity, group#member] = this | (parent, fact_writer)
  reader [identity, group#member] = this | writer | (parent, fact_reader)

namespace group
  parent        [root]
  owner         [identity, group#member] = this | (parent, admin)
  member        [identity, group#member]
  access_reader                          = owner | (parent, auditor)
```

Each relation is one line: its name, the subjects its facts may bind in brackets, and its rewrite after `=`.

- A relation with subjects and no `=` reads `this`, as `owner [identity]` does.
- A relation with `=` and no subjects accepts no facts, as `access_reader = auditor` does.
- A relation with both follows the rules in [Accepted subjects](../specs/namespace-documents.md#accepted-subjects): its rewrite has to read `this`.
- Subjects are the accepted subjects of the namespace document spec, separated by commas, at least one, none twice.
- The rewrite is the rewrite grammar of the namespace document spec, unchanged.

```ebnf
⟨document⟩  ::= [ 'theory' ⟨namespace name⟩ ] ⟨namespace⟩ { ⟨namespace⟩ }
⟨namespace⟩ ::= 'namespace' ⟨namespace name⟩ { ⟨relation⟩ }
⟨relation⟩  ::= ⟨relation name⟩ ⟨subjects⟩ [ '=' ⟨rewrite⟩ ]
              | ⟨relation name⟩ '=' ⟨rewrite⟩
⟨subjects⟩  ::= '[' ⟨accepted subject⟩ { ',' ⟨accepted subject⟩ } ']'
```

The rules on top of the grammar:

- Whitespace, line breaks included, only separates tokens. Indentation and alignment are for readers, and a long rewrite can wrap onto as many lines as it needs.
- A rewrite ends where its grammar can't take the next token. Two terms never sit side by side without an operator, so the next relation's name ends the rewrite before it, and a relation needs no terminator.
- `namespace` followed by a name opens a namespace, and its relations run until the next one or the end of the document. A relation can still be called `namespace`, because a relation's name is followed by `[` or `=`.
- `//` starts a comment that runs to the end of the line. `#` can't start a comment, because subjectsets use it. Comments aren't kept, because Sungma stores the JSON form.
- There are no quotes, escapes, numbers or booleans. Every token is a name, a keyword or punctuation, so the parser never has to guess a value's type, and `no` or `on` is only ever a name.
- With `theory`, unqualified namespace names in the document get the theory's prefix, as [It's shorthand](#its-shorthand) says. Without it, the document holds full names, and a document with one namespace and no `theory` is a namespace document in this form.
- Errors carry a line and column in the document itself. Unlike in the JSON and TOML forms, a rewrite isn't inside a string, so its columns are the document's columns.
- The limits in [Limits](../specs/namespace-documents.md#limits) apply to each namespace the document holds.

The printer writes one relation per line, indents relations by two spaces, and pads names and subject lists so the brackets and the `=` line up within a namespace. Author shows and edits this form, and Sungma goes on storing JSON.

## Open

1. **Names from outside the theory.** Inside a theory document, an unqualified name means a namespace in the theory. How a document names a namespace outside it, especially one with no dots such as `file`, needs a rule: always the full name, or a mark for "outside this theory".
2. **Writing a theory document.** Whether it's one write per namespace, sent in turn, or one write that stores all of them atomically. One write per namespace needs nothing new. Atomic writes of any size would come with [write ids](writes-commit-through-a-write-id.md).
3. **Forms.** Whether a theory document has a JSON form as well as TOML, and how the printer writes one.
4. **Limits.** How many namespaces a theory document may hold, and how big it may be.
5. **The theory name.** Whether it must be one segment, and whether the reserved `sungma` theory is written only by releases.
6. **Printing order.** A namespace holds its relations unordered, so the printer chooses their order, and an author's order is lost when a document is printed again. Sorting by name is stable, but readers follow a theory better when a relation comes after the relations its rewrite reads, such as `owner` before `admin`. Whether the printer orders relations that way, and breaks ties by name.
