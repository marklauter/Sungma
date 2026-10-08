---
title: snapshot
type: definition
summary: "Everything whose span covers one revision."
status: locked
---

Everything whose [[span]] covers one [[revision]].

## Examples

- A theory version holds over [3, 7) and `file:x#owner@carol` over [4, 9). The snapshot at revision 4 holds both. The snapshot at 3 holds the theory version but not the fact.

## Rationale

A snapshot is taken at a revision, not at the start of a span. Things written at different revisions all appear in one snapshot when their spans cover it.
