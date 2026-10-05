---
title: span
type: definition
summary: "An interval of revisions over which a fact or a theory version holds."
status: evolving
bounded-by: "[[revision]]"
has-a:
  - "[[fact]]"
  - "[[theory]]"
---

An interval of revisions over which a fact or a theory version holds.

## Examples

- `file:x#owner@carol`, written at revision 4 and deleted at revision 9, holds over [4, 9).

## Rationale

A fact deleted and written again holds over two spans.
