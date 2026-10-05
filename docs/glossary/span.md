---
title: span
type: definition
summary: "An interval of revisions over which a stored fact holds."
status: evolving
bounded-by: "[[revision]]"
has-a: "[[fact]]"
---

An interval of revisions over which a stored fact holds.

## Examples

- `file:x#owner@carol`, written at revision 4 and deleted at revision 9, holds over [4, 9).

## Rationale

A span opens at the revision that writes the fact and closes at the one that deletes it, if any. A fact deleted and written again has a span for each time. Theory versions may hold over spans on the same revisions.
