---
title: Theories are tags in Author
type: note
summary: Author lets a person group namespaces into a named theory with a tag kept in Author's own store; Sungma's model stays flat, and Check, Read and Write never read a theory.
status: evolving
cites:
  - "[[theory]]"
  - "[[namespace]]"
---

# Theories are tags in Author

## The need

The Namespaces screen lists every namespace flat. A system with many namespaces, such as Sungma authorizing its own services with one namespace per service plus one for itself, needs them grouped so a person can find and manage them together.

## The decision

A [[theory]] in Author is a tag: a name a person puts on a set of [[namespace]]s. The tags are Author's data, kept in Author's own store. Check, Read and Write never read them, and no namespace carries one.

- **The model stays flat.** A stored theory layer in Sungma would be a namespace of namespaces. The references between namespaces already imply which belong together, so a second place to declare it could only disagree with them.
- **Tags, not a naming convention.** Names like `sungma.check` would group by prefix, but a group would exist only by agreement, and moving a namespace to another group would mean renaming it. A tag changes without touching the namespace, and a namespace can carry more than one.
- **The glossary still holds.** A theory is a set of namespaces that together settle one domain's questions. Author lets a person write one down.

## On the screen

- The Namespaces screen groups its list by theory, with untagged namespaces in a section of their own.
- A person selects namespaces and tags them as a theory, or renames or removes a tag.
- A tag can outlive a namespace it names. Author shows a retired namespace as retired within its theory and doesn't drop it.

The wireframes are on the Author canvas in Claude Design: https://claude.ai/artifact/DFAXkfkCrLtvjCUx1JH2V3

## Open

1. **Where Author keeps its data:** the tags, each person's recent list and pins, and saved views. The same table under a tag of Author's own, or a store of its own.
2. **Writing a theory at once.** Sungma writes one namespace at a time, so changing two related namespaces takes two writes, with a revision between them where only one has changed. If that matters, the unit for writing several namespaces at once is a decision for Write, separate from these tags.
