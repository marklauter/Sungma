---
title: epoch
type: definition
summary: "A revision marking a relation's latest declaration in its theory."
status: evolving
is-a: "[[revision]]"
---

A revision marking a relation's latest declaration in its theory.

## Rationale

A fact under a relation counts only if its span opened at or after the relation's epoch in the theory version a revision selects. The epoch exists so that declaring a dropped relation again doesn't resurrect the facts orphaned by the drop.
