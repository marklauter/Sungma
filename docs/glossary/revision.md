---
title: revision
type: definition
summary: "A position in the sequence of writes."
status: evolving
---

A position in the sequence of writes.

## Rationale

Every write takes the next revision, and every read is as of one. Facts take revisions today. Whether theory writes share the sequence is open ([[theory-writes-wait-on-theory-versioning]]). Callers hold a mark, never a bare revision.
