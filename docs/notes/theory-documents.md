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

## Open

1. **Names from outside the theory.** Inside a theory document, an unqualified name means a namespace in the theory. How a document names a namespace outside it, especially one with no dots such as `file`, needs a rule: always the full name, or a mark for "outside this theory".
2. **Writing a theory document.** Whether it's one write per namespace, sent in turn, or one write that stores all of them atomically. One write per namespace needs nothing new. Atomic writes of any size would come with [write ids](writes-commit-through-a-write-id.md).
3. **Forms.** Whether a theory document has a JSON form as well as TOML, and how the printer writes one.
4. **Limits.** How many namespaces a theory document may hold, and how big it may be.
5. **The theory name.** Whether it must be one segment, and whether the reserved `sungma` theory is written only by releases.
