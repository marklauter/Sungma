---
title: orphaned-fact
type: definition
summary: "A fact under a relation dropped from its theory."
status: evolving
is-a: "[[fact]]"
---

A fact under a relation dropped from its theory.

## Rationale

Orphaning is relative to the theory version a revision selects: the same fact holds at revisions whose theory still declares the relation. An orphaned fact counts as nonexistent. It isn't deleted, and the epoch keeps it from coming back if the relation is declared again.
