---
title: span
type: definition
summary: "An interval of revisions over which a fact or a namespace version holds."
status: evolving
bounded-by: "[[revision]]"
has-a:
  - "[[fact]]"
  - "[[namespace]]"
---

An interval of revisions over which a fact or a namespace version holds.

## Examples

- `file:x#owner@carol`, written at revision 4 and retired at revision 9, holds over [4, 9).

## Rationale

A fact retired and written again holds over two spans.
