---
title: theory
type: definition
summary: "A set of namespaces that together model authorization for one domain."
status: evolving
has-a: "[[namespace]]"
---

A set of namespaces that together model authorization for one domain.

## Examples

- A document theory: the `file` and `folder` namespaces, where a file's viewers include its folder's viewers.

## Rationale

A theory in the logician's sense: a set of sentences that, together with the facts, settles every question asked of it. It sits between namespace and catalog: a set of related namespaces. It names an idea, not a mechanism. No type holds a theory, and a check never reads one. Kingo's theory was likewise a set of namespaces.
